# Bolt protocol adapter — implementation hand-off

Status: **v1 implemented** (branch `feature/bolt`, 2026-07). Steps 1 and 2
of the sequencing below landed:

- **Parsed-AST cache** — process-wide LRU in `fluree-db-api`
  (`FLUREE_CYPHER_AST_CACHE`, default 512 entries); all five parse sites
  route through `query/helpers.rs::parse_cypher_ast_cached`. The
  parse/lower split profiler is `fluree-db-api/examples/cypher_phase_profile.rs`.
  **Measured split (release, benchmark-shaped statements)**: parse 1–4 µs,
  AST clone 0.2–1 µs, substitute ≈0, lower 1–4 µs — engine-side per-request
  parse+lower tax is single-digit µs, far below this doc's ~50–100 µs
  estimate. Consequence: a lowered-IR cache is **not warranted** (its
  snapshot-keyed invalidation would buy back almost nothing); the AST cache
  stands on eliminating per-request work at zero risk, not on wall-clock.
  plan+exec dominates everything (the profiler's unindexed novelty-path
  numbers are ms-scale and not comparable to the indexed-server figures
  below).
- **`fluree-db-bolt` crate** — pure codec: PackStream, chunking,
  handshake (4.4 + 5.0–5.4), typed messages, autocommit session state
  machine. No Fluree deps; byte-fixture unit tests.
- **Server wiring** — `fluree-db-server` feature `bolt` (default off):
  `--bolt-listen-addr` / `--bolt-default-db` (+ `[server.bolt]` TOML),
  listener + per-connection tasks in `fluree-db-server/src/bolt.rs`, reads
  via `query_cypher_with_params` + `QueryResult::to_cypher_table` (the
  pre-flattening shared converter factored from cypher-json), writes via
  the consensus submit path with write-RETURN support. End-to-end tests:
  `fluree-db-server/tests/bolt_integration.rs`; official-driver smoke:
  `tests/bolt_driver_smoke.py`.
- Step 3 (driver-matrix polish, temporal/graph PackStream structures,
  auth, explicit transactions) remains open — see the support matrix's
  Bolt section for the current transport semantics.

The remainder of this document is the original design hand-off, kept for
the anchor points and performance expectations worked out during the
benchgraph/Pokec optimization effort (branch `fix/cypher-benchgraph-gaps`,
2026-07). Read alongside [Cypher (concept)](../query/cypher.md) and the
[openCypher support matrix](../reference/cypher-support-matrix.md).

## What and why

Bolt is Neo4j's versioned binary protocol (TCP handshake + PackStream
serialization + a session message state machine). Implementing the server
side gives Fluree two things:

1. **Ecosystem access** — every official Neo4j driver (Python, Java, JS,
   Go, .NET) and most Neo4j-compatible tooling can connect to Fluree's
   openCypher surface unmodified.
2. **Per-request floor parity** — published graph-database benchmarks
   (memgraph/benchgraph and similar) measure Bolt round trips over
   persistent sessions. Our HTTP numbers carry per-request connection and
   envelope costs the vendors' numbers do not. A Bolt session erases that
   gap and makes 12-worker-concurrency comparisons apples-to-apples.

