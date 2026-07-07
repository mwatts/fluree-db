# openCypher Support Matrix

A tracked feature matrix for Fluree's [openCypher](../query/cypher.md)
surface, against **openCypher 9** (the Cypher 9 language reference the
[openCypher TCK](https://github.com/opencypher/openCypher) exercises). For
syntax and semantics see [Cypher (concept)](../query/cypher.md); for recipes
see the [Cypher cookbook](../guides/cookbook-cypher.md).

## Legend

| Mark | Meaning |
|------|---------|
| ✅ | **Supported** — works as openCypher 9 specifies. |
| ◑ | **Partial** — a common subset works; specific forms are deferred (noted). |
| ⟂ | **Divergent by design** — intentionally different because Fluree is multi-modal graph. Rejected-or-adapted, never silently wrong. |
| ⏳ | **Deferred** — not yet implemented; rejected with a clear error. |

**Guiding invariant:** an unsupported construct produces a *clear error*, never a
silently wrong result. Divergences (⟂) are where openCypher's LPG assumptions
meet Fluree's RDF model.

## Core model divergences (⟂)

These shape everything below; read them first.

- **Nodes are durable subjects.** A node is an RDF subject, not an opaque LPG
  node. `labels(n)` are `rdf:type` assertions; node identity is the subject's
  stored name — a plain name by default, a full IRI in `@vocab` (RDF-compat)
  mode (see [names and IRIs](../query/cypher.md#names-and-opting-into-iris)).
- **Relationships are edge annotations.** A relationship is the base triple
  `(s, p, o)`; binding `-[r:T]->` reifies it into an `f:reifies*` annotation node
  (the LPG edge identity). Fluree does **not** implement RDF-star triple terms —
  see [Edge annotations](../concepts/edge-annotations.md).
- **`id(n)` / `elementId(n)`** return the node/relationship **identity string** — there
  is no integer element id.
- **No implicit per-statement transaction id** semantics; immutability/time-travel
  replace it (`f:t`, history queries).

## Clauses

| Clause | Status | Notes |
|--------|:------:|-------|
| `MATCH` | ✅ | Node/relationship patterns, `WHERE`. |
| `OPTIONAL MATCH` | ✅ | Nullable bindings; poisoned-binding semantics. |
| `WITH` | ◑ | Projection boundary; `WHERE`→HAVING when it references aggregates; `DISTINCT`/`ORDER BY`/`SKIP`/`LIMIT`; `collect()` carries forward as a list. Before a **write** clause only the pass-through / rename / computed-alias / filter subset is allowed (no aggregation / `DISTINCT` / `ORDER BY` / `SKIP` / `LIMIT`); `WITH` before `DELETE` deferred. |
| `UNWIND` | ✅ | Inline lists and runtime list expressions; `$param` lists via API substitution. Before a **write** clause: `$param` lists, inline literal lists, and constant `range()` (≤ 10000 rows). |
| `RETURN` | ✅ | `*`, aliases, `DISTINCT`, `ORDER BY`/`SKIP`/`LIMIT`. `SKIP`/`LIMIT` must be literal integers. |
| `RETURN` on a write (`CREATE … RETURN n`) | ◑ | Bare created-entity variables (a fresh CREATE node or relationship variable), optionally aliased; one row per WHERE solution. Deferred: expressions, RETURN modifiers, MATCH-bound variables, MERGE. |
| `UNION` / `UNION ALL` | ✅ | Column-name-match + uniform-variant rules enforced. |
| `CALL { … }` (subquery) | ✅ | Imports `(a,b)` / `(*)`, uncorrelated broadcast, inner `UNION`, nesting, strict scope/shadowing, correlated-aggregate soundness. |
| `CREATE` | ✅ | Nodes + relationships (relationships reify). Bare `CREATE ()` / `CREATE (n)` asserts a hidden `db:Node` existence marker (invisible to `labels()`). |
| `MERGE` (node) | ✅ | Find-or-create with `ON CREATE SET` / `ON MATCH SET`, and trailing `SET` clauses that apply on both branches — the upsert idiom `MERGE (n:User {id: $id}) SET n += $props` works. |
| `MERGE` (relationship) | ◑ | Standalone + bound-endpoint forms; property-bearing patterns (`-[:T {p:v}]->` matches on the annotation values — a different value creates a parallel edge); `ON CREATE SET` on endpoint and relationship variables; `ON MATCH SET` / trailing `SET` on the **standalone** form (probe-then-branch). Deferred: `ON MATCH SET` on the per-row form (leading `MATCH` — rows can mix create/match), multi-hop / multi-part MERGE, multiple `MERGE` clauses, and combining a MERGE with a non-`SET` write clause. |
| `SET` / `REMOVE` | ✅ | Properties, `+=` map merge, labels. The map side of `SET n = …` / `SET n += …` may be a whole-map parameter (`SET n += $props`). |
| `DELETE` / `DETACH DELETE` | ✅ | |
| `FOREACH` | ◑ | Constant lists (inline literals, constant `range()`, `$param` arrays) unroll at parse time — bodies of `CREATE` / `SET` / `REMOVE`, ≤ 10000 iterations; `SET` on the same property applies last-wins (sequential semantics). Deferred: runtime lists (e.g. a collected list), `MERGE`/`DELETE`/nested `FOREACH` bodies. |
| `CALL proc(...) YIELD` | ◑ | Introspection shims answered from ledger stats (novelty-merged, no scan): `db.labels`, `db.relationshipTypes`, `db.propertyKeys`, `db.schema.visualization` (best effort), `dbms.components`, `apoc.meta.data` (covers the LangChain `Neo4jGraph` schema queries). After the `YIELD` the statement continues like any read (`WHERE` / `WITH` / `UNWIND` / `MATCH` / `RETURN`); first-clause standalone only. Other/user procedures error with the supported list. |
| `LOAD CSV` | ⏳ | Bulk CSV import exists via the CLI (`fluree create --from *.csv` / `*.cypher`), not the `LOAD CSV` clause. |
| `CREATE / DROP INDEX \| CONSTRAINT` | ⟂ | Accepted as **no-op writes** (commits nothing): Fluree indexes everything and has no user-managed index/constraint catalog. Keeps framework migrations (spring-data, neo4j-migrations) from crashing at startup. |
| `SHOW INDEXES / SHOW CONSTRAINTS` | ⟂ | Answer **zero rows** (shared Neo4j column heads), for the same reason. |
| Multi-statement (`;`) | ◑ | The transact API (`transact_cypher*`) accepts `;`-separated write scripts: sequential autocommit, one commit per statement, later statements see earlier effects, final statement may `RETURN` (cypher-shell semantics; for atomicity use an explicit Bolt transaction). A lone trailing `;` on any statement is accepted. Transports stay one statement per Bolt `RUN` / query request (Neo4j parity — drivers and Browser split client-side). |

## Patterns & paths

| Feature | Status | Notes |
|---------|:------:|-------|
| Node pattern (labels, inline props) | ✅ | |
| Bare unconstrained `MATCH (n)` | ◑ | Rejected by default (a node must be constrained by a label, property, or relationship). Opt in to a whole-graph distinct-subject scan with the server flag `FLUREE_CYPHER_ALLOW_FULL_SCAN=1`. |
| Directed typed relationship `-[:T]->`, `<-[:T]-` | ✅ | |
| Type alternation `-[:A\|B]->` | ✅ | `Union` of concrete predicates. |
| Undirected `-[:T]-` | ✅ | Forward ∪ reverse `Union`. |
| Untyped relationship `-->` / `-[r]->` | ✅ | Follows relationships only: `rdf:type`, `f:reifies*`, and data properties (literal objects) are excluded from the hop. |
| Bounded var-length `-[:T*m..n]->` | ✅ | **Enumerates trails** (one row per path, relationship-uniqueness). |
| Unbounded var-length `-[:T*]->` | ⟂ | Without a binding: **reachability** (one row per reachable endpoint). With a rel/path binding (`-[r:T*]->` / `p = …`): **enumerates node-distinct paths** (one row each; end node bound per path or filtered when already bound). Node-distinctness stands in for Cypher's relationship-uniqueness — a walk revisiting a node is not enumerated. Guarded by visited/path caps that error (never silently truncate). |
| Untyped var-length `-[*m..n]->` | ⟂ | Wildcard reachability over node→node edges; excludes `rdf:type`/`f:reifies*`. A direction is required (`-[*]-` deferred); an unbounded lower bound above 1 (`-[*2..]->`) deferred unless a rel/path binding makes it enumerate — give an upper bound, name a type, or bind. |
| Bounded var-length **binding** `-[r:T*m..n]->` / `p = …` | ✅ | `r` = rel list, `p` = path; via per-branch construction. |
| Unbounded var-length binding `-[r:T*]->` / `p = (a)-[:T*]->(b)` | ✅ | Enumerate-mode path search: `p` = path value, `r` = `relationships(p)`. Works typed or untyped (wildcard), any direction, lower bounds ≥ 0, bound or free end. Type alternation deferred. |
| `shortestPath` / `allShortestPaths` | ✅ | Anchored; single typed predicate or the untyped wildcard form (`shortestPath((a)-[*..15]->(b))`, same edge-set as untyped var-length); `All` emits one row per minimal path. Type alternation deferred. |
| `relationships(p)` / `nodes(p)` / `pathPairs(p)` / `length(p)` | ✅ | `relationships(p)` carries the stored edge orientation. |
| Bounded type-alternation var-length `-[:A\|B*1..3]->` | ⏳ | Use the unbounded form. |
| Undirected **unbounded** path `-[:T*]-` | ◑ | With a rel/path binding: enumerates. Without one, use a bounded range (reachability operator is single-direction). |
| Free path value `MATCH p = (...)` | ◑ | A single relationship segment — fixed or variable-length (bounded via per-branch construction, unbounded via enumeration) — or a **multi-hop chain of fixed single-typed directed hops** (`p = (a)-[:R1]->(b)<-[:R2]-(c)`; the path value interleaves the bound nodes with per-hop relationship values in stored orientation). Deferred: a variable-length or undirected segment inside a multi-hop path value. |
| Zero-length *typed* bounded path `-[:T*0..M]->` | ◑ | With a rel/path binding: enumerates (the zero-length path binds the end to the start with an empty rel list). Without a binding, use `*1..M`. |
| Property filter on a var-length / `shortestPath` relationship | ◑ | Bounded single-typed directed ranges (`-[:T*1..3 {p: v}]->`) filter per hop on the edge-annotation values. Deferred: unbounded/untyped/undirected ranges, combining the filter with a rel/path binding, and `shortestPath` relationships. |

## Expressions & operators

| Feature | Status | Notes |
|---------|:------:|-------|
| Arithmetic `+ - * / %`, unary `-` | ✅ | `/` of integers → `xsd:decimal` (rendered as a string for precision). |
| Exponentiation `^` | ✅ | Right-associative. |
| Comparison `= <> < <= > >=` | ✅ | |
| Boolean `AND` / `OR` / `XOR` / `NOT` | ✅ | |
| `STARTS WITH` / `ENDS WITH` / `CONTAINS` | ✅ | |
| `x IN [ … ]` | ✅ | |
| `IS NULL` / `IS NOT NULL` | ✅ | |
| `CASE` (simple + generic) | ✅ | Aggregates inside `CASE` deferred; `CASE`/`EXISTS` inside a write-statement `MATCH … WHERE` deferred. |
| `NULL` literal | ✅ | A first-class expression value (the unbound binding): projected as JSON null, `= null` never true, `IS [NOT] NULL` folds, `coalesce` skips it, allowed as a list element / CASE branch. |
| Property access `n.prop` | ◑ | Bare-variable target; chained `n.a.b` deferred — except temporal-field chains like `x.date.month`, which lower to an extraction function. |
| List literal / indexing `[a,b]`, `list[i]` | ✅ | Negative index from end. |
| Map literal `{k: v}` | ✅ | Key-order-insensitive identity (⟂ vs strict insertion order for equality). |
| Map projection `n{.k, .*, x: e}` | ◑ | Mixing `.*` with other selectors deferred. |
| List comprehension / `reduce` / `all·any·none·single` | ✅ | Loop-local property access supported. |
| Pattern comprehension `[(a)-->(b) \| e]` | ✅ | Correlated; reuses the EXISTS path. |
| `EXISTS { … }` (predicate + value) | ✅ | Incl. inside map/projection entries. |
| Parameters `$p` | ✅ | Scalars, lists, maps; substituted everywhere incl. inside `CALL`/patterns. |
| Keyword-as-identifier (`AS end`, `WITH count(*) AS count … RETURN count`) | ✅ | Deliberate leniency (⟂): reserved words usable as aliases/variables unescaped; backticks also accepted. `count(*)`/`exists {}`/`all()` keep their construct meaning when followed by their delimiter. |

## Functions

| Group | Status | Notes |
|-------|:------:|-------|
| Casts: `toString` `toInteger` `toFloat` | ✅ | `toFloat`→xsd:double. |
| String: `toUpper` `toLower` `substring` `left` `right` `trim` `ltrim` `rtrim` `replace` `split` `reverse` | ✅ | `substring` 0-indexed; `replace` literal. |
| Math: `abs` `round` `floor` `ceil` `sign` `sqrt` `log` `rand` | ✅ | `log` = natural log. |
| `coalesce` | ✅ | |
| Aggregates: `count` `sum` `avg` `min` `max` `collect` (+ `DISTINCT`) | ✅ | Implicit grouping by non-aggregate projections; HAVING via `WITH`. |
| List: `size` `head` `last` `tail` `reverse` `range` | ✅ | |
| Path/metadata: `length` `nodes` `relationships` `pathPairs` `labels` `type` `startNode` `endNode` `keys` `properties` | ✅ | |
| `id` / `elementId` | ⟂ | Returns the identity string (name, or IRI in `@vocab` mode). |
| Temporal accessors `<date>.year/.month/.day/.hour/.minute/.second` | ✅ | |
| Temporal constructors `date()` `datetime()` `duration()` | ◑ | A constant lexical argument folds to a typed value (`date('2024-01-15')`, `datetime('…T…Z')`, `time('…')`, `duration('P1D')` — durations pick the narrowest orderable XSD type), in reads and as write property values. Zero-arg `datetime()` / `date()` = current instant / date (one instant per write statement). Component maps with constant fields fold too (`date({year: 2024, month: 1})`, `datetime({…, timezone: '+02:00'})`, `duration({days: 3, hours: 12})`); zero-arg `localdatetime()` = current instant. Deferred: non-constant arguments, duration arithmetic (`date + duration`), zero-arg `time()`/`localtime()`. |
| Spatial `point()` / `distance()` | ⏳ | |

## Null & type semantics

| Aspect | Status | Notes |
|--------|:------:|-------|
| Three-valued logic in `WHERE` / filters | ✅ | Unbound comparison → filter-false. |
| Null propagation through arithmetic / functions | ✅ | |
| `IS NULL` for absent property (nullable accessor) | ✅ | `OPTIONAL`-wrapped accessor. |
| Mixed-representation equality (encoded vs decoded) | ✅ | Normalized at DISTINCT/GROUP BY/join/MINUS/VALUES. |
| `xsd:float` string-backed numeric coercion | ✅ | In SUM/AVG, comparisons, math. |
| List / map ordering in `ORDER BY` | ⏳ | `ORDER BY <list/map>` rejected (defensive total order internally). |

## Bolt protocol (Neo4j drivers)

The server accepts official Neo4j drivers (`bolt://` scheme) against
the openCypher surface — versions 4.4 and 5.0–5.4, autocommit and
explicit transactions. See the [Bolt guide](../guides/bolt.md) and
[Bolt reference](../api/bolt.md). Transport-specific semantics:

| Aspect | Status | Notes |
|--------|:------:|-------|
| Autocommit `RUN` (read + write), params, reactive `PULL`/`DISCARD` | ✅ | Same execution paths as the HTTP routes. |
| Explicit transactions (`BEGIN`/`COMMIT`/`ROLLBACK`) | ✅ | Optimistic: `BEGIN` pins the head; statements stage privately (read-your-writes; statement errors surface at `RUN`, poisoning the transaction until `RESET`); `COMMIT` publishes atomically only if the head is still the pinned base, else fails `Neo.TransientError.*` — managed transaction functions (`execute_write` etc.) retry automatically. Isolation is serializable-against-base, stronger than Neo4j's read-committed. Single-node (local commit) deployments only; Raft/peer reject `BEGIN` clearly. |
| `db` selection | ✅ | Driver `database=` (HELLO defaults or per-RUN) → ledger id; fallback `--bolt-default-db`. |
| `xsd:decimal` values | ⟂ | Bolt/PackStream has no decimal type: rendered as **Float** (Neo4j parity, precision loss). The JSON transport keeps exact lexical strings. Integer `/` produces decimals, so this shows on ordinary division. |
| Temporal values (`xsd:date` / `dateTime` / `time`) | ✅ | Bolt `Date` / `DateTime` / `Time` structures (4.4 gets the legacy local-seconds `DateTime`; lexical forms without a timezone map to the Local variants). |
| Node values (`RETURN n`) | ✅ | Bolt `Node` structures: `element_id` = the durable identity string (name, or full IRI in `@vocab` mode), numeric `id` = stable hash of the IRI (opaque handle), labels via the `labels()` rule (`db:Node` marker hidden), properties fetched per node at format time — **literal-valued predicates only** (multi-valued become lists); ref-valued predicates are relationships and never inline into the node map (Neo4j parity). Under a view policy the hydration filters per flake through the same enforcer as the scan path. |
| Relationship / path values | ✅ | Bolt `Relationship` (endpoints + type + annotation properties when reified; synthesized stable `id` otherwise) and `Path` structures (unique node/rel lists + walk indices). |
| Auth | ✅ | `bearer` (data-plane JWT/JWS, same tokens/issuers as HTTP), `basic` as a token carrier (password = token), `none` when auth isn't required. Ledger scopes + token expiry re-checked per statement; identity drives in-ledger policy. See the [Bolt reference](../api/bolt.md#authentication). |
| `ROUTE` / cluster routing | ⏳ | Use the `bolt://` (direct) scheme; `neo4j://` routing answers a failure unless an advertised address is configured. |

## Maintaining this matrix

This is a hand-maintained matrix, not yet a TCK-driven report. When a Cypher
feature lands or a divergence changes:

1. Update the relevant row here **and** the [concept doc](../query/cypher.md).
2. Prefer ⟂ over ⏳ when the divergence is an intentional RDF-model choice —
   and record *why* in the Notes column.

A future step is to wire a subset of the openCypher TCK `.feature` scenarios as
executable tests and generate the supported/deferred columns from their pass/fail
results, replacing the hand-maintained status marks.
