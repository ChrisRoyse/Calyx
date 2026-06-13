# PH72 · T01 — Streaming ingest pipeline + on-the-fly quantize + backpressure

| Field | Value |
|---|---|
| **Phase** | PH72 — Streaming + Reactive + Time-Travel + Universal Summarization |
| **Stage** | S20 — Critical Capabilities |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/stream/mod.rs` (≤500), `crates/calyx-aster/src/stream/quantize_online.rs` (≤500), `crates/calyx-aster/src/stream/backpressure.rs` (≤500) |
| **Depends on** | PH41 (`ingest_at` + recurrence series), PH14 (TurboQuant rotate+scalar+QJL) |
| **Axioms** | A26, A25, A15, A16 |
| **PRD** | `17 §8`, `04 §6` |

## Goal

Build the streaming ingestion pipeline that makes Calyx a native event-stream
store: a channel-backed `StreamIngester` that accepts events at a real-time rate,
quantizes each slot's vector on-the-fly via TurboQuant (seed content-addressed per
`LensId + CxId`, never random), and flushes microbatches into `ingest_at`. A token-
bucket `BackpressureGuard` enforces A26: when the token budget is exhausted the call
returns `CALYX_STREAM_BACKPRESSURE` rather than growing unbounded.

## Build (checklist of concrete, code-level steps)

- [ ] `BackpressureGuard { tokens: AtomicUsize, capacity: usize, refill_rate: usize }` — `acquire(n) -> Result<(), CalyxError::StreamBackpressure>` returns `CALYX_STREAM_BACKPRESSURE` when `tokens < n`; a background ticker refills at `refill_rate` tokens/ms; `tokens` never exceeds `capacity` (A26)
- [ ] `QuantizeOnlineConfig { lens_id: LensId, rotation_seed_fn: fn(LensId, CxId) -> [u8;32] }` — seed is `blake3(lens_id || cx_id)`, never a random value; deterministic across restarts (A25)
- [ ] `quantize_slot_online(raw: &[f32], config: &QuantizeOnlineConfig, cx_id: CxId) -> QuantizedVec` — calls `TurboQuant::rotate_and_scalar` with the derived seed; result written to the slot CF via `ingest_at`; slot tagged with `quantized: true` in its metadata row
- [ ] `StreamIngester { sender: mpsc::Sender<StreamEvent>, guard: BackpressureGuard }` where `StreamEvent = { raw_input: ConstellationInput, at: Timestamp }`
- [ ] `StreamIngester::send(input, at) -> Result<(), CalyxError>` — acquires one backpressure token, sends on the channel; returns `CALYX_STREAM_BACKPRESSURE` if token unavailable; never blocks indefinitely
- [ ] Background `flush_loop` task: drains the channel in microbatches of ≤256 events; calls `ingest_at` for each; writes a single Ledger entry per microbatch (A15)
- [ ] `StreamIngester::drain_and_close() -> StreamStats { ingested, quantized, backpressured }` — clean shutdown: flushes remaining events, closes channel
- [ ] `stream/mod.rs` re-exports `StreamIngester`, `StreamStats`, `BackpressureGuard`, `QuantizeOnlineConfig`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: send 10 events through `StreamIngester` with `FakeClock`; call `drain_and_close`; assert `stats.ingested == 10`, `stats.backpressured == 0`; readback each `CxId` from the vault — all present
- [ ] unit: `quantize_slot_online` with fixed seed `[0u8;32]` on input `[1.0_f32; 128]` — assert output length == 128, all values finite; re-run with same seed → bit-identical result (A25)
- [ ] proptest: `∀ input: Vec<f32> (len 64–512, all finite)` → `quantize_slot_online` produces output with same length and all values finite; re-quantizing with same seed is bit-identical
- [ ] edge: send 0 events → `drain_and_close` returns `ingested == 0`, no panic, no WAL entry written
- [ ] edge: set `BackpressureGuard { capacity: 5 }` and send 6 events without refill → exactly the 6th call returns `CALYX_STREAM_BACKPRESSURE`; first 5 succeed; `stats.backpressured == 1`
- [ ] edge: send event with `at` timestamp in the past (backfill) — `ingest_at` receives the explicit `at`; `FakeClock` reports current time ≠ `at`; no silent re-stamp
- [ ] fail-closed: `BackpressureGuard::acquire` with `n > capacity` → `CALYX_STREAM_BACKPRESSURE` immediately (not a panic)
- [ ] fail-closed: non-finite value in input slot (`f32::NAN`) → `CALYX_FORGE_INPUT_NAN` before quantization; event not written to vault

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the slot CF rows written by `StreamIngester` for a 100-event stream; the `StreamStats` struct; the Ledger microbatch entries
- **Readback:** `calyx readback cx-list --vault $VAULT_PATH --after-seq 0 | wc -l` → must equal 100; `calyx readback slot-meta <CxId> <LensId>` on one event → `quantized: true` field present; `xxd $CALYX_HOME/wal/active.wal | grep -c STREAM_BATCH` → ≥1 Ledger entry
- **Prove:** before the run: 0 cx in the vault; after: exactly 100 cx, all with quantized slot metadata; one `CALYX_STREAM_BACKPRESSURE` error returned when capacity exceeded (error code present in the returned `CalyxError`); re-running with the same seed produces bit-identical quantized vectors — verified by `xxd` diff on two readback outputs

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green (337 calyx-aster tests incl. 17 new; workspace `cargo check` clean)
- [x] file(s) ≤ 500 lines (mod.rs 311, backpressure.rs 206, quantize_online.rs 189) ✅
- [x] CPU↔GPU bit-parity: quantize uses the CPU TurboQuant scalar+QJL path; determinism (A25) proven bit-identical across runs
- [x] FSV evidence: `cargo run -p calyx-aster --example stream_ingest_fsv` — 100-event SoT readback + on-disk WAL `STREAM_BATCH` byte grep + 4 edge cases (attached to issue #571)
- [x] no anti-pattern (DOCTRINE §9): real durable-vault FSV (not harness-as-FSV); fail-closed NaN/backpressure; content-addressed seed (no random)

## Implementation notes (design adaptations to the real code)

- **Error codes:** `CalyxError` is a closed PRD-18 catalog struct (not an enum), enforced by `catalog_matches_prd_18_exactly`. `CALYX_STREAM_BACKPRESSURE` and `CALYX_FORGE_INPUT_NAN` are therefore **module-local** codes built beside the module (the blessed pattern, like `dedup_error`), not catalog additions.
- **Token bucket:** a single `AtomicUsize` mutated by a `compare_exchange` RMW loop — the *correct* lock-free shape (one counter, atomic RMW), deliberately avoiding the multi-counter over-admit race documented in #703. Refill is driven by an explicit `elapsed_ms` argument (testable token-bucket pattern), not a hidden wall-clock thread.
- **NaN fail-closed** is enforced at `send` (the boundary) — before the event is queued, quantized, or written.
- **Quantized output** persists into the Base-CF constellation `metadata` (`quantized=true` + per-slot `quant_slot_<id>` hex), making it readback-verifiable; FSV recomputes the bytes independently and byte-compares.
- **Per-microbatch ledger marker** is written via `append_ledger_entry` (the real hash-chained ledger), which exists only in a **durable** vault — so tests/FSV run against a durable on-disk vault (the production path), giving a real WAL to grep for `STREAM_BATCH`.
