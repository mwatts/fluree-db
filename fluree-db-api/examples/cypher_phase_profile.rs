//! Per-phase Cypher cost profiler: parse vs AST-clone vs param-substitution
//! vs lowering vs full round trip.
//!
//! Built to answer one question from `docs/design/bolt-adapter.md`: where does
//! the per-request engine-side time go for benchmark-shaped statements, and
//! therefore is a parsed-AST cache (text -> Arc<CypherAst>, clone + substitute
//! per request) enough, or does lowered IR need caching too (which would need
//! snapshot-keyed invalidation)?
//!
//! Phases measured per statement, all in-process (no HTTP):
//!   parse   - `fluree_db_cypher::parse_cypher` (text -> AST; what the cache skips)
//!   clone   - `CypherAst::clone` (per-request cost *under* the cache)
//!   subst   - `substitute_params` on the clone
//!   lower   - `lower_cypher_with_context` against a real ledger snapshot
//!   e2e     - `Fluree::query_cypher_with_params` (parse..execute, full result)
//!
//! plan+exec is derived: e2e - (parse + clone + subst + lower).
//!
//! ## Config (env vars)
//! | Var | Default | Meaning |
//! |---|---|---|
//! | `PROF_USERS` | `2000` | users in the synthetic graph |
//! | `PROF_EDGES_PER_USER` | `10` | `knows` edges per user |
//! | `PROF_ITERS` | `500` | timed iterations per phase |
//! | `PROF_WARMUP` | `50` | untimed warmup iterations |
//!
//! ## Run
//! ```bash
//! cargo run --release --example cypher_phase_profile -p fluree-db-api
//! ```

use std::time::Instant;

use fluree_db_api::{Fluree, FlureeBuilder};
use fluree_db_cypher::{
    lower_cypher_with_context, parse_cypher, substitute_params, LoweringContext, ParamMap,
};
use fluree_db_query::var_registry::VarRegistry;
use serde_json::{json, Value as JsonValue};

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

struct Stmt {
    name: &'static str,
    text: &'static str,
    params: ParamMap,
}

fn statements() -> Vec<Stmt> {
    let with_id = |v: JsonValue| -> ParamMap {
        let mut m = ParamMap::new();
        m.insert("id".into(), v);
        m
    };
    vec![
        Stmt {
            name: "expand_1",
            text: "MATCH (n:User {id: $id})-[:knows]->(m) RETURN m.id",
            params: with_id(json!(42)),
        },
        Stmt {
            name: "expand_2_agg",
            text: "MATCH (n:User {id: $id})-[:knows]->()-[:knows]->(m) \
                   RETURN count(DISTINCT m) AS c",
            params: with_id(json!(42)),
        },
        Stmt {
            name: "class_count",
            text: "MATCH (n:User) RETURN count(n) AS c",
            params: ParamMap::new(),
        },
        Stmt {
            name: "filter_topk",
            text: "MATCH (n:User) WHERE n.age > $id \
                   RETURN n.id AS id, n.age AS age ORDER BY age DESC LIMIT 10",
            params: with_id(json!(30)),
        },
        Stmt {
            name: "long_statement",
            text: "MATCH (a:User {id: $id})-[:knows]->(b:User) \
                   WHERE b.age > 18 AND b.age < 65 AND b.name <> a.name \
                   WITH b, count(*) AS paths \
                   MATCH (b)-[:knows]->(c:User) \
                   WHERE c.age >= b.age OR c.id < 100 \
                   RETURN b.id AS id, paths, count(DISTINCT c) AS fanout \
                   ORDER BY fanout DESC, id ASC LIMIT 25",
            params: with_id(json!(42)),
        },
    ]
}

