# W3C SPARQL Burn-Down — Wave 3+ Handoff Dossier

**Audience:** the dev/team picking up Wave 3 and beyond without the orchestration context. Everything you need is either in this file or pointed to from it. **This file is deliberately untracked** — share internally; do not commit.

**State as of 2026-07-07, coverage branch tip `e2e63a9f6`, PR fluree/db#1437 open.**

---

## 1. Mission and definition of done

The whole W3C SPARQL test suite (`testsuite-sparql/rdf-tests` submodule, ~1,420 tests) runs green in CI on every PR. "Green" means: every test either passes or appears in an explicit **skip register** (`testsuite-sparql/tests/registers/mod.rs`), and the register is policed **in both directions** — an unregistered failure fails CI, and a registered test that starts passing ALSO fails CI ("stale entry"). Registers can only shrink. The burn-down = shrinking them to the floor: ~63 not-applicable (protocol/HTTP suites) + ~47 entailment (deliberate non-goal) + whatever the team consciously registers as documented divergence.

**Definition of done for every burn-down PR** (this is enforced culture, not aspiration — see `docs/audit/burn-down/ROADMAP.md` §7):
1. Register entries the PR greens are removed in the same PR (both copies for double-registered update-syntax tests). CI enforces this mechanically.
2. JSON-LD parity per `docs/contributing/sparql-compliance.md` § "Query Surface Parity": IR/engine-level fixes flow to JSON-LD automatically but STILL get a JSON-LD regression test (W3C tests only guard the SPARQL surface); anything newly possible in SPARQL must become possible in JSON-LD query syntax in the same effort. **Cypher is explicitly excluded** (openCypher grammar isn't ours to extend).
3. Bench gates, no waivers: `query_hot_bsbm`, `query_hot_bsbm_bi`, `insert_formats`, `import_bulk` within `regression-budget.json`; plus PR-specific gates named in the ROADMAP row (e.g. `query_hot_property_path` exists now).
4. Behavior changes carry a changelog/migration note.
5. **Decision transparency (AJ's standing rule):** any PR actioning a D-decision must present the decision point, ALL options considered, and why the adopted one was chosen, in the PR description, linking ROADMAP §4. Silent adoption is not acceptable.
6. Verification: full W3C suite (`cd testsuite-sparql && cargo test`) + touched crates' tests + the JSON-LD groups (`grp_query`, `grp_query_sparql`, `grp_transact` as applicable) + fmt + clippy `-D warnings`.

## 2. Read-order rule for the knowledge base (IMPORTANT)

All analysis lives in `docs/audit/` on the coverage branch:
- `2026-07-sparql-testsuite-audit.md` — parent audit: taxonomy, perf discipline (§6 is BINDING), phases.
- `burn-down/<nine cluster audits>.md` — per-cluster root causes with file:line evidence.
- `burn-down/ROADMAP.md` — the adversarially verified plan: PR slate (§2), sequencing (§3), decisions (§4), wave outcome records (§4b area), issues (§5), register hygiene (§6), DoD (§7).

**Never implement from a cluster audit alone.** Thirteen of their load-bearing claims were REFUTED under adversarial verification. The rule: read the cluster doc, THEN read ROADMAP §1.1's refutations of it, THEN the relevant "Wave N implementation status" tables (they record further corrections discovered during implementation — e.g. the ninth audit voided a ROADMAP claim about W-1's fix territory; PR-G1's agent refuted the audit's seeding design when tests regressed). The register file's per-entry comments are kept current and name the owning future PR — trust them over any doc's stale expectations.

## 3. Current state: branches, worktrees, merge choreography

**Pushed:** only `test/sparql-testsuite-full-coverage` (= PR #1437): harness + registers + CI + docs. Includes a merge of origin/main (their #1436 sub-SELECT fix landed mid-effort; CI runs the PR **merge commit**, so registers must track main — that's why re-baseline commits exist).

**Local implementation branches** (14, all suite-green, each with an UNCOMMITTED `pr-description.md` draft at its worktree root — these become the PR bodies; each worktree also has an uncommitted `[workspace]` marker in `testsuite-sparql/Cargo.toml`, see footguns):

| Branch | Worktree (`/Users/ajohnson/fluree/db/.claude/worktrees/`) | Wave | Δ | Stacks on |
|---|---|---|---|---|
| burndown/pr-h1 | agent-a57f39385cdc1ed50 | 0 | −3 | tip |
| burndown/pr-1 | agent-ac13cedd62d781044 | 0 | −29 | tip |
| burndown/pr-l1 | agent-a0ba638ccc059d860 | 0 | −5 | tip |
| burndown/pr-l2 | agent-aa5a54edba4185b8e | 0 | −1 | tip |
| burndown/pr-x1 | agent-a90720144bd5437bb | 0 | −37 | tip |
| burndown/pr-pp | agent-af7074d90ea736bd0 | 0 | −2 | tip |
| burndown/pr-2 | agent-ad98123dbdff3555f | 1 | −28 | tip |
| burndown/pr-3 | agent-ad929aff8faa8ddc8 | 1 | −15 | tip |
| burndown/pr-u1 | agent-a12da03c6b88f9d1a | 1 | −19 | tip |
| burndown/pr-w2a | agent-a00ef829fca12c239 | 1 | −61 | **pr-1** |
| burndown/pr-base | agent-a99415c5c8daa5f65 | 2 | −6 | tip |
| burndown/pr-g1 | agent-a67c8e752aae34c9c | 2 | −14 | tip |
| burndown/pr-u2 | agent-a7b564a0c7ccca730 | 2 | −9 | **pr-3** |
| burndown/pr-w15 | agent-a287e083750bb4df0 | 2 | −2 | tip |

("tip" = the coverage branch head at the time; all were rebased onto `6ef63bc63`+ after the main merge. Waves 0/1 tables in ROADMAP record per-PR content.)

**Merge choreography:** #1437 first → `pr-h1` (owns the register comment batch + the committed workspace-marker fix) → all other tip-based branches in any order → `pr-w2a` after `pr-1`; `pr-u2` after `pr-3`. When #1437 merges to main, rebase everything onto main — mechanical; the register gate catches drift (proven: it caught #1436).

**Known joint/double-claimed entries** (whoever lands SECOND drops the stale hunk; CI forces it): `syntax-order-07` (pr-1 ∩ pr-3, byte-identical hunk); `syntax-update-anonreifier-02` (pr-2 ∩ pr-3, two independent defenses — keep both); `pp34`/`exists03` (removed on pr-base, kept-with-comment on pr-g1); `pp35` (needs BASE+G1 both); `eq-graph-1/2/4` (needs G1 + future PR-X2).

**Ledger:** 538 original registered gaps → ≈307 remaining pre-integration (63 NA + ~47 entailment + ~200 fixable). The integrated number after all 14 branches merge is authoritative — expect small deltas from joint entries.

## 4. Decisions D-1..D-13 (all adopted as recommended; ROADMAP §4 is the source of truth)

D-1 triple-terms accept-then-defer (actioned: W2A; remaining: W2BC) · D-2 GRAPH ?g W3C-by-default (SHIPPED in pr-g1, 3 pinning tests updated) · D-3 within-ledger datasets Option A (Wave 3: PR-G2) · D-4 reject-more = hard errors + changelog (shipped: pr-2/pr-3) · D-5 remote LOAD = documented divergence (Wave 3: PR-U3) · D-6 DROP≡CLEAR now; empty-named-graph = one decision, three consumers (PR-U3 shape; bindings#graph; agg-empty-group-count-graph) · D-7 base-direction: B if in product scope else C (tail: PR-W16) · D-8 TriG: try routing harness qt:data through existing `parse_trig_phase1` before writing parser code (tail) · D-9 entailment owl2rl PRAGMA named-subset (tail: PR-ENT) · D-10 multi-op: guard (shipped pr-3) + one-atomic-commit sequential staging (shipped pr-u2) · D-11 lexical-form preservation split decision (tail: PR-X3; gates ~8 expr + group01) · D-12 SPARQL-scoped strict type errors (actioned in pr-x1 D2b; governs PR-X2 scoping) · **D-13 (NEW, undecided):** Turtle ingest stores RDF collections as native `list_index`, never `rdf:first/rest` — materialize at ingest vs translate at query time vs documented divergence; gates `basic#list-1..4`.

## 5. Wave 3 work specs

Each spec = ROADMAP §2 row + corrections. Read the owning docs per the §2 read-order rule.

### PR-U3 — graph-management verbs (~91 entries; biggest mover)
Owning doc: `update-completeness.md` §2 class A (+ROADMAP row). Grammar/AST/parse for LOAD/CLEAR/CREATE/DROP/COPY/MOVE/ADD + SILENT (keywords already lexed, `token.rs:255-263`); execution: a retract-all-in-g_id staging primitive; DROP≡CLEAR (D-6); CREATE ≈ no-op; COPY/MOVE/ADD composed over CLEAR + insert; LOAD parses, remote fetch = documented divergence (D-5), SILENT swallows. Expose `clear_graph`/`drop_graph`/`copy_graph` on the transact builder + JSON-LD surface with tests (parity rule case 2). **Base it on `burndown/pr-u2`** (multi-op requests end in DROP GRAPH — insert-05a + 3 same-bnode tests green only with BOTH; their register comments say so). Off query hot path entirely; standard gates. No decision newly actioned beyond D-5/D-6 — include their transparency sections.

### PR-X2 — equality/EBV/promotion lattice (~40 entries; HIGHEST ENGINE RISK in the slate)
Owning docs: `expression-semantics.md` §2/§5 PR-B + ROADMAP §1.1 items 7/13 + §1.2-1. Scope: D5+D7 datatype-aware comparison (=, sameTerm, IN), D-EBV (only xsd:boolean false is falsy today), D4 float/double∘decimal promotion (incl. tP-03/04/05/21 and the tP-29/30 that PR-X1's D2 fix unmasked), D11 CONCAT coercion, D12-scoped STRLANG/lang normalization, aggregates (agg-err-01 poison-on-non-numeric — issue #1447; agg-count-rows-distinct new IR aggregate; **agg02 is PROBE-FIRST** — the audit's claimed site was refuted, find the real re-typing site before fixing), plus `quotes-3/4` (ninth audit reclassified: pattern-object datatype-drop = D5b family) and the eq-graph-1/2/4 second-lander joins with pr-g1. **The §6 discipline is the acceptance bar:** same-type fast paths (int-int, string-string, iri-iri) byte-identical; type-pair dispatch chosen at prepare time; the D5b scan-path datatype-constraint item is an explicitly at-risk carve-out — own bench sign-off or defer with a register entry. Run BOTH BSBM benches every commit. NOTE: `subquery12` is NOT in scope (greened by main's #1436); `bnode01` was PR-X1's.

### PR-W2BC — triple-term functions + accept-then-defer syntax (27 entries)
Owning doc: `sparql12-wave2-triple-terms.md` §7 PR-B/C + grammar guardrails §2. **Stack on `burndown/pr-w2a`.** Parse TRIPLE/SUBJECT/PREDICATE/OBJECT/isTRIPLE as arity-validated builtins lowering to `not_implemented`; accept `<<( )>>` in subject/object/VALUES/BIND honoring the 15-negative-test guardrails. Actions D-1 — transparency section required (incl. the runtime-not_implemented-instead-of-parse-error trade-off). Parse-time only; low risk.

### PR-G2 — within-ledger FROM/FROM NAMED (13 entries + constructwhere04)
Owning doc: `named-graph-dataset.md` §2 (Option A design, verified mechanically sound). **Stack on `burndown/pr-base`** — `resolve_dataset_clause()` is already public and waiting. Replace the `view/query.rs` single-ledger dataset guard with within-ledger DataSet construction over one snapshot reusing the existing `DatasetOperator`; route through `dataset_query.rs` (policy/reasoning); reuse `as_runtime_dataset`. Harness change: pre-load FROM/FROM-NAMED-referenced files as named graphs (dataset tests carry no qt:data). Actions D-3 — transparency section. BSBM never instantiates dataset code — confirm flat anyway.

### PR-W1 — FILTER-scope + subquery correlation (3 entries: filter-nested-2, optional-filter-005, join-scope-1)
Owning doc: `algebra-serialization.md` families A+B (ninth audit; NOT covered by ROADMAP §2's original text — its W-1 description was corrected: territory is `parse/query/pattern.rs:228` single-child group unwrapping + lowerer FILTER flattening + `subquery.rs:694` self_produced_vars/Optional reconciliation. NO PR-X1 dependency). Note pr-g1 already touched `self_produced_vars` — coordinate/rebase onto g1 if merged, else expect a small conflict. Low-med risk, plan-time.

### PR-W2 — CONSTRUCT bnode instantiation (2-3 entries: construct-3/4 now; constructlist joint with pr-1)
Owning doc: `algebra-serialization.md` family E. One generic fix: per-solution blank-node instantiation in `format/construct.rs:104` instantiate_row (template bnodes currently lower to never-bound vars → triples skipped → empty graph). SPARQL-only surface (record classification). Per-solution output path — not scan-hot; standard gates.

### Deferred / decision-gated tail (do not start without the decision)
PR-W1-OPT (nested-opt-1/2: OPTIONAL correlated-join → independent-scope-then-leftjoin; hot OPTIONAL path; prepare-time detection design in `algebra-serialization.md`; double-bench-gate) · PR-X3 (D-11) · PR-W16 (D-7; JSON-LD `@direction` must ship in the same PR if B) · PR-ENT (D-9: harness injects `# PRAGMA reasoning: owl2rl` per named test; start rdfs03/04/09) · Option-1 first-class triple-terms epic (gates ~39 eval entries; scoping intel: W15's re-baseline table + its base-triple-assertion divergence note + the 8-closed-enums/ObjKey-arena analysis in `sparql12-wave2-triple-terms.md` §5) · D-13 list_index (gates list-1..4) · empty-named-graph model (D-6 second half; gates bindings#graph, agg-empty-group-count-graph).

## 6. Perf discipline playbook (this repo is speed-first; §6 of the parent audit is BINDING)

Rules, with landed examples to imitate:
- **Fix at parse/lower/prepare/plan time whenever possible; never add per-row work to common paths.** Examples: PR-X1's constant-FILTER fix is plan-time classification in `planner.rs`; PR-BASE constant-folds IRI() at lowering and deliberately does NOT resolve variable arguments (documented partial-spec choice) to keep eval base-free.
- **Preserve fast paths byte-identical; route new semantics through slow paths selected off-row.** Examples: PR-G1's `GraphVarCorrelated` EXISTS fallback (strategy-time selection; per-row only for a shape that previously returned wrong-empty); PR-PP's zero-length universe gated on the ZeroOrMore/ZeroOrOne modifier at plan level with `*`/`+` traversal untouched; PR-U2's single-op path delegating unchanged.
- **Import-hot lexers get extra care.** Example: PR-L1's greedy-scan + trailing-dot-REWIND (the naive lookahead loop broke `_:a..b` and didn't compile — ROADMAP §1.1-9); PR-W15's one-extra-peek budget with a byte-identical non-star corpus assertion.
- **Bench methodology (hard-won):** single before/after runs are noise-dominated on dev boxes — ±11–15% measured on IDENTICAL binaries. Use interleaved A/B (ABAB), n=100 or ≥6 rounds, medians; expect "phantom" deltas at low round counts to dissolve (a +11.5% at 3 rounds vanished at 6). Budgets live in `regression-budget.json`; `cargo test -p fluree-bench-support --test workspace_reconcile` must pass after adding any bench. Known gap: the insert_formats corpus has no `_:` tokens (blank-node lexer changes are invisible to it).
- Slower numbers are acceptable ONLY when the output itself is spec-mandated larger/correct where it was wrong before (PR-PP's closure case) — never for common correct paths.

## 7. Validation playbook

- Full suite: `cd testsuite-sparql && cargo test` (36 suite fns; ~15s after build). Single suite: `cargo test --test w3c_sparql <fn> -- --exact`. Per-suite JSON report: `W3C_REPORT_JSON=out.json cargo test --test w3c_sparql <fn> -- --exact`; `make help` for analysis targets.
- Drive ONE test through the harness without the suite: `W3C_TEST_DESCRIPTOR='{"Eval":{...}}' ./target/debug/run-w3c-test` (see `subprocess.rs` for descriptor shapes incl. UpdateEval) — this is how agents probed guard behavior end-to-end.
- **Register iteration loop:** implement → run suite → it FAILS with "unexpectedly PASSED" for everything you greened → remove exactly those entries → rerun to 0 failed / 0 stale. Never pre-remove entries on faith; never re-register a failure without diagnosing.
- **Expect these dynamics (all observed):** *stale-entry flips* when main moves (CI tests the merge commit — re-baseline registers against main, as commit `6ef63bc63` did for #1436); *guard-carried greens* (a coarse rejection greens a negative test; when you replace the guard with real support, implement the real validation — pr-u2's test_54); *green-by-recovery unmasking* (PR-3 made parse errors authoritative; any reject-more change can unmask tests that only passed via error recovery — fix or register them honestly); *fix-unmasks-deeper-defect* (PR-X1's DATATYPE fix exposed tP-29/30 promotion failures — register with a pointer, don't force).
- Suites also self-police: zero-test manifests fail; register entries matching no test fail; registered tests that die by timeout/crash fail (register excuses wrong answers, not infra deaths).

## 8. Footguns

1. **Nested-worktree builds:** `testsuite-sparql` won't build in a git worktree without a `[workspace]` marker table in its `Cargo.toml` (root path-exclusion doesn't reach worktrees). `pr-h1` carries the committed fix; until it merges, add the marker locally and NEVER commit it.
2. **Git signing hangs on TTY:** `git config commit.gpgsign false` is set in the shared repo config (covers all worktrees). If you see a hang on any commit-creating op (merge/rebase/cherry-pick), that's it.
3. **CI runs the PR merge commit** — local green ≠ CI green if main moved. Merge main in and re-baseline registers (both-way failures tell you exactly what changed).
4. **Submodule:** pinned at `efccbc6b8`; `? testsuite-sparql/rdf-tests` untracked-content noise in git status is pre-existing and harmless. Bumping the submodule = a deliberate event: run the suite, triage every flip.
5. **The harness crate is EXCLUDED from the workspace** — `cargo test -p testsuite-sparql` from the root fails; always `cd testsuite-sparql`.
6. **No AI attribution in commits/PRs** (repo rule). Commit style: lowercase imperative with scope prefixes matching history.
7. Each implementation worktree has its **PR body draft as uncommitted `pr-description.md` at the worktree root** — don't lose them (they carry the decision-transparency sections); GitHub PR creation: `gh pr create --body-file`.

## 9. Issue map

Filed 2026-07-06/07 from verified findings: #1438 multi-op silent drop (FIXED on pr-3+pr-u2), #1439 variable-free FILTER (FIXED on pr-x1), #1440 DATATYPE/LANG args (FIXED on pr-x1), #1441 USING+GRAPH over-delete (open → future PR-U6), #1442 GRAPH ?g literal+default-graph (FIXED on pr-g1), #1443 EncodedSid GRAPH ?g (FIXED on pr-g1), #1444 bnode-dot lexer (FIXED on pr-l1), #1445 double canonicalization (FIXED on pr-l2), #1446 no-default-features build (open, unowned — trivial cfg fix), #1447 aggregate skip (open → PR-X2). Close each when its branch merges. Cross-references posted on #1319 (fix at lowering — done on pr-x1; do NOT extend dt_compatible) and #1317 (distinct defect class — do NOT close against PR-G1).

## 10. If you also inherit the un-merged PR queue

Everything above assumes #1437 merges roughly as-is. If review demands splitting it: the only clean seam is (a) harness+CI+registers, (b) docs/audit. The registers reference the docs' rationale; if split, land (a) first and keep the register comments' doc-pointers intact.
