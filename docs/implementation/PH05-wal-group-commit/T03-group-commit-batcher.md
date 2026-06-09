# PH05 · T03 — Group-commit batcher (≤2 ms window)

| Field | Value |
|---|---|
| **Phase** | PH05 — WAL + group-commit + fsync |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/wal/batch.rs` (≤500), `crates/calyx-aster/src/wal/mod.rs` (≤500) |
| **Depends on** | T01 (record framing), T02 (segment rotation) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/04 §5` |

## Goal

Implement a `GroupCommitBatcher` that collects write requests from concurrent
callers for at most `group_commit_window` (≤2 ms, default
`DEFAULT_GROUP_COMMIT_WINDOW`), then issues a single `append_batch` + fsync and
wakes all callers. Uses the `Clock` trait for the deadline, never
`std::time::Instant::now()` in logic paths, so tests can inject a `FixedClock` or
`ManualClock`. The batcher is a stand-alone component that wraps `Wal`; the vault
write path calls it in PH09.

## Build (checklist of concrete, code-level steps)

- [x] Define `BatchRequest` in `batch.rs`: `payload: Vec<u8>` +
  `respond: oneshot::Sender<Result<AppendAck>>`.
- [x] Define `GroupCommitBatcher`: wraps `Arc<Mutex<Wal>>`, holds a
  `mpsc::Sender<BatchRequest>` for callers and a background thread that drains
  the channel.
- [x] Batcher thread loop: receive first request (blocking), then drain any
  immediately available requests, but stop once the elapsed wall time since the
  first request exceeds `group_commit_window` (use `Clock::now()` injected at
  construction). Issue `wal.append_batch(all_payloads)` and distribute acks.
- [x] Expose `GroupCommitBatcher::submit(&self, payload: Vec<u8>) -> Result<AppendAck>`
  that blocks the caller until the batcher flushes.
- [x] Expose `GroupCommitBatcher::flush_sync(&self) -> Result<()>` for graceful
  shutdown that drains the queue and fsyncs.
- [x] Add `WalOptions::group_commit_window` enforcement: assert ≤2 ms or return
  `CalyxError::disk_pressure("group_commit_window exceeds 2 ms limit")`.
- [x] Re-export `GroupCommitBatcher` from `wal/mod.rs`.
- [x] Ensure the background thread does not hold the `Wal` mutex across the caller
  wakeup; release lock before sending acks.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: submit 5 payloads concurrently; assert all 5 return distinct seqs in
  monotonic order and the WAL directory contains exactly the expected bytes.
- [x] unit: two concurrent submitters; `replay_dir` after `flush_sync` returns
  both records with correct seqs and payloads byte-exact.
- [x] proptest: `∀ n in 1..=50` concurrent submitters each sending a random
  payload: all seqs are distinct, monotonic, and `replay_dir` returns all n records.
- [x] edge (≥3): (1) single submit, no other callers — still fsyncs within 2 ms
  deadline; (2) `flush_sync` on empty batcher is a no-op; (3) batcher closed
  returns `CalyxError` to any pending submitter.
- [x] fail-closed: `WalOptions` with `group_commit_window > 2 ms` →
  `CalyxError::disk_pressure` containing `"group_commit_window exceeds 2 ms limit"`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** WAL segment file after a multi-threaded submit run.
- **Readback:**
  ```
  xxd /home/croyse/calyx/test-vault/wal/00000000000000000000.wal | grep -c "^"
  ```
  (count lines = count 16-byte rows = confirms expected total byte length)
- **Prove:** The number of complete records decoded by `replay_dir` equals the
  number of `submit` calls made. Each record carries the correct caller payload;
  seqs are contiguous from 1..=n. The single fsync-per-batch property is verified
  by checking that the segment file contains exactly one write boundary (all records
  in the same batch share one segment, no intermediate torn boundaries).

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH05 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
