# 28 — Full State Verification & Test Data (Per Aspect, Every Step)

> **Living-system role:** conscience / proof — nothing is "true" until the bytes prove it (A15 — DOCTRINE §0)

Defines, concretely, **what FSV is for every implementation and aspect of Calyx, at every step**: how each thing is tested, whether data is synthesized or downloaded, what datasets are needed, where they come from, and how they are verified against. FSV discipline is `DOCTRINE §0`/§8 + `AICodingAgentSuperPrompt.md` §4. **Everything is built, stored, run, and tested on `aiwonder`** (§5); this WSL box only authors code.

## 1. The two kinds of test data (and when each is law)

| Kind | For | Source | Why |
|---|---|---|---|
| **Synthetic, deterministic (known input → known output)** | *mechanics* — storage, math, dedup logic, kernel algorithm, guard logic, crash recovery, GC | generated in-repo from a fixed seed; ground truth is computed by construction | FSV is *exact*: the expected bytes/numbers are known a priori. Doctrine requires synthetic FSV data with known I/O + ≥3 edge cases, cleanup-tagged. |
| **Real datasets** | *intelligence claims* — recall, bits, kernel recall, oracle accuracy, calibration, growth `J` | downloaded to `aiwonder` (HuggingFace/Kaggle/academic) | the intelligence must be measured against *reality* (A2), not a toy; grounded anchors come from real labels/qrels/oracles. |

Rule: **mechanics are FSV'd on synthetic data; intelligence is FSV'd on real data.** Both read the persisted source of truth — never a return value, never a harness verdict (A15).

## 2. FSV per aspect — what to build, what data, what bytes to read

For each: the synthetic FSV (mechanics) and the real-data FSV (intelligence). "Read SoT" = read the actual Aster CF rows / WAL / Ledger / index / metric, before and after, and inspect the delta.

> **§2.0 — Multi-embedder minimum (binding gate, A35 / `DOCTRINE §10.26`).** Any FSV that exercises **retrieval, recall, signal/bits, fusion, scale, or an SLO** (Sextant, Loom/Assay, Lodestar, Ward, Anneal/`J`, the partitioned/DiskANN/SPANN scale soaks, the Oracle panel-sufficiency runs) MUST run a **real panel of ≥ 10 frozen embedder lenses** and read back, from the persisted artifact, the **lens roster (`lens_id` + `weights_sha256` for each of the ≥ 10 lenses), the per-lens bits, the ensemble decomposition (per-lens marginal value `I(panel;anchor) − I(panel∖k;anchor)`, cross-term/synergy gain, `n_eff`, panel sufficiency `I(panel;anchor)`), and the fused (RRF) result** — not a single lens, not synthetic vectors. **Floor = 10.** The earlier **4–5-lens bootstrap floor is retired**: it was only enough to stand up the fusion path; ≥ 10 is now required for every gate, **scaling toward 20+**. **Use more whenever warranted and resources allow** (resource-aware, A26 — check GPU/VRAM/RAM headroom and which lens endpoints are live, e.g. the TEI services; size the panel to the hardware, never below 10). **Why ≥ 10, not fewer:** a lens's value is **associational, not intrinsic** — it is the unique+redundant+synergistic bits it adds *relative to the rest of the panel* (PID / interaction information / conditional MI, `07`), so 1–2 lenses cannot be valued at all and **≥ 3 is the minimum to begin computing it, ≥ 10 for stable, decision-grade estimates**. A single-/few-embedder or synthetic-vector run is **diagnostic-only** and **NEVER satisfies the gate / phase-exit**: such data does not measure the Constellation fused via the calculus of association, so the recall/SLO/fusion/`J` math cannot be tuned from it. An FSV artifact recording **< 10 real lenses fails closed** (A16) — the readback step asserts `len(lens_roster) >= 10`. This makes the prior "recall@10 ≥ 0.85 / p99 < 25 ms" scale gates valid **only over a fused ≥ 10-lens panel** (per-slot partitioned vaults fused), never a single lens. Which 10+ lenses to try first, where to get them, and how to swap template sets per situation: `05 §7`/`§9`, A36.

