# Poly Daily Jobs Runbook

Issue: #129

## Scope And Boundary

This runbook defines the local nightly job chain for one Poly workstation. Jobs are read-only or
local-diagnostic writes only. They must not place trades, sign orders, manage bankroll, use browser
profiles, or depend on CI/CD.

## Schedule

Run once per night after source capture and before human review. Use one local scheduler entry
(Windows Task Scheduler or cron-equivalent) that invokes a script from the checked-out repository.
The script must take an exclusive lock before starting and must fail if a previous run is still
active.

## Job Order

1. `weave-loom` - rebuild association cross-terms and graph evidence for the current domain corpus.
2. `kernel-build` - rebuild the grounding kernel and enforce the configured minimum recall gate.
3. `guard calibrate` - refresh Ward/guard calibration from grounded residual evidence.
4. `verify-chain` - verify ledger/hash-chain integrity and record the chain tip.
5. `bits scan` - scan lens/panel bits, sufficiency deficits, and changed signal coverage.

Do not run later jobs when an earlier job fails. Persist the failure and stop.

## Success Verification

Each nightly run must persist a JSON summary and read it back before marking the run successful. The
summary must include:

- schedule id and UTC start/end timestamps
- git commit and branch
- domain list
- each job name, command, exit code, duration, output artifact path, and readback hash
- kernel recall threshold and measured recall
- guard calibration version and stale-after time
- verify-chain result and ledger tip hash
- bits scan summary and sufficiency status

Success requires every job exit code to be zero, every named artifact to exist, every readback hash
to match, kernel recall to clear the configured gate, guard calibration to be non-stale, ledger
verification to be intact, and bits scan to write a current sufficiency report.

## Failure Policy

Fail closed on:

- missing exclusive lock
- overlapping run
- missing source corpus
- nonzero job exit
- missing artifact
- readback hash mismatch
- kernel recall below gate
- stale or uncalibrated guard
- broken verify-chain result
- missing bits scan report
- any command that would trade, sign orders, manage bankroll, or use a browser profile

Failures must write a local evidence record and must not be hidden by retries. A retry is a new run
with its own schedule id.

## Evidence To Save

- nightly summary JSON
- `BLAKE3SUMS.txt` for all job artifacts
- stdout/stderr logs with secrets redacted
- verify-chain JSON
- kernel-build recall report
- guard calibration report
- bits scan report
- operator note for any failed-closed run

## Minimum Manual Drill

Before enabling unattended scheduling, run the script once with a tiny known-good local corpus that
exercises all five job names and produces a summary. Then run three failure drills: overlapping lock,
missing artifact, and readback hash mismatch. Enable the scheduler only after all four drill reports
are read back from disk.
