# PH44 · T06 — Integration FSV: corrupt ANN → degraded + rebuild, no data loss

| Field | Value |
|---|---|
| **Phase** | PH44 — Self-Heal (Rebuild Derived, Degrade Flags) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/tests/fsv_corrupt_rebuild.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04, T05 |
| **Axioms** | A16, A15, A14 |
| **PRD** | `dbprdplans/12 §2`, `dbprdplans/24 §7` |

## Goal

Prove the full self-heal loop end-to-end in a single deterministic test: flip a
byte in the HNSW ANN index file, run `FaultMonitor`, observe `Degraded`
transition, run `RebuildScheduler`, confirm rebuild completes, observe `Ok`
transition, verify base CF bytes are untouched throughout (no data loss). Also:
kill a lens endpoint (simulate by returning probe errors), confirm search
degrades gracefully (remaining lenses serve), no hang. This is the phase FSV
gate as a runnable scenario.

## Build (checklist of concrete, code-level steps)

- [ ] Test scenario `corrupt_ann_rebuild`: (a) create a test vault with two slots and synthetic constellation data; (b) record SHA-256 of base CF and ANN index; (c) flip byte 42 of the ANN index file; (d) run `FaultMonitor.check` → assert `DegradeRegistry: AnnIndex(slot_0): Degraded`; (e) run `RebuildScheduler.run_next` → assert `RebuildOutcome::Completed`; (f) assert `DegradeRegistry: AnnIndex(slot_0): Ok`; (g) assert base CF SHA-256 unchanged (data loss = test failure); (h) assert Ledger contains `Rebuild` entry with both prior and new pointer hashes.
- [ ] Test scenario `failing_lens_route`: (a) create test vault with two lens endpoints (`L1`, `L2`); (b) `LensProbeDetector` probe `L1` returns timeout → `Failing`; (c) call search with panel `[L1, L2]`; assert results returned from `L2` only, with `degraded: true` flag; assert no timeout in the search response; (d) Ledger has `action=DegradeChange` for `L1`.
- [ ] Both scenarios fully deterministic: seeded RNG, injected clock, synthetic data only (no live TEI call).
- [ ] `AnnealSubstrate` used for rebuild path; verify tripwire passes for the rebuilt index (compare rebuild recall against a brute-force baseline on the synthetic data).
- [ ] All scenario assertions are byte-level or value-exact (no approximate "about right" checks in FSV).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] `corrupt_ann_rebuild`: all 8 assertions above must pass (see Build steps a–h).
- [ ] `failing_lens_route`: all 4 assertions above must pass (see Build steps a–d).
- [ ] edge: flip byte in base CF → `verify_base_shards` fires `Corrupt`; `fail_reads_on_range` returns `CALYX_ASTER_BASE_CORRUPT`; no auto-rebuild of base (only operator restore); base bytes unchanged.
- [ ] edge: `RebuildScheduler` queue is empty → `run_next` returns `NothingQueued`; calling `run_next` twice does not produce duplicate Ledger entries.
- [ ] fail-closed: rebuilt ANN index fails tripwire (recall below `0.90`) → `RebuildOutcome::Failed`; component stays `Degraded`; prior artifact unchanged; Ledger records failure with metric values.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** base CF bytes (SHA-256 before/after), Ledger entries, `DegradeRegistry` state transitions.
- **Readback:** `calyx readback ledger --kind Anneal --last 5`; `calyx anneal status --health`; `sha256sum $CALYX_HOME/vault/ann/slot_0.hnsw` before and after.
- **Prove:** run `cargo test fsv_corrupt_rebuild` on aiwonder; all assertions green; before-SHA vs after-SHA of base CF are identical; Ledger shows `Rebuild` entry; health shows `Ok` after rebuild. Attach stdout of the test run + `readback ledger` output to the PH44 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH44 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
