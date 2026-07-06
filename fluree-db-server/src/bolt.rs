//! Bolt protocol listener: Neo4j drivers speaking to the openCypher surface.
//!
//! The protocol machinery (PackStream, chunking, handshake, session state)
//! lives in `fluree-db-bolt` and is pure; this module owns the TCP side and
//! the execution glue. Each connection gets one tokio task, holds the shared
//! [`AppState`], and executes autocommit `RUN` statements through exactly
//! the entry points the HTTP routes use: `query_cypher_with_params` for
//! reads, the consensus submit path for writes. See
//! `docs/api/bolt.md`.
//!
//! v1 runs open (no auth): the listener refuses to start when the server
//! has credentials configured for the data plane, rather than inventing a
//! parallel identity path.

use std::sync::Arc;
use std::time::Instant;

use fluree_db_api::cypher_txn::CypherTransaction;
use fluree_db_api::format::cypher_typed::{
    CypherCell, CypherNode, CypherPath, CypherRelationship, CypherTemporal,
};
use fluree_db_bolt::chunk::{write_message, ChunkAssembler};
use fluree_db_bolt::handshake::{negotiate, HandshakeOutcome, HANDSHAKE_LEN, REJECT};
use fluree_db_bolt::message::{Request, Response};
use fluree_db_bolt::session::{
    BeginRequest, ResultStream, RunRequest, Session, SessionConfig, Turn,
};
use fluree_db_bolt::value::{sig, MapValue, Structure, Value};
use fluree_db_bolt::BoltVersion;
use serde_json::Value as JsonValue;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, info, warn};

use crate::state::AppState;

const CODE_SYNTAX: &str = "Neo.ClientError.Statement.SyntaxError";
const CODE_DB_NOT_FOUND: &str = "Neo.ClientError.Database.DatabaseNotFound";
const CODE_INVALID: &str = "Neo.ClientError.Request.Invalid";
const CODE_GENERAL: &str = "Neo.DatabaseError.General.UnknownError";
/// Optimistic-concurrency conflict at COMMIT. The `TransientError`
/// classification is what makes official drivers' managed transaction
/// functions retry the whole (retryable-closure) transaction.
const CODE_TX_CONFLICT: &str = "Neo.TransientError.Transaction.OptimisticConflict";

/// Bind the Bolt listener and spawn the accept loop. Returns the bound
/// address (`addr` may name port 0) and the accept-loop task.
pub async fn spawn_listener(
    state: Arc<AppState>,
    addr: std::net::SocketAddr,
) -> std::io::Result<(std::net::SocketAddr, tokio::task::JoinHandle<()>)> {
    let listener = TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    info!(addr = %bound, "Bolt listener starting");
    let task = tokio::spawn(async move {
        let mut next_conn_id: u64 = 0;
        loop {
            match listener.accept().await {
                Ok((socket, peer)) => {
                    next_conn_id += 1;
                    let conn_id = next_conn_id;
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(state, socket, conn_id).await {
                            debug!(conn_id, peer = %peer, error = %e, "bolt connection closed");
                        }
                    });
                }
                Err(e) => warn!(error = %e, "bolt accept failed"),
            }
        }
    });
    Ok((bound, task))
}

