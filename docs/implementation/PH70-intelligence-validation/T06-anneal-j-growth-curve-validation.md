# PH70 · T06 — Anneal J growth curve validation — real corpus soak

| Field | Value |
|---|---|
| **Phase** | PH70 — Intelligence validation on real corpora |
| **Stage** | S18 — Datasets & Intelligence FSV |
| **Crate** | `calyx-anneal` |
| **Files** | `scripts/validate_anneal_j.sh` (≤500), `scripts/ph70_evidence_bundle.sh` (≤500) |
| **Depends on** | PH69 T02/T03 (text corpus verified); PH48 (J objective + growth_curve + intelligence_report) |
| **Axioms** | A2, A15, A32 |
| **PRD** | `28 §2` (Anneal/J row), `28 §3` |

## Goal

Prove the Anneal intelligence objective J on a real corpus soak on aiwonder:
J rises over a 1e6-query soak; p99 latency decreases ≥20%; no recall regression;
Goodhart held-out passes. Read J values from the persisted metric CF / Grafana on
aiwonder and capture the J-curve screenshot as FSV evidence. Also collect all PH70
evidence into a bundled GitHub issue attachment.

## Build (checklist of concrete, code-level steps)

- [ ] `scripts/validate_anneal_j.sh`:
      (1) Ingest a real text corpus (AG News or MS MARCO passages, ≥50 K docs) into
          an Aster vault on aiwonder.
      (2) Register a Synapse `reflex_register` trigger on the soak-completion
          condition (metric file written with `soak_status=complete`) so the agent
          FSVs the real end-state the moment it appears, not by polling.
      (3) Run `calyx anneal soak --queries 1000000 --vault calyx_ph70_validation`;
          this writes J samples to `/zfs/hot/calyx/metrics/anneal_j_series.jsonl`
          every 10 000 queries (100 samples total).
      (4) After soak completes (reflex fires): read `anneal_j_series.jsonl`;
          assert J at step 100 > J at step 1 (growth); write summary to
          `/zfs/hot/calyx/metrics/anneal_j_summary.json`.
      (5) Read p99 latency at step 1 and step 100 from the series; assert
          `p99[100] ≤ p99[1] × 0.80` (≥20% decrease); write to
          `/zfs/hot/calyx/metrics/anneal_p99_delta.txt`.
      (6) Read recall at step 100; assert recall ≥ recall at step 1 (no regression).
      (7) Run Goodhart held-out: present an unseen query distribution; assert J does
          not drop (Goodhart held-out passes); write to
          `/zfs/hot/calyx/metrics/anneal_goodhart.txt`.
      (8) Capture a Grafana screenshot of the J-curve panel via Synapse
          `capture_screenshot`; save to
          `/zfs/hot/calyx/metrics/anneal_j_grafana.png`.
- [ ] `scripts/ph70_evidence_bundle.sh`:
      Reads all metric files from `/zfs/hot/calyx/metrics/` (T01–T06 outputs);
      calls Synapse `audit_export_bundle` to package screenshots + readbacks;
      posts the bundle to the PH70 GitHub issue.
- [ ] Fail-closed: J at step 100 ≤ J at step 1 → exits 1,
      `CALYX_FSV_ANNEAL_J_NOT_GROWING`.
      p99 regression (step 100 > step 1 × 1.0) → exits 1,
      `CALYX_FSV_ANNEAL_P99_REGRESSION`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: a synthetic 1000-query, 10-step soak fixture (seed=42); simulate J
      values increasing from 0.5 to 0.8; assert growth detected; assert p99
      decreases by ≥20%.
- [ ] proptest: property that a J-series where J monotonically increases is always
      classified as "growing" by the validation logic.
- [ ] edge (≥3):
      (1) J oscillates (up then down) → validation still passes if final J > initial J;
      (2) recall drops 0.001 between steps → regression flag fires,
          `CALYX_FSV_ANNEAL_RECALL_REGRESSION`;
      (3) Goodhart held-out with severe distribution shift → J drops;
          validation reports `goodhart_pass: false`, exits 1.
- [ ] fail-closed: soak exits without writing `anneal_j_series.jsonl` →
      `CALYX_FSV_ANNEAL_SOAK_INCOMPLETE`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/zfs/hot/calyx/metrics/anneal_j_series.jsonl`,
  `anneal_j_summary.json`, `anneal_p99_delta.txt`, `anneal_goodhart.txt`,
  `anneal_j_grafana.png` on aiwonder.
- **Readback:**
  ```
  # Read first and last J values from the series:
  python3 -c "
  import json, pathlib
  lines = pathlib.Path('/zfs/hot/calyx/metrics/anneal_j_series.jsonl').read_text().splitlines()
  first = json.loads(lines[0]); last = json.loads(lines[-1])
  print('J[0]:', first['j'], 'J[-1]:', last['j'], 'growing:', last['j'] > first['j'])
  print('p99[0]:', first['p99'], 'p99[-1]:', last['p99'], 'p99_pass:', last['p99'] <= first['p99']*0.80)
  "
  cat /zfs/hot/calyx/metrics/anneal_goodhart.txt
  cat /zfs/hot/calyx/metrics/anneal_j_summary.json
  # Grafana screenshot attached as evidence
  ```
- **Prove:** before: metric files absent; after: `anneal_j_series.jsonl` has 100
  lines; J[-1] > J[0] (growth proven); p99[-1] ≤ p99[0] × 0.80; `goodhart_pass: true`
  in goodhart file; Grafana screenshot shows upward J-curve attached to issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output + Grafana J-curve screenshot showing upward trend
      + p99 delta + goodhart pass) attached to the PH70 GitHub issue
- [ ] `ph70_evidence_bundle.sh` runs successfully and posts the full bundle to the
      PH70 GitHub issue (all 6 intelligence metrics in one issue)
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