| Aspect (phase) | Build | Test data | FSV (the bytes/numbers + assertion) |
|---|---|---|---|
| **Aster core** (P0) | Constellation CRUD, WAL, MVCC, recovery | synthetic constellations, fixed seed; ≥3 edge cases (empty, max-slots, torn write) | put N cx → read `base`/`slot_*` CFs back **byte-exact**; `kill -9` mid-write → recover → assert last-acked present, un-acked absent (read WAL + manifest) |
| **Forge** (P1) | matmul/distance/quantize, CUDA sm_120 + SIMD | synthetic random vectors (fixed seed) + reference outputs from a trusted lib (numpy/BLAS) → **golden files** | CPU vs GPU **bit-parity ≤ 1e-3**; matmul vs cuBLAS ref within 10%; TurboQuant unbiased-inner-product within distortion bound — read computed vs golden |
| **Registry / lenses** (P2) | hot add/retire, frozen contract, capability cards | **real embedder models** (HF, §3) + a small labeled corpus | embed a known input twice → identical (deterministic); `weights_sha256` matches; dim = `Slot.shape`; add lens → read panel + observe backfill on the cx columns; frozen mutation → `CALYX_LENS_FROZEN_VIOLATION` |
| **Sextant** (P3) | multi-lens RRF, per-slot ANN | **real retrieval benchmark with qrels** (BEIR / MS MARCO subset), embedded through a **≥ 10-lens real panel** (§2.0, A35) | measure recall@10 **fused ≥ 10-lens panel** vs single-lens against the qrels → assert **Δ ≥ 15%** (the single-lens number is the *baseline being beaten*, never the gate); read back the lens roster (≥ 10 `lens_id`+`weights_sha256`), per-lens bits, the ensemble decomposition (marginal value + cross-term gain + `n_eff`), fused result; every Hit carries provenance (read LedgerRef) |
| **Loom / Assay** (P4) | cross-terms, KSG/NMI MI, differentiation contract, `n_eff`, sufficiency | **labeled classification dataset** (label = grounded anchor) + a **planted-redundancy synthetic** (two lenses corr > 0.6) | compute per-lens MI + pairwise corr → read `bits_about`/`assay` rows; assert `≥0.05`/`≤0.6` gates (planted-redundant lens REJECTED); `I(panel;anchor)` reported with CI; per-stratum bits present |
| **Lodestar** (P5) | directed-MFVS kernel, kernel recall | **dictionary/definition graph** (WordNet/Wiktionary) + a **synthetic graph with a planted MFVS** | synthetic: assert the algorithm finds the planted feedback-vertex-set; real: build kernel → **kernel-only recall ≥ 0.95·full** (read both, compare); grounding gaps listed |
| **Ward** (P6) | `Gτ` calibration, novelty | clean set + **injection/OOD set** (prompt-injection corpus); for identity: **speaker-verification set** (VoxCeleb) | calibrate τ on grounded outcomes → **injection block ≥ 99% at calibrated FAR** (read per-slot cos + verdict); valid-novelty → new region; conflicting-anchor dedup never merges |
| **Ledger** (P7) | hash-chain, reproduce() | any ingested data (synthetic + real) | verify chain intact; flip one byte → `verify_chain` detects break at the right seq; `reproduce(answer)` → bit-parity within tolerance |
| **Anneal / `J`** (P8, A32) | self-opt loop, growth curve | **real corpus + query stream** over a **≥ 10-lens real panel** (§2.0, A35) — autotune optimizes the **fused** panel, not one lens | 1e6-query soak → read p99 + recall + `J` over time: **p99 ↓ ≥ 20%, no recall regression, `J` rises, Goodhart held-out passes**; lens roster (≥ 10) + per-lens bits + ensemble decomposition + fused result read back; every change reversible (read Ledger `kind=Anneal`) |
| **Temporal / dedup / recurrence** (P2b, A27–A29) | E2/E3/E4, dedup, recurrence series | **synthetic event stream with planted periodicity + planted duplicates** + real timestamped logs | assert recurrence signature fires (content agree + time differ); period detected = planted period; dedup merges duplicates, **never merges conflicting anchors**; oracle self-consistency computed from recurring outcomes |
| **Oracle** (P6b, A20) | consequence prediction, sufficiency, super-intelligence predicate | **a domain with a real deterministic oracle** — **SWE-bench Lite** (code + test pass/fail, the paper's own instantiation) | predict Pass/Fail → measure `I(panel;oracle)` (expect the paper's ≈0.46 deficit on a form-only panel → sufficiency-refusal fires); calibration capped at `oracle_self_consistency`; reverse_query recovers a known cause |
| **Universal data layer** (P4c, A19) | collections-as-any-model | synthetic per-paradigm fixtures (rows/docs/KV/TS/blob) | each paradigm's root op (point/range/join/aggregate/traverse) → read back; one cross-model txn spans modes atomically (read consistent seq) |
| **Memory/GC** (P8b, A26) | reclaimers, watchdog | synthetic high-churn / long-reader / disk-pressure workloads | 1e7-op soak → RSS/VRAM bounded; tombstones reclaimed; long reader aborted on lease → old version GC'd (read disk/heap metrics) |

