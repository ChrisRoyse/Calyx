# 19 — Roadmap, FSV Protocol & `BUILD_DONE`

Phased build, verification discipline, performance targets, mechanical completion predicate. Scope is **Vault-only** (`15`): Calyx replaces the SQLite/`sqlite-vec` Vaults; PostgreSQL is untouched, so there is no control-plane phase. Front-loads the durable-storage core before the customer-facing Vault swap.

> **Current status (2026-06-12; Stage 9 PH40-PH42 closeout current): P0-P7 are DONE,
> PH40 #373-#378 plus #615/#616/#618/#619 are FSV-backed, PH41 T01-T08
> #379-#386 plus #623/#578/#621/#624/#617/#622/#620/#626/#627/#628 are
> FSV-backed, and PH42 readback-surface gate #625 is FSV-backed.** Stages 0-8 (PH00-PH39) are implemented, pushed, and FSV-signed-off on
> aiwonder. Stage 6 Lodestar is closed through #240 plus readiness follow-ups
> #331/#332, docs caveat #629, helper split #632, and real-corpus readbacks
> #630/#631; Stage 7 Ledger is closed through #256; Stage 8 Ward is closed
> through exit #280 after #258-#274, #275-#279, #349, #350, #351, #352, #353,
> #354, #355, #356, #357, #358, and #359. The #280 exit root is
> `/home/croyse/calyx/data/fsv-issue280-stage8-exit-20260609-477d4a4`; full
> manifest SHA-256
> `5849dada4934955e4e60ef83588adfff4782297bbc78d7d7a319d42a03d5b58c`. Live
> PH40 roots now include #373 policy manifest persistence, #373 cold-open
> policy hardening, #374 time-window filtering, #375 temporal boost readback,
> #376 causal-gate readback, #377 temporal-search integration, #378
> temporal-never-dominant proof, #615 AP-60 final-surface hardening, and PH41
> #379 DedupPolicy manifest persistence, #380 dedup engine cosine-gate
> readback, #381 anchor-conflict guard readback, #382 ingest_at readback,
> #383 recurrence series store readback, #384 recurrence signature detector
> readback, #385 dedup audit/undo readback, #386 dedup invariant exit readback,
> #623 recurrence fallback readback, #578 public periodic recurrence
> readback at
> `/home/croyse/calyx/data/fsv-issue578-periodic-recall-20260610-240de5a`.
> #621 recurrence concurrency readback is at
> `/home/croyse/calyx/data/fsv-issue621-recurrence-concurrency-20260610-b1fdf5d`.
> #624 WAL recovery/open serialization readback is at
> `/home/croyse/calyx/data/fsv-issue624-wal-recovery-lock-20260610-1e4b34c`.
> #617 durable dedup policy validation readback is at
> `/home/croyse/calyx/data/fsv-issue617-dedup-panel-validation-20260610-07884d9`.
> #622 recurrence WAL-failure readback is at
> `/home/croyse/calyx/data/fsv-issue622-recurrence-wal-failure-20260610-bf0d380`.
> #640 embedded Sextant scale-budget readback is at
> `/home/croyse/calyx/data/fsv-issue640-embedded-scale-exactfast-20260611T055130Z`
> and proves 1e6-cx release-mode SingleLens/RRF-6/pipeline p99 budgets plus
> exact known-I/O readback; PH70 still owns the real-corpus recall delta gate.
> PH40 follow-ups #616/#618/#619, PH41 follow-ups #620/#626/#627/#628, and PH42
> readback-surface gate #625 are closed and FSV-backed. Newer PH42 gaps such as
> #634/#635/#636 are tracked separately from those closeout gates.
> phase status:
> `docs/implementation/03_PHASE_MAP.md` and
> GitHub context issue #23.
> The detailed per-phase build plan below lives in `docs/implementation/`.

## 1. Phasing principle

Ship **a clear win at low risk** first, prove it by FSV, then take the next. Calyx is a multi-year systems effort; the roadmap guarantees usable value early and never bets the company on the riskiest step.

## 2. Phases

