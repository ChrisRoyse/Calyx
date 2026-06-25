# 08 - blind spot sweep

- **Issue:** #875   **Phase:** P0 discovery   **Date (UTC):** 2026-06-25   **Vault/panel:** synthetic multi-lens observations while #869 corpus ingest runs
- **Goal:** sweep cross-lens disagreement observations, preserve text and neighbor evidence, gate-check each alert, and rank high-severity candidates.

## What was run (exact commands)
```bash
# Windows authoring checkout
cargo fmt --all
cargo test -p calyx-lodestar --test issue875_blind_spot_sweep_tests -- --nocapture
cargo fmt --all -- --check
git diff --check
bash scripts/linecount.sh

# aiwonder source-of-truth FSV archive
git archive --format=tar -o issue875-20260625T112409Z.tar HEAD
ssh aiwonder "mkdir -p /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo"
scp issue875-20260625T112409Z.tar aiwonder:/home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo.tar
ssh aiwonder "tar -xf /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo.tar -C /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo && CARGO_INCREMENTAL=0 CARGO_TARGET_DIR=/home/croyse/calyx/repo/target CALYX_FSV_ROOT=/home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z cargo test -p calyx-lodestar --test issue875_blind_spot_sweep_tests -- --nocapture"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo && cargo fmt --all -- --check"
ssh aiwonder "cd /home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/repo && bash scripts/linecount.sh"
```

## Raw evidence / FSV
Implemented source:
- `crates/calyx-lodestar/src/blind_spot_sweep.rs`
- `crates/calyx-lodestar/tests/issue875_blind_spot_sweep_tests.rs`
- `crates/calyx-lodestar/src/lib.rs` public exports

Local test evidence:
- `cargo test -p calyx-lodestar --test issue875_blind_spot_sweep_tests -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo fmt --all -- --check`: exit 0.
- `git diff --check`: exit 0.
- `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

aiwonder FSV:
- FSV root: `/home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z`
- Artifact: `/home/croyse/calyx/fsv/issue875-blind-spot-sweep-20260625T112409Z/issue875_blind_spot_sweep_readback.json`
- Artifact bytes: `1933`
- Artifact SHA256: `e7fd375f8c359e2fdd2ce3e1b142d614d266b907847c7ec785c34528b59c4ff3`
- Readback scalar leaves:
  - `schema_version=1`
  - `observation_count=3`
  - `detected_alert_count=3`
  - `gate_refused_count=1`
  - `severity_filtered_count=1`
  - `candidate_count=1`
  - `top_severity=High`
  - `top_delta=0.9800000190734863`
  - `neighbor_evidence_count=4`
- aiwonder tests from archived source: 5 passed, 0 failed, 0 ignored.
- aiwonder `cargo fmt --all -- --check`: exit 0.
- aiwonder `bash scripts/linecount.sh`: `all .rs <= 500 lines`.

Boundary and edge behavior covered by tests:
- High-severity, gate-passing disagreements are ranked and keep source text plus both lens neighbor lists.
- Gate-refused high-severity alerts are counted but not returned as candidates.
- Medium-severity alerts are detected and filtered when `min_severity=High`.
- `max_candidates` truncates after deterministic ranking.
- Non-finite similarity and out-of-range gate confidence fail closed with `CALYX_KERNEL_INVALID_PARAMS`.

## Findings (honest)
- The existing Loom detector remains the primitive for `(cx, lens_a, lens_b)` disagreement.
- Lodestar now has a discovery sweep log that preserves the evidence #875 needs: text, lens slots, two neighbor sets, gate verdict, severity, delta, and rank score.
- The synthetic FSV proves durable readback of one high-severity gate-passing candidate, one gate refusal, and one severity-filtered alert.
- This is not yet the final #875 anchored-corpus acceptance. The real ranked biomedical candidate list requires sweeping the actual anchored corpus after #869/#870/#871.

## Conclusion & next step
The #875 sweep/ranking surface is ready for the real corpus. Keep #875 open until the anchored corpus is fully ingested, Loom cross-terms are woven, the kernel is grounded, and real gate-passing disagreement candidates are read back with source text and neighbors.