Every row is proven by **reading the persisted bytes/numbers** — perceived through Synapse (`§2c`: `reality_baseline`→act→`reality_audit`, `read_text`/`find` the real output, `audit_export_bundle` the evidence) — and recorded in a GitHub issue (`AICodingAgentSuperPrompt.md` §3/§4); no harness asserts success.

## 2c. FSV with Synapse — full use of its abilities (binding)

FSV is *perceptual*: the agent must see the actual source-of-truth, not a claimed return. **Synapse (`31`) is the perception+action substrate that makes every FSV step concrete and un-fakeable**, and FSV MUST use its full ability set — not just `read_text`. The 5-step FSV protocol (`19 §3`) mapped to Synapse:

| FSV step | Synapse abilities | What it does |
|---|---|---|
| **0. Target** | `set_capture_target`, `set_perception_mode`, `health` | focus perception on the right terminal/window on aiwonder; confirm Synapse is live |
| **1. Read SoT *before*** | `reality_baseline`, `act_run_shell` (readback cmd: `calyx readback` / `xxd` / `zfs list` / `psql` / `cat metric`), `read_text` (OCR the output), `find` (locate the exact row/number), `capture_screenshot` | snapshot the *actual* persisted bytes/numbers before acting — a baseline a harness can't forge |
| **2. Act** | `act_run_shell` / `act_type` / `act_launch` / `act_combo` | execute the operation under test (ingest, `kill -9`, query, build, `cargo test`) in a real terminal |
| **3. Read SoT *after*** | `observe`, `observe_delta`, `reality_audit`, `read_text`, `find`, `capture_screenshot` | perceive the new actual state; **`observe_delta`/`reality_audit` *is* the before→after delta**, computed from perceived reality vs the baseline |
| **4. Inspect the delta** | `reality_audit` (drift flag), `find` (assert the expected value present / unexpected absent) | judge the delta against the expectation; drift = fail; this is the agent *seeing* the truth |
| **5. Record evidence** | `capture_screenshot`, `replay_record`, `audit_export_bundle`, `audit_intelligence_query` | attach screenshots + a recorded session + an exported audit bundle to the `chrisroyse/calyx` GitHub issue — reproducible FSV evidence, not a green checkmark |

**Reactive FSV (async operations):** for things that complete later (a 1e6-query soak, a build, a dataset download, a `calyxd` recovery), `reflex_register` fires on the observed completion/error condition (`reflex_history` audits what fired), so the agent FSVs the *real* end-state the moment it appears instead of polling blindly. `subscribe`/`observe_delta` stream live changes (e.g. watch Aster compaction, VRAM, the growth curve `J`).

**Driving the work (`31 §4`):** when Synapse commands Claude/Codex worker agents to build a phase, the orchestrator FSVs each worker's result the same way — `read_text` the worker's real terminal output, `reality_audit` the Aster/Ledger bytes the worker changed, `audit_export_bundle` the evidence. A worker's "done" is a claim; Synapse-perceived bytes are the verdict.

**Screenshot + AI-vision (use heavily, `31 §6c`):** `capture_screenshot` → the agent *looks at* the image to confirm the real state — Grafana charts, the `J` growth curve, GUI/error state, rendered output — catching what `read_text` can't. Combine `find` (locate) + `read_text` (exact numbers) + screenshot-vision (holistic "what's happening") for complete FSV perception.

