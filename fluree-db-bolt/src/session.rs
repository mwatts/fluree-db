//! The per-connection message state machine.
//!
//! Pure and transport-free: the caller decodes a [`Request`], hands it to
//! [`Session::on_request`], and acts on the returned [`Turn`] — writing
//! replies, closing the connection, or executing a statement and reporting
//! the outcome via [`Session::run_succeeded`] / [`Session::run_failed`].
//!
//! v1 scope: autocommit only. Results are fully materialized at RUN time by
//! the caller; PULL serves batches from the buffered [`ResultStream`] with
//! reactive `has_more` metadata. `BEGIN`/`COMMIT`/`ROLLBACK` answer a clear
//! FAILURE (never a silent wrong behavior), matching the Cypher support
//! matrix contract.

use std::collections::VecDeque;

use crate::handshake::BoltVersion;
use crate::message::{Request, Response};
use crate::value::{MapValue, Value};

/// Connection-level configuration the server glue supplies.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub version: BoltVersion,
    /// Advertised in the HELLO SUCCESS `server` field. Official drivers
    /// parse a `Neo4j/<semver>` prefix for feature gating.
    pub server_agent: String,
    /// Advertised in the HELLO SUCCESS `connection_id` field.
    pub connection_id: String,
    /// Ledger used when neither HELLO defaults nor RUN extra name a `db`.
    pub default_db: Option<String>,
    /// `host:port` for the single-entry ROUTE table. `None` answers ROUTE
    /// with a failure directing clients at the `bolt://` scheme.
    pub advertised_address: Option<String>,
}

/// One executed statement's buffered result, served out via PULL.
#[derive(Debug, Clone, Default)]
pub struct ResultStream {
    pub fields: Vec<String>,
    pub rows: VecDeque<Vec<Value>>,
    /// Metadata for the final SUCCESS once the stream drains (`type`,
    /// `t_last`, `db`, write `stats`, ...).
    pub summary: MapValue,
}

/// What the caller must do after feeding a request in.
#[derive(Debug, Clone, PartialEq)]
pub enum Turn {
    /// Write these replies; connection stays open.
    Reply(Vec<Response>),
    /// Write these replies (possibly none), then close the connection.
    Close(Vec<Response>),
    /// Execute the statement, then call `run_succeeded` / `run_failed` and
    /// write the response that returns.
    Execute(RunRequest),
}

/// An autocommit statement the server glue must execute.
#[derive(Debug, Clone, PartialEq)]
pub struct RunRequest {
    pub query: String,
    pub parameters: MapValue,
    /// From RUN extra `db`, HELLO defaults, or the configured default — in
    /// that precedence. `None` means the server has no default ledger and
    /// the request named none: fail the RUN.
    pub db: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    AwaitingHello,
    /// 5.1+: HELLO done, awaiting LOGON.
    Authentication,
    Ready,
    Streaming,
    Failed,
}

pub struct Session {
    config: SessionConfig,
    state: State,
    stream: Option<ResultStream>,
    /// `db` from HELLO extra defaults (Bolt 5.x drivers can set a session
    /// database there); RUN extra still overrides per statement.
    hello_db: Option<String>,
}

const CODE_INVALID: &str = "Neo.ClientError.Request.Invalid";
const CODE_TX_UNSUPPORTED: &str = "Neo.ClientError.Statement.TypeError";

