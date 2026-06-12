# PH70 — Intelligence validation on real corpora

**Stage:** S18 — Datasets & Intelligence FSV  ·  **Crate:** `(cross-crate)`  ·
**PRD roadmap:** `28 §2`  ·  **Axioms:** A2, A8, A32

## Objective

Prove every intelligence claim against grounded ground truth on aiwonder, using the
verified dataset catalog from PH69. For each aspect — Sextant recall, Assay
bits/contract, Lodestar kernel recall, Ward injection-block, Oracle sufficiency,
Anneal J growth — run the engine against persisted state on aiwonder, read back the
persisted numbers/bytes, and record evidence (screenshots + readback output) in the
phase GitHub issue. No harness verdicts count; the bytes are the verdict (DOCTRINE §0).

## Dependencies

- **Phases:** PH69 (all datasets verified present with MANIFEST rows);
  PH24 (Sextant RRF fusion), PH30 (Assay sufficiency + attribution),
  PH33 (Lodestar kernel index + recall), PH38 (Ward τ calibration),
  PH48 (Anneal J objective + growth curve), PH49 (Oracle sufficiency gate).
- **Note:** PH70 cannot start until all six engine phases (PH24/PH30/PH33/PH38/PH48/PH49)
  are DONE and PH69 datasets are verified present on aiwonder.
- **Provides for:** `BUILD_DONE` DATA clause; Stage 18 exit.

## Current state (build off what exists)

Greenfield. All engines (Sextant, Assay, Lodestar, Ward, Anneal, Oracle) are built
in their respective phases. PH69 delivers the verified corpora. PH70 is the
cross-cutting validation harness that exercises each engine on real data and reads
persisted metrics.

Related scale evidence: #640 proves the Sextant embedded-scale absolute budget
surface at 1e6 synthetic cx on aiwonder (SingleLens p99=686 us, RRF-6 p99=3570
us, pipeline p99=17507 us, exact known-I/O readback, Aster vault bytes read).
That evidence supports PH70 readiness, but it does not satisfy PH70 T01's
real-qrels multi-lens recall delta exit gate.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `scripts/validate_sextant_recall.sh` | Ingest BEIR/MS MARCO qrels; run multi-lens vs single-lens; read recall metric from Aster/Ledger |
| `scripts/validate_assay_bits.sh` | Ingest AG News/banking77; compute per-lens MI + differentiation contract; read `bits_about` CF rows |
| `scripts/validate_lodestar_kernel.sh` | Build kernel on ≥3 graph corpora; read kernel-only recall vs full-recall metrics |
| `scripts/validate_ward_guard.sh` | Calibrate τ on clean set; run injection corpus; read per-slot guard verdict counts |
| `crates/calyx-ward/src/polis.rs` | Validate Polis 21-slot civic synthetic personas; read guard verdicts + tie outcomes |
| `scripts/validate_oracle_sufficiency.sh` | Ingest SWE-bench Lite; run Oracle sufficiency gate; read `I(panel;oracle)` and deficit |
| `scripts/validate_anneal_j.sh` | Run 1e6-query soak; read J growth curve over time; capture Grafana screenshot |
| `scripts/ph70_evidence_bundle.sh` | Collect all readback outputs + screenshots into a GitHub issue evidence bundle |
| `crates/calyx-cli/src/temporal_log_recurrence_readback/` | Validate real timestamped logs through recurrence signature, periodic fit, and next-occurrence readback |
| `crates/calyx-ward/tests/identity_fsv/voxceleb_identity/` | Validate Ward speaker identity-lock on real VoxCeleb audio: WavLM speaker slot, per-slot guard verdicts, and speaker-MI readbacks |
| `crates/calyx-assay/src/mmd.rs` | Validate drift-pair distribution shift via Gaussian-kernel MMD and change-point readbacks |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Sextant recall validation — Δ≥15% on real qrels | — (PH69+PH24 done) |
| T02 | Assay bits/contract validation — labeled classification corpora | T01 or parallel |
| T03 | Lodestar kernel-only recall validation — ≥0.95 on ≥3 corpora | T01 or parallel |
| T04 | Ward injection-block validation — ≥99% at calibrated FAR | T01 or parallel |
| T05 | Oracle sufficiency validation — SWE-bench ≈0.46 deficit | T01 or parallel |
| T06 | Anneal J growth curve validation — real corpus soak | T01 or parallel |