**Full-ability checklist (FSV/dev must exploit all of it):** perceive (`observe`/`observe_delta`/`capture_screenshot`/`read_text`/`find`) · act (`act_run_shell`/`act_type`/`act_launch`/`act_press`/`act_combo`/`act_scroll`/`act_clipboard`) · reality (`reality_baseline`/`reality_audit`) · reactive (`reflex_register`/`reflex_history`/`subscribe`) · evidence (`capture_screenshot`/`replay_record`/`audit_export_bundle`/`audit_intelligence_query`) · control/hygiene (`set_capture_target`/`set_perception_mode`/`health`/`release_all`/`storage_*`). If an FSV step could be satisfied by a return value instead of a Synapse-perceived byte, it is **not** FSV (`DOCTRINE §0`).

## 3. Datasets to gather (real) — the catalog

Gather a **variety** so the intelligence is tested across modalities, embedder types, and grounded outcomes. Primary source = **HuggingFace `datasets`** (uses `hf_hub_token` from Infisical, §4/`16`); some via **Kaggle** (add `kaggle_username`/`kaggle_key` to Infisical if used); some academic mirrors. All download **onto aiwonder** at `/zfs/archive/calyx/datasets/<name>/` (cold) and are checksum-verified on arrival (§3.2).

| # | Dataset(s) | Modality / embedder exercised | Grounded outcome (anchor) | Tests | Source |
|---|---|---|---|---|---|
| 1 | **BEIR**, **MS MARCO**, Natural Questions, TREC-COVID | text semantic + keyword (E1/SPLADE), paraphrase | relevance qrels | Sextant recall, RRF, pipeline (P3) | HF |
| 2 | **AG News**, IMDB, SST-2/GLUE, **banking77**, DBpedia-14 | text semantic; classification | class label | Assay bits/MI, differentiation contract (P4) | HF |
| 3 | **SWE-bench Lite** (300×8), HumanEval, MBPP | code (AST/CFG/dataflow/type/trace lenses) | **test pass/fail (deterministic oracle)** | Oracle, sufficiency, ME-JEPA negative (P6b) | GitHub/HF |
| 4 | **WordNet**, ConceptNet, Wiktionary defn graph, **Cora/ogbn** citation graph | graph / definition edges | known communities / core | Lodestar kernel, kernel-only recall (P5) | NLTK/HF/OGB |
| 5 | **Quora Question Pairs**, **PAWS** | text; near-duplicate | duplicate / not (label) | TCT cosine-`Gτ` dedup correctness (P2b) | HF |
| 6 | **VoxCeleb1/2**, LibriSpeech | audio speaker (WavLM), wave | speaker identity (verification) | Ward identity-lock, speaker MI (P6) | HF/academic |
| 7 | RAVDESS, IEMOCAP | audio emotion | emotion label | media-panel emotion lens (P4) | HF/academic |
| 8 | **ImageNet-subset**, CIFAR-100, COCO | image (CLIP) | class / caption | media-panel image lens, cross-modal (P4) | HF |
| 9 | server/app **event logs**, financial tick, user-activity streams (or synthetic if private) | temporal events | timestamps + recurrence | temporal understanding, recurrence, next-occurrence (P2b) | Kaggle/synthetic |
| 10 | **prompt-injection / jailbreak corpora**, OOD splits | adversarial text | injection / benign | Ward injection-block ≥99% (P6) | HF |
| 11 | **synthetic personas** (Polis `0701-synthetic-persona-spec`) | civic 21-slot | tie-formation (simulated) | Polis constellation/guard (privacy-safe) | synthetic (in-repo) |
| 12 | a labeled **drift** pair (month-A vs month-B distributions) | any | distribution shift | change-point/MMD, Anneal (P8/§17 §8) | derived from 1/2 |

**Coverage rule:** at least one dataset per (modality × grounded-outcome-type) so every lens family and every intelligence metric has a real, grounded test. `BUILD_DONE` clauses that say "on a real corpus / ≥3 corpora" are satisfied from this catalog.