| Phase | Name | Delivers | Exit gate (FSV) |
|---|---|---|---|
| **P0** | Aster core | `calyx-core` + `calyx-aster`: Constellation CRUD, WAL, MVCC, crash recovery, 1-slot panel | round-trip byte-exact; `kill -9` mid-write recovers to last-acked; idempotent ingest proven on bytes |
| **P1** | Forge | CUDA(sm_120) + SIMD matmul/distance/quantize; bit-parity golden tests | CPU↔GPU parity within tolerance on golden corpus; matmul within target of cuBLAS |
| **P2** | Registry + multi-lens | hot add/retire lens, frozen contract, default panels, capability cards | add/retire with no re-embed (lazy backfill observed); 3+ modalities; frozen violation fails closed |
| **P3** | Sextant | per-slot HNSW (embedded), RRF/WeightedRRF/SingleLens, provenance on hits | multi-lens recall@10 ≥ single-lens + Δ on a real corpus; every hit carries lineage |
| **P4** | Loom + Assay | cross-terms (lazy), agreement graph, KSG/NMI MI, differentiation contract, n_eff, sufficiency | bits + pairwise corr computed; ≥0.05/≤0.6 gated before merge (run on aiwonder; no CI pipeline — FSV is CI); DPI ceiling reported; `abundance_report` honest |
| **P5** | Lodestar | directed-FVS kernel, kernel index, kernel_answer, grounding_gaps | kernel built on ≥3 real corpora; final/tuned kernel-only recall ≥ 0.95·full with raw/tuned/pass_mode readback; grounding gaps listed |
| **P6** | Ward | Gτ calibration (conformal), per-slot τ, novelty→new-region | injection corpus blocked ≥99% at calibrated FAR; valid-novelty path proven; τ provenance stored |
| **P7** | Ledger | hash-chain, merkle, reproduce(), audit | chain verifies; a real answer replays within tolerance; tamper breaks chain (detected) |
| **P8** | Anneal | self-heal, mistake-closure, autotune, lens proposal | 1e6-query soak: p99 ↓ ≥ target, no recall regression, no oscillation; every change reversible+logged |
| **P9** | Server (`calyxd`) on aiwonder | systemd, ZFS, GPU budget, Infisical, Prometheus, restic, DR drill | health `"pass"`; metrics live; DR restore byte-verified; coexists with resident TEI under SLO |
| **P10** | DiskANN/SPANN scale | disk-resident graphs, sparse posting tiering | server vault 1e8–1e9 cx within search SLO |
| **P4b** | Array+compression (`23`) | array bundle, grouped GEMM, TurboQuant default, MXFP4, measured-compression contract | grouped GEMM ≥ batched-loop on N-lens panel; TurboQuant unbiased inner-product within distortion bound; quant level accepted only if bits/cosine/FAR preserved (A25) |
| **P4c** | Universal data layer (`20`) | collections-as-any-model: relational/doc/KV/columnar/TS/blob over Aster; cross-model query in one txn | each paradigm's root op verified by readback; one query spans modes atomically |
| **P5b** | Multi-scope kernel (`08 §4b`) | `build_kernel(scope)` over all/collection/domain/subgraph/time/tenant/filter | kernel built at ≥4 scopes on a real corpus, each with measured kernel-only recall |
| **P6b** | Oracle + Q↔A (`21`) | consequence prediction + sufficiency gate; super-intelligence predicate; reverse_query | predict with calibrated confidence capped at oracle self-consistency; refuse when `I(panel;oracle)<H(Y)`; reverse a real answer to its cause |
| **P8b** | Resource/GC hardening (`24`) | bounded caches/queues, reclaimers, VRAM budgeter, long-reader watchdog | the 25-hazard register: each mitigation FSV-proven; 1e7-op soak RSS/VRAM bounded, no leak |
| **P2b** | Temporal + dedup (`25`) | E2/E3/E4 retrieval lenses (AP-60), DedupPolicy/TemporalPolicy at creation, recurrence series, content-slot `Gτ` dedup | dedup never merges conflicting anchors; temporal lenses never dominant; recurring event → one series; merges reversible + audited |
| **P11b** | Critical capabilities (`17 §8`) | streaming ingestion, reactive triggers, time-travel/as-of, universal summarization (multi-scope kernel) | each named capability FSV-proven on a real stream/corpus |
| **P11** | Leapable Vault swap (V0–V2) | `libcalyx` embedded, migration tool, multi-lens user Vaults, `sqlite-vec` retired | shadow parity → flip → Calyx-only; migrate a real `.db` byte-exact on content; user gets kernel/guard; control-plane queries for that Vault return identical results |
| **P12** (optional) | Discover Vault host (V3) | `calyxd` serves published/Discover Vaults on aiwonder | a Discover Vault is Calyx-backed; PostgreSQL control-plane listing/billing **unchanged and verified untouched** |

P0–P8 build the engine; P9–P10 productionize on the box; **P11 is the shippable customer value and the only required Leapable phase**; P12 is optional Vault hosting. **There is no PostgreSQL control-plane phase — that layer is out of scope and untouched (`15`).**

