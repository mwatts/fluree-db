# Connecting with Neo4j drivers (Bolt)

Fluree serves its [openCypher surface](../query/cypher.md) over the Bolt
protocol, so official Neo4j drivers and Bolt-compatible tools connect
directly. This guide covers enabling the listener and connecting from
driver code; protocol details live in the [Bolt reference](../api/bolt.md).

## Enable the listener

Bolt is compiled in by default but binds only when an address is
configured (conventional port 7687):

```bash
fluree-server \
  --bolt-listen-addr 0.0.0.0:7687 \
  --bolt-default-db mydb:main
```

The CLI's server management exposes the same flags on `run`, `start`,
and `restart` (background servers keep them across restarts, and
`fluree server status` shows the Bolt address):

```bash
fluree server start --bolt-listen-addr 0.0.0.0:7687 --bolt-default-db mydb:main
```

or in the config file:

```toml
[server.bolt]
listen_addr = "0.0.0.0:7687"
default_db = "mydb:main"
```

| Setting | Flag / Env | Meaning |
|---|---|---|
| `listen_addr` | `--bolt-listen-addr` / `FLUREE_BOLT_LISTEN_ADDR` | Address to serve Bolt on. Unset = disabled. |
| `default_db` | `--bolt-default-db` / `FLUREE_BOLT_DEFAULT_DB` | Ledger served to sessions that select no database. |

Sessions pick their ledger with the driver's standard database
selection; `default_db` is the fallback. A session with neither fails
its queries with a clear error.

The v1 listener is **unauthenticated** — any credentials (or none) are
accepted. It refuses to start when the server requires data-plane auth
(`data_auth_mode=required`) rather than silently bypassing it. Front
with a TCP proxy for TLS if the wire crosses trust boundaries.

## Connect (Python)

```python
from neo4j import GraphDatabase

# bolt:// (direct) scheme — neo4j:// routing is not served.
driver = GraphDatabase.driver("bolt://localhost:7687", auth=None)

with driver.session(database="mydb:main") as session:
    # Autocommit query with parameters.
    result = session.run(
        "MATCH (p:Person) WHERE p.age > $min RETURN p.name AS name",
        min=30,
    )
    for record in result:
        print(record["name"])

    # Nodes come back typed: labels + properties + elementId (the IRI).
    node = session.run("MATCH (p:Person) RETURN p LIMIT 1").single()["p"]
    print(node.labels, dict(node), node.element_id)

    # Managed transaction function — retried automatically on
    # concurrent-write conflicts.
    def add_person(tx):
        tx.run("CREATE (n:Person {name: $name})", name="Ada").consume()
        return tx.run("MATCH (n:Person) RETURN count(n) AS c").single()["c"]

    count = session.execute_write(add_person)
```

## Connect (JavaScript)

```javascript
const neo4j = require("neo4j-driver");
const driver = neo4j.driver("bolt://localhost:7687");

const session = driver.session({ database: "mydb:main" });
const result = await session.executeWrite(async (tx) => {
  await tx.run("CREATE (n:Person {name: $name})", { name: "Ada" });
  const res = await tx.run("MATCH (n:Person) RETURN count(n) AS c");
  return res.records[0].get("c");
});
await session.close();
```

## Transactions and retries

Explicit transactions are optimistic: statements execute against the
state the transaction began on (with read-your-writes), and `COMMIT`
succeeds only if no concurrent write landed in between. On conflict the
driver receives a `Neo.TransientError.*` failure — **use transaction
functions** (`execute_read` / `execute_write` / `executeQuery`), which
retry automatically; hand-rolled `begin_transaction()` code must be
prepared to retry on transient errors. Explicit transactions require a
single-node server; Raft/peer deployments reject `BEGIN` (autocommit
works everywhere).

## Troubleshooting

- **Connection refused** — the listener only binds when
  `bolt_listen_addr` is set; check the startup log for
  `Bolt listener starting`.
- **"unsupported protocol version" / immediate close** — the driver
  proposed only versions outside 4.4/5.0–5.4 (very old drivers).
  Upgrade the driver.
- **`neo4j://` scheme fails** — use `bolt://`; server-side routing
  tables are not served.
- **BEGIN fails with "single-node server"** — the deployment replicates
  writes (Raft) or is a peer; use autocommit queries.
- **Numbers look different from the JSON API** — `xsd:decimal` is
  rendered as Float over Bolt (Neo4j parity); the JSON transport keeps
  exact decimal strings. See the
  [support matrix](../reference/cypher-support-matrix.md#bolt-protocol-neo4j-drivers).
- **Bare `MATCH (n)` rejected** — whole-graph scans are opt-in via
  `FLUREE_CYPHER_ALLOW_FULL_SCAN=1`, same as over HTTP.