It does **not** speed up engine-bound queries; it removes wire and
per-request overhead only. See [Performance expectations](#performance-expectations).

## Sequencing recommendation

Do these in order — the first is engine work that pays off regardless of
protocol and is a prerequisite for Bolt hitting its floor:

1. **Parsed-AST / lowered-IR cache** (no protocol work; see
   [Plan cache](#the-plan-cache-do-this-first)).
2. **Bolt v1**: autocommit only, versions 4.4 + 5.x, the value mappings we
   already have. ~1–2 weeks.
3. **Driver-matrix polish + explicit transactions**: open-ended tail;
   schedule separately.

## Scope

### v1 (benchmark- and driver-smoke-grade)

- Handshake: magic `0x6060B017`, 4-slot version negotiation. Offer 5.x
  (pick one minor, e.g. 5.4) and 4.4 — that covers current official
  drivers and most third-party clients.
- Messages: `HELLO` (+ `LOGON` for 5.1+), `RUN`, `PULL {n}` (reactive
  batching with `has_more`), `DISCARD`, `RESET`, `GOODBYE`. Autocommit
  only: `BEGIN` answers a clear `FAILURE`
  (`Neo.ClientError.Statement.TypeError`-family code with a "explicit
  transactions not supported" message), never a silent wrong behavior —
  same contract as the Cypher support matrix.
- Auth: `none` scheme when the server runs open; `basic` deferred unless
  trivially mappable to the existing credential machinery. Do not invent a
  new identity path for v1.
- One database per session: Bolt's `db` metadata field (in `HELLO`
  defaults + per-`RUN` extra) maps to a ledger id (`pokec:main`). Missing
  `db` → a configurable default ledger.
- Result streaming: `SUCCESS {fields: [...]}` → `RECORD` per row →
  `SUCCESS {type, t_first, t_last, db}` summary. Write queries surface the
  commit receipt counters in the summary metadata (`stats` map).

### Explicitly deferred

- Explicit transactions (`BEGIN`/`COMMIT`/`ROLLBACK`). This is the
  genuinely hard part: an interactive multi-round-trip transaction needs
  session-pinned staged state held across messages, reconciled with the
  consensus commit path (`resolve_cypher_under_lock` runs under the ledger
  lock; see the commit-conflict/refresh machinery in
  `fluree-db-api/src/lib.rs` and `fluree-db-consensus/src/local.rs`). Do
  not attempt in v1.
- `ROUTE` / routing tables (cluster-aware drivers fall back fine for a
  single server; answer `ROUTE` with a single-entry table if a driver
  insists).
- TLS (front with a proxy if needed initially).

## Where it goes

```
Layer 5   fluree-db-server ──uses──► fluree-db-bolt (new crate)
```

- **New crate `fluree-db-bolt`** (Layer 5 sibling): PackStream
  encode/decode, message framing (chunked transport: u16-length chunks,
  0x0000 terminator), version negotiation, and the session state machine
  (`READY` / `STREAMING` / `FAILED` / `INTERRUPTED`). No Fluree
  dependencies beyond `fluree-db-api` types at the edge — keep the codec
  pure so it unit-tests against captured byte fixtures.
  - Evaluate vendoring/depending on the `bolt-proto` crate (message +
    value types through Bolt 4.x; used by `bb8-bolt`) and the PackStream
    implementation inside `neo4rs`. Expect to write the 5.x deltas
    (element ids, `LOGON` split) ourselves either way.
- **Wiring in `fluree-db-server`**, feature-gated `bolt` (default off
  initially):
  - Config: `bolt_listen_addr: Option<SocketAddr>` (conventional port
    7687) in `fluree-db-server/src/config.rs` + the config file schema +
    `docs/operations/configuration.md`.
  - Listener: `Server::run()` in `fluree-db-server/src/lib.rs` (~line
    239) already binds the HTTP `TcpListener` and spawns background
    tasks; bind the Bolt listener alongside and spawn one tokio task per
    connection. Each connection holds an `Arc<AppState>` — the same
    ledger-manager, consensus, and full-scan configuration the HTTP
    routes use.

### Execution glue — reuse these, do not fork them

| Concern | Existing entry point |
|---|---|
| Read query with params | `Fluree::query_cypher_with_params` (`fluree-db-api/src/view/query.rs:161`) → `parse_cypher_to_ir` (`fluree-db-api/src/query/helpers.rs:108`) |
| Ledger view resolution | `fluree.db(ledger_id)` (`fluree-db-api/src/view/fluree_ext.rs:525`) — cheap snapshot clone, read-scaling already handled |
| Write (autocommit `RUN` of a write statement) | the same path `execute_cypher_transact` uses in `fluree-db-server/src/routes/transact.rs` (~line 810): `plan_write_return_source` → `TxnOpts { skolem_txn_id }` → `submit_via_consensus` → `wait_for_committed_state` → `write_return_rows` for `CREATE … RETURN` |
| Read/write classification | same statement sniffing the HTTP route uses (write clauses present → transact path) |
| Bare `MATCH (n)` gating | `cypher_full_scan_enabled()` (`fluree-db-api/src/query/helpers.rs`) — process-wide `FLUREE_CYPHER_ALLOW_FULL_SCAN`, read once; Bolt sessions inherit it automatically |
| Plan inspection while developing | `fluree_db_api::explain::explain_cypher` — added during this effort precisely because Cypher had no explain surface and plan-shape defects were invisible |

### Result mapping — mostly already built

The cypher-json formatter (`QueryResult::to_cypher_json{,_async}` in
`fluree-db-api/src/query/mod.rs:245,432`) already implements the
Neo4j-compatible **semantic** mapping (native scalars, date rendering,
list/map values, novelty-aware IRI resolution). Bolt needs the same
mapping targeting PackStream values instead of JSON. Factor the
per-binding conversion so both formatters share it rather than
copy-pasting.

| Fluree value | Bolt / PackStream | Notes |
|---|---|---|
| IRI / node (`RETURN n`) | `Node { id, element_ids, labels, properties }` | `elementId` = IRI string. `id` (int64, required pre-5.x) = encoded `s_id` (u64 → i64; document the cast). Labels via the existing `labels()` machinery — it already hides the `db:Node` marker. Properties need a per-node property fetch at format time — same cost profile as cypher-json's node rendering. |
| Relationship value (`Binding::Rel { start, predicate, end, reifier }`) | `Relationship { id, start, end, type, properties }` | The Cypher relationship value model landed for cypher-json; `type()` / `startNode()` / `endNode()` / `properties()` already resolve. `id` = reifier's encoded s_id when reified, else synthesize (e.g. hash) — pre-5.x drivers only need it to be stable within a result. |
| `Binding::Path { nodes, preds }` | `Path { nodes, rels, indices }` | Same source data as `nodes()` / `relationships()` / `pathPairs()`. |
| xsd:integer/long | Integer | |
| xsd:double/float | Float | |
| **xsd:decimal** | Float (document the precision loss) | cypher-json renders decimals as strings for precision; PackStream has no decimal type and Neo4j returns Float. Integer division produces xsd:decimal in our engine (see `docs/query/cypher.md`), so this shows up on ordinary `a / b`. Decide once, write it in the matrix. |
| xsd:date / dateTime | Bolt 5 `Date` / `DateTime` structures (4.4: legacy structs) | cypher-json emits ISO strings; Bolt drivers expect the structures. |
| unbound | Null | |

## The plan cache (do this first)

The remaining engine-side per-request cost is parse → lower → plan,
re-done for every request. Two facts make caching easy to get wrong today
and easy to get right with a small change:

- `parse_cypher_to_ir` substitutes `$params` **into the AST before
  lowering** (`fluree-db-api/src/query/helpers.rs:130-136`). A cache keyed
  on statement text must therefore cache the **pre-substitution parsed
  AST** (text-only key, ledger-independent, immutable — clone per request,
  then substitute). This is exactly the split Bolt's `RUN {text, params}`
  model assumes, and our HTTP params envelope already delivers.
- Lowering depends on the snapshot (IRI → SID encoding added in
  `ee7285380`, vocab/context) — caching *lowered IR* needs an
  invalidation key tied to the ledger's namespace/dict state. Measure
  before building it: if parse dominates lowering (likely for benchmark
  statements), the AST cache alone captures most of the win at zero
  invalidation risk.

Suggested v1: a bounded LRU (`text → Arc<CypherAst>`) in
`fluree-db-api`, consulted by `parse_cypher_to_ir`. Benchmark the
parse/lower/plan split first (a `pattern_short` flamegraph or coarse
timers) so the follow-up decision is data-driven.

## Performance expectations

Baseline measurements from this effort (pokec small = 10k nodes/122k
edges, single-client, per-request `curl`, warm, Apple-silicon dev box —
the r7a.4xlarge numbers scale similarly):

| Component | Evidence |
|---|---|
| Full HTTP round trip, `pattern_short` | ~0.87–0.92 ms |
| Same-shape work in-process (parse+plan+exec) | ~90–300 µs |
| ⇒ transport + curl process/connect + HTTP/JSON envelope | **~0.4–0.7 ms** |
| Vendors' published p50 (Bolt, persistent sessions, 12 workers) | Neo4j 0.22–0.23 ms, Memgraph 0.15–0.16 ms |

Expected after Bolt v1 (persistent session, PackStream, no HTTP):

- **Sub-ms micro queries (12 of 35 in benchgraph): ~0.6–0.9 ms → ~0.2–0.4 ms**
  client-observed. Floor ≈ execution (50–300 µs) + protocol (~50 µs) +
  parse/plan (~50–100 µs, → ~10–20 µs with the AST cache).
- **Writes: ~1.2–1.6 ms → ~1.0–1.4 ms.** Commit dominates; wire savings
  only.
- **Engine-bound queries (expansion_3/4, filtered scans): unchanged.**
- **Concurrency runs become honest**: the 12-worker comparison stops
  penalizing us for per-request connections. The engine side already
  scales (snapshot reads are `Arc` + `RwLock`; see
  `fluree-db-api/tests/it_scaling_bench.rs`).

Cheap partial alternative: a benchmark runner holding persistent HTTP
connections (or any keep-alive client) captures roughly the transport
half of the win with no protocol work — worth doing for fair numbers even
before Bolt lands.

## Testing

- **Codec**: byte-fixture unit tests in `fluree-db-bolt` (captured frames
  from a real Neo4j/driver exchange are the fastest way to get these).
- **Integration**: `neo4rs` (Rust Bolt client) as a dev-dependency
  driving a spawned server — mirror `fluree-db-server/tests/cypher_http_integration.rs`
  (read, write with `RETURN`, params, error surfaces, `RESET` mid-stream).
- **Driver smoke**: a small Python script with the official `neo4j`
  driver, run manually or behind an ignored test — official drivers are
  the compatibility bar, and they probe things ad-hoc clients don't
  (`server` version string in `SUCCESS` of `HELLO`, `qid` handling,
  summary metadata shapes).
- **Benchmark protocol** (hard-won lessons; see also
  `docs/troubleshooting/performance-tracing.md`):
  - Always re-import fresh and restart the server before trusting
    numbers. Two separate investigations in this effort were poisoned by
    a stale server process running an older binary, and one "regression"
    (create__edge 1.2→2.4 ms) was entirely accumulated-novelty ledger
    state, not code.
  - The whole-graph/class aggregate folds require a clean HEAD (no
    novelty) and — for the bare `MATCH (n)` anchor — a graph without
    `f:reifies*` facts; benchmark write passes create both conditions'
    violations, so aggregate timings taken mid-suite measure the
    fallback pipeline.
  - `FLUREE_CYPHER_ALLOW_FULL_SCAN=1` is required for the two bare
    `MATCH (n)` aggregation queries and is read once per process.

## Effort estimate

| Piece | Estimate |
|---|---|
| AST cache + parse/lower/plan measurement | 1–2 days |
| PackStream + framing + handshake + state machine | 3–5 days |
| Execution glue + result mapping (sharing the cypher-json converter) | 3–4 days |
| Integration tests + driver smoke + config/docs | 2–3 days |
| **v1 total** | **~2 weeks** |
| Driver-matrix polish (5 official drivers' quirks, `ROUTE`, auth schemes) | +1–2 weeks, schedule separately |
| Explicit transactions | Design first; touches consensus locking — not a protocol task |

## Open questions

1. **xsd:decimal → Float**: accept Neo4j-parity precision loss on Bolt
   while cypher-json keeps strings? (Recommended: yes, note it in the
   support matrix.)
2. **Node property eagerness**: Bolt `Node` carries full properties;
   `RETURN n` over large result sets pays a per-node property fetch.
   Mirror whatever cypher-json does today; revisit only if it shows up.
3. **Auth**: is `none`-scheme-when-open acceptable for v1, with `basic`
   mapped to the existing credential/OIDC machinery later?
4. **Numeric `id` stability**: encoded s_id is stable per index build but
   can change across reindex. Fine for driver compatibility (ids are
   opaque handles within a session); document that `elementId` (the IRI)
   is the durable identity.
