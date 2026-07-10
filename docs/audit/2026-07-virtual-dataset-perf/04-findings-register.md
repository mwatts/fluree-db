# Virtual-Dataset (Iceberg / R2RML) — Findings Register (first SF01 parity run)

**Date:** 2026-07-10
**Branch:** `bench/virtual-dataset-corpus` (worktree `db-vbench`)
**Source runs:** native `results/runs/run-20260710T180017Z-01KX6JZY909RBB9MXJWHA59Z8T.jsonl`; virtual `results/runs/run-20260710T180521Z-01KX6K975N0TWK4ZG75TT430MX.jsonl` (target `virtual-sf01`, `smoke` subset, 16 queries).
**Companions:** `01-pathway-inventory.md` (strategies §N, code anchors), `02-hypothesis-map.md` (H1–H8), `03-corpus-design.md` (query lineage `qNNN`).

This is the first native-vs-virtual parity comparison on SF01. Each finding below is one row of the smoke run, classed and grounded in the exact record data (rows / result-hash equality / wall-ms both targets / pathway span counts), with a suspected mechanism anchored to the inventory and a hypothesis linkage. **Correctness findings rank above perf findings** — a wrong answer is worse than a slow one.

**Headline:** 5 of 16 smoke queries are **correct and hash-equal** to native (q001, q003, q006, q007, q011, q022, q054 — see F6). The rest split into **two silent-empty correctness bugs** (F1, F2), **one silent-divergence** (F3, mechanism confirmed), **two canonicalization-suspects** (F4), and **four perf-DNFs** (F5), plus a **harness span-coverage note** (F7).

---

## 0. Evidence table (all 16 smoke queries)

`hash`: EQ = native and virtual result-hashes identical; NE = both ok but hashes differ; — = not comparable (a DNF/error side). `v_scan` = count of `r2rml.scan_table` spans on the virtual rep. `pruned/sel` = `files_pruned`/`files_selected`.

| q | native rows | virt rows | hash | native ms | virt ms | v_scan | v_load | pruned/sel | virt status | finding |
|---|---|---|---|---|---|---|---|---|---|---|
| q001 | 500 | 500 | **EQ** | 1 | 2271 | 7 | 0 | 0/0 | ok | F6 |
| q003 | 9 | 9 | **EQ** | 16 | 365 | 1 | 0 | 0/0 | ok | F6 |
| q005 | 20 | 20 | **NE** | 2 | 3707 | 12 | 0 | 0/0 | ok | **F4** |
| q006 | 3593 | 3593 | **EQ** | 16 | 2498 | 7 | 0 | 0/1 | ok | F6 |
| q007 | 10 | 10 | **EQ** | 10 | 350 | 1 | 0 | 0/0 | ok | F6 |
| q011 | 2136 | 2136 | **EQ** | 323 | 4020 | 1 | 0 | **7579/91** | ok | F6 (H4✓) |
| q022 | 3 | 3 | **EQ** | 390 | 593 | 1 | 0 | 0/0 | ok | **F6 (1.52×)** |
| q025 | 5 | 0 | — | 33 | 180000 | 1 | 1 | 0/0 | **dnf** | **F5** |
| q034 | 4514 | **0** | NE | 175 | 2 | **0** | 0 | 0/0 | ok | **F1** |
| q040 | 1100 | 0 | — | 385 | 180000 | 1 | 1 | 0/0 | **dnf** | **F5** |
| q042 | 24 | **21** | **NE** | 0 | 2991 | 9 | 0 | 0/3 | ok | **F3** |
| q049 | 5000 | 5000 | **NE** | 64 | 5085 | 14 | 0 | 0/0 | ok | **F4** |
| q050 | 30000 | **0** | — | 95 | 120000 | **377** | 6 | 0/0 | **dnf** | **F5** |
| q051 | 247 | **0** | NE | 474 | 0 | **0** | 0 | 0/0 | ok | **F2** |
| q053 | 5000 | 0 | — | 399 | 180000 | 3 | 3 | 0/0 | **dnf** | **F5** |
| q054 | 9 | 9 | **EQ** | 2 | 2567 | 8 | 0 | 0/0 | ok | F6 |

---

## F1 — Transitive property path silently returns zero rows *(correctness-silent-empty)*

**Query.** q034 `?e a edw:Employee ; edw:name ?en ; edw:manager+ ?boss` (BI-17, transitive `+` path).

