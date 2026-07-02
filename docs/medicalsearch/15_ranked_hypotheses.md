# 15 - ranked hypotheses

- **Issue:** #882   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25 / real FSV 2026-07-02   **Vault/panel:** #881 retained real evaluator outputs from #880 anchored-corpus hypotheses
- **Goal:** rank surviving A-B-C hypotheses by novelty, grounded confidence, cross-domain distance, evaluator plausibility, sufficiency proof, and provenance.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue882_ranked_hypotheses_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue882-20260625T120857Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo"
scp issue882-20260625T120857Z.tar aiwonder:/home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo.tar -C /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z cargo test -p calyx-lodestar --test issue882_ranked_hypotheses_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/repo && bash scripts/linecount.sh"

# final live-checkout FSV after push/pull on aiwonder
ssh aiwonder "cd /home/croyse/calyx/repo && git pull --ff-only"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue882-ranked-hypotheses-final-20260625T121100Z; mkdir -p \"$root\"; cd /home/croyse/calyx/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue882_ranked_hypotheses_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/ranked_hypotheses.rs`
- `crates/calyx-lodestar/tests/issue882_ranked_hypotheses_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue882_ranked_hypotheses_tests -- --nocapture`: 4 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z`
- Artifact: `/home/croyse/calyx/fsv/issue882-ranked-hypotheses-20260625T120857Z/issue882_ranked_hypotheses_readback.json`
- Artifact bytes: `2817`
- Artifact SHA256: `460fdd90d759a774c750e6c6d021d725b98dbf666b4fbc05fd2fa793c7366124`

aiwonder final live-checkout FSV:
- FSV root: `/home/croyse/calyx/fsv/issue882-ranked-hypotheses-final-20260625T121100Z`
- Artifact: `/home/croyse/calyx/fsv/issue882-ranked-hypotheses-final-20260625T121100Z/issue882_ranked_hypotheses_readback.json`
- Artifact bytes: `2817`
- Artifact SHA256: `460fdd90d759a774c750e6c6d021d725b98dbf666b4fbc05fd2fa793c7366124`
- Readback scalar leaves:
  - `schema_version=1`
  - `input_count=3`
  - `ranked_count=3`
  - `human_review_count=2`
  - `top_hypothesis_id=h-top`
  - `top_rank=1`
  - `top_rank_score=0.8999999761581421`
  - `top_human_review_flag=True`
  - `top_evidence_count=1`
- aiwonder tests from archived source: 4 passed, 0 failed, 0 ignored.
- aiwonder tests from final live checkout: 4 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0 for archived source and final live checkout.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines` for archived source and final live checkout.

Boundary and edge behavior covered by tests:
- Rank score combines novelty, grounded confidence, normalized cross-domain distance, and evaluator plausibility.
- Ranked rows retain sufficiency proof, provenance, evidence IDs, and A-B-C nodes.
- Human-review flags apply only after deterministic ranking and score-floor checks.
- `max_ranked` truncates after sorting.
- Empty inputs, zero cross-domain distance, missing sufficiency proof, and non-finite scores fail closed with `CALYX_KERNEL_INVALID_PARAMS`.

## Findings (honest)
- Lodestar now has a serializable ranked-hypothesis report for surviving evaluated hypotheses.
- The report can flag top candidates for human review without converting hypotheses into verdicts.
- The 2026-06-25 slice was not a real biomedical ranked list. It was the output/report surface for later real chain/evaluator rows.

## 2026-07-02 real ranked-list FSV

Real source:
- #881 evaluator artifact: `/home/croyse/calyx/fsv/issue881-real-hypothesis-evaluation-20260702T093012Z/hypothesis_evaluation_report.json`
- #881 evaluator artifact SHA256: `836a00ca7bc137194e1ea60831e4110283252fe9f17fe8e8d1ce15f49ccd470b`
- #881 readback summary SHA256: `5654f7d9780b6fbca5e6a2ad000434d1448d0c78e6a36a824e4bd76534a7f43f`
- #881 retained rows consumed for ranking: `44`

