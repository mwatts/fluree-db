//! vbench — corpus benchmark runner for native-vs-virtual SPARQL execution.
//!
//! Subcommands:
//! - `setup --verify` — open each target and run a trivial probe (and, for a
//!   native target with a known triple count, assert schema stability).
//! - `run` — for each query × target, run a priming rep + measured reps and
//!   stream [`schema::RunRecord`]s to a `run.jsonl`.
//! - `exec-one` — a single execution to stdout (the cold-mode hook).
//! - `report` — render a `run.jsonl` as a native-vs-virtual comparison table.
//!
//! See `README.md` for the run protocol, corpus conventions, and caveats.

mod canon;
mod corpus;
mod exec;
mod report;
mod schema;
mod spans;
mod targets;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::corpus::Corpus;
use crate::exec::{Engine, RunParams};
use crate::schema::{Line, RunMeta, RunRecord, Status, TargetFingerprint, SCHEMA_VERSION};
use crate::targets::{Target, TargetKind};

const DEFAULT_NATIVE_REPS: usize = 5;
const DEFAULT_VIRTUAL_REPS: usize = 3;

/// Probe: count a small dimension class (works on native and virtual alike).
const PROBE_CLASS: &str =
    "PREFIX edw: <http://ns.fluree.dev/edw#> SELECT (COUNT(*) AS ?n) WHERE { ?s a edw:Store }";
/// Probe: total triple count, for the native schema-stability assertion.
const PROBE_TOTAL: &str = "SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }";

