# PH72 · T05 — Retention-horizon enforcement + fail-closed before horizon

| Field | Value |
|---|---|
| **Phase** | PH72 — Streaming + Reactive + Time-Travel + Universal Summarization |
| **Stage** | S20 — Critical Capabilities |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/timetravel/retention.rs`, `src/timetravel/mod.rs`, `src/vault/retention_horizon.rs`, `src/manifest/*` (each ≤500) |
| **Depends on** | T04 (`as_of(t)` + `TimeIndex`), PH08 (MVCC snapshot reads) |
| **Axioms** | A16, A15, A26 |
| **PRD** | `17 §8`, `17 §7.2` |

## Goal

Declare a vault-level `RetentionHorizon` (expressed as a `Duration` before `now`
or an absolute `Timestamp`). Any `as_of(t)` call where `t` falls before the
retention horizon MUST return `CALYX_TIMETRAVEL_BEFORE_HORIZON` — a structured
error that includes the horizon timestamp — and must never silently return the
oldest available snapshot as an approximation (A16). This prevents stale-data
silent failures where historical reads look valid but the requested point has
been GC'd or compacted. The retention horizon is stored in the vault manifest and
written to the Ledger on change (A15).

## Build (checklist of concrete, code-level steps)

- [x] `RetentionHorizon` enum: `Rolling { min_age: Duration }` | `Absolute { horizon_millis: u64 }` | `None` (no horizon, all `as_of` allowed); stored in `VaultManifest`
- [x] `RetentionHorizon::effective_horizon_millis(clock: &dyn Clock) -> Option<u64>` — `Rolling` computes `clock.now() - min_age.as_millis()` with saturating epoch math; `Absolute` returns the fixed value; `None` returns `None`
- [x] `fn check_horizon(horizon: &RetentionHorizon, t_millis: u64, clock: &dyn Clock) -> Result<(), CalyxError>` — if `effective_horizon_millis` returns `Some(h)` and `t_millis < h` → return module-local `CALYX_TIMETRAVEL_BEFORE_HORIZON` with `requested_millis` and `horizon_millis` in the message
- [x] Integrate `check_horizon` at the top of `as_of(vault, t, clock)` (from T04) **before** calling `TimeIndex::resolve`; horizon check precedes any disk access
- [x] `Vault::set_retention_horizon(horizon: RetentionHorizon) -> Result<(), CalyxError>` — writes updated `RetentionHorizon` to the vault manifest (atomic manifest swap, same pattern as PH10); writes a `RETENTION_HORIZON_CHANGED` Ledger entry containing the old and new horizon values (A15)
- [x] `Vault::retention_horizon() -> RetentionHorizon` — clone accessor used by `as_of` and any other time-travel surface
- [x] `VaultManifest` serde: `RetentionHorizon` serializes as `{ "kind": "Rolling", "min_age_secs": 86400 }` / `{ "kind": "Absolute", "horizon_millis": N }` / `{ "kind": "None" }`; round-trip is byte-exact (proptest)
- [x] Ensure GC (PH58) can read the retention horizon to know which MVCC versions are safe to discard: expose `RetentionHorizon::safe_to_gc_before_millis(clock) -> Option<u64>` as the GC input (no actual GC logic here — just the boundary function)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: set `RetentionHorizon::Rolling { min_age: Duration::from_secs(1) }` with `FakeClock(now=10_000ms)`; `as_of(t=8_999ms)` → `CALYX_TIMETRAVEL_BEFORE_HORIZON` with `horizon_millis=9000ms`; `as_of(t=9000ms)` succeeds
- [x] unit: `RetentionHorizon::Absolute { horizon_millis: 5000 }`; `as_of(t=4999ms)` → `CALYX_TIMETRAVEL_BEFORE_HORIZON`; `as_of(t=5000ms)` → succeeds (horizon is inclusive on the boundary)
- [x] unit: `RetentionHorizon::None`; `as_of(t=0ms)` returns `CALYX_TIMETRAVEL_NO_DATA` (from T04, no data), NOT `CALYX_TIMETRAVEL_BEFORE_HORIZON` — error codes must not be confused
- [x] unit: `set_retention_horizon` writes a `RETENTION_HORIZON_CHANGED` Ledger entry; `Vault::retention_horizon()` reflects the new value immediately after the call
- [x] proptest: `∀ (horizon_millis: u64, t_millis: u64)` with `t_millis < horizon_millis` → `check_horizon` always returns `CALYX_TIMETRAVEL_BEFORE_HORIZON`; `∀ t_millis ≥ horizon_millis` → returns `Ok(())`
- [x] edge: `RetentionHorizon::Rolling { min_age: Duration::ZERO }` — horizon == now; `as_of(t = now - 1ms)` → `CALYX_TIMETRAVEL_BEFORE_HORIZON`
- [x] edge: update horizon from one `Absolute` value to another → Ledger shows the old value and the new value in the `RETENTION_HORIZON_CHANGED` entry (byte-assert on the Ledger entry struct)
- [x] fail-closed: `as_of` with `t` before horizon MUST NOT return any constellation data even if that data is still physically present (MVCC not yet GC'd); assert that the returned error contains no `Constellation` bytes

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the vault manifest's `retention_horizon` field on disk; the `CALYX_TIMETRAVEL_BEFORE_HORIZON` error returned by `as_of`; the Ledger `RETENTION_HORIZON_CHANGED` entry
- **Readback:** `calyx readback vault-manifest --field retention_horizon --vault $VAULT_PATH` → shows the configured horizon; `calyx readback as-of --vault $VAULT_PATH --t-millis <T_BEFORE_HORIZON>` → prints `CALYX_TIMETRAVEL_BEFORE_HORIZON` error with the horizon millis value; `calyx ledger-tail --vault $VAULT_PATH --last 5` → shows `RETENTION_HORIZON_CHANGED` entry
- **Prove:** configure `RetentionHorizon::Absolute { horizon_millis: H }`; ingest a constellation at `t < H`; call `as_of(t)` → error code `CALYX_TIMETRAVEL_BEFORE_HORIZON` in the output (not data); call `as_of(H + 1)` → if data exists at that time, succeeds; verify the manifest on disk encodes the horizon (jq readback matches the configured value); the horizon-change Ledger entry is byte-readable and contains both old and new horizon values

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH72 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
