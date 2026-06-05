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

| Aspect (phase) | Build | Test data | FSV (the bytes/numbers + assertion) |
|---|---|---|---|
| **Aster core** (P0) | Constellation CRUD, WAL, MVCC, recovery | synthetic constellations, fixed seed; ≥3 edge cases (empty, max-slots, torn write) | put N cx → read `base`/`slot_*` CFs back **byte-exact**; `kill -9` mid-write → recover → assert last-acked present, un-acked absent (read WAL + manifest) |
| **Forge** (P1) | matmul/distance/quantize, CUDA sm_120 + SIMD | synthetic random vectors (fixed seed) + reference outputs from a trusted lib (numpy/BLAS) → **golden files** | CPU vs GPU **bit-parity ≤ 1e-3**; matmul vs cuBLAS ref within 10%; TurboQuant unbiased-inner-product within distortion bound — read computed vs golden |
| **Registry / lenses** (P2) | hot add/retire, frozen contract, capability cards | **real embedder models** (HF, §3) + a small labeled corpus | embed a known input twice → identical (deterministic); `weights_sha256` matches; dim = `Slot.shape`; add lens → read panel + observe backfill on the cx columns; frozen mutation → `CALYX_LENS_FROZEN_VIOLATION` |
| **Sextant** (P3) | multi-lens RRF, per-slot ANN | **real retrieval benchmark with qrels** (BEIR / MS MARCO subset) | measure recall@10 multi-lens vs single-lens against the qrels → assert **Δ ≥ 15%**; every Hit carries provenance (read LedgerRef) |
| **Loom / Assay** (P4) | cross-terms, KSG/NMI MI, differentiation contract, `n_eff`, sufficiency | **labeled classification dataset** (label = grounded anchor) + a **planted-redundancy synthetic** (two lenses corr > 0.6) | compute per-lens MI + pairwise corr → read `bits_about`/`assay` rows; assert `≥0.05`/`≤0.6` gates (planted-redundant lens REJECTED); `I(panel;anchor)` reported with CI; per-stratum bits present |
| **Lodestar** (P5) | directed-MFVS kernel, kernel recall | **dictionary/definition graph** (WordNet/Wiktionary) + a **synthetic graph with a planted MFVS** | synthetic: assert the algorithm finds the planted feedback-vertex-set; real: build kernel → **kernel-only recall ≥ 0.95·full** (read both, compare); grounding gaps listed |
| **Ward** (P6) | `Gτ` calibration, novelty | clean set + **injection/OOD set** (prompt-injection corpus); for identity: **speaker-verification set** (VoxCeleb) | calibrate τ on grounded outcomes → **injection block ≥ 99% at calibrated FAR** (read per-slot cos + verdict); valid-novelty → new region; conflicting-anchor dedup never merges |
| **Ledger** (P7) | hash-chain, reproduce() | any ingested data (synthetic + real) | verify chain intact; flip one byte → `verify_chain` detects break at the right seq; `reproduce(answer)` → bit-parity within tolerance |
| **Anneal / `J`** (P8, A32) | self-opt loop, growth curve | **real corpus + query stream** | 1e6-query soak → read p99 + recall + `J` over time: **p99 ↓ ≥ 20%, no recall regression, `J` rises, Goodhart held-out passes**; every change reversible (read Ledger `kind=Anneal`) |
| **Temporal / dedup / recurrence** (P2b, A27–A29) | E2/E3/E4, dedup, recurrence series | **synthetic event stream with planted periodicity + planted duplicates** + real timestamped logs | assert recurrence signature fires (content agree + time differ); period detected = planted period; dedup merges duplicates, **never merges conflicting anchors**; oracle self-consistency computed from recurring outcomes |
| **Oracle** (P6b, A20) | consequence prediction, sufficiency, super-intelligence predicate | **a domain with a real deterministic oracle** — **SWE-bench Lite** (code + test pass/fail, the paper's own instantiation) | predict Pass/Fail → measure `I(panel;oracle)` (expect the paper's ≈0.46 deficit on a form-only panel → sufficiency-refusal fires); calibration capped at `oracle_self_consistency`; reverse_query recovers a known cause |
| **Universal data layer** (P4c, A19) | collections-as-any-model | synthetic per-paradigm fixtures (rows/docs/KV/TS/blob) | each paradigm's root op (point/range/join/aggregate/traverse) → read back; one cross-model txn spans modes atomically (read consistent seq) |
| **Memory/GC** (P8b, A26) | reclaimers, watchdog | synthetic high-churn / long-reader / disk-pressure workloads | 1e7-op soak → RSS/VRAM bounded; tombstones reclaimed; long reader aborted on lease → old version GC'd (read disk/heap metrics) |

Every row is proven by **reading the persisted bytes/numbers**, recording evidence in a GitHub issue (`AICodingAgentSuperPrompt.md` §3/§4); no harness asserts success.

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
- **Build:** compile on aiwonder (it has the toolchain/GPU) or cross-build and sync the binary to `/opt/leapable/calyx/` (no `rustc` on box today → see `16`); the authoritative build artifact lives on aiwonder.
- **Store:** Aster vaults + all datasets live on aiwonder ZFS (`/zfs/hot/calyx`, `/zfs/archive/calyx`); the source-of-truth bytes FSV reads are *there*.
- **Run:** `calyxd` + the resident lens/TEI services run on aiwonder (the RTX 5090, sm_120).
- **Test:** every test — synthetic mechanics and real-dataset intelligence — executes **on aiwonder**, reading aiwonder's persisted state. Local runs are for authoring only and never count as FSV.
- **Reach it:** SSH via `~/.config/aiwonder.env` (`16 §0`); secrets via Infisical.

So the FSV loop in practice: author on WSL → sync/build on aiwonder → ingest synthetic + real data on aiwonder → read aiwonder's Aster/Ledger/metrics bytes → record evidence in a GitHub issue. The project's *truth* is the state on aiwonder, nowhere else.

## 6. Verification maturity & edge audits (inherited)

Aim for **L3+** verification (`AICodingAgentSuperPrompt.md` §4.7): not "it returned ok" but "the SoT changed exactly as specified, edges included." **≥3 edge audits per code path** (more for security/guard/dedup): e.g. dedup edges = identical content / near-threshold / conflicting-anchor / temporal-only-difference. When a test fails, **STOP and root-cause** (5 Whys to a structural cause), never patch the symptom.

## 7. The data/FSV `BUILD_DONE` contribution

```
DATA := datasets/MANIFEST.md lists ≥1 verified real dataset per (modality × outcome-type)
        ∧ each dataset checksum-verified present on aiwonder (§3.2)
        ∧ every §2 aspect has a passing FSV (synthetic mechanics + real intelligence), evidence in issues
        ∧ all tests executed on aiwonder against persisted state (§5)
```
Added to the `BUILD_DONE` conjunction (`19`).

**One sentence:** FSV for every Calyx aspect is concrete — synthetic deterministic data proves the mechanics (storage, math, dedup, kernel, guard, recovery) by reading the exact persisted bytes, and a catalog of real datasets (text/code/graph/audio/image/temporal/adversarial, from HuggingFace/Kaggle, acquired and checksum-verified onto aiwonder) proves the intelligence (recall, bits, kernel recall, oracle accuracy, growth `J`) against grounded ground truth — all built, run, stored, and tested on the aiwonder datacenter box, with secrets (the HuggingFace token) from Infisical, and every claim recorded by reading the source of truth, never a harness.
