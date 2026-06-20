# 24 — Roadmap & Remaining Work (Open Issues + Planning State)

**Sources covered:**
- GitHub open issues (`ChrisRoyse/Calyx-Dev`, 25 open as of 2026-06-16) — dump in `_open_issues.json`; bodies read via `gh issue view`.
- `docs/dbprdplans/19_ROADMAP_FSV_BUILD_DONE.md` (phasing, FSV protocol, `BUILD_DONE` predicate, performance targets).
- `docs/implementation/03_PHASE_MAP.md` (PH00–PH72, stage status, FSV gates).
- `docs/dbprdplans/29_STATE_GITHUB_ISSUES.md` (issue-tracker discipline).
- `docs/implementation/FSV_NOTES.md` (what "FSV-verified" means).
- `README.md` "Project status" section.
- Pinned context issues #689, #688, #23 ("You are here").
- "Gaps / not covered" sections of sibling docs 05–20 in this set.

> **Status-source caveat.** This doc reports two states that disagree because the
> planning docs lag the live tracker. The committed phase map
> (`03_PHASE_MAP.md`, dated 2026-06-10) records the frontier at **Stage 9 PH40–PH42**.
> The pinned `[CONTEXT] You are here` issue #23 (last verified 2026-06-10, commit
> `5ebd8e85`) records work having since advanced into **Stage 10 PH43 (Anneal)**,
> with PH43 T01–T05 (#394–#398) closed and PH43 T06 (#399) `status:in-progress`.
> Where they differ, the live issue state is the more current claim; neither is a
> byte-level FSV verdict, which is the only authority per `DOCTRINE §0`.

---

## 1. Current status (what is built)

Per the phase map (`03_PHASE_MAP.md`) and README "Project status", Calyx is **pre-1.0,
under active development**; on-disk format and public interfaces may still change.

| Stage | Phases | Crate(s) | Status (per phase map / #23) |
|---|---|---|---|
| S0 Foundation | PH00–PH04 | calyx-core | DONE |
| S1 Aster | PH05–PH11 | calyx-aster | DONE, FSV-signed-off |
| S2 Forge | PH12–PH16 | calyx-forge | DONE, FSV-signed-off (CPU SIMD + CUDA sm_120 + TurboQuant + MXFP4/grouped GEMM + autotune) |
| S3 Registry | PH17–PH22 | calyx-registry | DONE, FSV-signed-off |
| S4 Sextant | PH23–PH26 | calyx-sextant | DONE, FSV-signed-off |
| S5 Loom + Assay | PH27–PH30 | calyx-loom, calyx-assay | DONE, FSV-signed-off |
| S6 Lodestar | PH31–PH34 | calyx-paths, calyx-mincut, calyx-lodestar | DONE, FSV-signed-off |
| S7 Ledger | PH35–PH36 | calyx-ledger | DONE, FSV-signed-off |
| S8 Ward | PH37–PH39 | calyx-ward | DONE, FSV-signed-off |
| S9 Temporal & Dedup | PH40–PH42 | aster/loom/sextant (cross) | Closed out per #23 (PH40–PH42 + follow-ups FSV-backed) |
| S10 Anneal + Objective J | PH43–PH48 | calyx-anneal | **Active frontier** — PH43 T01–T05 closed; PH43 T06 (#399) in-progress |
| S11–S20 | PH49–PH72 | various | Pending |

README's prose summary matches: storage, math, lens registry, multi-signal search,
association/information layers (Loom/Assay), grounding kernel (Lodestar), guard
(Ward), ledger, self-optimization (Anneal), and **first** oracle capabilities are
"built and working"; the rest is expansion toward 1.0 (§3).

The kernel-first search funnel referenced in planning (KernelFirst@1e8 SLO target) is
the subject of the active scale work (PH68, issue #550), not yet closed.

---

## 2. Open GitHub issues (grouped)

25 issues are open. They split into four classes: **pinned context/state** (not work
items), **stage epics** (tracking shells — close only when all child tasks close with
FSV evidence), **substantive remaining-work tasks**, and the **capstone audit**.

### 2.1 Context / state issues (read-first, not work)

| # | Title | Labels | What it is | Touches |
|---|---|---|---|---|
| 689 | 📌 PROJECT STATE (READ FIRST) — public/dev repo split | (pinned) | Authoritative state note: `Calyx-Dev` (private) is dev + tracker; `Calyx` (public) is a scrubbed, squashed, 0-issue release mirror. Infra strings (`aiwonder`→`gpuhost`, `/home/croyse`→`/opt/calyx`) must never reach public. | repo/process |
| 688 | [CONTEXT] Public-repo hygiene — never commit internal material | type:context, p0, area:env | Lists gitignored paths (`docs/ scripts/ infra/ datasets/ env.sh`) and the rule that no infra/secret string enters any tracked file or issue text. | repo/process |
| 22 | [CONTEXT] Mission & invariants | type:context, p0, area:env | Thesis + binding invariants (A1–A32); scope: universal DB + AGI, Leapable = Vault-only, PostgreSQL untouched. | all |
| 23 | [CONTEXT] You are here | type:context, p0, area:env | Current-phase snapshot + latest FSV evidence roots (see §0 caveat). | all |
| 24 | [CONTEXT] Environment & ops | type:context, p0, area:env | Everything builds/tests on the GPU host; secrets via Infisical. | env/ops |
| 25 | [CONTEXT] Landmines | type:context, p0, area:env | Recurring gotchas (Rust IS installed; build under the project home; ≤500-line rule; FSV reads bytes; dedup never merges conflicting anchors). | all |
| 26 | [CONTEXT] Datasets | type:context, p0, area:env | Which real datasets are acquired + checksum-verified; what is still needed. | datasets/FSV |

### 2.2 Stage epics (tracking shells, S9–S20)

Each epic groups its stage's `type:task` cards; it closes only after every child task
closes with FSV evidence plus a stage-exit rollup. All are `p1`. These are the
backbone of "remaining work toward `BUILD_DONE`."

| # | Epic / stage | Phases | `BUILD_DONE` clause(s) | Subsystem |
|---|---|---|---|---|
| 361 | S9 — Temporal & Dedup | PH40–PH42 | TEMPORAL ∧ DEDUP ∧ RECURRENCE | aster/loom/sextant |
| 362 | S10 — Anneal + Intelligence Objective J | PH43–PH48 | SELFOPT ∧ INTELLIGENCE | calyx-anneal |
| 363 | S11 — Oracle & AGI | PH49–PH52 | ORACLE | calyx-oracle |
| 364 | S12 — Universal Data Layer | PH53–PH55 | UNIVERSAL | calyx-aster/sextant |
| 365 | S13 — Resource / GC / Reliability | PH56–PH59 | RESOURCE | aster/forge/anneal (cross) |
| 366 | S14 — Security & Privacy | PH60–PH61 | SECURITY | aster/calyxd (cross) |
| 367 | S15 — Interfaces (CLI / MCP / Migration) | PH62–PH64 | LEAPABLE (tooling) | calyx-cli, calyx-mcp |
| 368 | S16 — Server & Deployment | PH65–PH67 | DEPLOY | calyxd / infra |
| 369 | S17 — Scale (DiskANN / SPANN) | PH68 | SCALE | calyx-sextant |
| 370 | S18 — Datasets & Intelligence FSV | PH69–PH70 | DATA | cross |
| 371 | S19 — Leapable Vault Swap | PH71 | LEAPABLE | calyx-cli / calyx-mcp |
| 372 | S20 — Critical Capabilities | PH72 | critical-caps | cross |

### 2.3 Substantive task issues (concrete remaining engineering)

| # | Title | Labels | What it asks for | Subsystem / crate |
|---|---|---|---|---|
| 550 | PH68 · T06 — Anneal autotune of beamwidth/posting-cutoff + 1e8-cx SLO soak | type:task, p1, area:sextant, phase:PH68, status:in-progress | Implement `BwPostcutoffTuner` (Anneal hook tuning DiskANN beamwidth / SPANN posting-cutoff with bandit + recall-floor tripwire + anti-oscillation), then run the definitive billion-scale SLO soak (1e8 cx, KernelFirst, p99 < 25 ms). This is the **PH68 exit gate**. | calyx-sextant `index/autotune.rs` (+ Anneal) |
| 587 | [GAP] PH53 — HTAP row+column side-by-side serving + FSV | type:task, p2, area:universal, phase:PH53 | Unowned gap: maintain a row mirror + Arrow column for a collection at one seq and prove a point (row path) and aggregate (column path) read return identical data at the same snapshot. | calyx-aster (universal layer) |
| 560 | PH70 · T02 — Assay bits/contract validation on labeled corpora | type:task, p2, area:assay, phase:PH70 | Validate Assay claims on real labeled data (AG News / banking77): per-lens `bits_about` ≥ 0.05, planted-redundant lens (corr > 0.6) rejected, `I(panel;anchor)` with CI, per-stratum bits — read from `assay` CF bytes. | calyx-assay (validation script) |
| 562 | PH70 · T04 — Ward injection-block validation ≥99% at calibrated FAR | type:task, p2, area:ward, phase:PH70 | Validate Ward on a real prompt-injection/jailbreak corpus: calibrate τ at ~1% FAR, prove injection-block ≥ 99%, confirm valid-novelty routes to new-region — read `guard_verdicts` CF bytes. | calyx-ward (validation script) |
| 563 | PH70 · T05 — Oracle sufficiency validation — SWE-bench ≈0.46 deficit | type:task, p2, area:oracle, phase:PH70 | Prove the sufficiency-refusal gate fires on SWE-bench Lite: a form-only panel yields `I(panel;oracle) < H(Y)` (≈0.46-bit deficit), oracle refuses, confidence capped at self-consistency — read `oracle_sufficiency` metric bytes. | calyx-oracle (validation script) |
| 642 | [TASK] Capstone — final `BUILD_DONE` predicate audit | type:task, p1, area:env, status:blocked | Standing definition-of-done marker. Walk the full `BUILD_DONE` conjunction clause-by-clause, re-read each clause's FSV evidence bytes (roots still exist, hashes still match), spot-check a live subset per clause, emit the final evidence table. **Blocked by all stage epics #361–#372**; intentionally kept open. | cross / project gate |

Note: the four PH70 validation tasks (#560/#562/#563) and dataset epic #370 are
**intelligence validation against public benchmark corpora**, not new engine code —
they prove existing claims on real data. README §3's "broader validation of the
oracle and guard against public benchmark corpora" maps directly to these.

---

## 3. Roadmap toward 1.0

README's "Actively expanding toward 1.0" list, mapped to the phase plan and open
issues. (These are stated direction; only the linked open issues are committed work.)

| README 1.0 theme | Phase plan | Open issue(s) | Status |
|---|---|---|---|
| Richer public CLI + populated MCP toolset (agents drive the full engine) | PH62–PH64 (S15) | epic #367 | CLI/MCP libraries exist; MCP transport not wired into `calyxd` (see §4); many CLI subcommands are FSV/diagnostic |
| Scale-out vector indexing (on-disk DiskANN graph + SPANN centroid-partitioned) | PH68 (S17) | epic #369, task #550 | In progress (#550 `status:in-progress`); 1e8-cx SLO soak is the exit gate |
| Server & deployment polish (run `calyxd` as a managed service) | PH65–PH67 (S16) | epic #368 | Daemon serves `/metrics`; systemd/ZFS/Prometheus/restic-DR pending |
| Broader oracle/guard validation on public corpora | PH70 (S18) | epic #370, tasks #560/#562/#563 | Validation scripts pending; depends on dataset acquisition (PH69) |
| (Aspirational north star) grounded substrate for general intelligence | PH48–PH52, Objective J | epics #362, #363 | Honesty/sufficiency gate built; full `J` loop + Oracle stages pending |

Beyond the README list, the phase plan also has open stages not foregrounded in the
README: Universal Data Layer (S12 / #364), Resource-GC hardening (S13 / #365),
Security & Privacy (S14 / #366), Critical Capabilities — streaming/triggers/time-travel
(S20 / #372), and the Leapable Vault swap (S19 / #371), which is the **only required
customer-shipping phase** (PostgreSQL control plane stays untouched by design).

---

## 4. Known implementation gaps (consolidated from docs 05–20)

These are stubs, `todo!()`-equivalents, fail-loud-only paths, and unimplemented
branches found while documenting each subsystem. They are **code-state facts**,
distinct from open-issue work and from aspirations. Source doc in the last column.

| Subsystem / crate | Gap (what the code actually does) | Source doc |
|---|---|---|
| calyx-core | `CALYX_TEMPORAL_INVALID_WINDOW` declared but raised by no core path; module-local error codes sit outside the closed PRD-18 catalog (pending governance). | 05_core.md |
| calyx-aster | No quantization/compression codec here — slot CFs are raw big-endian IEEE bits; ANN/DiskANN/SPANN live in Sextant, not aster; single-level `SstLevel` (no L0/L1 fan-out); JSON manifest; compaction cadence fixed (`FIXME(PH46)` Anneal hook not wired). | 06_aster_storage_engine.md |
| calyx-forge | `Bits8` declared non-lossy but **no codec produces/consumes it**; deferred backend ops (`knn`, `histogram_nmi`, `spmm_sparse_ops`, `bilinear_cross_term`, `graph_ops`, `colbert_maxsim`) declared, not implemented; no native FP4 cuBLAS GEMM (custom kernel); no runtime CPU↔GPU auto-dispatch/fallback. | 07_forge_math_runtime.md |
| calyx-registry | `MultimodalAdapterLens` / `CommissionedLens` produce **hash-derived** vectors, not learned outputs; `bits`/`redundancy` in `LensExplanation` are placeholders pending Assay; real model paths are `#[ignore]`/env-gated. | 08_registry_lenses.md |
| calyx-sextant | GPU parity unimplemented (`cpu_gpu_delta` always returns `CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE`); DiskANN prefetch is Unix-only (no-op on Windows); DiskANN/token-DiskANN inserts are **non-incremental** (full rebuild); planner cost model is a heuristic, not measured latency. | 09_sextant_search.md |
| calyx-loom | Interaction is **Hadamard only** (no low-rank `vₐᵀW vᵦ`); only `StaticPairGainGate` (constant) — no live Assay/Sextant integration; `mean_agreement` == `raw_mean_agreement`; `n_eff`/DPI are carried inputs, not computed; `SignatureResult::NewContent` never produced; graph recomputed per call (no persisted adjacency). | 10_loom_associations.md |
| calyx-assay | No Forge/GPU execution (`project_gpu` is a stub; all math CPU, O(n²)); thresholds hard-coded consts (not per-vault config); logistic "probe" is a mean-difference linear classifier, not fitted regression; no auto park/retire loop; `n_eff` reports but does not prune; `admit_lens` never returns `admitted:false` (rejects via `Err`). | 11_assay_signal_bits.md |
| calyx-lodestar | **LP relaxation not wired to a solver** — default pipeline always takes the heuristic DFVS fallback (`CALYX_KERNEL_LP_UNAVAILABLE`); build pipeline does not measure recall (fields 0 until separate test); `kernel_answer` default ledger ref is a stub; incremental rebuild is whole-graph; time-travel summarization partial. | 12_lodestar_kernel.md |
| calyx-mincut / calyx-paths | **No min-cut / max-flow / FVS solver** despite the crate name — `lp_scaffold` only formulates the LP (missing cycle-elim constraints); `LpSolution`/`SolveStatus` are inert; `reach` ignores edge weights; in-edge access O(E); `gft_project/reconstruct` panic on mismatch; spectral routines treat directed graph as undirected, dense O(n²)/O(n³); recursive Tarjan SCC (stack-overflow risk). | 17_graph_mincut_paths.md |
| calyx-ward | `AnnealHook` is an interim object-safe seam ("until Anneal's PH48 queue is live"); ONNX needs real model files + CUDA (no CPU fallback); novelty classification fails closed when recurrence data absent; Polis civic guard is a synthetic-persona proof surface. | 13_ward_guard.md |
| calyx-ledger | Payload schemas are conventions, not types (`payload: Vec<u8>` opaque, enforced only by readers); no key management (caller supplies `[u8;32]`); `DirectoryLedgerStore` is "manual FSV before Aster group-commit"; `LedgerGroupCommitHook::on_commit` is a **disabled stub**; plan API name drift (`verify_chain` returns enum). | 14_ledger_provenance.md |
| calyx-anneal | `heal/` and `j/` (Objective-J machinery) summarized, not detailed; no `todo!()`/stub paths observed in the core files read (tripwire/shadow/rollback/budget/ledger are real). | 15_anneal_optimization.md |
| calyx-oracle | `OracleError::FlakyAnchor` declared but **never constructed** (dead constructor); no confidence-based refusal beyond the sufficiency gate; **no code for Objective `J` / `intelligence_report` / `growth_curve`** in this crate; PRD-22 butterfly/reverse operate on a generic graph, separate from the vault-backed production path. | 16_oracle_prediction.md |
| calyx-hazard-soak / testkit | No crate module doc; **H24 (DR drill) intentionally incomplete** — restic restore gated behind `CALYX_PH59_RESTIC_DR=1`, skipped pending PH66 (passes on skip with `dr_restore_verified:false`); benchmark is a no-op off Linux; hazard probes reachable only via the binary. | 18_hazard_soak_and_testkit.md |
| calyx-mcp | No batch JSON-RPC at the binary; `initialize` ignores client params/version (fixed `2024-11-05`); only `{tools:{}}` advertised (no resources/prompts/logging); `search.fresh` is a no-op; `search.guard=in_region` is unreachable (always errors). | 19_mcp_api_tools_reference.md |
| calyx-cli / calyxd | **MCP transport (T05) not wired into the running daemon** — `calyxd` serves only `/metrics`; production tool registration deferred (PH63/T06); CUDA path feature-gated (default build cannot serve in server mode, fails loud); most `CalyxMetrics` observers dormant in the daemon; many CLI subcommands are FSV/diagnostic harnesses, not production ops. | 20_cli_and_daemon_reference.md |

Several of these gaps have an explicit phase owner: Forge `Bits8`/deferred ops (#338),
aster compaction cadence (PH46/Anneal), Sextant GPU parity & incremental DiskANN
(PH68 / #550), Lodestar LP-rounding + mincut solver (PH31/PH32 follow-up), Loom live
Assay/Sextant promotion (PH28+), MCP/daemon transport (PH63/PH64), DR drill (PH66/PH67).

---

## 5. Phase model — what PH## and "done"/FSV-verified mean

### 5.1 Phases (`03_PHASE_MAP.md`)

Work is organized as **PH00–PH72**, grouped into stages **S0–S20**. Each phase is a
stable handle with: dependencies (phases that must be DONE first), the crate(s) it
lands in, the PRD roadmap phase + axioms it satisfies, and a one-line FSV exit gate.
Phase IDs referenced throughout the source (e.g. `FIXME(PH46)`, "until Anneal's PH48
queue is live") point into this map. Status markers: ✅ DONE · ▶ ACTIVE · · pending.

The phase map maps onto the PRD's coarse roadmap phases P0–P12 / P4b–P11b
(`19 §2`): P0–P8 build the engine; P9–P10 productionize; **P11 (Leapable Vault swap)
is the only required customer-shipping phase**; P12 (Discover hosting) is optional.

### 5.2 What "done" means — `BUILD_DONE` (`19 §5`)

"Done" is a **mechanical predicate**, not a judgement: `BUILD_DONE` is a single
conjunction over ~26 clauses (CORE ∧ LENS ∧ SEARCH ∧ DDA_BITS ∧ KERNEL ∧ GUARD ∧
PROVENANCE ∧ SELFOPT ∧ MATH ∧ ARRAYMATH ∧ COMPRESS ∧ UNIVERSAL ∧ ORACLE ∧
KERNEL_ANY ∧ FORMULAS ∧ RESOURCE ∧ TEMPORAL ∧ DEDUP ∧ RECURRENCE ∧ INTELLIGENCE ∧
DATA ∧ SECURITY ∧ DEPLOY ∧ SCALE ∧ LEAPABLE ∧ FSV). Each clause binds a phase gate
to measured threshold constants. Representative thresholds:

| Metric | Target |
|---|---|
| Multi-lens recall@10 vs single-lens (Δ) | ≥ +15% on a real labeled corpus |
| Kernel-only recall / full recall | ≥ 0.95 |
| Guard injection-block rate @ calibrated FAR | ≥ 99% |
| Anneal p99 improvement over 1e6 queries | ≥ 20%, recall non-regressing |
| Forge matmul vs cuBLAS | within 10% on sm_120 |
| CPU↔GPU bit-parity | within 1e-3 rel tolerance |
| Differentiation contract | ≥ 0.05 bits, ≤ 0.6 corr |
| Search p99 (SingleLens / RRF-6 / KernelFirst@1e8 / Pipeline) | < 5 / < 15 / < 25 / < 60 ms |
| Vault migration fidelity (SQLite→Calyx) | byte-exact on content; control-plane responses identical |

Issue #642 is the standing owner of the final clause-by-clause `BUILD_DONE` audit.

### 5.3 What "FSV-verified" means (`FSV_NOTES.md`, `19 §3`)

**FSV = Forensic Source-of-truth Verification.** A passing test or a function return
is a *claim*; the **bytes are the verdict** (`DOCTRINE §0`). The protocol:

1. Identify the bytes that prove the claim (Aster CF row, WAL record, SSTable, Ledger
   entry, on-disk file, Prometheus metric).
2. Read them **before** the action (baseline).
3. Execute the action.
4. Read them **after** (independent read — never trust the function's return).
5. Inspect the delta; record command, SoT path, expected vs actual bytes, ≥3 edge
   cases, and fail-closed `CALYX_*` proof in the GitHub issue.

Rules: **FSV scripts/harnesses are banned** — Calyx may ship *readback tools*
(`calyx readback --cf … --vault …`, `calyx verify-chain …`) that print bytes for a
human/agent to judge, never a green-checkmark harness. There is **no CI pipeline —
FSV is CI**; everything is built/run/tested on the GPU host. Every "done" in §1 carries
a recorded FSV evidence root (`…/data/fsv-issue<N>-…`) with artifact hashes.

A per-merge code-quality gate also binds (`19 §3b`): `cargo check`/`test`/`clippy
-D warnings` green, **no `.rs` file > 500 lines** (over-limit → file a modularization
issue), and CPU↔GPU bit-parity on Forge-touching changes.

---

## Gaps / not covered

- Open-issue bodies were read for the substantive tasks (#550, #587, #560, #562, #563,
  #642) and the epics; the 7 context/state issues were read for #689/#688/#23 and
  summarized from the dump for the rest. Closed issues (the bulk of completed FSV work)
  were not enumerated here — see #23 and `03_PHASE_MAP.md` for completed evidence roots.
- The §0 status disagreement (phase map PH40–PH42 vs issue #23 PH43) is reported, not
  resolved: confirming the true frontier requires reading the GPU-host bytes, which is
  outside this documentation pass.
- §4 consolidates the gaps **as recorded in sibling docs 05–20**; it does not re-derive
  them from source. If a doc's gap section is stale, this table inherits that staleness.
