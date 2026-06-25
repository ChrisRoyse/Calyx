# 15 - ranked hypotheses

- **Issue:** #882   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic ranked hypotheses while #869 corpus ingest runs
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
- This is not a real biomedical ranked list yet. It is the output/report surface that will receive real chain/evaluator rows once #869/#870/#871/#880/#881 produce anchored candidates.

## Conclusion & next step
The #882 ranked-list surface is ready. Keep #882 open until the real anchored hypotheses are ranked with full provenance chains and sufficiency proofs read back from aiwonder.