### 3.2 Acquisition is itself FSV'd
Downloading a dataset is an operation that must be verified against the SoT (not "the script said done"):
- record expected (rows, bytes, sha256/manifest) from the dataset card;
- download to `/zfs/archive/calyx/datasets/<name>/`;
- **read back**: row count, byte size, checksum → assert == expected; sample N records and eyeball schema;
- write a `datasets/MANIFEST.md` row (name, source, version, sha256, rows, license, what-it-tests). A dataset is "gotten" only when its bytes are verified present and correct on aiwonder.

## 4. Secrets for data/models (Infisical)

Model/dataset acquisition needs exactly one secret today: **`hf_hub_token`** (HuggingFace, for gated models/datasets), already in Infisical (`HF_HUB_TOKEN`/`HF_TOKEN`). If Kaggle datasets are used, add **`kaggle_username`** + **`kaggle_key`** via the CLI (`infisical secrets set …`). Full secrets policy: `16 §5b`. Never write a secret value into a repo/issue/chat — env-var names only (`AICodingAgentSuperPrompt.md` §3.16).

## 5. Build / run / store / test — all on aiwonder

**Division of labor (binding):** this WSL dev box **authors** code; **`aiwonder` is where the project exists, runs, is tested, and stores its state.** Per `DOCTRINE §8c`:
- **Build:** compile natively on aiwonder under `CALYX_HOME=/home/croyse/calyx` (Rust via rustup + CUDA 13.3 are installed — the "no `rustc` on box / cross-build to `/opt/leapable/calyx/`" note is superseded; see `docs/implementation/01_AIWONDER_ENVIRONMENT.md`); the authoritative build artifact lives on aiwonder.
- **Store:** Aster vaults + all datasets live on aiwonder ZFS (`/zfs/hot/calyx`, `/zfs/archive/calyx`); the source-of-truth bytes FSV reads are *there*.
- **Run:** `calyxd` + the resident lens/TEI services run on aiwonder (the RTX 5090, sm_120).
- **Test:** every test — synthetic mechanics and real-dataset intelligence — executes **on aiwonder**, reading aiwonder's persisted state. Local runs are for authoring only and never count as FSV.
- **Reach it:** SSH via `~/.config/aiwonder.env` (`16 §0`); secrets via Infisical.

So the FSV loop in practice: author on WSL → sync/build on aiwonder → ingest synthetic + real data on aiwonder → read aiwonder's Aster/Ledger/metrics bytes → record evidence in a GitHub issue. The project's *truth* is the state on aiwonder, nowhere else.

## 6. Verification maturity & edge audits (inherited)