impl Session {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            state: State::AwaitingHello,
            stream: None,
            hello_db: None,
        }
    }

    pub fn version(&self) -> BoltVersion {
        self.config.version
    }

    /// Whether the connection has completed HELLO (+LOGON where required).
    pub fn is_ready(&self) -> bool {
        matches!(self.state, State::Ready | State::Streaming | State::Failed)
    }

    pub fn on_request(&mut self, request: Request) -> Turn {
        match request {
            Request::Goodbye => Turn::Close(vec![]),
            Request::Reset => self.on_reset(),
            other => match self.state {
                State::AwaitingHello => self.on_awaiting_hello(other),
                State::Authentication => self.on_authentication(other),
                State::Ready => self.on_ready(other),
                State::Streaming => self.on_streaming(other),
                State::Failed => Turn::Reply(vec![Response::Ignored]),
            },
        }
    }

    fn on_reset(&mut self) -> Turn {
        if self.state == State::AwaitingHello {
            // RESET before HELLO is a protocol violation.
            return Turn::Close(vec![Response::failure(CODE_INVALID, "RESET before HELLO")]);
        }
        self.stream = None;
        self.state = if self.state == State::Authentication {
            State::Authentication
        } else {
            State::Ready
        };
        Turn::Reply(vec![Response::success_empty()])
    }

    fn on_awaiting_hello(&mut self, request: Request) -> Turn {
        match request {
            Request::Hello { extra } => {
                self.hello_db = extra.get_str("db").map(str::to_string);
                let mut meta = MapValue::new();
                meta.insert("server", self.config.server_agent.as_str());
                meta.insert("connection_id", self.config.connection_id.as_str());
                self.state = if self.config.version.uses_logon() {
                    State::Authentication
                } else {
                    State::Ready
                };
                Turn::Reply(vec![Response::success(meta)])
            }
            other => Turn::Close(vec![Response::failure(
                CODE_INVALID,
                format!("expected HELLO, got {}", request_name(&other)),
            )]),
        }
    }

    fn on_authentication(&mut self, request: Request) -> Turn {
        match request {
            // v1 runs open: any principal/scheme is accepted. Servers that
            // require auth do not expose the Bolt listener.
            Request::Logon { auth: _ } => {
                self.state = State::Ready;
                Turn::Reply(vec![Response::success_empty()])
            }
            other => Turn::Close(vec![Response::failure(
                CODE_INVALID,
                format!("expected LOGON, got {}", request_name(&other)),
            )]),
        }
    }

    fn on_ready(&mut self, request: Request) -> Turn {
        match request {
            Request::Run {
                query,
                parameters,
                extra,
            } => {
                let db = extra
                    .get_str("db")
                    .map(str::to_string)
                    .or_else(|| self.hello_db.clone())
                    .or_else(|| self.config.default_db.clone());
                Turn::Execute(RunRequest {
                    query,
                    parameters,
                    db,
                })
            }
            Request::Begin { .. } => self.fail(Response::failure(
                CODE_TX_UNSUPPORTED,
                "Explicit transactions (BEGIN/COMMIT/ROLLBACK) are not supported; \
                 use autocommit queries. See the Fluree Cypher support matrix.",
            )),
            Request::Commit | Request::Rollback => self.fail(Response::failure(
                CODE_INVALID,
                "COMMIT/ROLLBACK outside a transaction",
            )),
            Request::Logoff if self.config.version.uses_logon() => {
                self.state = State::Authentication;
                Turn::Reply(vec![Response::success_empty()])
            }
            Request::Route { extra, .. } => Turn::Reply(vec![self.route_table(&extra)]),
            Request::Telemetry { .. } => Turn::Reply(vec![Response::success_empty()]),
            Request::Pull { .. } | Request::Discard { .. } => self.fail(Response::failure(
                CODE_INVALID,
                "no result stream to consume",
            )),
            other => self.fail(Response::failure(
                CODE_INVALID,
                format!("unexpected {} in READY state", request_name(&other)),
            )),
        }
    }

    fn on_streaming(&mut self, request: Request) -> Turn {
        match request {
            Request::Pull { extra } => {
                let n = extra.get_int("n").unwrap_or(-1);
                Turn::Reply(self.serve_pull(n))
            }
            Request::Discard { extra } => {
                let n = extra.get_int("n").unwrap_or(-1);
                Turn::Reply(vec![self.serve_discard(n)])
            }
            other => self.fail(Response::failure(
                CODE_INVALID,
                format!("unexpected {} while streaming", request_name(&other)),
            )),
        }
    }

    /// The RUN handed out via [`Turn::Execute`] succeeded. `t_first_ms` is
    /// the time from RUN to result availability, surfaced in the RUN
    /// SUCCESS metadata like Neo4j's.
    pub fn run_succeeded(&mut self, stream: ResultStream, t_first_ms: i64) -> Response {
        let mut meta = MapValue::new();
        meta.insert(
            "fields",
            Value::List(
                stream
                    .fields
                    .iter()
                    .map(|f| Value::from(f.as_str()))
                    .collect(),
            ),
        );
        meta.insert("t_first", t_first_ms);
        meta.insert("qid", 0i64);
        self.stream = Some(stream);
        self.state = State::Streaming;
        Response::success(meta)
    }

    /// The RUN handed out via [`Turn::Execute`] failed.
    pub fn run_failed(&mut self, code: impl Into<String>, message: impl Into<String>) -> Response {
        self.state = State::Failed;
        self.stream = None;
        Response::Failure {
            code: code.into(),
            message: message.into(),
        }
    }

    fn serve_pull(&mut self, n: i64) -> Vec<Response> {
        let Some(stream) = self.stream.as_mut() else {
            return vec![Response::failure(CODE_INVALID, "no result stream")];
        };
        let take = if n < 0 {
            stream.rows.len()
        } else {
            (n as usize).min(stream.rows.len())
        };
        let mut replies: Vec<Response> = Vec::with_capacity(take + 1);
        for _ in 0..take {
            let row = stream.rows.pop_front().expect("row count checked");
            replies.push(Response::Record(row));
        }
        if stream.rows.is_empty() {
            let mut summary = std::mem::take(&mut stream.summary);
            summary.insert("has_more", false);
            replies.push(Response::success(summary));
            self.stream = None;
            self.state = State::Ready;
        } else {
            let mut meta = MapValue::new();
            meta.insert("has_more", true);
            replies.push(Response::success(meta));
        }
        replies
    }

    fn serve_discard(&mut self, n: i64) -> Response {
        let Some(stream) = self.stream.as_mut() else {
            return Response::failure(CODE_INVALID, "no result stream");
        };
        let drop_n = if n < 0 {
            stream.rows.len()
        } else {
            (n as usize).min(stream.rows.len())
        };
        stream.rows.drain(..drop_n);
        if stream.rows.is_empty() {
            let mut summary = std::mem::take(&mut stream.summary);
            summary.insert("has_more", false);
            self.stream = None;
            self.state = State::Ready;
            Response::success(summary)
        } else {
            let mut meta = MapValue::new();
            meta.insert("has_more", true);
            Response::success(meta)
        }
    }

    fn fail(&mut self, failure: Response) -> Turn {
        self.state = State::Failed;
        self.stream = None;
        Turn::Reply(vec![failure])
    }

    /// Single-server routing table: this server plays every role.
    fn route_table(&self, extra: &Value) -> Response {
        let Some(address) = self.config.advertised_address.clone() else {
            return Response::failure(
                CODE_INVALID,
                "server-side routing is not configured; connect with the bolt:// scheme",
            );
        };
        let db = extra
            .as_map()
            .and_then(|m| m.get_str("db"))
            .map(str::to_string)
            .or_else(|| self.config.default_db.clone())
            .unwrap_or_default();
        let server_entry = |role: &str| {
            let mut entry = MapValue::new();
            entry.insert(
                "addresses",
                Value::List(vec![Value::from(address.as_str())]),
            );
            entry.insert("role", role);
            Value::Map(entry)
        };
        let mut rt = MapValue::new();
        rt.insert("ttl", 300i64);
        rt.insert("db", db);
        rt.insert(
            "servers",
            Value::List(vec![
                server_entry("WRITE"),
                server_entry("READ"),
                server_entry("ROUTE"),
            ]),
        );
        let mut meta = MapValue::new();
        meta.insert("rt", rt);
        Response::success(meta)
    }
}