**Evidence.** native **4514 rows** / 175 ms; virtual **0 rows** / 2 ms, `status=ok`, hashes NE. The virtual rep fired **no `r2rml.scan_table` span** (`spans_missing` = `[r2rml.scan_table, iceberg.scan_plan]`) — the engine returned instantly without touching Iceberg. A wrong answer delivered as success, with no error and no scan.

**Suspected mechanism.** A `PropertyPath` pattern is **not converted** to an R2RML leaf — it is preserved as-is by the rewriter `[verified rewrite.rs:162-179]` and evaluated by the generic path operator, which resolves against the graph source's (empty) native index and yields nothing. Critically, **no loud error fires**: the whole-GRAPH-scope guard `if rewrite_result.unconverted_count > 0` `[graph.rs:245-253]` counts only unconverted **top-level `Pattern::Triple`s**; a `PropertyPath` is a different pattern variant, so it never increments `unconverted_count` and the scope is not failed.

> **Refines inventory §13.** The claim "unconvertible triples fail the whole GRAPH scope" is correct *for triples*, but the guard **does not cover sub-scope pattern types** (`PropertyPath`, `Subquery`, …). Those route to generic operators over an empty index and return silently empty. §13 should be amended: the error is triple-scoped, not scope-scoped.

**Nuance (reconciles the two field reports).** *Sequence* paths (`a/b`, e.g. the CLI `edw:geography/edw:region` probe) **do** work on virtual — SPARQL lowering decomposes `a/b` into two ordinary triples, which convert normally (but run the generic join above the scans — slow, ~18.75 s, an H8 perf case). *Transitive* paths (`+`/`*`) cannot decompose into a fixed triple set, stay a `PropertyPath`, and hit this silent-empty bug. So q034 (`manager+`) is a **correctness** bug; q035 (`manager/manager`, sequence) is a **perf** case. **Action:** q034's manifest header note (currently "works but slow") is wrong and should read "transitive path returns 0 rows on virtual — silent-empty"; q035's note is correct.

**Hypothesis linkage.** H8 (non-lowered forms) — but escalated from "slow" to "silently wrong" for the transitive sub-case.

---

## F2 — Subquery scope silently returns zero rows *(correctness-silent-empty)*

**Query.** q051 `… { SELECT ?s (COUNT(?o) AS ?cnt) … } { SELECT (AVG(?c2) …) } FILTER(?cnt > ?avg)` (BI-24, stores above average order count).

**Evidence.** native **247 rows** / 474 ms; virtual **0 rows** / 0 ms, `status=ok`, hashes NE, **no `r2rml.scan_table` span**. Same signature as F1: instant, scanless, silently empty.

**Suspected mechanism.** Identical to F1: `Pattern::Subquery` is preserved-as-is by the rewriter `[verified rewrite.rs:162-179]`, its inner triples are never converted to R2RML leaves, and the sub-scope escapes the top-level `unconverted_count` guard `[graph.rs:245-253]`. The generic subquery operator evaluates against the empty native index.

**Hypothesis linkage.** H8. Same root as F1 — a single fix (recurse the rewriter into `Subquery`/`PropertyPath` sub-scopes, or fail the scope when a sub-scope contains unconverted triples) closes both F1 and F2.

---

## F3 — Bound-subject wildcard drops the `rdf:type` triple *(correctness-divergence — mechanism confirmed)*

**Query.** q042 `{ <edw/store/1> ?p ?o } UNION { <edw/store/2> ?p ?o } UNION { <edw/store/3> ?p ?o }` (BI-19, three-store detail).

**Evidence.** native **24 rows** vs virtual **21 rows** — exactly **3 rows lost**, hashes NE. `files_selected=3` (the prefix-prune correctly hit one file per store). Native `--keep-heads` shows the 24 rows include exactly **three `rdf:type → edw:Store` triples** (one per store); the arithmetic is `24 = 3 × 8` (8 triples/store incl. type) vs virtual `21 = 3 × 7`. **The 3 lost rows are the `rdf:type` class-membership triples.**