#[derive(Parser)]
#[command(name = "vbench", about = "Native-vs-virtual SPARQL corpus benchmark runner")]
struct Cli {
    /// Corpus directory (default: <crate>/corpus).
    #[arg(long, global = true)]
    corpus_dir: Option<PathBuf>,
    /// Targets directory (default: <crate>/targets).
    #[arg(long, global = true)]
    targets_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Open each target and run a probe; assert triple count for native targets.
    Setup {
        /// Run the verification probe (the only mode today; kept explicit).
        #[arg(long)]
        verify: bool,
        /// Targets to verify (comma-separated). Default: native-sf01.
        #[arg(long, value_delimiter = ',', default_value = "native-sf01")]
        targets: Vec<String>,
    },
    /// Run the corpus against one or more targets, writing a run.jsonl.
    Run {
        /// Targets (comma-separated), e.g. native-sf01,virtual-sf20.
        #[arg(long, value_delimiter = ',', required = true)]
        targets: Vec<String>,
        /// Restrict to a subset (e.g. smoke).
        #[arg(long)]
        subset: Option<String>,
        /// Output run.jsonl path (default: <crate>/results/runs/run-<ts>.jsonl).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Retain the first 20 canonical rows per record.
        #[arg(long)]
        keep_heads: bool,
        /// Measured reps for native targets.
        #[arg(long, default_value_t = DEFAULT_NATIVE_REPS)]
        native_reps: usize,
        /// Measured reps for virtual targets.
        #[arg(long, default_value_t = DEFAULT_VIRTUAL_REPS)]
        virtual_reps: usize,
    },
    /// Run a single query against a single target; print one RunRecord as JSON.
    ExecOne {
        #[arg(long)]
        query: String,
        #[arg(long)]
        target: String,
        #[arg(long)]
        keep_heads: bool,
    },
    /// Render a run.jsonl as a comparison table (or --json).
    Report {
        #[arg(long)]
        run: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let corpus_dir = cli
        .corpus_dir
        .clone()
        .unwrap_or_else(|| crate_dir().join("corpus"));
    let targets_dir = cli
        .targets_dir
        .clone()
        .unwrap_or_else(|| crate_dir().join("targets"));

    match cli.command {
        Command::Setup { verify: _, targets } => cmd_setup(&targets_dir, &targets),
        Command::Run {
            targets,
            subset,
            out,
            keep_heads,
            native_reps,
            virtual_reps,
        } => cmd_run(
            &corpus_dir,
            &targets_dir,
            &targets,
            subset.as_deref(),
            out,
            keep_heads,
            native_reps,
            virtual_reps,
        ),
        Command::ExecOne {
            query,
            target,
            keep_heads,
        } => cmd_exec_one(&corpus_dir, &targets_dir, &query, &target, keep_heads),
        Command::Report { run, json } => cmd_report(&run, json),
    }
}

/// The crate root (used for default corpus/targets/results locations).
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn cmd_setup(targets_dir: &Path, target_ids: &[String]) -> Result<()> {
    let engine = Engine::new()?;
    let mut all_ok = true;
    for id in target_ids {
        let target = Target::load(targets_dir, id)?;
        print!("target {id} ({}) ... ", target.kind_str());
        std::io::stdout().flush().ok();
        if let Err(e) = target.ensure_runnable() {
            println!("SKIP: {e}");
            continue;
        }
        let fluree = match engine.open(&target) {
            Ok(f) => f,
            Err(e) => {
                println!("FAIL (open): {e}");
                all_ok = false;
                continue;
            }
        };
        match engine.probe(&fluree, &target, PROBE_CLASS, Duration::from_secs(120)) {
            Ok((wall, doc)) => {
                let n = exec::scalar_count(&doc).unwrap_or(0);
                println!("ok (Store count = {n}, {} ms)", wall.as_millis());
            }
            Err(e) => {
                println!("FAIL (probe): {e}");
                all_ok = false;
                continue;
            }
        }

        if let Some(expected) = target.expected_total_triples {
            print!("  total-triples assertion ... ");
            std::io::stdout().flush().ok();
            match engine.probe(&fluree, &target, PROBE_TOTAL, Duration::from_secs(600)) {
                Ok((wall, doc)) => {
                    let actual = exec::scalar_count(&doc).unwrap_or(0);
                    if actual == expected {
                        println!("ok ({actual} triples, {} ms)", wall.as_millis());
                    } else {
                        println!("MISMATCH: expected {expected}, got {actual}");
                        all_ok = false;
                    }
                }
                Err(e) => {
                    println!("FAIL: {e}");
                    all_ok = false;
                }
            }
        }
    }
    if all_ok {
        Ok(())
    } else {
        anyhow::bail!("one or more targets failed verification")
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_run(
    corpus_dir: &Path,
    targets_dir: &Path,
    target_ids: &[String],
    subset: Option<&str>,
    out: Option<PathBuf>,
    keep_heads: bool,
    native_reps: usize,
    virtual_reps: usize,
) -> Result<()> {
    let corpus = Corpus::load(corpus_dir)?;
    let queries = corpus.select(subset);
    if queries.is_empty() {
        anyhow::bail!("no queries match subset {:?}", subset);
    }

    // Resolve + validate every target up front (fail fast on a pending target).
    let mut resolved = Vec::new();
    for id in target_ids {
        let target = Target::load(targets_dir, id)?;
        target.ensure_runnable()?;
        resolved.push(target);
    }

    let engine = Engine::new()?;
    let prov = provenance();
    let meta = RunMeta {
        schema_version: SCHEMA_VERSION,
        run_id: prov.run_id.clone(),
        timestamp: prov.timestamp.clone(),
        git_commit: prov.git_commit.clone(),
        git_dirty: prov.git_dirty,
        build_profile: prov.build_profile.clone(),
        host: prov.host.clone(),
        runtime: engine.runtime_shape(),
        subset: subset.map(str::to_string),
        targets: resolved.iter().map(target_fingerprint).collect(),
    };

    let out_path = out.unwrap_or_else(|| {
        crate_dir()
            .join("results")
            .join("runs")
            .join(format!("run-{}-{}.jsonl", prov.file_stamp, prov.run_id))
    });
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating results dir {}", parent.display()))?;
    }
    let mut writer = RunWriter::create(&out_path)?;
    writer.write(&Line::Meta(meta))?;

    eprintln!(
        "vbench run {} -> {}  ({} queries x {} targets)",
        prov.run_id,
        out_path.display(),
        queries.len(),
        resolved.len()
    );

    for target in &resolved {
        eprintln!("== target {} ({}) ==", target.id, target.kind_str());
        let fluree = engine.open(target)?;
        let reps = match target.kind {
            TargetKind::Native => native_reps,
            TargetKind::Virtual => virtual_reps,
        };
        for q in &queries {
            let sparql = corpus.read_query(q)?;
            let params = RunParams {
                timeout: Duration::from_secs(q.timeout_s),
                reps,
                keep_heads,
            };
            let record = engine.run_query(&fluree, target, &q.id, &sparql, &params);
            log_progress(q, &record);
            check_expectations(q, &record);
            writer.write(&Line::Record(record))?;
        }
    }

    eprintln!("done -> {}", out_path.display());
    Ok(())
}

fn cmd_exec_one(
    corpus_dir: &Path,
    targets_dir: &Path,
    query_id: &str,
    target_id: &str,
    keep_heads: bool,
) -> Result<()> {
    let corpus = Corpus::load(corpus_dir)?;
    let q = corpus
        .get(query_id)
        .with_context(|| format!("no query '{query_id}' in corpus"))?;
    let target = Target::load(targets_dir, target_id)?;
    target.ensure_runnable()?;
    let engine = Engine::new()?;
    let fluree = engine.open(&target)?;
    let sparql = corpus.read_query(q)?;
    let record = engine.exec_one(
        &fluree,
        &target,
        &q.id,
        &sparql,
        Duration::from_secs(q.timeout_s),
        keep_heads,
    );
    println!("{}", serde_json::to_string_pretty(&record)?);
    Ok(())
}

fn cmd_report(run: &Path, json: bool) -> Result<()> {
    let (meta, records) = report::read_run(run)?;
    if json {
        report::print_json(&meta, &records)
    } else {
        report::print_table(&meta, &records);
        Ok(())
    }
}

/// One-line stderr progress for a completed record.
fn log_progress(q: &corpus::QueryDef, record: &RunRecord) {
    let status = match record.status {
        Status::Ok => "ok",
        Status::Dnf => "DNF",
        Status::Error => "ERR",
    };
    let missing = if record.spans_missing.is_empty() {
        String::new()
    } else {
        format!("  spans_missing={:?}", record.spans_missing)
    };
    eprintln!(
        "  {:<6} {:<5} {:>7} ms  rows={:<5} reps={}{}",
        q.id, status, record.wall_ms, record.rows, record.reps, missing
    );
}

/// Warn (do not fail the run) when a record's row count violates its expected
/// bound — the run keeps going so one bad query doesn't abort a long sweep.
fn check_expectations(q: &corpus::QueryDef, record: &RunRecord) {
    if record.status != Status::Ok {
        return;
    }
    if !q.expected_rows.contains(record.rows) {
        eprintln!(
            "    !! {} rows={} outside expected {} (target {})",
            q.id, record.rows, q.expected_rows, record.target
        );
    }
}

fn target_fingerprint(t: &Target) -> TargetFingerprint {
    TargetFingerprint {
        id: t.id.clone(),
        kind: t.kind_str().to_string(),
        alias: t.alias.clone(),
        fluree_home: t.fluree_home.display().to_string(),
    }
}

/// Run provenance gathered at start.
struct Provenance {
    run_id: String,
    timestamp: String,
    file_stamp: String,
    git_commit: String,
    git_dirty: bool,
    build_profile: String,
    host: String,
}

fn provenance() -> Provenance {
    let now = chrono::Utc::now();
    Provenance {
        run_id: ulid::Ulid::new().to_string(),
        timestamp: now.to_rfc3339(),
        file_stamp: now.format("%Y%m%dT%H%M%SZ").to_string(),
        git_commit: git_short_commit(),
        git_dirty: git_dirty(),
        build_profile: if cfg!(debug_assertions) {
            "debug".to_string()
        } else {
            "release".to_string()
        },
        host: hostname(),
    }
}

fn git_short_commit() -> String {
    run_capture("git", &["-C", env!("CARGO_MANIFEST_DIR"), "rev-parse", "--short", "HEAD"])
        .unwrap_or_else(|| "unknown".to_string())
}

fn git_dirty() -> bool {
    run_capture("git", &["-C", env!("CARGO_MANIFEST_DIR"), "status", "--porcelain"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

fn hostname() -> String {
    run_capture("hostname", &[]).unwrap_or_else(|| "unknown".to_string())
}

/// Run a command and capture trimmed stdout, or `None` on any failure.
fn run_capture(cmd: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(cmd).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// A streaming JSONL writer that flushes after every line so a crash leaves a
/// readable partial run.
struct RunWriter {
    file: std::fs::File,
}

impl RunWriter {
    fn create(path: &Path) -> Result<Self> {
        let file = std::fs::File::create(path)
            .with_context(|| format!("creating run file {}", path.display()))?;
        Ok(Self { file })
    }

    fn write(&mut self, line: &Line) -> Result<()> {
        let json = serde_json::to_string(line)?;
        writeln!(self.file, "{json}")?;
        self.file.flush()?;
        Ok(())
    }
}
