# fluree load

Stream a local CSV file into a ledger, one row at a time, through a Cypher
upsert template. This is Fluree's analog to Cypher's `LOAD CSV`.

## Usage

```bash
fluree load [LEDGER] --from <FILE.csv> --cypher <TEMPLATE> [OPTIONS]
```

## Arguments

| Argument | Behavior |
|----------|----------|
| `[LEDGER]` | Target ledger (defaults to the active ledger) |

## Options

| Option | Description |
|--------|-------------|
| `--from <PATH>` | CSV file to read (required). The first line must be a header row naming the columns. |
| `--cypher <TEMPLATE>` | Per-row Cypher template (required). Each row is bound as `row`, a map keyed by CSV column; reference cells as `row.<column>`. |
| `--batch-size <N>` | Rows per commit (default `1000`). Each batch is one transaction / one commit. |
| `--field-terminator <CHAR>` | CSV field delimiter (default `,`). Single character. |
| `--remote <NAME>` | Load into a ledger on a remote server instead of locally. |

## Description

Unlike Neo4j's `LOAD CSV`, the file is never handed to the server — the CLI
holds the file, so it reads and parses the CSV **client-side**, groups rows into
batches, and sends each batch to the ledger as a single transaction:

```cypher
UNWIND $batch AS row
<your --cypher template>
```

The `$batch` parameter carries that batch's rows; `UNWIND … AS row` binds one row
map at a time, exactly like `LOAD CSV … AS row`. The server only ever receives
ordinary parameterized Cypher writes — no server-side file access, no import
directory, no URL fetching.

Every cell arrives as a **string**; an empty cell is `null` (matching Neo4j).
Cast in the template with `toInteger(row.age)`, `toFloat(row.score)`, etc.

Writes route the same way as `fluree update`: to the local ledger by default
(auto-forwarded to a running local server unless `--direct`), or to a named
remote with `--remote`.

### Upsert semantics

The template is usually a per-row `MERGE` so re-running the load updates
existing rows instead of duplicating them:

```cypher
MERGE (n:Person {id: row.id})
SET n.name = row.name, n.age = toInteger(row.age)
```

`MERGE (n:Person {id: row.id})` matches an existing `Person` with that `id` or
creates one; the `SET` then applies on both the create and the match. Load the
same file twice and the second run is a no-op update, not a duplicate.

## Examples

```bash
# Upsert people from a CSV keyed by id
fluree load people --from people.csv \
  --cypher 'MERGE (n:Person {id: row.id}) SET n.name = row.name, n.age = toInteger(row.age)'

# Tab-separated, 5000 rows per commit
fluree load metrics --from metrics.tsv --field-terminator '\t' --batch-size 5000 \
  --cypher 'CREATE (m:Reading {sensor: row.sensor, value: toFloat(row.value)})'

# Load into a remote ledger
fluree load people --from people.csv --remote origin \
  --cypher 'MERGE (n:Person {id: row.id}) SET n.name = row.name'
```

## Output

```
  … 1000 rows
  … 2000 rows
Loaded 2412 rows into `people` in 3 commit(s)
```

## `fluree load` vs `create --from data.csv`

These solve different problems:

| | `fluree load` | `create --from data.csv` |
|---|---|---|
| When | Incremental upsert into an existing ledger | One-shot bulk import of a fresh dataset |
| Header convention | Your own columns, mapped by a Cypher template | neo4j-admin `:ID` / `:LABEL` / `:START_ID` headers |
| Mechanism | Batched Cypher transactions (one commit each) | Bulk index build |
| Re-runnable | Yes — `MERGE` upserts | Creates a new ledger |

Use `load` to keep a ledger in sync with a changing CSV; use `create --from`
to bootstrap a ledger from a node/relationship CSV export. See
[create](create.md) for the bulk path.

## See Also

- [update](update.md) — Cypher / SPARQL / JSON-LD writes
- [create](create.md) — Bulk CSV & Cypher import into a new ledger
- [query](query.md) — Query a ledger
- [Cypher support matrix](../reference/cypher-support-matrix.md) — `MERGE`, `UNWIND`, and per-row upsert semantics