async fn handle_connection(
    state: Arc<AppState>,
    mut socket: TcpStream,
    conn_id: u64,
) -> std::io::Result<()> {
    socket.set_nodelay(true)?;

    let mut handshake = [0u8; HANDSHAKE_LEN];
    socket.read_exact(&mut handshake).await?;
    let version = match negotiate(&handshake) {
        HandshakeOutcome::Accept(v) => {
            socket.write_all(&v.to_bytes()).await?;
            v
        }
        HandshakeOutcome::NoVersionOverlap => {
            socket.write_all(&REJECT).await?;
            return Ok(());
        }
        HandshakeOutcome::BadMagic => return Ok(()),
    };
    debug!(conn_id, version = %version, "bolt session negotiated");

    let mut session = Session::new(SessionConfig {
        version,
        server_agent: server_agent(),
        connection_id: format!("bolt-{conn_id}"),
        default_db: state.config.bolt_default_db.clone(),
        advertised_address: None,
    });

    let mut assembler = ChunkAssembler::new();
    let mut read_buf = vec![0u8; 16 * 1024];
    let mut out_buf: Vec<u8> = Vec::new();
    // The open explicit transaction, if any. Dropping it rolls back
    // (nothing was published).
    let mut open_txn: Option<CypherTransaction> = None;
    loop {
        let n = socket.read(&mut read_buf).await?;
        if n == 0 {
            return Ok(());
        }
        if let Err(e) = assembler.push(&read_buf[..n]) {
            let failure = Response::failure(CODE_INVALID, e.to_string());
            out_buf.clear();
            write_message(&failure.encode(), &mut out_buf);
            socket.write_all(&out_buf).await?;
            return Ok(());
        }

        out_buf.clear();
        let mut close = false;
        while let Some(payload) = assembler.next_message() {
            let request = match Request::decode(&payload) {
                Ok(r) => r,
                Err(e) => {
                    write_message(
                        &Response::failure(CODE_INVALID, e.to_string()).encode(),
                        &mut out_buf,
                    );
                    close = true;
                    break;
                }
            };
            match session.on_request(request) {
                Turn::Reply(replies) => {
                    for reply in replies {
                        write_message(&reply.encode(), &mut out_buf);
                    }
                }
                Turn::Close(replies) => {
                    for reply in replies {
                        write_message(&reply.encode(), &mut out_buf);
                    }
                    close = true;
                    break;
                }
                Turn::Execute(run) => {
                    let version = session.version();
                    let reply =
                        execute_run(&state, &mut session, run, version, &mut open_txn).await;
                    write_message(&reply.encode(), &mut out_buf);
                }
                Turn::Begin(begin) => {
                    let reply = handle_begin(&state, &mut session, &mut open_txn, begin).await;
                    write_message(&reply.encode(), &mut out_buf);
                }
                Turn::Commit => {
                    let reply = handle_commit(&state, &mut session, open_txn.take()).await;
                    write_message(&reply.encode(), &mut out_buf);
                }
                Turn::Rollback => {
                    open_txn = None;
                    write_message(&session.rollback_done().encode(), &mut out_buf);
                }
                Turn::Reset(replies) => {
                    open_txn = None;
                    for reply in replies {
                        write_message(&reply.encode(), &mut out_buf);
                    }
                }
            }
        }
        if !out_buf.is_empty() {
            socket.write_all(&out_buf).await?;
        }
        if close {
            return Ok(());
        }
    }
}

fn server_agent() -> String {
    // Official drivers parse a `Neo4j/<semver>` prefix for feature gating;
    // everything after it identifies the actual implementation.
    format!(
        "Neo4j/5.4.0 (compatible; Fluree/{})",
        env!("CARGO_PKG_VERSION")
    )
}

#[derive(Debug)]
struct RunFailure {
    code: &'static str,
    message: String,
}

