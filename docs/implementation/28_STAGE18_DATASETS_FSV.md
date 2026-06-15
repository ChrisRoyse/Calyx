# Stage 18 — Datasets & Intelligence FSV (PH69–PH70)

Mechanics are FSV'd on synthetic data throughout; **intelligence** must be FSV'd
on **real** datasets, acquired + checksum-verified onto aiwonder, against
grounded ground truth. This stage gathers the catalog and runs the intelligence
validations. *PH69 (acquisition) can run early — start once PH00 exists.*

---

## PH69 — Dataset acquisition + MANIFEST + checksum FSV
- **Objective.** Gather a variety of real datasets so every lens family + every
  intelligence metric has a real, grounded test, each verified on arrival.
- **Deps.** PH00 (HF token in env; storage under `CALYX_HOME/datasets` →
  `/zfs/archive/calyx/datasets`).
- **Deliverables.** the PRD `28 §3` catalog acquired to
  `/zfs/archive/calyx/datasets/<name>/`: BEIR/MS MARCO (retrieval qrels),
  AG News/banking77 (classification anchors), **SWE-bench Lite** (deterministic
  test oracle), WordNet/Cora (graph/kernel), Quora/PAWS (dedup), VoxCeleb
  (speaker), prompt-injection corpora, temporal logs, etc.; `datasets/MANIFEST.md`
  (name, source, version, sha256, rows, license, what-it-tests).
- **Key tasks.** download with `hf_hub_token` (Kaggle creds added to Infisical if
  needed); acquisition is itself FSV'd (record expected rows/bytes/sha256 →
  download → read back → assert == expected → MANIFEST row).
- **FSV gate.** ≥1 verified dataset per (modality × outcome-type); each checksum-
  verified present on aiwonder; MANIFEST rows match readback (PRD `28 §3.2`).
- **Axioms/PRD.** `28 §3`, A2, A34 (free sources).

## PH70 — Intelligence validation on real corpora
- **Objective.** Prove the intelligence claims against grounded ground truth on
  aiwonder.
- **Deps.** PH69, and the engines they test (PH24/PH30/PH33/PH38/PH48/PH49).
- **Deliverables.** the per-aspect real-data FSV runs (PRD `28 §2`): Sextant
  recall Δ≥15% (qrels), Assay bits/contract (labeled classes), Lodestar kernel-
  only recall ≥0.95 (≥3 corpora), Ward injection-block ≥99% (calibrated FAR),
  Oracle sufficiency (SWE-bench ≈0.46 deficit), Anneal `J` growth curve.
- **Key tasks.** run each on aiwonder against persisted state; record evidence
  (readback + screenshots of the `J` curve / Grafana) in GitHub issues; no
  harness verdicts.
- **FSV gate.** each intelligence metric proven on real data by reading the
  persisted numbers/bytes; evidence bundles attached to issues (PRD `28 §2c`).
- **Axioms/PRD.** `28 §2`, A2, A8, the `BUILD_DONE` intelligence clauses.

---

## Stage 18 exit
Mechanics proven on synthetic data (throughout), intelligence proven on a real,
checksum-verified dataset catalog (recall/bits/kernel/oracle/J) — all built/run/
stored/tested on aiwonder, evidence in issues — PRD `DATA`.
