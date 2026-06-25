# Medical Search — Discovery Findings Log (index)

This directory is the **append-only record of all biomedical association-mining work**: every
search, exploration, calibration, sweep, chain walk, and hypothesis is written here as a dated
`.md` file. Nothing about the discovery program lives only in chat or in a GitHub comment — it lands
here, traceably.

- **Strategy / plan:** [`../../docs2/BIOMEDICAL_DISCOVERY_STRATEGY.md`](../../docs2/BIOMEDICAL_DISCOVERY_STRATEGY.md)
- **Recording rule:** one file per atomic task (named `NN_<topic>.md`), using `_TEMPLATE.md`.
  Record what was run, the exact commands, the raw outputs/FSV evidence, and the honest conclusion
  (including refusals + per-sensor deficits — a refusal is a finding, not a failure).
- **GitHub:** every file here corresponds to a `[DISCOVERY]` issue; cross-link both ways.

## Honesty contract (binding)
Only record a result as "grounded" if it cleared the Calyx honesty gate
(`I(panel;outcome) ≥ H(outcome)`). Never assert a capability or a finding that was not actually
run and verified against stored artifacts. A discovered association is a **ranked, traceable
hypothesis**, never a verdict — it carries its full provenance chain and a sufficiency proof, and
still requires experimental confirmation.

## Files
| File | Task / topic | Status |
|---|---|---|
| `_TEMPLATE.md` | findings template | — |
| `01_anchors_at_ingest.md` | #868 anchors-at-ingest: thread typed anchors through streaming ingest | ✅ done + FSV |
| `05_degraded_flag_fix.md` | #872 degraded flag ignores retrieval-only temporal sidecar absence | done + FSV |
| `06_calibration_fsv.md` | #873 planted signal calibration and gate readback | done + FSV |
| `07_power_gate_verify.md` | #874 assay power-calibration and entropy-floor gate verification | done + FSV |
| `08_blind_spot_sweep.md` | #875 blind-spot sweep/ranking log | implementation slice + synthetic FSV; corpus sweep pending |
| `09_domain_bridges.md` | #876 domain-pair bridge B-term ranking report | implementation slice + synthetic FSV; corpus bridge mining pending |
| `10_spectral_communities.md` | #877 spectral community and inter-community bridge report | implementation slice + synthetic FSV; corpus run pending |
| `11_discovery_harness.md` | #878 gated discovery chain harness | implementation slice + synthetic FSV; corpus run pending |
| `12_probe_matrix.md` | #879 probe matrix planner and productive-combination log | implementation slice + synthetic FSV; corpus run pending |
| `13_chain_walks_synthetic.md` | #880 grounded chain-walk report for static/operator seeds | implementation slice + synthetic FSV; corpus chain walks pending |
| `14_hypothesis_evaluation.md` | #881 transparent multi-prompt hypothesis evaluation report | implementation slice + synthetic FSV; real evaluator runs pending |
| `15_ranked_hypotheses.md` | #882 ranked traceable hypothesis list | implementation slice + synthetic FSV; real ranked list pending |
| `16_refusal_driven_expansion.md` | #883 refusal-expansion planner and before/after verifier | implementation slice + synthetic FSV; real evidence addition pending |
| `17_discovery_vault_molecular.md` | #884 molecular discovery vault source-data preflight | preflight + FSV |
| `18_oracle_event_structuring.md` | #885 Oracle event/domain structuring for recurrence reverse-query | done + FSV |
| (added as work proceeds) | | |