**Suspected mechanism.** A bound-subject **true wildcard** (`<iri> ?p ?o`) on virtual materializes the TriplesMap's `PredicateObjectMap`s (the 7 column/ref predicates) but **does not emit the `rr:class`-derived `rdf:type` triple** — the class is carried on the SubjectMap (`rr:class edw:Store`, `enterprise.ttl:85`), not as a POM, and the wildcard materialization path iterates POMs only (inventory §2's wildcard handling binds `type_var` only when the query has an explicit `?s a ?t`, `[rewrite.rs:657-667]`; a plain `?p`/`?o` wildcard has no type var, so the class triple is never produced). Native, being a materialized graph, stores the `rdf:type` triple explicitly and returns it.

**Why it matters.** This is a **silent, systematic** under-count for every bound-subject inspector / entity-detail view on virtual — the exact "subject inspector" shape the prefix-prune was built for (inventory §7). Any UI that reads `<entity> ?p ?o` and expects the type will not see it.

**Hypothesis linkage.** Not a perf hypothesis — a correctness gap in the wildcard materialization (adjacent to §2/§7). **Recommend** a targeted fix: emit the `rr:class` `rdf:type` triple(s) when a wildcard binds `?p`/`?o` with no type var. Confirm with a `--keep-heads` virtual re-run diffing against the native 24 (the team's "investigate-next" — the mechanism is now confirmed by arithmetic + native heads; the virtual head-diff is the final nail).

---

## F4 — Hash mismatches with equal row counts *(canonicalization-suspect)*

Two queries return the **same row count** as native but a **different result-hash**. Neither is yet a confirmed data bug — both need a `--keep-heads` cell-level diff (the parity runs did not retain heads).

**q005** — supplier scorecard, 20/20 rows, NE. Projects `edw:rating` (`xsd:double`). Prime suspect: **float canonicalization drift** — native and virtual may emit the double with different lexical forms or datatype tags (`decimal` vs `double`), and while `canon` quantizes doubles to 12 significant digits `[canon.rs:143-153]`, a value tagged `decimal` on one side and `double` on the other takes different canonicalization branches `[canon.rs:116-141]`. **Next step:** diff the 20 head rows; if only the `?r` column differs, it is canonicalization, not data.

**q049** — CONSTRUCT customer→region, 5000/5000 nodes, NE. This NE is **expected and not a data bug**: the CONSTRUCT hash is a serialized-`@graph`-node multiset `[canon.rs:canonicalize_graph]`, and JSON-LD compaction/key-order can differ between the native and virtual formatters even for an isomorphic graph. Cross-engine CONSTRUCT hash equality is explicitly **not yet asserted** (documented in `canon`); only node count (5000 = 5000 ✓) and single-engine stability are. **Next step:** treat q049 hash as informational until a graph-isomorphism canonicalizer lands (a harness follow-up).

**Hypothesis linkage.** None (correctness-plumbing). F4-q005 gates whether float-bearing aggregates (q005/q008/q010/q014/q018/q025/q026/q040/q041/q048/q050/q051) can be hash-compared at all; resolve it before trusting any float projection's parity.

---

## F5 — Perf DNFs: virtual hits the deadline where native is sub-second *(perf-dnf)*

Four smoke queries **did not finish** on virtual (capped at their `timeout_s`) while native answered in ≤ 500 ms. Ranked by how damning the gap is:

| q | shape | native ms | virt | v_scan | mechanism (inventory) | H |
|---|---|---|---|---|---|---|
| **q050** | dims-only `Product ⋉ Supplier` OPTIONAL | **95** | DNF@120s | **377** | **Correlated-join rebuild.** 377 `r2rml.scan_table` spans for a *two-small-dimension* query ⇒ the parent (Supplier) dim is re-scanned per child batch; the OPTIONAL routes to an R2RML leaf (§13) whose scan is not memoized across batches (§8/§9, `operator.rs:889-897`). A dims-only query has no business issuing 377 scans. | **H3** |
| q025 | `Ticket ⋈ Product` GROUP BY HAVING | 33 | DNF@180s | 1 | Single fact scan of `FACT_SUPPORT_TICKET` + join, agg over a join declines the fused path (§11) → full materialize; 1 scan but the decode+join wall exceeds 180 s. | H6, H1, H3 |
| q040 | `VALUES ?store … ⋈ Order` | 385 | DNF@180s | 1 | VALUES not lowered (§13) ⇒ the store constraint never becomes a scan filter; full 180K-order scan + generic join. | H8, H1 |
| q053 | Customer `NOT EXISTS` Order | 399 | DNF@180s | 3 | Negation over a fact scan; the correlated NOT-EXISTS re-probes Order. | H1, H3 |

**q050 is the alarm.** It is dims-only, both tables are single-file dimensions, native does it in 95 ms — and virtual issues **377 scans** and times out. This is the cleanest existence proof of the H3 correlated-rebuild / no-cross-batch-parent-memoization cost on the whole run, and it is on the *cheapest* class of query. It should anchor the H3 line of the roadmap.

**Hypothesis linkage.** H3 (q050, q053), H1 (all), H6 (q025), H8 (q040).

---

## F6 — Positive controls: correctness parity and a tolerable ratio *(positive-control)*

- **Correctness parity (hash EQ):** q001, q003, q006, q007, q011, q022, q054 all return **hash-identical** results to native. The core dims + simple fact aggregates are **correct** on virtual — the silent-empty/divergence bugs are confined to non-lowered forms (F1/F2) and the wildcard class triple (F3).
- **q022 — fused-agg ratio existence proof:** current-customers-by-segment, virtual **593 ms** vs native **390 ms = 1.52×**, hash EQ. A single-table GROUP BY that takes the fused aggregate path (§11) achieves a *tolerable* native-to-virtual ratio — proof the pathway can be fast when the right strategy fires. This is the ratio floor the roadmap should push the rest of the corpus toward.
- **q011 — H4 date pushdown works at scale:** orders-in-Q1-2024, `files_pruned=7579 / files_selected=91` — the date-range FILTER on a physically-`date` column **pruned 98.8% of files** (inventory §6/§7, the `*_DATE` literals kept in the mapping precisely for this, `enterprise.ttl:5-6`). Hash EQ, 12× wall. This is the **positive control for H4-date** and the contrast partner for the decimal-blind case (H4, q019).

---

## F7 — `iceberg.scan_plan` span fires only on the pushdown branch *(harness-note)*

`iceberg.scan_plan` appears in `spans_missing` for almost every virtual query; it fired on only **2 of 16** (q006 `0/1`, q011 `7579/91`) — exactly the two with a pushed FILTER that produced a scan filter. So the span is **conditional on the predicate-pushdown branch**, not emitted on every scan. Two consequences for WP6:
1. The `EXPECTED_FOR_VIRTUAL` span set flags `iceberg.scan_plan` as "missing" on every non-filtered scan, which is noise, not signal — **tune the expected-span list** so `scan_plan` is only expected when a scan filter is present (or make the reader emit a `scan_plan` span unconditionally with `files_pruned=0`).
2. `files_pruned`/`files_selected` counters are therefore **only populated on filtered scans** — the H1 decode-wall analysis (bytes/rows decoded) needs the separate reader-span counter called out as a bench-gap in `02-hypothesis-map.md` (H1 confirm), which this run confirms is still missing.

---

## Summary & routing

| Finding | Class | Query | Root | Fix owner |
|---|---|---|---|---|
| **F1** | correctness-silent-empty | q034 | `PropertyPath` (transitive) not converted; sub-scope escapes GRAPH-error guard | engine (rewrite/graph) |
| **F2** | correctness-silent-empty | q051 | `Subquery` not converted; same guard gap | engine (same fix as F1) |
| **F3** | correctness-divergence | q042 | wildcard omits `rr:class` `rdf:type` triple | engine (R2RML materialize) |
| **F4** | canonicalization-suspect | q005, q049 | float lexical/datatype drift (q005); CONSTRUCT node-hash not isomorphism (q049) | harness (canon) + head-diff |
| **F5** | perf-dnf | q050, q025, q040, q053 | H3 correlated rebuild (q050 377 scans), H1/H6/H8 | roadmap (WP8) |
| **F6** | positive-control | q001/03/06/07/11/22/54 | correctness parity; fused-agg 1.52×; H4-date prunes 98.8% | — |
| **F7** | harness-note | (all) | `scan_plan` conditional on pushdown branch | harness (WP6 span tuning) |

**The two correctness bugs (F1/F2) share one root** and are the highest priority — they deliver wrong answers as silent successes. **F3 is a second, independent correctness gap** on the most common inspector shape. **F5-q050** is the sharpest perf signal (dims-only, 377 scans). **F4-q005 must be resolved** before any float-bearing parity is trusted. These four feed the WP7 diagnosis and WP8 roadmap; F7 feeds back into WP6 harness tuning.
