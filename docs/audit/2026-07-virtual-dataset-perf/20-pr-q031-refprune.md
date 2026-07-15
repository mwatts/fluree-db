# PR-q031 (F20) — RefObjectMap-target resolution prune — DESIGN SKETCH

**Branch:** to stack as `perf/r2rml-q031-refprune` (off `perf/r2rml-pr4d`, itself off the docs branch off #1499). Stacking (lead-ruled): #1499 → docs branch → `perf/r2rml-pr4d` → `perf/r2rml-q031-refprune` (PR-4d implements first; this sketch queues the lead's review meanwhile).
**Status:** SKETCH — **STOP for lead review**. No engine code until approved.
**North-star slate item 1** (F20). Supersedes the F18 cold-floor framing (measurement refuted the pin-leak/residency premise — `18-pr8tail-...` MEASUREMENT ADDENDUM; register F20).
**Target:** q031 72 s → low-single-digit s cache-thrashed, **without re-opening PR-2a** (see §arithmetic).

## The seam (measured, code-anchored)

q031 = `?inv a edw:InventorySnapshot ; edw:onHandQty ?oh ; edw:reorderPoint ?rp ; edw:product ?p . ?p edw:name ?pn . FILTER(?oh<?rp) LIMIT 5000`.

Measured (fresh-cache run): **7 DISTINCT tables loaded once each** — `FACT_INVENTORY_SNAPSHOT` + `DIM_ACCOUNT, DIM_CUSTOMER, DIM_EMPLOYEE, DIM_PRODUCT, DIM_STORE, DIM_SUPPLIER`. The 6 dims are **exactly** the 6 that map `edw:name`. The per-query pin held (no leak); the 21.2 s is a **resolution fan-out**.

**Mechanism.** TriplesMap resolution is base-predicate-driven (`rewrite.rs:690` "the base predicate drives TriplesMap selection"). The second triple `?p edw:name ?pn` is a **variable-subject, shared-base-predicate** pattern with **no class assertion on `?p`**, so `edw:name` resolves to every map bearing it → the 6-dim fan-out (6 dead `loadTable`s incl. 390 K-row DIM_CUSTOMER, and ≈ `241 batches × 6` of the 1448 `scan_table` re-scans). The class-fusion machinery (`fuse_class_if_safe`/`class_fusion_is_safe`, `rewrite.rs:661-739`) only constrains resolution when a `?x a Class` co-locates — it never fires here. But `?p` is bound by `edw:product`, a **RefObjectMap whose parent TriplesMap is provably `DIM_PRODUCT`** — that target is simply **not propagated** to constrain the `?p edw:name` resolution.

## (1) The fix — propagate the RefObjectMap target as a resolution constraint

When a variable `?p` is the OBJECT of a `RefObjectMap` POM (`edw:product` → parent TriplesMap `T` = DIM_PRODUCT), constrain the TriplesMap resolution of every downstream pattern `?p <pred> ?o` to `T` (and its subclass/partition set — see soundness). Concretely, mirror the existing **`class_prune_hint`** mechanism (rewrite records a resolution hint; the operator's `tm_passes_star_prune` filters the candidate maps, `operator.rs`, `star_tm_prune_enabled`), but derive the hint from the **ref target** rather than a class. This is the query-path generalization of the crawl-only `trust_fk_refs` prior art (`execute/runner.rs:687`, `r2rml/operator.rs:1042`, `rewrite.rs:99` — child-templated RefObjectMap target trusted to skip the parent scan; here we trust it to prune resolution). Effect: `?p edw:name` resolves to DIM_PRODUCT only → `load_table.n` **7 → 2**, fan-out re-scans collapse.

## (2) THE SOUNDNESS INVARIANT (load-bearing — two independent conditions)

The prune constrains `?p <pred>`'s resolution to `T` only when BOTH hold; if either fails, **DECLINE** (fall back to the full fan-out — always correct, just slow):

**(A) Join-var provenance — `?p`'s bindings are all provably `T` subjects.** Collect EVERY binding source of `?p` in the query scope. Allow the prune ONLY when every source is a RefObjectMap resolving to the **same** parent `T`. DECLINE if any source is:
- a **UNION** branch that binds `?p` (e.g. `{?inv edw:product ?p} UNION {?inv edw:supplier ?p}` — the supplier branch makes `?p` a DIM_SUPPLIER subject; pruning to DIM_PRODUCT drops those names);
- **another triple pattern** that binds `?p` as a subject/object from a non-ref source (e.g. `?p a edw:OtherClass`, or `?x edw:rel ?p`) that could produce non-`T` subjects;
- a **`VALUES ?p { … }`** (arbitrary IRIs, not guaranteed `T`);
- a **second RefObjectMap with a DIFFERENT parent** (`?inv edw:product ?p . ?x edw:store ?p` → parents DIM_PRODUCT ≠ DIM_STORE; constraining to one is wrong).
- *Allow* multiple ref sources IF all share the same parent `T`. Conservative first cut (recommended): allow ONLY the single-required-RefObjectMap case (exactly q031); DECLINE anything more complex, widen later behind the same switch + differential.
- Scope care: bindings inside an OPTIONAL/subquery vs the required part must be handled — treat any binding producer of `?p` reachable at the point `?p <pred>` is evaluated as a source; when in doubt, DECLINE.

**(B) Template-disjointness — `T` is the ONLY `<pred>`-bearing map whose subject template can match `?p`'s IRIs.** This is the PR-3 (b')/F10 lesson (reuse `wildcard_class_fusion_is_safe`): if a **vertically-partitioned** map shares `T`'s subject template AND maps `<pred>` (the value lives in the partition, not `T`), pruning to `T` drops rows. So require every OTHER `<pred>`-bearing map to be subject-template prefix-**disjoint** from `T`. In the SF01 corpus the dims are template-disjoint (`.../product/{k}` vs `.../supplier/{k}` …), so this holds — but the guard must be explicit so a hand-written vertically-partitioned mapping DECLINEs rather than silently drops. (Without (B), F20 would re-introduce exactly the unsoundness PR-3 corrected when it replaced raw (b) with (b').)

