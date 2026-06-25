# 16 - refusal driven expansion

- **Issue:** #883   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic before/after probe logs while #869 corpus ingest runs
- **Goal:** convert gate refusals and per-sensor deficits into ranked evidence/lens expansion actions, then verify whether a later run closed the refusal and produced new grounded hits.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue883_refusal_expansion_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue883-20260625T111607Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo"
scp issue883-20260625T111607Z.tar aiwonder:/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo.tar -C /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z cargo test -p calyx-lodestar --test issue883_refusal_expansion_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/refusal_expansion.rs`
- `crates/calyx-lodestar/tests/issue883_refusal_expansion_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue883_refusal_expansion_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder FSV:
- FSV root: `/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z`
- Artifact: `/home/croyse/calyx/fsv/issue883-refusal-expansion-20260625T111607Z/issue883_refusal_expansion_readback.json`
- Artifact bytes: `1620`
- Artifact SHA256: `f96e8235771b8d469713692057bc88000182e994ee23be99e8a640b86df21e53`
- Readback scalar leaves:
  - `schema_version=1`
  - `action_count=2`
  - `top_action_kind=AddLens`
  - `before_refusal_count=2`
  - `after_refusal_count=0`
  - `closed_refusal_count=2`
  - `new_grounded_count=1`
  - `closed=True`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

Boundary and edge behavior covered by tests:
- Refusals with deficits are turned into ranked actions.
- Lens/sensor deficit text maps to `AddLens`; evidence gaps map to evidence addition.
- Deficit floor filters low-value actions.
- Before/after verification requires both refusal reduction and new grounded hits.
- Refusal reduction without a new grounded hit does not close the expansion.
- Non-finite deficit parameters fail closed with `CALYX_KERNEL_INVALID_PARAMS`.

## Findings (honest)
- The refusal-expansion planner now turns probe-matrix refusal rows into reusable, ranked expansion actions.
- The verifier produces a serializable before/after closure proof with refusal counts and new grounded hit IDs.
- The synthetic FSV proves the state-machine shape: two refusals planned, later zero refusals, one new grounded hit, `closed=True`.
- This is not yet the final #883 anchored-corpus acceptance. No real biomedical evidence has been added yet; the final issue requires a real refusal on the anchored corpus to become a grounded answer after targeted evidence addition.

## Conclusion & next step
The #883 planning and verification surface is ready. Keep #883 open until #869/#870/#871 produce the real corpus substrate, a real refusal is captured, targeted evidence/lens data is added, and the same verifier reads back a closed refusal with a new grounded answer.