| T07 | Living concert FSV - A31 engines operating together on one corpus loop | T01 or parallel |
| T08 | Polis civic-panel constellation/guard FSV - synthetic personas | T04 or parallel |
| T09 | Temporal real-log recurrence / next-occurrence FSV | PH41 + PH69 temporal logs |
| T10 | Ward speaker identity-lock + speaker-MI FSV - VoxCeleb | PH39 + PH69 VoxCeleb |
| T11 | Drift / change-point / MMD FSV - month-A/month-B drift pair | PH69 drift pair |
| T12 | Dedup correctness FSV - QQP/PAWS merge precision/recall + conflicting-anchor never-merge | PH41 + PH69 QQP/PAWS |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Per PRD `28 §2c` and `28 §7`: each intelligence metric proven on real data by
reading the persisted numbers/bytes on aiwonder (not a harness return value):

1. Sextant: `recall@10` multi-lens Δ≥15% over single-lens, read from the metric
   CF row on aiwonder; every Hit carries a `LedgerRef`.
2. Assay: per-lens `bits_about` ≥0.05; planted-redundant lens (corr > 0.6) rejected;
   `I(panel;anchor)` with CI present; read from `assay` CF rows.
3. Lodestar: kernel-only recall ≥0.95·full on ≥3 real corpora (WordNet, Cora, + 1
   more from PH69); read from the `kernel_recall` metric file on aiwonder.
4. Ward: injection-block ≥99% at calibrated FAR on the prompt-injection corpus;
   read from the `guard_verdicts` CF row / metric file.
4b. Polis civic: 21-slot civic panel guard over synthetic personas; planted tie
   pairs pass all required slots, planted non-ties fail the intended slots; read
   guard verdicts and tie outcomes from the persisted proof artifact.
4c. Temporal recurrence: real timestamped log rows fire recurrence signatures,
   persist one recurrence series, detect the observed cadence, and predict the
   next occurrence from a cold-open Aster vault artifact.
4d. Ward speaker identity: real VoxCeleb WAV bytes embed through the WavLM
   speaker slot; calibrated tau separates genuine and impostor pairs; same-speaker
   guard pairs pass, impostor pairs reject closed, and speaker-MI exceeds the
   load-bearing threshold; read fixture, embedding, guard, MI, edge, and checksum
   artifacts from the aiwonder evidence root.
4e. Drift/MMD: the PH69 drift-pair month-A vs month-B split produces a
   significant Gaussian-kernel MMD report and a change-point near the split
   boundary, while month-A vs month-A-control stays non-significant; read the
   persisted report and edge artifact from the aiwonder evidence root.
5. Oracle: `I(panel;oracle)` ≈0.46 deficit on SWE-bench Lite form-only panel →
   sufficiency-refusal fires; read from the `oracle_sufficiency` metric.
6. Anneal: `J` rises over a real corpus soak; `p99 ↓ ≥ 20%`; no recall regression;
   Goodhart held-out passes; read `J` values from the metric CF / Grafana.

Evidence (screenshots of Grafana J-curve + terminal readback) attached to PH70
GitHub issue.

## Risks / landmines

- **Engine deps not done:** PH70 is a hard blocker on PH24/PH30/PH33/PH38/PH48/PH49
  — do not attempt PH70 until all six are individually FSV'd.
- **Persisted-state prerequisite:** each validation script must first ingest the
  relevant dataset into a real Aster vault on aiwonder; the metric read-back reads
  from persisted Aster CF rows, not from in-memory return values.
- **SWE-bench run environment:** running SWE-bench requires Docker + the repo
  under test to be checked out; this is on aiwonder (RTX 5090, Docker installed).
  The Oracle validation does not re-run SWE-bench from scratch — it reads the
  persisted `I(panel;oracle)` metric computed during PH49 against the ingested
  SWE-bench Lite dataset.
- **Anneal soak time:** a 1e6-query soak takes wall time; use `reflex_register`
  (Synapse) to fire on completion rather than polling; evidence captured once the
  soak completes.
- **Screenshot as evidence:** Grafana J-curve screenshots are the FSV evidence for
  T06; use Synapse `capture_screenshot` + `audit_export_bundle` to attach them to
  the issue (PRD `28 §2c`).