fn build_dataset(users: usize, edges_per_user: usize) -> JsonValue {
    let names = ["ana", "bo", "cy", "dee", "ed", "fay", "gus", "hal"];
    let graph: Vec<JsonValue> = (0..users)
        .map(|i| {
            let knows: Vec<JsonValue> = (1..=edges_per_user)
                .map(|k| json!({ "@id": format!("ex:u{}", (i + k * 7) % users) }))
                .collect();
            json!({
                "@id": format!("ex:u{i}"),
                "@type": "ex:User",
                "ex:id": i,
                "ex:name": format!("{}{}", names[i % names.len()], i),
                "ex:age": 18 + (i % 60),
                "ex:knows": knows,
            })
        })
        .collect();
    json!({
        "@context": { "ex": "http://example.org/" },
        "@graph": graph,
    })
}

#[derive(Default, Clone)]
struct PhaseUs {
    parse: f64,
    clone: f64,
    subst: f64,
    lower: f64,
    e2e: f64,
}

fn time_us<R>(iters: usize, warmup: usize, mut f: impl FnMut() -> R) -> f64 {
    for _ in 0..warmup {
        std::hint::black_box(f());
    }
    let start = Instant::now();
    for _ in 0..iters {
        std::hint::black_box(f());
    }
    start.elapsed().as_secs_f64() * 1e6 / iters as f64
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let users = env_usize("PROF_USERS", 2000);
    let edges = env_usize("PROF_EDGES_PER_USER", 10);
    let iters = env_usize("PROF_ITERS", 500);
    let warmup = env_usize("PROF_WARMUP", 50);

    let fluree: Fluree = FlureeBuilder::memory().build_memory();
    let ledger = fluree
        .create_ledger("cypherprof:main")
        .await
        .expect("create ledger");
    let dataset = build_dataset(users, edges);
    let res = fluree.insert(ledger, &dataset).await.expect("insert");
    eprintln!(
        "dataset: {} users, {} edges/user, {} flakes committed",
        users, edges, res.receipt.flake_count
    );

    let db = fluree.db("cypherprof:main").await.expect("db view");

    println!(
        "\n{:<16} {:>10} {:>10} {:>10} {:>10} {:>10} {:>12}",
        "statement", "parse_us", "clone_us", "subst_us", "lower_us", "e2e_us", "plan+exec_us"
    );
    for stmt in statements() {
        let mut p = PhaseUs::default();

        p.parse = time_us(iters, warmup, || parse_cypher(stmt.text));

        let ast = parse_cypher(stmt.text).ast.expect("ast");
        p.clone = time_us(iters, warmup, || ast.clone());

        p.subst = time_us(iters, warmup, || {
            let mut c = ast.clone();
            substitute_params(&mut c, &stmt.params).expect("subst");
            c
        }) - p.clone;

        let mut substituted = ast.clone();
        substitute_params(&mut substituted, &stmt.params).expect("subst");
        p.lower = time_us(iters, warmup, || {
            let mut vars = VarRegistry::new();
            let mut ctx =
                LoweringContext::new(&*db.snapshot, &mut vars).with_vocab("http://example.org/");
            lower_cypher_with_context(&substituted, &mut ctx).expect("lower")
        });

        // Full round trip, sequential requests (single client).
        let e2e_start = Instant::now();
        let e2e_iters = iters.min(200);
        for _ in 0..warmup.min(20) {
            fluree
                .query_cypher_with_params(&db, stmt.text, Some(&stmt.params))
                .await
                .expect("query");
        }
        let t0 = Instant::now();
        for _ in 0..e2e_iters {
            std::hint::black_box(
                fluree
                    .query_cypher_with_params(&db, stmt.text, Some(&stmt.params))
                    .await
                    .expect("query"),
            );
        }
        p.e2e = t0.elapsed().as_secs_f64() * 1e6 / e2e_iters as f64;
        let _ = e2e_start;

        let plan_exec = p.e2e - p.parse - p.subst - p.lower;
        println!(
            "{:<16} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>10.1} {:>12.1}",
            stmt.name, p.parse, p.clone, p.subst, p.lower, p.e2e, plan_exec
        );
    }

    println!(
        "\nInterpretation: an AST cache turns `parse` into `clone` per request; \
         a lowered-IR cache would additionally remove `subst`+`lower` but needs \
         snapshot-keyed invalidation."
    );
}
