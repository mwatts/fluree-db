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
| `MERGE` (relationship) | ◑ | Standalone + bound-endpoint forms; `ON CREATE SET`. Deferred: `ON MATCH SET` on a relationship MERGE, property-bearing rel MERGE (`-[:T {p:v}]->`), multi-hop / multi-part MERGE, multiple `MERGE` clauses, and combining a MERGE with a non-`SET` write clause. |
| `SET` / `REMOVE` | ✅ | Properties, `+=` map merge, labels. The map side of `SET n = …` / `SET n += …` may be a whole-map parameter (`SET n += $props`). |
| `DELETE` / `DETACH DELETE` | ✅ | |
| `FOREACH` | ⏳ | |
| `CALL proc(...) YIELD` | ⏳ | Stored/builtin procedures (distinct from `CALL { … }`). |
| `LOAD CSV` | ⏳ | Bulk CSV import exists via the CLI, not the `LOAD CSV` clause. |
| Multi-statement (`;`) | ⏳ | One statement per request. |

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
| Unbounded var-length `-[:T*]->` | ⟂ | **Reachability** (one row per reachable endpoint), not path enumeration. |
| Untyped var-length `-[*m..n]->` | ⟂ | Wildcard reachability over node→node edges; excludes `rdf:type`/`f:reifies*`. A direction is required (`-[*]-` deferred); an unbounded lower bound above 1 (`-[*2..]->`) deferred — give an upper bound or name a type. |
| Bounded var-length **binding** `-[r:T*m..n]->` / `p = …` | ✅ | `r` = rel list, `p` = path; via per-branch construction. |
| Unbounded var-length binding | ⏳ | Needs a path-enumeration operator. |
| `shortestPath` / `allShortestPaths` | ✅ | Anchored; single typed predicate or the untyped wildcard form (`shortestPath((a)-[*..15]->(b))`, same edge-set as untyped var-length); `All` emits one row per minimal path. Type alternation deferred. |
| `relationships(p)` / `nodes(p)` / `pathPairs(p)` / `length(p)` | ✅ | `relationships(p)` carries the stored edge orientation. |
| Bounded type-alternation var-length `-[:A\|B*1..3]->` | ⏳ | Use the unbounded form. |
| Undirected **unbounded** path `-[:T*]-` | ⏳ | |
| Free path value `MATCH p = (...)` without a `shortestPath`/`allShortestPaths` wrapper | ⏳ | Wrap with a path-finding function. |
| Zero-length *typed* bounded path `-[:T*0..M]->` | ⏳ | Use `*1..M`. |
| Property filter on a var-length / `shortestPath` relationship | ⏳ | |

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
| `NULL` literal | ⏳ | Use an absent value / `IS NULL`. |
| Property access `n.prop` | ◑ | Bare-variable target; chained `n.a.b` deferred — except temporal-field chains like `x.date.month`, which lower to an extraction function. |
| List literal / indexing `[a,b]`, `list[i]` | ✅ | Negative index from end. |
| Map literal `{k: v}` | ✅ | Key-order-insensitive identity (⟂ vs strict insertion order for equality). |
| Map projection `n{.k, .*, x: e}` | ◑ | Mixing `.*` with other selectors deferred. |
| List comprehension / `reduce` / `all·any·none·single` | ✅ | Loop-local property access supported. |
| Pattern comprehension `[(a)-->(b) \| e]` | ✅ | Correlated; reuses the EXISTS path. |
| `EXISTS { … }` (predicate + value) | ✅ | Incl. inside map/projection entries. |
| Parameters `$p` | ✅ | Scalars, lists, maps; substituted everywhere incl. inside `CALL`/patterns. |

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
| Temporal constructors `date()` `datetime()` `duration()` | ◑ | A constant lexical argument folds to a typed value (`date('2024-01-15')`, `datetime('…T…Z')`, `time('…')`, `duration('P1D')` — durations pick the narrowest orderable XSD type), in reads and as write property values. Zero-arg `datetime()` / `date()` = current instant / date (one instant per write statement). Deferred: non-constant arguments, component-map constructors (`date({year: …})`), duration arithmetic (`date + duration`), zero-arg `time()`/`localdatetime()`. |
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
