# Bolt protocol

Fluree speaks the Bolt protocol — Neo4j's binary wire protocol — against
its [openCypher surface](../query/cypher.md). Official Neo4j drivers
(Python, JavaScript, Java, Go, .NET) and most Bolt-compatible tooling
connect unmodified. For setup and driver examples see the
[Bolt guide](../guides/bolt.md); for per-feature semantics see the
[support matrix's Bolt section](../reference/cypher-support-matrix.md#bolt-protocol-neo4j-drivers).

## Protocol surface

| Aspect | Supported |
|---|---|
| Versions | 4.4 and 5.0–5.4 (4-slot handshake negotiation; version-range proposals honored) |
| Session | `HELLO` (+ `LOGON`/`LOGOFF` on 5.1+), `RESET`, `GOODBYE`, NOOP keep-alives |
| Queries | Autocommit `RUN` with parameters; reactive `PULL {n}` / `DISCARD` batching with `has_more` |
| Transactions | `BEGIN` / `COMMIT` / `ROLLBACK`, multiple open results per transaction addressed by `qid` |
| Database selection | `db` in HELLO defaults or per-`RUN`/`BEGIN` extra → ledger id; server-wide default via `--bolt-default-db` |
| Auth | v1 runs open: `none` accepted, any credentials accepted. The listener refuses to start when `data_auth_mode=required` rather than bypass it. |
| Routing | Direct (`bolt://`) scheme. `ROUTE` answers a failure unless an advertised address is configured. |
| TLS | Not terminated by the server — front with a proxy if needed. |

The server agent string is `Neo4j/5.4.0 (compatible; Fluree/<version>)`;
drivers parse the `Neo4j/<semver>` prefix for feature gating.

## Value mapping

Results carry typed PackStream values, sharing the per-binding semantic
conversion with the Cypher JSON transport:

- **Nodes** (`RETURN n`): Bolt `Node` structures. `element_id` is the
  node's full IRI (the durable identity); the numeric `id` is a stable
  hash of it (an opaque handle — use `element_id` for identity). Labels
  follow the `labels()` rule (`rdf:type` local names; the `db:Node`
  existence marker is hidden). Properties are fetched per node at result
  time; multi-valued literal predicates become lists. Ref-valued
  predicates are **relationships, not node properties** (Neo4j parity):
  they never inline into the node's property map — bind a relationship
  or path variable to read them.
- **Relationships**: endpoints, type (predicate local name), and
  annotation properties when the edge is reified; a reified edge's
  `element_id` is the reifier IRI, otherwise a stable synthetic id.
- **Paths**: Bolt `Path` structures (unique node/relationship lists plus
  walk indices).
- **Temporal values**: `xsd:date`/`dateTime`/`time` map to Bolt
  `Date`/`DateTime`/`Time` structures (4.4 sessions get the legacy
  local-seconds `DateTime`; lexical forms without a timezone map to the
  `Local*` variants).
- **`xsd:decimal`**: rendered as Float — Neo4j parity, with precision
  loss; the JSON transport keeps exact lexical strings instead. Integer
  division produces decimals, so this shows up on ordinary `a / b`.

## Explicit transactions

Transactions are optimistic and serializable against the state they
began on — stronger isolation than Neo4j's read-committed:

- `BEGIN` pins the ledger's current head as the transaction's **base**.
- Each write statement stages privately, one commit per statement; reads
  inside the transaction see earlier statements (read-your-writes).
  Statement errors surface at `RUN` time and poison the transaction
  until `RESET`, matching Bolt semantics.
- `COMMIT` publishes atomically **only if the head is still the base**;
  intermediate states are never observable. If a concurrent write moved
  the head, the whole transaction fails with a `Neo.TransientError.*`
  code — official drivers' managed transaction functions
  (`execute_read`/`execute_write`, `driver.execute_query`) retry the
  transaction automatically, which is the intended recovery path.
- `COMMIT` returns a `bookmark` (`fluree:t:<t>`).

Constraint: explicit transactions require a single-node server (the
local commit path). Raft and peer deployments reject `BEGIN` with a
clear error; autocommit queries work everywhere.

## Implementation notes

The protocol lives in the `fluree-db-bolt` crate — a pure, IO-free
codec and session state machine (PackStream, chunked framing, handshake
negotiation) with no Fluree dependencies, unit-tested against byte
fixtures. The TCP listener and execution glue live in
`fluree-db-server/src/bolt.rs` and reuse the HTTP routes' entry points:
`query_cypher_with_params` for reads, the consensus submit path for
autocommit writes, and `fluree-db-api/src/cypher_txn.rs` for explicit
transactions (its module docs describe the commit model in detail).
End-to-end tests: `fluree-db-server/tests/bolt_integration.rs`;
official-driver smoke script: `fluree-db-server/tests/bolt_driver_smoke.py`.