fn request_name(request: &Request) -> &'static str {
    match request {
        Request::Hello { .. } => "HELLO",
        Request::Logon { .. } => "LOGON",
        Request::Logoff => "LOGOFF",
        Request::Goodbye => "GOODBYE",
        Request::Reset => "RESET",
        Request::Run { .. } => "RUN",
        Request::Begin { .. } => "BEGIN",
        Request::Commit => "COMMIT",
        Request::Rollback => "ROLLBACK",
        Request::Discard { .. } => "DISCARD",
        Request::Pull { .. } => "PULL",
        Request::Route { .. } => "ROUTE",
        Request::Telemetry { .. } => "TELEMETRY",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(version: BoltVersion) -> SessionConfig {
        SessionConfig {
            version,
            server_agent: "Neo4j/5.4.0 (compatible; Fluree)".into(),
            connection_id: "bolt-1".into(),
            default_db: Some("test:main".into()),
            advertised_address: None,
        }
    }

    fn hello(extra: MapValue) -> Request {
        Request::Hello { extra }
    }

    fn ready_session_54() -> Session {
        let mut s = Session::new(config(BoltVersion::V5_4));
        s.on_request(hello(MapValue::new()));
        s.on_request(Request::Logon {
            auth: MapValue::new(),
        });
        assert!(s.is_ready());
        s
    }

    fn run_req(query: &str) -> Request {
        Request::Run {
            query: query.into(),
            parameters: MapValue::new(),
            extra: MapValue::new(),
        }
    }

    fn pull(n: i64) -> Request {
        let mut extra = MapValue::new();
        extra.insert("n", n);
        Request::Pull { extra }
    }

    fn three_row_stream() -> ResultStream {
        let mut summary = MapValue::new();
        summary.insert("type", "r");
        ResultStream {
            fields: vec!["x".into()],
            rows: (1..=3).map(|i| vec![Value::Integer(i)]).collect(),
            summary,
        }
    }

    #[test]
    fn bolt5_hello_requires_logon() {
        let mut s = Session::new(config(BoltVersion::V5_4));
        let turn = s.on_request(hello(MapValue::new()));
        let Turn::Reply(replies) = turn else {
            panic!("expected reply")
        };
        let Response::Success(meta) = &replies[0] else {
            panic!("expected success")
        };
        assert!(meta.get_str("server").unwrap().starts_with("Neo4j/"));
        assert!(!s.is_ready(), "5.4 needs LOGON before READY");

        let turn = s.on_request(Request::Logon {
            auth: MapValue::new(),
        });
        assert_eq!(turn, Turn::Reply(vec![Response::success_empty()]));
        assert!(s.is_ready());
    }

    #[test]
    fn bolt44_hello_goes_straight_to_ready() {
        let mut s = Session::new(config(BoltVersion::V4_4));
        s.on_request(hello(MapValue::new()));
        assert!(s.is_ready());
    }

    #[test]
    fn run_pull_all_round_trip() {
        let mut s = ready_session_54();
        let Turn::Execute(run) = s.on_request(run_req("MATCH (n) RETURN n.x AS x")) else {
            panic!("expected execute")
        };
        assert_eq!(run.db.as_deref(), Some("test:main"));

        let success = s.run_succeeded(three_row_stream(), 1);
        let Response::Success(meta) = &success else {
            panic!()
        };
        assert_eq!(
            meta.get("fields"),
            Some(&Value::List(vec![Value::from("x")]))
        );

        let Turn::Reply(replies) = s.on_request(pull(-1)) else {
            panic!()
        };
        assert_eq!(replies.len(), 4); // 3 records + summary
        let Response::Success(summary) = replies.last().unwrap() else {
            panic!()
        };
        assert_eq!(summary.get_str("type"), Some("r"));
        assert_eq!(summary.get("has_more"), Some(&Value::Boolean(false)));
    }

    #[test]
    fn pull_batches_with_has_more() {
        let mut s = ready_session_54();
        s.on_request(run_req("q"));
        s.run_succeeded(three_row_stream(), 0);

        let Turn::Reply(first) = s.on_request(pull(2)) else {
            panic!()
        };
        assert_eq!(first.len(), 3); // 2 records + has_more success
        let Response::Success(meta) = first.last().unwrap() else {
            panic!()
        };
        assert_eq!(meta.get("has_more"), Some(&Value::Boolean(true)));

        let Turn::Reply(rest) = s.on_request(pull(2)) else {
            panic!()
        };
        assert_eq!(rest.len(), 2); // 1 record + final summary
        let Response::Success(summary) = rest.last().unwrap() else {
            panic!()
        };
        assert_eq!(summary.get("has_more"), Some(&Value::Boolean(false)));
    }

    #[test]
    fn discard_finishes_stream() {
        let mut s = ready_session_54();
        s.on_request(run_req("q"));
        s.run_succeeded(three_row_stream(), 0);
        let mut extra = MapValue::new();
        extra.insert("n", -1i64);
        let Turn::Reply(replies) = s.on_request(Request::Discard { extra }) else {
            panic!()
        };
        let Response::Success(summary) = &replies[0] else {
            panic!()
        };
        assert_eq!(summary.get_str("type"), Some("r"));
        // Next RUN is accepted again.
        assert!(matches!(s.on_request(run_req("q2")), Turn::Execute(_)));
    }

    #[test]
    fn begin_fails_clearly_and_reset_recovers() {
        let mut s = ready_session_54();
        let Turn::Reply(replies) = s.on_request(Request::Begin {
            extra: MapValue::new(),
        }) else {
            panic!()
        };
        let Response::Failure { code, message } = &replies[0] else {
            panic!()
        };
        assert_eq!(code, "Neo.ClientError.Statement.TypeError");
        assert!(message.contains("not supported"));

        // Everything is IGNORED until RESET.
        assert_eq!(
            s.on_request(run_req("q")),
            Turn::Reply(vec![Response::Ignored])
        );
        assert_eq!(
            s.on_request(Request::Reset),
            Turn::Reply(vec![Response::success_empty()])
        );
        assert!(matches!(s.on_request(run_req("q")), Turn::Execute(_)));
    }

    #[test]
    fn run_failure_ignores_pipelined_pull() {
        let mut s = ready_session_54();
        s.on_request(run_req("bad query"));
        let failure = s.run_failed("Neo.ClientError.Statement.SyntaxError", "parse error");
        assert!(matches!(failure, Response::Failure { .. }));
        // The pipelined PULL that followed the RUN must be IGNORED.
        assert_eq!(s.on_request(pull(-1)), Turn::Reply(vec![Response::Ignored]));
    }

    #[test]
    fn reset_mid_stream_drops_result() {
        let mut s = ready_session_54();
        s.on_request(run_req("q"));
        s.run_succeeded(three_row_stream(), 0);
        s.on_request(Request::Reset);
        // Stream gone: PULL now fails (READY state).
        let Turn::Reply(replies) = s.on_request(pull(-1)) else {
            panic!()
        };
        assert!(matches!(replies[0], Response::Failure { .. }));
    }

    #[test]
    fn db_precedence_run_extra_over_hello_over_default() {
        let mut s = Session::new(config(BoltVersion::V4_4));
        let mut hello_extra = MapValue::new();
        hello_extra.insert("db", "hello:db");
        s.on_request(hello(hello_extra));

        let Turn::Execute(run) = s.on_request(run_req("q")) else {
            panic!()
        };
        assert_eq!(run.db.as_deref(), Some("hello:db"));

        let mut run_extra = MapValue::new();
        run_extra.insert("db", "run:db");
        let Turn::Execute(run) = s.on_request(Request::Run {
            query: "q".into(),
            parameters: MapValue::new(),
            extra: run_extra,
        }) else {
            panic!()
        };
        assert_eq!(run.db.as_deref(), Some("run:db"));
    }

    #[test]
    fn goodbye_closes_without_reply() {
        let mut s = ready_session_54();
        assert_eq!(s.on_request(Request::Goodbye), Turn::Close(vec![]));
    }

    #[test]
    fn hello_out_of_order_closes() {
        let mut s = ready_session_54();
        let turn = s.on_request(hello(MapValue::new()));
        assert!(matches!(turn, Turn::Reply(ref r) if matches!(r[0], Response::Failure { .. })));
    }

    #[test]
    fn route_without_advertised_address_fails() {
        let mut s = ready_session_54();
        let Turn::Reply(replies) = s.on_request(Request::Route {
            routing: MapValue::new(),
            bookmarks: vec![],
            extra: Value::Null,
        }) else {
            panic!()
        };
        assert!(matches!(replies[0], Response::Failure { .. }));
    }

    #[test]
    fn route_with_advertised_address_returns_single_entry_table() {
        let mut cfg = config(BoltVersion::V5_4);
        cfg.advertised_address = Some("db.example.com:7687".into());
        let mut s = Session::new(cfg);
        s.on_request(hello(MapValue::new()));
        s.on_request(Request::Logon {
            auth: MapValue::new(),
        });
        let Turn::Reply(replies) = s.on_request(Request::Route {
            routing: MapValue::new(),
            bookmarks: vec![],
            extra: Value::Null,
        }) else {
            panic!()
        };
        let Response::Success(meta) = &replies[0] else {
            panic!()
        };
        let Some(Value::Map(rt)) = meta.get("rt") else {
            panic!("no rt")
        };
        assert_eq!(rt.get_int("ttl"), Some(300));
        let Some(Value::List(servers)) = rt.get("servers") else {
            panic!()
        };
        assert_eq!(servers.len(), 3);
    }
}