## (3) Expected-wall arithmetic (does q031 land ≤3 s WITHOUT PR-2a?)

Post-prune, q031's plan = one `FACT_INVENTORY_SNAPSHOT` scan (7,670 files / 300 K rows / **51 MB** — file-count-bound) + one DIM_PRODUCT (1-file) hash-join + `FILTER(?oh<?rp)` (per-row, cheap) + `LIMIT 5000` materialize. The `FILTER` is un-prunable (two-column compare) so the LIMIT cannot cut the fact scan — one full 7,670-file read is the residual.

**Empirical reference (SAME clean baseline, cache-thrashed full-corpus order):** a single full 7,670-file fact scan is **~1 s**: q018 = 1.06 s (`files_selected=7670`, 200 K rows, GROUP BY + FILTER), q044 = 0.96 s (`files_selected=7670`, 250 K rows). Both have `load_table.n=0` (cross-query-amortized in the full-corpus protocol). So q031 post-prune ≈ **~1 s fact scan + the DIM_PRODUCT join + 5000-row materialize** → **expected low-single-digit s, ≤3 s cache-thrashed, WITHOUT PR-2a.** The join/materialize is the only term above the q018 reference; it is bounded by the LIMIT (5000 rows). **PR-2a (the 7,670-file decode-wall / master lever) stays CLOSED** unless the gate misses — if the materialize term pushes >3 s, PR-2a (or a materialization lever) opens then, and it is the shared lever for the whole fact-scanning tail (q016-post-PR-4d bottoms out on the identical 7,670-file FACT_SHIPMENT scan).

## (4) Kill switch

**Own switch** (new soundness surface — do NOT overload `FLUREE_R2RML_STAR_TM_PRUNE`): `FLUREE_R2RML_REF_TARGET_PRUNE` (default on). Off ⇒ today's full fan-out, byte-identical. The DECLINE path is also byte-identical to off for any query that fails (A) or (B).