impl RunFailure {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

async fn execute_run(
    state: &AppState,
    session: &mut Session,
    run: RunRequest,
    version: BoltVersion,
    open_txn: &mut Option<CypherTransaction>,
) -> Response {
    let started = Instant::now();
    let outcome = match open_txn.as_mut() {
        Some(txn) => try_execute_txn_run(state, txn, &run, version).await,
        None => try_execute_run(state, &run, version).await,
    };
    match outcome {
        Ok(stream) => session.run_succeeded(stream, started.elapsed().as_millis() as i64),
        Err(f) => {
            debug!(code = f.code, error = %f.message, "bolt RUN failed");
            // A failed statement poisons the open transaction.
            *open_txn = None;
            session.run_failed(f.code, f.message)
        }
    }
}

/// Whether this deployment can honor explicit transactions: the optimistic
/// commit path publishes through the local commit machinery, so replicated
/// (Raft) and peer deployments must reject BEGIN clearly.
fn explicit_tx_supported(state: &AppState) -> bool {
    let single_node = !state.config.is_peer_mode();
    #[cfg(feature = "raft")]
    let single_node = single_node && !state.config.raft_enabled;
    single_node
}

async fn handle_begin(
    state: &AppState,
    session: &mut Session,
    open_txn: &mut Option<CypherTransaction>,
    begin: BeginRequest,
) -> Response {
    if !explicit_tx_supported(state) {
        return session.begin_failed(
            CODE_INVALID,
            "explicit transactions require a single-node server (local commit path);              this deployment replicates writes — use autocommit queries",
        );
    }
    let Some(ledger_id) = begin.db.as_deref() else {
        return session.begin_failed(
            CODE_DB_NOT_FOUND,
            "no database selected: pass `database=` in the driver session              (or set --bolt-default-db on the server)",
        );
    };
    match state.fluree.begin_cypher_transaction(ledger_id).await {
        Ok(txn) => {
            *open_txn = Some(txn);
            session.begin_succeeded()
        }
        Err(e) => session.begin_failed(CODE_DB_NOT_FOUND, e.to_string()),
    }
}

async fn handle_commit(
    state: &AppState,
    session: &mut Session,
    open_txn: Option<CypherTransaction>,
) -> Response {
    let Some(txn) = open_txn else {
        return session.commit_failed(CODE_INVALID, "no open transaction");
    };
    match state.fluree.commit_cypher_transaction(txn).await {
        Ok(t) => session.commit_succeeded(Some(format!("fluree:t:{t}"))),
        Err(fluree_db_api::ApiError::Transact(
            fluree_db_api::TransactError::CommitConflict { expected_t, head_t },
        )) => session.commit_failed(
            CODE_TX_CONFLICT,
            format!(
                "the transaction's base state (t={expected_t}) was superseded by a                  concurrent commit (t={head_t}); retry the transaction"
            ),
        ),
        Err(e) => session.commit_failed(CODE_GENERAL, e.to_string()),
    }
}

/// One RUN inside an open explicit transaction: reads query the
/// transaction's private state; writes stage into it (publishing waits
/// for COMMIT). Statement-level failures surface here, at RUN time.
async fn try_execute_txn_run(
    state: &AppState,
    txn: &mut CypherTransaction,
    run: &RunRequest,
    version: BoltVersion,
) -> Result<ResultStream, RunFailure> {
    let params = params_to_json(&run.parameters)?;
    let is_write = fluree_db_api::cypher_write::cypher_statement_is_write(&run.query)
        .map_err(|e| RunFailure::new(CODE_SYNTAX, e.to_string()))?;
    let ledger_id = txn.ledger_id().to_string();

    if is_write {
        let outcome = state
            .fluree
            .cypher_transaction_write(txn, &run.query, params.as_ref())
            .await
            .map_err(|e| RunFailure::new(CODE_SYNTAX, e.to_string()))?;

        let mut summary = MapValue::new();
        summary.insert("type", "w");
        summary.insert("t_last", 0i64);
        summary.insert("db", ledger_id.as_str());
        let mut stats = MapValue::new();
        stats.insert("contains-updates", outcome.flake_count > 0);
        stats.insert("fluree-flakes", outcome.flake_count as i64);
        summary.insert("stats", stats);

        let (fields, rows) = match &outcome.return_envelope {
            Some(envelope) => envelope_to_rows(envelope),
            None => (Vec::new(), Default::default()),
        };
        return Ok(ResultStream {
            fields,
            rows,
            summary,
        });
    }

    let view = state
        .fluree
        .cypher_transaction_view(txn)
        .await
        .map_err(|e| RunFailure::new(CODE_GENERAL, e.to_string()))?;
    let result = state
        .fluree
        .query_cypher_with_params(&view, &run.query, params.as_ref())
        .await
        .map_err(|e| RunFailure::new(CODE_SYNTAX, e.to_string()))?;
    let (columns, rows) = result
        .to_cypher_typed_table(&view)
        .await
        .map_err(|e| RunFailure::new(CODE_GENERAL, e.to_string()))?;

    let mut summary = MapValue::new();
    summary.insert("type", "r");
    summary.insert("t_last", 0i64);
    summary.insert("db", ledger_id.as_str());
    Ok(ResultStream {
        fields: columns,
        rows: rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|c| typed_cell_to_bolt(c, version))
                    .collect()
            })
            .collect(),
        summary,
    })
}

async fn try_execute_run(
    state: &AppState,
    run: &RunRequest,
    version: BoltVersion,
) -> Result<ResultStream, RunFailure> {
    let Some(ledger_id) = run.db.as_deref() else {
        return Err(RunFailure::new(
            CODE_DB_NOT_FOUND,
            "no database selected: pass `database=` in the driver session \
             (or set --bolt-default-db on the server)",
        ));
    };
    let params = params_to_json(&run.parameters)?;
    let is_write = fluree_db_api::cypher_write::cypher_statement_is_write(&run.query)
        .map_err(|e| RunFailure::new(CODE_SYNTAX, e.to_string()))?;
    if is_write {
        execute_write(state, ledger_id, &run.query, params).await
    } else {
        execute_read(state, ledger_id, &run.query, params, version).await
    }
}