Aim for **L3+** verification (`AICodingAgentSuperPrompt.md` §4.7): not "it returned ok" but "the SoT changed exactly as specified, edges included." **≥3 edge audits per code path** (more for security/guard/dedup): e.g. dedup edges = identical content / near-threshold / conflicting-anchor / temporal-only-difference. Use Synapse (`31`) to read the *actual* terminal/test output (FSV's perception arm), not a claimed return. When a test fails, **STOP and root-cause** (5 Whys to a structural cause), never patch the symptom.

## 6b. No CI pipeline — FSV is our CI

**Calyx has no CI/CD pipeline.** A hosted pipeline would slow the build, cost money (A34), and is unnecessary: **FSV — manual source-of-truth readback on aiwonder — is our CI.** It is the gate that decides whether a change is right (`DOCTRINE §0`). Tests (`§6c`) are the *fast inner loop* (a passing test is a *claim*); FSV is the *truth gate* (reads the bytes). Both run **locally / on aiwonder, invoked by the agent** — never in a hosted pipeline. The per-merge checks (`cargo check`/`test`/`clippy -D warnings`, the ≤500-line gate `DOCTRINE §8`, bit-parity) are run on aiwonder before merge, not by a CI service.

## 6c. What makes a test useful (Rust, tailored)

Tests support FSV; they don't replace it. Every test must pass the **two questions**: *does it fail when the code is wrong, and pass when the code is right?* A test that passes on broken code is anti-knowledge. Rust-tailored discipline (from the test-usefulness doctrine, mapped to our stack):

**FIRST + properties (binding):**
- **Fast** — unit test < 100 ms; whole `cargo test` unit run in seconds. No DB/network/file/clock/RNG in unit tests; the full suite must stay fast enough to run constantly.
- **Independent / parallel-safe** — `cargo test` runs in parallel by default; every test arranges its own world, no shared mutable global, no order dependence. Inject deps (traits), don't reach into statics.
- **Repeatable / deterministic** — **seed all RNG** (`StdRng::seed_from_u64`), **inject the clock** (a `Clock` trait, never `SystemTime::now()` in logic), no wall-clock/locale/path dependence. (Matches the frozen-lens determinism probe, `05`.)
- **Self-validating** — `assert!`/`assert_eq!` decide pass/fail; never "print and eyeball." Use `#[should_panic]`/`Result`-returning tests for error paths.
- **Behavior, not implementation** — test through the public API; assert on the **observable outcome** (return value, the persisted Aster/Ledger bytes, the emitted error code), not private fields. A behavior-preserving refactor must keep tests green. Mock at the **boundary** (lens endpoint, disk), not internals.

**Test types we use (all free OSS Rust — A34):**
| Type | Rust tool | Where it earns its keep |
|---|---|---|
| Unit | `#[cfg(test)] mod tests` | pure logic: math kernels, dedup signature, kernel-graph, guard math, parsers |
| Property-based | **`proptest`** | invariants: `decode(encode(x))==x` (Aster round-trip), quant round-trip within bound, MFVS on planted graphs, `cos` symmetry, dedup never merges conflicting anchors — ~50× the bug-catching of a unit test on algorithmic code |
| Fuzz | **`cargo-fuzz`** (libFuzzer) | every untrusted-input boundary: Aster shard parser, query parser, lens-output decoder, wire format — finds the fail-open crashes |
| Mutation | **`cargo-mutants`** (nightly, on diff) | proves the tests actually assert — a survived mutant = a test gap (file an issue); coverage is vanity, mutation score is truth |
| Integration | `tests/` dir | several engines together against a real Aster vault on aiwonder (Testcontainers not needed — we own the box) |
| Perf | **`criterion`** | latency budgets (`19 §4`); baseline + regression threshold, not "looks fast" |
| FSV (the gate) | manual readback | the truth check — reads aiwonder's persisted bytes (`§2`); no harness asserts it |

**Smells to refuse:** assertion roulette (unlabeled asserts); `sleep()` to wait (use polling-with-timeout / `tokio::time`); order-dependent / shared-state tests; mystery-guest fixtures (arrange inline, seeded, tagged synthetic data); over-mocking (prefer in-memory **fakes** + stubs over mock-with-expectations); `#[ignore]` that lingers (fix, file an issue with a date, or delete — ignored tests are lies that look like work); testing private internals via `pub(crate)` hacks.

**Flakiness is the silent killer** — zero tolerance. A flaky test usually signals a *real* race/timeout bug in the engine (a P1, not "just rerun"). Determinism (seed+clock injection) + independence kills most flakes by construction. Treat test code as production code (clippy, same review bar). **Bug → write the failing regression test → fix → keep it (tagged with the issue).**

## 7. The data/FSV `BUILD_DONE` contribution

```
DATA := datasets/MANIFEST.md lists ≥1 verified real dataset per (modality × outcome-type)
        ∧ each dataset checksum-verified present on aiwonder (§3.2)
        ∧ every §2 aspect has a passing FSV (synthetic mechanics + real intelligence), evidence in issues
        ∧ all tests executed on aiwonder against persisted state (§5)
```
Added to the `BUILD_DONE` conjunction (`19`).

**One sentence:** FSV for every Calyx aspect is concrete — synthetic deterministic data proves the mechanics (storage, math, dedup, kernel, guard, recovery) by reading the exact persisted bytes, and a catalog of real datasets (text/code/graph/audio/image/temporal/adversarial, from HuggingFace/Kaggle, acquired and checksum-verified onto aiwonder) proves the intelligence (recall, bits, kernel recall, oracle accuracy, growth `J`) against grounded ground truth — all built, run, stored, and tested on the aiwonder datacenter box, with secrets (the HuggingFace token) from Infisical, and every claim recorded by reading the source of truth, never a harness.