Implementation added for this real run:
- `calyx hypothesis-rank --input <json> --out <json>`
- The command persists `RankedHypothesisReport` plus source input path, bytes, and SHA256, then separately re-reads the physical report bytes before printing `status=ok`.

Command path:
```bash
cargo run -p calyx-cli -- hypothesis-rank \
  --input /home/croyse/calyx/fsv/issue882-real-ranked-hypotheses-20260702T100214Z/ranked_hypotheses_input.json \
  --out /home/croyse/calyx/fsv/issue882-real-ranked-hypotheses-20260702T100214Z/ranked_hypotheses_report.json \
  --max-ranked 44 \
  --review-top-n 10 \
  --min-review-score 0.65
```

Persisted artifacts:
- FSV root: `/home/croyse/calyx/fsv/issue882-real-ranked-hypotheses-20260702T100214Z`
- Ranking input SHA256: `ba8558d19b4fae0fdc8a7725fbfc42503ca3f78dddcbca6faa15c9ad1268c00a`
- Ranking report SHA256: `0483d8bc475526f65d76cd2fbb8a2a42c59751fa463c338b6e3fba54ac992257`
- CLI stdout SHA256: `c30da197be9a6734ad16e58089fa83b31805acdea23e5646949daaffdb1db55e`
- CLI stderr SHA256: `cf39a82fa5cd874bf1def6db7969dcbde3e2ace8786d2a96310e3731ecbba5e0`
- Readback summary SHA256: `229260e9c1c24a7ded5919f992b2d04e8736dbfa337474f7e7e2d2eb978534c3`

Readback leaves:
- `input_count=44`
- `ranked_count=44`
- `human_review_count=10`
- `top_hypothesis_id=operator-centrality-2::01`
- `top_rank_score=0.82295454`
- `top_evaluator_aggregate_score=0.7325`
- `top_cross_domain_distance=51`
- `top_evidence_count=3`

Top 10 ranked traceable hypotheses:

| Rank | Hypothesis | Rank score | Novelty | Grounded | Distance | Plausibility | Eval aggregate | Evidence | Human review |
|---:|---|---:|---:|---:|---:|---:|---:|---:|---|
| 1 | `operator-centrality-2::01` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.73250 | 3 | yes |
| 2 | `operator-centrality-2::02` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.73250 | 3 | yes |
| 3 | `operator-centrality-2::03` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.73250 | 3 | yes |
| 4 | `spectral-bridge-2-src::01` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.75250 | 3 | yes |
| 5 | `spectral-bridge-2-src::02` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.75250 | 3 | yes |
| 6 | `spectral-bridge-2-src::03` | 0.822955 | 0.50 | 1.00 | 51 | 0.850 | 0.75250 | 3 | yes |
| 7 | `spectral-bridge-2-src::04` | 0.819318 | 0.50 | 1.00 | 50 | 0.850 | 0.74250 | 3 | yes |
| 8 | `spectral-bridge-2-src::05` | 0.819318 | 0.50 | 1.00 | 50 | 0.850 | 0.74250 | 3 | yes |
| 9 | `spectral-bridge-2-src::06` | 0.819318 | 0.50 | 1.00 | 50 | 0.850 | 0.74250 | 3 | yes |
| 10 | `spectral-bridge-2-src::07` | 0.809432 | 0.50 | 1.00 | 49 | 0.825 | 0.73375 | 3 | yes |

Scoring boundary:
- Rank score combines novelty, grounded confidence, normalized cross-domain distance, and evaluator plausibility.
- Repeated or near-duplicate endpoint rows remain separate traceable hypotheses because their A/B/C CxIds and provenance differ; deduplication for presentation would be a separate policy layer, not part of this acceptance criterion.
- Human-review flags are triage markers only. They are not wet-lab validation, biomedical verdicts, treatment recommendations, or claims of clinical utility.

## Conclusion & next step
The #882 acceptance criterion is complete: retained real #881 evaluator outputs were ranked into a persisted, traceable hypothesis list with provenance/evidence IDs, sufficiency proofs, human-review flags, and aiwonder physical readback. The next queue item is #884 molecular-vault commissioning and clinical-to-molecular bridge proof.