**Recommended first milestone to demo:** P0→P3 + P11-V0 — a Leapable Vault answering with multiple lenses and provenance, shadowing `sqlite-vec`. That alone justifies the project.

## 3. FSV protocol (binding, inherited)

Per-aspect FSV definitions, test-data strategy, and the real-dataset catalog are in **`28_FSV_AND_TEST_DATA.md`** (synthetic data proves mechanics; real datasets prove intelligence; everything built/run/tested on aiwonder).

Every gate above is proven by **direct source-of-truth readback**, not a return value or a harness (Leapable §0, aiwonder §9), and **perceived via Synapse** — full step→ability mapping in `28 §2c`:

1. Identify the bytes that prove the claim (the Aster CF rows, the WAL, the Ledger entry, the ZFS file, the metric). *(Synapse: `set_capture_target`, `health`.)*
2. Read them **before** the action. *(Synapse: `reality_baseline` + `act_run_shell` readback + `read_text`/`find`/`capture_screenshot`.)*
3. Execute the action. *(Synapse: `act_run_shell`/`act_type`/`act_launch`.)*
4. Read them **after**. *(Synapse: `observe`/`observe_delta`/`reality_audit` + `read_text`/`find`.)*
5. Inspect the delta; record evidence in a GitHub issue. *(Synapse: `reality_audit` drift flag + `capture_screenshot`/`replay_record`/`audit_export_bundle`.)* Async ops: `reflex_register` to catch the real end-state.

**FSV scripts/harnesses are banned and cannot satisfy FSV.** Calyx may ship *readback tools* that print bytes for a human/agent to judge — never a green-checkmark harness that asserts success. Synthetic verification data on the box must be cleanup-tagged and provably inert before the turn ends.

### 3b. Per-merge code-quality gate (binding, DOCTRINE §8)
Every merge MUST pass, in addition to `cargo check`/`test`/`clippy -D warnings`:
- **File-size gate:** no `.rs` source/test file > 500 lines (docs unlimited). Run the line-count check (`DOCTRINE §8`). Any over-limit file → **open a GitHub issue to modularize and resolve it** (split per `docs2/modulateprompt.md`: SRP modules, `mod.rs` facade, explicit re-exports, no circular deps, identical public API, tests green) before the gate passes.
- CPU↔GPU bit-parity on the golden set (A13); fail-closed error paths exercised (A16).

## 4. Performance & quality targets (the `X/Y/Δ` in `BUILD_DONE`)

| Metric | Target | Where |
|---|---|---|
| Ingest p95 (1-slot / 15-slot batched) | < 5 ms / < 20 ms | `04` |
| Search p99 (SingleLens / RRF-6 / KernelFirst@1e8 / Pipeline) | < 5 / < 15 / < 25 / < 60 ms | `10` |
| Multi-lens recall@10 vs single-lens (Δ) | ≥ +15% on a real labeled corpus | `10` |
| Forge matmul vs cuBLAS (Y) | within 10% on sm_120 | `13` |
| CPU↔GPU bit-parity | within 1e-3 rel tolerance | `13` |
| Differentiation contract | ≥ 0.05 bits, ≤ 0.6 corr (verbatim) | `07` |
| Kernel-only recall / full recall | ≥ 0.95 | `08` |
| Guard injection-block rate @ calibrated FAR | ≥ 99% | `09` |
| Anneal p99 improvement over 1e6 queries (X) | ≥ 20%, recall non-regressing | `12` |
| Vault migration fidelity (SQLite→Calyx) | byte-exact on content; control-plane responses identical | `15` |
| Crash recovery | byte-exact to last-acked | `04` |

## 5. `BUILD_DONE` predicate (mechanical)