async fn execute_read(
    state: &AppState,
    ledger_id: &str,
    query: &str,
    params: Option<fluree_db_api::CypherParamMap>,
    version: BoltVersion,
) -> Result<ResultStream, RunFailure> {
    let view = state
        .fluree
        .db_with_default_context(ledger_id)
        .await
        .map_err(|e| RunFailure::new(CODE_DB_NOT_FOUND, e.to_string()))?;
    let result = state
        .fluree
        .query_cypher_with_params(&view, query, params.as_ref())
        .await
        .map_err(|e| RunFailure::new(CODE_SYNTAX, e.to_string()))?;
    let (columns, rows) = result
        .to_cypher_typed_table(&view)
        .await
        .map_err(|e| RunFailure::new(CODE_GENERAL, e.to_string()))?;

    let mut summary = MapValue::new();
    summary.insert("type", "r");
    summary.insert("t_last", 0i64);
    summary.insert("db", ledger_id);
    Ok(ResultStream {
        fields: columns,
        rows: rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|c| typed_cell_to_bolt(c, version))
                    .collect()
            })
            .collect(),
        summary,
    })
}

async fn execute_write(
    state: &AppState,
    ledger_id: &str,
    query: &str,
    params: Option<fluree_db_api::CypherParamMap>,
) -> Result<ResultStream, RunFailure> {
    use fluree_db_api::{CommitOpts, TxnOpts};
    use fluree_db_consensus::{TransactionBody, TransactionRequest};

    // Same shape as the HTTP route (`execute_cypher_transact`): plan a
    // trailing RETURN pre-submission so created-entity rows are
    // reconstructible from the skolem id after commit.
    let return_plan = fluree_db_api::cypher_write::plan_write_return_source(query, params.as_ref())
        .map_err(|e| RunFailure::new(CODE_SYNTAX, e.to_string()))?;
    let skolem_txn_id = return_plan
        .as_ref()
        .map(|_| fluree_db_api::cypher_write::fresh_skolem_txn_id());
    let txn_opts = TxnOpts {
        skolem_txn_id: skolem_txn_id.clone(),
        ..TxnOpts::default()
    };

    let request = TransactionRequest {
        idempotency_key: None,
        ledger_id: ledger_id.to_string(),
        body: TransactionBody::Cypher {
            query: query.to_string(),
            params,
        },
        txn_opts,
        commit_opts: CommitOpts::default(),
        tracking: None,
        governance: fluree_db_api::GovernanceOptions::default(),
    };

    let empty_headers = axum::http::HeaderMap::new();
    let receipt = crate::routes::transact::submit_via_consensus(state, request, &empty_headers)
        .await
        .map_err(|e| RunFailure::new(CODE_GENERAL, e.to_string()))?;

    let mut summary = MapValue::new();
    summary.insert("type", "w");
    summary.insert("t_last", 0i64);
    summary.insert("db", ledger_id);
    let mut stats = MapValue::new();
    stats.insert("contains-updates", true);
    stats.insert("fluree-flakes", receipt.commit.flake_count as i64);
    stats.insert("fluree-commit-t", receipt.commit.t);
    summary.insert("stats", stats);

    let (Some(plan), Some(skolem_id)) = (return_plan, skolem_txn_id) else {
        return Ok(ResultStream {
            fields: Vec::new(),
            rows: Default::default(),
            summary,
        });
    };

    // Write with RETURN: wait for local visibility, then answer the RETURN
    // rows exactly like the HTTP path's Cypher-JSON envelope.
    let ledger_state = crate::routes::transact::wait_for_committed_state(
        state,
        ledger_id,
        receipt.commit.t,
        &receipt,
    )
    .await
    .map_err(|e| RunFailure::new(CODE_GENERAL, e.to_string()))?;
    let envelope = fluree_db_api::cypher_write::write_return_rows(&plan, &skolem_id, &ledger_state)
        .await
        .map_err(|e| RunFailure::new(CODE_GENERAL, e.to_string()))?;

    let (fields, rows) = envelope_to_rows(&envelope);
    Ok(ResultStream {
        fields,
        rows,
        summary,
    })
}

