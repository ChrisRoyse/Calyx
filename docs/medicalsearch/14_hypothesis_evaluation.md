# 14 - hypothesis evaluation

- **Issue:** #881   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25 / real FSV 2026-07-02   **Vault/panel:** #880 real anchored-corpus chain hypotheses + GitHub Models evaluator runs
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
- The 2026-06-25 slice did not claim a real LLM evaluated real biomedical hypotheses. It was the report/aggregation surface needed to store and verify those runs once real surviving chains existed.

## 2026-07-02 real evaluator FSV

Real source:
- #880 source artifact: `/home/croyse/calyx/fsv/issue880-real-chain-walks-20260702T080913Z/real_chain_walks.json`
- #880 source artifact SHA256: `676e9c27f3e8cc57e82c6124ea5dd41282b034bcf1488e03193ffe25ce5efcfd`
- #880 readback SHA256: `7cc3485e1bb9201f04db2c4ce3a48ea8eb0a1323c878bb78122bdb75c0fdc14b`
- Source rows: `/zfs/archive/calyx/biomed-rx/ingest/anchored-issue869-20260625T080546Z/medmcqa.anchored.jsonl`

Real evaluator run:
- FSV root: `/home/croyse/calyx/fsv/issue881-real-hypothesis-evaluation-20260702T093012Z`
- External evaluator: `gh models run openai/gpt-4.1`
- Prompt variants / temperatures: `clinical_plausibility_v1` at `0.2`, `falsification_v1` at `0.8`
- Raw LLM output batches: `12` physical files, summarized in `llm_raw_summary.json`
- Raw LLM summary SHA256: `3fd99ac92d8c70587391de91f4a59e85b4233fe4284bcf0cb3a3fae7cbb3e87b`

Persisted artifacts:
- Source evidence SHA256: `919d39a89027e71189141bd7cf629f5494d5018c3d06fc6639145114e128d341`
- Prompt payload SHA256: `24127a1dffbf8a91a4751c6a1d4c191745344865cb0f7a5f73408283e6b813d2`
- Evaluation input SHA256: `257c893b583078d765567a9eca9a088422f9c4796bb3fd22d5bccf02c8250e1a`
- Evaluation report SHA256: `836a00ca7bc137194e1ea60831e4110283252fe9f17fe8e8d1ce15f49ccd470b`
- Readback summary SHA256: `5654f7d9780b6fbca5e6a2ad000434d1448d0c78e6a36a824e4bd76534a7f43f`

Command path:
```bash
gh extension install https://github.com/github/gh-models
gh models run openai/gpt-4.1 --temperature 0.2 --max-tokens 6000 < prompt_clinical_<seed>.txt > llm_raw_clinical_<seed>.json
gh models run openai/gpt-4.1 --temperature 0.8 --max-tokens 6000 < prompt_falsification_<seed>.txt > llm_raw_falsification_<seed>.json
cargo run -p calyx-cli -- hypothesis-evaluate \
  --input /home/croyse/calyx/fsv/issue881-real-hypothesis-evaluation-20260702T093012Z/hypothesis_evaluation_input.json \
  --out /home/croyse/calyx/fsv/issue881-real-hypothesis-evaluation-20260702T093012Z/hypothesis_evaluation_report.json
```

Readback leaves from `readback_summary.json`:
- `closed=true`
- `input_count=48`
- `evaluation_count=48`
- `retained_count=44`
- `needs_more_evidence_count=0`
- `rejected_count=4`
- `all_run_count_2=true`
- `all_prompt_variants_2=true`
- `all_temperature_variants_2=true`
- `all_evidence_count_3=true`
- `top_hypothesis_id=spectral-bridge-2-src::01`
- `top_aggregate_score=0.75250006`
- `rejected_ids=[spectral-bridge-3-src::07, spectral-bridge-3-src::08, spectral-bridge-4-src::07, spectral-bridge-4-src::08]`

Honest boundary:
- The evaluator found mostly coherent asthma pharmacology associations grounded in MedMCQA rows, but scored several repeated endpoint rows low enough to reject.
- The output is an evaluator-ranked hypothesis artifact for #882 ranking, not a biomedical verdict and not a treatment recommendation.

## Conclusion & next step
The #881 acceptance criterion is complete: every #880 real-corpus terminal A-B-C hypothesis has cited evidence, two LLM evaluator runs, transparent scores, justifications, falsification tests, and physical readback from aiwonder. The next queue item is #882 ranking over the retained evaluator outputs.