```
BUILD_DONE :=
  CORE       := P0 gate ∧ P1 gate
  LENS       := P2 gate
  SEARCH     := P3 gate ∧ recall Δ ≥ 15%
  DDA_BITS   := P4 gate ∧ contract gated before merge (run on aiwonder; no CI pipeline — FSV is CI) ∧ abundance_report shows DPI ceiling
  KERNEL     := P5 gate ∧ final/tuned kernel-only recall ≥ 0.95·full on ≥3 corpora ∧ raw_recall/tuned_recall/pass_mode read back
  GUARD      := P6 gate ∧ injection block ≥ 99% @ calibrated FAR
  PROVENANCE := P7 gate ∧ reproduce() passes on a real answer ∧ tamper detected
  SELFOPT    := P8 gate ∧ p99 ↓ ≥ 20% over 1e6 queries, no recall regression
  MATH       := P1 gate ∧ matmul within 10% cuBLAS ∧ bit-parity
  ARRAYMATH  := P4b gate ∧ grouped GEMM invariant to N ∧ one co-located array bundle per cx
  COMPRESS   := P4b gate ∧ TurboQuant unbiased inner-product ∧ quant accepted only if bits/cosine/FAR preserved (A25)
  UNIVERSAL  := P4c gate ∧ every paradigm root op served ∧ cross-model query in one txn (20)
  ORACLE     := P6b gate ∧ calibrated consequence prediction ∧ sufficiency-refusal ∧ reverse_query (21)
  KERNEL_ANY := P5b gate ∧ kernel built at ≥4 scopes, each with measured recall (08)
  FORMULAS   := every Royse formula callable + self-tuning (22), exercised in tests run on aiwonder
  RESOURCE   := P8b gate ∧ 25-hazard register FSV-proven ∧ 1e7-op soak bounded, no leak (24, A26)
  TEMPORAL   := P2b gate ∧ E2/E3/E4 retrieval-only (AP-60) ∧ recurrence series ∧ next-occurrence sufficiency-gated (25, A27)
  DEDUP      := P2b gate ∧ content-slot Gτ dedup ∧ never merges conflicting anchors ∧ merges reversible+audited (25, A28)
  RECURRENCE := P2b gate ∧ recurrence signature auto-detected (content agree + time differ) ∧ oracle self-consistency measured from recurring outcomes ∧ frequency feeds kernel/Oracle/Loom (25, A29)
  INTELLIGENCE := J measurable ∧ growth_curve rises on a real corpus under the loop ∧ math self-adjusts parameters online ∧ Goodhart-defended (held-out) ∧ DPI-capped ∧ no data deleted ∧ compression/retrieval as facets (27, A32)
  DEPLOY     := P9 gate ∧ DR restore byte-verified ∧ SLO under resident-TEI load
  SCALE      := P10 gate ∧ 1e9-cx server vault within search SLO
  LEAPABLE   := P11 gate (Vault swap V0–V2; PostgreSQL untouched, verified)   // P12 Discover hosting is optional, not required
  FSV        := every clause proven by direct SoT readback, recorded, no harness

BUILD_DONE := CORE ∧ LENS ∧ SEARCH ∧ DDA_BITS ∧ KERNEL ∧ GUARD ∧ PROVENANCE
            ∧ SELFOPT ∧ MATH ∧ ARRAYMATH ∧ COMPRESS ∧ UNIVERSAL ∧ ORACLE
            ∧ KERNEL_ANY ∧ FORMULAS ∧ RESOURCE ∧ TEMPORAL ∧ DEDUP ∧ RECURRENCE ∧ INTELLIGENCE ∧ DATA ∧ SECURITY ∧ DEPLOY ∧ SCALE ∧ LEAPABLE ∧ FSV
  // DATA := per-aspect FSV passing (synthetic mechanics + real intelligence) ∧ datasets/MANIFEST.md verified on aiwonder ∧ all tests run on aiwonder (28)
  // SECURITY := STRIDE defenses FSV-proven ∧ cross-vault read denied+audited ∧ at-rest+in-transit encryption verified ∧ erase() crypto-shreds (content unrecoverable incl backups/Ledger payload; tombstone remains) ∧ secret-scan clean (30, A33)
```

`LEAPABLE` is satisfied by the **Vault swap alone** (P11). No control-plane clause: PostgreSQL stays as the control plane by design (`15`), so Calyx is complete as a Vault engine — the PG layer is never part of `BUILD_DONE`.

## 6. Team / build notes
- Build `calyxd`/`calyx` **natively on aiwonder** (Rust via rustup is installed — the earlier "no `rustc` on the box" note is superseded; cross-build is retained only as an optional minimal-deploy path) under `CALYX_HOME=/home/croyse/calyx`; no CI pipeline.
- Reuse, don't reinvent: lift `context-graph-mincut`/`-paths`/`-solver`/`-witness` and the `mejepa` Assay/kernel/guard logic as seeds of `calyx-mincut`/`-lodestar`/`-assay`/`-ledger`/`-ward`. Calyx is the *unification and hardening* of code that already works (`17 §2`).
- One change at a time; plan before architectural/auth/storage changes; document failure as carefully as success (Leapable non-negotiables).

## 7. Definition of done, in one sentence
**Calyx is done when an agent can `add_lens`, `ingest`, `anchor`, then `search`/`kernel_answer`/`guard` a real Leapable vault — multi-lens, kernel-grounded, drift-guarded, fully provenanced, self-optimizing — on the aiwonder box, with every claim proven by reading the bytes.**
