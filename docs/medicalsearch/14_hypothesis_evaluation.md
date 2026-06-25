# 14 - hypothesis evaluation

- **Issue:** #881   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic evaluator rows while #869 corpus ingest runs
- **Goal:** give each surviving A-B-C hypothesis a transparent evaluator score, justification, falsification test, and cited grounded evidence.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue881_hypothesis_evaluation_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue881-20260625T120221Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo"
scp issue881-20260625T120221Z.tar aiwonder:/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo.tar -C /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z cargo test -p calyx-lodestar --test issue881_hypothesis_evaluation_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/repo && bash scripts/linecount.sh"

# final live-checkout FSV after push/pull on aiwonder
ssh aiwonder "cd /home/croyse/calyx/repo && git pull --ff-only"
ssh aiwonder "root=/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-final-20260625T120500Z; mkdir -p \"$root\"; cd /home/croyse/calyx/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=\"$root\" cargo test -p calyx-lodestar --test issue881_hypothesis_evaluation_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/hypothesis_evaluation.rs`
- `crates/calyx-lodestar/tests/issue881_hypothesis_evaluation_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue881_hypothesis_evaluation_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder archived-source FSV:
- FSV root: `/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z`
- Artifact: `/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-20260625T120221Z/issue881_hypothesis_evaluation_readback.json`
- Artifact bytes: `3365`
- Artifact SHA256: `e5a70ad454b238b44222e0bf6a93fb17e23176e67e0365e7ea2e25d90e8ed936`

aiwonder final live-checkout FSV:
- FSV root: `/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-final-20260625T120500Z`
- Artifact: `/home/croyse/calyx/fsv/issue881-hypothesis-evaluation-final-20260625T120500Z/issue881_hypothesis_evaluation_readback.json`
- Artifact bytes: `3365`
- Artifact SHA256: `e5a70ad454b238b44222e0bf6a93fb17e23176e67e0365e7ea2e25d90e8ed936`
- Readback scalar leaves:
  - `schema_version=1`
  - `input_count=2`
  - `evaluation_count=2`
  - `retained_count=1`
  - `rejected_count=1`
  - `top_hypothesis_id=h-top`
  - `top_aggregate_score=0.815000057220459`
  - `top_prompt_variant_count=2`
  - `top_temperature_variant_count=2`
  - `top_evidence_count=1`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder tests from final live checkout: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0 for archived source and final live checkout.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines` for archived source and final live checkout.

Boundary and edge behavior covered by tests:
- Multiple prompt IDs and multiple temperature settings are required and counted.
- Plausibility, novelty, testability, and falsifiability dimensions aggregate into a transparent score.
- Retrieved evidence must be cited by evaluator runs; missing citations fail closed.
- Too few evaluator runs and non-finite scores fail closed with `CALYX_KERNEL_INVALID_PARAMS`.
- Evidence insufficiency is represented explicitly as `needs_more_evidence`.
- `max_ranked` truncates after deterministic score sorting.

## Findings (honest)
- Lodestar now has a serializable hypothesis-evaluation report for externally produced evaluator runs.
- The implementation validates transparent score dimensions, justifications, falsification tests, prompt diversity, temperature diversity, and evidence citations.
- This does not claim a real LLM evaluated real biomedical hypotheses. It is the report/aggregation surface needed to store and verify those runs once real surviving chains exist.
- Final #881 acceptance still requires RAG over grounded provenance abstracts and real LLM evaluator outputs for surviving A-B-C hypotheses after #869/#870/#871/#880 real runs.

## Conclusion & next step
The #881 transparent evaluation ledger is ready for real evaluator rows. Keep #881 open until real surviving hypotheses are evaluated with multiple prompt/temperature runs and cited evidence read back from aiwonder.