/// Pull columns + rows out of the Cypher-JSON envelope
/// (`{"results":[{"columns":[…],"data":[{"row":[…]},…]}]}`).
fn envelope_to_rows(envelope: &JsonValue) -> (Vec<String>, std::collections::VecDeque<Vec<Value>>) {
    let result = &envelope["results"][0];
    let fields = result["columns"]
        .as_array()
        .map(|cols| {
            cols.iter()
                .filter_map(|c| c.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let rows = result["data"]
        .as_array()
        .map(|data| {
            data.iter()
                .map(|entry| {
                    entry["row"]
                        .as_array()
                        .map(|cells| cells.iter().cloned().map(cell_to_bolt).collect())
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();
    (fields, rows)
}

/// Bolt RUN parameters (PackStream map) → the Cypher `$param` map the
/// engine substitutes. Graph structures and byte arrays have no Cypher
/// parameter equivalent here and are rejected.
fn params_to_json(params: &MapValue) -> Result<Option<fluree_db_api::CypherParamMap>, RunFailure> {
    if params.is_empty() {
        return Ok(None);
    }
    let mut map = fluree_db_api::CypherParamMap::new();
    for (k, v) in &params.0 {
        map.insert(k.clone(), bolt_to_json(v)?);
    }
    Ok(Some(map))
}

fn bolt_to_json(value: &Value) -> Result<JsonValue, RunFailure> {
    Ok(match value {
        Value::Null => JsonValue::Null,
        Value::Boolean(b) => JsonValue::Bool(*b),
        Value::Integer(i) => JsonValue::from(*i),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Value::String(s) => JsonValue::String(s.clone()),
        Value::List(items) => {
            JsonValue::Array(items.iter().map(bolt_to_json).collect::<Result<_, _>>()?)
        }
        Value::Map(m) => JsonValue::Object(
            m.0.iter()
                .map(|(k, v)| bolt_to_json(v).map(|v| (k.clone(), v)))
                .collect::<Result<_, _>>()?,
        ),
        Value::Bytes(_) | Value::Structure(_) => {
            return Err(RunFailure::new(
                CODE_INVALID,
                "byte-array and structure parameters are not supported",
            ))
        }
    })
}

/// One RDF-faithful result cell (from `QueryResult::to_cypher_table`) →
/// PackStream. Scalars pass through natively; `{"@value","@type"}` literals
/// flatten with datatype-aware mapping (`xsd:decimal` → Float, Neo4j parity,
/// documented precision loss; oversized `xsd:integer` → Float); `{"@id"}`
/// refs become IRI strings; temporal values stay ISO strings in v1 (the
/// value mappings the JSON transport already has — see the support matrix).
fn cell_to_bolt(cell: JsonValue) -> Value {
    match cell {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(b) => Value::Boolean(b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i)
            } else {
                Value::Float(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        JsonValue::String(s) => Value::String(s),
        JsonValue::Array(items) => Value::List(items.into_iter().map(cell_to_bolt).collect()),
        JsonValue::Object(mut m) => {
            if let Some(v) = m.remove("@value") {
                let datatype = m.remove("@type");
                let datatype = datatype.as_ref().and_then(|t| t.as_str()).unwrap_or("");
                return typed_literal_to_bolt(v, local_name(datatype));
            }
            if let Some(JsonValue::String(iri)) = m.remove("@id") {
                return Value::String(iri);
            }
            // A Cypher map value (`{a: n.name}`, `properties(n)`).
            Value::Map(m.into_iter().map(|(k, v)| (k, cell_to_bolt(v))).collect())
        }
    }
}

/// The fragment of a datatype IRI after the last `#`, `/`, or `:` —
/// tolerant of both full XSD IRIs and context-compacted forms.
fn local_name(datatype_iri: &str) -> &str {
    datatype_iri
        .rsplit(['#', '/', ':'])
        .next()
        .unwrap_or(datatype_iri)
}

fn typed_literal_to_bolt(value: JsonValue, datatype_local: &str) -> Value {
    match (datatype_local, &value) {
        // PackStream has no decimal type; Neo4j returns Float. The JSON
        // transport keeps the exact lexical string instead — see the
        // value-mapping notes in docs/api/bolt.md.
        ("decimal", JsonValue::String(s)) => s
            .parse::<f64>()
            .map(Value::Float)
            .unwrap_or_else(|_| Value::String(s.clone())),
        // Arbitrary-precision integer that exceeded i64 (rendered as a
        // string): degrade to Float like Neo4j's own out-of-range behavior.
        ("integer" | "long", JsonValue::String(s)) => s
            .parse::<i64>()
            .map(Value::Integer)
            .or_else(|_| s.parse::<f64>().map(Value::Float))
            .unwrap_or_else(|_| Value::String(s.clone())),
        _ => cell_to_bolt(value),
    }
}

/// A stable 64-bit id for Bolt's numeric `id` fields, derived from the
/// durable identity (the IRI) via FNV-1a. Numeric ids only need to be
/// opaque, stable handles — `element_id` (the IRI itself) is the durable
/// identity, as documented in the support matrix.
fn stable_id(identity: &str) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in identity.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash as i64
}

/// One typed result cell (`QueryResult::to_cypher_typed_table`) →
/// PackStream. Graph values become Bolt structures (element-id fields on
/// 5.x only); temporal values become Date/Time/DateTime structures (the
/// 4.4 legacy DateTime encodes local-adjusted seconds); `xsd:decimal`
/// degrades to Float (Neo4j parity, documented precision loss).
fn typed_cell_to_bolt(cell: CypherCell, version: BoltVersion) -> Value {
    match cell {
        CypherCell::Value(json) => cell_to_bolt(json),
        CypherCell::Decimal(s) => s
            .parse::<f64>()
            .map(Value::Float)
            .unwrap_or_else(|_| Value::String(s)),
        CypherCell::BigInt(s) => match (s.parse::<i64>(), s.parse::<f64>()) {
            (Ok(i), _) => Value::Integer(i),
            (Err(_), Ok(f)) => Value::Float(f),
            _ => Value::String(s),
        },
        CypherCell::Temporal(t) => temporal_structure(t, version),
        CypherCell::List(cells) => Value::List(
            cells
                .into_iter()
                .map(|c| typed_cell_to_bolt(c, version))
                .collect(),
        ),
        CypherCell::Map(entries) => Value::Map(
            entries
                .into_iter()
                .map(|(k, c)| (k, typed_cell_to_bolt(c, version)))
                .collect(),
        ),
        CypherCell::Node(node) => node_structure(*node, version),
        CypherCell::Relationship(rel) => relationship_structure(*rel, version),
        CypherCell::Path(path) => path_structure(*path, version),
    }
}

fn properties_map(properties: Vec<(String, CypherCell)>, version: BoltVersion) -> Value {
    Value::Map(
        properties
            .into_iter()
            .map(|(k, c)| (k, typed_cell_to_bolt(c, version)))
            .collect(),
    )
}

fn node_structure(node: CypherNode, version: BoltVersion) -> Value {
    let mut fields = vec![
        Value::Integer(stable_id(&node.iri)),
        Value::List(node.labels.into_iter().map(Value::String).collect()),
        properties_map(node.properties, version),
    ];
    if version.has_element_ids() {
        fields.push(Value::String(node.iri));
    }
    Value::Structure(Structure {
        signature: sig::NODE,
        fields,
    })
}

/// The relationship's numeric id: the reifier IRI when the edge is
/// reified (durable), else a synthesized hash of (start, type, end) —
/// stable within a result set, which is all pre-5.x drivers need.
fn relationship_id(rel: &CypherRelationship) -> (i64, String) {
    match &rel.reifier_iri {
        Some(iri) => (stable_id(iri), iri.clone()),
        None => {
            let synthetic = format!("{}|{}|{}", rel.start_iri, rel.type_name, rel.end_iri);
            (stable_id(&synthetic), synthetic)
        }
    }
}

fn relationship_structure(rel: CypherRelationship, version: BoltVersion) -> Value {
    let (id, element_id) = relationship_id(&rel);
    let mut fields = vec![
        Value::Integer(id),
        Value::Integer(stable_id(&rel.start_iri)),
        Value::Integer(stable_id(&rel.end_iri)),
        Value::String(rel.type_name.clone()),
        properties_map(rel.properties, version),
    ];
    if version.has_element_ids() {
        fields.push(Value::String(element_id));
        fields.push(Value::String(rel.start_iri));
        fields.push(Value::String(rel.end_iri));
    }
    Value::Structure(Structure {
        signature: sig::RELATIONSHIP,
        fields,
    })
}

fn unbound_relationship_structure(rel: CypherRelationship, version: BoltVersion) -> Value {
    let (id, element_id) = relationship_id(&rel);
    let mut fields = vec![
        Value::Integer(id),
        Value::String(rel.type_name.clone()),
        properties_map(rel.properties, version),
    ];
    if version.has_element_ids() {
        fields.push(Value::String(element_id));
    }
    Value::Structure(Structure {
        signature: sig::UNBOUND_RELATIONSHIP,
        fields,
    })
}

fn path_structure(path: CypherPath, version: BoltVersion) -> Value {
    let nodes = path
        .nodes
        .into_iter()
        .map(|n| node_structure(n, version))
        .collect();
    let rels = path
        .rels
        .into_iter()
        .map(|r| unbound_relationship_structure(r, version))
        .collect();
    let indices = path.indices.into_iter().map(Value::Integer).collect();
    Value::Structure(Structure {
        signature: sig::PATH,
        fields: vec![Value::List(nodes), Value::List(rels), Value::List(indices)],
    })
}

fn temporal_structure(temporal: CypherTemporal, version: BoltVersion) -> Value {
    match temporal {
        CypherTemporal::Date { days, .. } => Value::Structure(Structure {
            signature: sig::DATE,
            fields: vec![Value::Integer(days)],
        }),
        CypherTemporal::DateTime {
            epoch_seconds,
            nanos,
            tz_offset_seconds: Some(offset),
            ..
        } => {
            // 5.x: UTC epoch seconds + offset. 4.4 (no `utc` patch
            // negotiated): the legacy struct bakes the offset into the
            // seconds field ("local epoch seconds").
            if version.has_element_ids() {
                Value::Structure(Structure {
                    signature: sig::DATE_TIME,
                    fields: vec![
                        Value::Integer(epoch_seconds),
                        Value::Integer(nanos as i64),
                        Value::Integer(offset as i64),
                    ],
                })
            } else {
                Value::Structure(Structure {
                    signature: sig::DATE_TIME_LEGACY,
                    fields: vec![
                        Value::Integer(epoch_seconds + offset as i64),
                        Value::Integer(nanos as i64),
                        Value::Integer(offset as i64),
                    ],
                })
            }
        }
        CypherTemporal::DateTime {
            epoch_seconds,
            nanos,
            tz_offset_seconds: None,
            ..
        } => Value::Structure(Structure {
            signature: sig::LOCAL_DATE_TIME,
            fields: vec![Value::Integer(epoch_seconds), Value::Integer(nanos as i64)],
        }),
        CypherTemporal::Time {
            nanos_since_midnight,
            tz_offset_seconds: Some(offset),
            ..
        } => Value::Structure(Structure {
            signature: sig::TIME,
            fields: vec![
                Value::Integer(nanos_since_midnight),
                Value::Integer(offset as i64),
            ],
        }),
        CypherTemporal::Time {
            nanos_since_midnight,
            tz_offset_seconds: None,
            ..
        } => Value::Structure(Structure {
            signature: sig::LOCAL_TIME,
            fields: vec![Value::Integer(nanos_since_midnight)],
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalar_cells_map_natively() {
        assert_eq!(cell_to_bolt(json!(42)), Value::Integer(42));
        assert_eq!(cell_to_bolt(json!(1.5)), Value::Float(1.5));
        assert_eq!(cell_to_bolt(json!("hi")), Value::String("hi".into()));
        assert_eq!(cell_to_bolt(json!(true)), Value::Boolean(true));
        assert_eq!(cell_to_bolt(JsonValue::Null), Value::Null);
    }

    #[test]
    fn decimal_literal_becomes_float() {
        let cell = json!({"@value": "2.5", "@type": "http://www.w3.org/2001/XMLSchema#decimal"});
        assert_eq!(cell_to_bolt(cell), Value::Float(2.5));
        // Context-compacted datatype form.
        let cell = json!({"@value": "0.1", "@type": "xsd:decimal"});
        assert_eq!(cell_to_bolt(cell), Value::Float(0.1));
    }

    #[test]
    fn id_ref_becomes_iri_string() {
        let cell = json!({"@id": "http://example.org/u1"});
        assert_eq!(
            cell_to_bolt(cell),
            Value::String("http://example.org/u1".into())
        );
    }

    #[test]
    fn date_stays_iso_string() {
        let cell = json!({"@value": "1990-11-23", "@type": "xsd:date"});
        assert_eq!(cell_to_bolt(cell), Value::String("1990-11-23".into()));
    }

    #[test]
    fn language_tagged_string_flattens() {
        let cell = json!({"@value": "hallo", "@language": "de"});
        assert_eq!(cell_to_bolt(cell), Value::String("hallo".into()));
    }

    #[test]
    fn map_and_list_cells_recurse() {
        let cell = json!({"a": {"@value": "1.5", "@type": "xsd:decimal"}, "b": [1, 2]});
        let Value::Map(m) = cell_to_bolt(cell) else {
            panic!("expected map")
        };
        assert_eq!(m.get("a"), Some(&Value::Float(1.5)));
        assert_eq!(
            m.get("b"),
            Some(&Value::List(vec![Value::Integer(1), Value::Integer(2)]))
        );
    }

    #[test]
    fn params_round_trip_to_json() {
        let mut params = MapValue::new();
        params.insert("id", 42i64);
        params.insert("name", "ana");
        params.insert(
            "scores",
            Value::List(vec![Value::Float(1.5), Value::Integer(2)]),
        );
        let json = params_to_json(&params).unwrap().unwrap();
        assert_eq!(json.get("id"), Some(&json!(42)));
        assert_eq!(json.get("name"), Some(&json!("ana")));
        assert_eq!(json.get("scores"), Some(&json!([1.5, 2])));
    }

    #[test]
    fn structure_params_rejected() {
        let mut params = MapValue::new();
        params.insert(
            "point",
            Value::Structure(fluree_db_bolt::value::Structure {
                signature: 0x58,
                fields: vec![],
            }),
        );
        assert!(params_to_json(&params).is_err());
    }

    #[test]
    fn node_structure_field_count_by_version() {
        let node = CypherNode {
            iri: "http://example.org/u1".into(),
            labels: vec!["Person".into()],
            properties: vec![("name".into(), CypherCell::Value(json!("Ana")))],
        };
        let Value::Structure(v5) = node_structure(node.clone(), BoltVersion::V5_4) else {
            panic!()
        };
        assert_eq!(v5.signature, sig::NODE);
        assert_eq!(v5.fields.len(), 4, "5.x nodes carry element_id");
        assert_eq!(v5.fields[3], Value::String("http://example.org/u1".into()));

        let Value::Structure(v44) = node_structure(node, BoltVersion::V4_4) else {
            panic!()
        };
        assert_eq!(v44.fields.len(), 3, "4.4 nodes have no element_id");
        let Value::Map(props) = &v44.fields[2] else {
            panic!("properties map")
        };
        assert_eq!(props.get_str("name"), Some("Ana"));
    }

    #[test]
    fn relationship_structure_shape() {
        let rel = CypherRelationship {
            start_iri: "http://example.org/a".into(),
            end_iri: "http://example.org/b".into(),
            type_name: "knows".into(),
            reifier_iri: None,
            properties: vec![],
        };
        let Value::Structure(v5) = relationship_structure(rel.clone(), BoltVersion::V5_4) else {
            panic!()
        };
        assert_eq!(v5.signature, sig::RELATIONSHIP);
        assert_eq!(v5.fields.len(), 8);
        assert_eq!(v5.fields[3], Value::String("knows".into()));

        let Value::Structure(v44) = relationship_structure(rel, BoltVersion::V4_4) else {
            panic!()
        };
        assert_eq!(v44.fields.len(), 5);
    }

    #[test]
    fn temporal_structures_by_version() {
        let date = CypherTemporal::Date {
            days: 7631,
            iso: "1990-11-23".into(),
        };
        let Value::Structure(d) = temporal_structure(date, BoltVersion::V5_4) else {
            panic!()
        };
        assert_eq!(
            (d.signature, &d.fields[..]),
            (sig::DATE, &[Value::Integer(7631)][..])
        );

        // 2024-01-15T10:30:00+05:00 => utc epoch 1705289400, offset 18000.
        let dt = CypherTemporal::DateTime {
            epoch_seconds: 1_705_289_400,
            nanos: 0,
            tz_offset_seconds: Some(18_000),
            iso: String::new(),
        };
        let Value::Structure(modern) = temporal_structure(dt.clone(), BoltVersion::V5_4) else {
            panic!()
        };
        assert_eq!(modern.signature, sig::DATE_TIME);
        assert_eq!(modern.fields[0], Value::Integer(1_705_289_400));

        let Value::Structure(legacy) = temporal_structure(dt, BoltVersion::V4_4) else {
            panic!()
        };
        assert_eq!(legacy.signature, sig::DATE_TIME_LEGACY);
        assert_eq!(
            legacy.fields[0],
            Value::Integer(1_705_289_400 + 18_000),
            "legacy DateTime bakes the offset into the seconds"
        );

        let naive = CypherTemporal::DateTime {
            epoch_seconds: 100,
            nanos: 5,
            tz_offset_seconds: None,
            iso: String::new(),
        };
        let Value::Structure(local) = temporal_structure(naive, BoltVersion::V5_4) else {
            panic!()
        };
        assert_eq!(local.signature, sig::LOCAL_DATE_TIME);
    }

    #[test]
    fn decimal_cell_degrades_to_float() {
        assert_eq!(
            typed_cell_to_bolt(CypherCell::Decimal("2.5".into()), BoltVersion::V5_4),
            Value::Float(2.5)
        );
        assert_eq!(
            typed_cell_to_bolt(CypherCell::BigInt("12".into()), BoltVersion::V5_4),
            Value::Integer(12)
        );
    }

    #[test]
    fn envelope_rows_extract() {
        let envelope = json!({
            "results": [{
                "columns": ["n"],
                "data": [
                    {"row": ["http://example.org/u1"], "meta": [null]},
                    {"row": [7], "meta": [null]}
                ]
            }]
        });
        let (fields, rows) = envelope_to_rows(&envelope);
        assert_eq!(fields, vec!["n"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], vec![Value::String("http://example.org/u1".into())]);
        assert_eq!(rows[1], vec![Value::Integer(7)]);
    }
}