## (5) Blast radius

- **q031** — the clear case (this PR). Any query with `?x <refPredicate> ?p . ?p <sharedPred> ?o` where `?p`'s sole binding is that ref benefits (fewer map loads/scans); result-identical by soundness.
- **Corpus scan for the shape:** other fact→dim-attribute joins (q032 `?inv edw:store ?st . ?st edw:name`, q016's dim attrs, etc.) — check which take the prune and confirm no regression; each is a co-benefit, none may change results.
- **DECLINE shapes must stay correct:** the differential's whole point (§gate). No native path change — `R2rmlScanOperator`/the R2RML rewrite never run on a native query.

## (6) DoD / gate (lead-specified)

1. **`load_table.n` 7 → 2** on q031 (deterministic, cache-independent — the crisp sentinel) + **`scan_table` collapse** (the fan-out re-scans gone).
2. **Live q031 rows-parity vs oracle** (`rows_only` per manifest), cache-thrashed full-corpus order, wall low-single-digit (≤3 s target; reported, PR-2a-deferred if missed).
3. **DECLINE cases as hermetic tests** — one per unsound binding-source shape (UNION-bound `?p`, second non-ref binder, `VALUES ?p`, different-parent second ref, and the (B) vertically-partitioned-template case): each must show the prune **NOT firing**, output byte-identical to switch-off.
4. **Full-corpus cache-thrashed baseline at head** — no other query's wall/hash regresses; the 42/50 ≤3 s set stays put.
5. Native 54/54 + W3C + unit sweeps green; kill-switch off = byte-identical.

## (7) Implementation trace (first step, before code)

Confirm the exact injection point: (a) where the rewrite has both the RefObjectMap POM (`edw:product` → parent T) and the downstream `?p edw:name` pattern in one BGP scope, to compute `?p`'s binding-source set for invariant (A); (b) whether `class_prune_hint` + `tm_passes_star_prune` can carry a ref-target hint as-is or needs a sibling `ref_target_prune_hint`; (c) reuse `wildcard_class_fusion_is_safe` for invariant (B). Mirrors the trace-first discipline of docs 17/18.

**STOP — design review before implementation.** Open questions for the lead: (i) conservative first cut = single-required-RefObjectMap only (exactly q031), widen later — agreed? (ii) is invariant (B) (template-disjointness reuse of PR-3's guard) in-scope for THIS PR or can the corpus's known-disjoint templates let it ship with (A) only + a `debug_assert` (I recommend in-scope — it's the F10 unsoundness guard, cheap via the existing predicate)? (iii) confirm the own-switch name.

*(Implemented as `perf/r2rml-q031-refprune`: commit 9d3a5864f — invariant A via `Pattern::produced_vars`, invariant B via `wildcard_class_fusion_is_safe`, cross-scope pre-bound `?p` sound by conjunction; 75f6b4514 — engine test. Gate GREEN: `load_table` 7→2, `scan_table` 1448→243, rows=5000, kill switch `FLUREE_R2RML_REF_TARGET_PRUNE`.)*

## (8) Residual attribution — MEASURED (2026-07-14 night, F20 head 75f6b4514)

The §3 arithmetic predicted q031 lands ≤3 s post-prune. It does NOT (F20 gate: 17.2 s cache-thrashed, warm-disk exec-one ~9–10 s). This section attributes the residual **from counters/trace, not inference** — the identity of the surviving 243 `scan_table`, and the measured wall cost of the part the prune left behind.

**Identity of the 243 (env `FLUREE_R2RML_TRACE_SCANS`, per-scan trace at the two `operator.rs` scan sites; instrumentation reverted after).** The 243 decompose as:
- **241 × `DIM_PRODUCT` MAIN scans** of `?p edw:name` — each trace line: `pred=edw:name star=false filters=0 budget=Some(5000) topk=false cacheable=false arm=BYPASS`.
- 1 × `FACT_INVENTORY_SNAPSHOT` MAIN star scan (`budget=None cacheable=true arm=MISS-COLLECT`) — collected once.
- 1 × `DIM_PRODUCT` **PARENT** lookup (the fact's `edw:product` RefObjectMap → DIM_PRODUCT, `parent_tm` resolved) — **deduped to 1** across all 241 driving batches (the per-operator `parent_lookup_cache` / PR-8b memo works; corroborates the F19 refutation, `04-findings-register.md`).

So the 241 are **budgeted MAIN re-scans**, not parent lookups and not the fan-out. The *sole* reason they miss the `scan_cache` is `row_budget = Some(5000)` (the `LIMIT` propagated to the topmost row-preserving scan = the dim scan): the `cacheable` predicate (`operator.rs:987`) requires `self.row_budget.is_none()`, so each of the 241 `build_progress` calls re-reads DIM_PRODUCT's one file. **`filters=0`** — the correlated dim scan is UNFILTERED (reads the full table, probes `?p` in memory), so the exclusion's *other* rationale (a pushed `scan_filter` yielding a pruned SUBSET → cache poison) does **not** apply here; only the budget rationale (caching collects a full window before the operator can stop early, defeating the LIMIT) does.

**Pre-registered dispute (recorded BEFORE the measurement).** hyp-A (team-lead): the ~241 surviving DIM_PRODUCT re-scans cost ~5–7 s of the wall → removing them drops it materially. hyp-B (predecessor / fact-floor reading): the re-scans are near-free (DIM_PRODUCT is one small cached file); the wall is the q016-class fact-scan + materialize + `loadTable` floor → removing them barely moves it.

**A/B differential (measurement lever: env `FLUREE_R2RML_EXP_SMALLDIM_CACHE` relaxes the budget disqualifier so the fully-collected sub-window dim is cached → 241 collapse to 1; same warm-disk state, 3 interleaved reps each; scaffolding reverted after).**

| arm | `scan_table.n` | walls (s) | median |
|---|---:|---|---:|
| A — OFF (today, 241 re-scans) | 243 ×3 | 9.32, 8.28, 9.22 | **9.22 s** |
| B — ON (collapsed to 3) | 3 ×3 | 9.72, 8.74, 9.80 | **9.72 s** |

**DELTA = −0.5 s (B marginally SLOWER, within noise). hyp-B WINS; hyp-A REFUTED.** The 241 DIM_PRODUCT re-scans cost **nothing** — corroborated by the spans (`r2rml.prefetch` for the 242 dim reads = 0.00 s; `iceberg.decode` = 0.61–0.81 s total; the big cold number is `iceberg.fetch_bytes`, 100 % fact-file-dominated — DIM_PRODUCT is 1 file vs FACT's 7 670). The residual is the **fact-scan + materialize + `loadTable` floor**, the SAME floor as q016-post-PR-4d.

**Verdict (lead-approved, the chain's 6th entry).** A query-scoped small-dim main-scan cache is **REJECTED as a slate item — measured worthless** (−0.5 s). q031's path to ≤3 s is exactly q016's: **(i) the loadTable-METADATA cache** (the `load_table.n=2` ≈ ~2–3 s of REST/OAuth per the arithmetic-hole rider) **+ (ii) the shared PR-2a / materialization fact-scan floor.** The two queries now share ONE residual story, which strengthens the priority of the PR-2a + materialization follow-on when AJ weighs it. F20 ships on its **deterministic** gate (`load_table` 7→2, `scan_table` 1448→243, rows-parity), wall explicitly deferred to that shared floor — NOT on a ≤3 s claim.

**Soundness note for the record (the rejected fix's design question, had it been pursued).** The `:987` budget exclusion exists because caching a budgeted scan collects a full window before the operator can stop early — defeating the LIMIT. Caching it would be sound ONLY for a scan that is (1) **unfiltered** (`filters=0`, so no pruned subset — the poison rationale) AND (2) **fully collected below one window** (`collect_scan_capped` → `Complete`, so the table is exhausted in one pull regardless of the LIMIT). q031's DIM_PRODUCT satisfies both. But since the measured payoff is −0.5 s, the design is moot; recorded so a future revisit doesn't re-derive it.
