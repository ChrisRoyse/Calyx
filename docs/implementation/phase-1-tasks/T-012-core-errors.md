# T-012 — calyx-core: error catalog (`CALYX_*`)

**Phase:** PH03 · **Dep:** T-008 · **Sudo:** no

## Objective
The single, closed error vocabulary — every `CALYX_*` code from PRD `18 §6` as a
structured, actionable error. Fail-closed by construction (A16); agent-self-
correcting (A17).

## Preconditions
- T-008 (workspace).

## Steps
1. `crates/calyx-core/src/error.rs`:
   - `struct CalyxError { code: &'static str, message: String, remediation: &'static str }`
     (+ `thiserror`), `pub type Result<T> = std::result::Result<T, CalyxError>;`.
   - One constructor/variant per code in PRD `18 §6`, verbatim codes:
     `CALYX_LENS_FROZEN_VIOLATION, _DIM_MISMATCH, _NUMERICAL_INVARIANT,
     _UNREACHABLE; CALYX_ASSAY_INSUFFICIENT_SAMPLES, _LOW_SIGNAL, _REDUNDANT;
     CALYX_KERNEL_UNGROUNDED; CALYX_GUARD_PROVISIONAL, _OOD;
     CALYX_FORGE_NUMERICAL_INVARIANT, _DEVICE_UNAVAILABLE, _VRAM_BUDGET;
     CALYX_ASTER_CORRUPT_SHARD, _TORN_WAL; CALYX_LEDGER_CHAIN_BROKEN;
     CALYX_VAULT_ACCESS_DENIED; CALYX_STALE_DERIVED; CALYX_ORACLE_INSUFFICIENT;
     CALYX_BACKPRESSURE; CALYX_DISK_PRESSURE; CALYX_QUANT_INTELLIGENCE_LOSS;
     CALYX_READER_LEASE_EXPIRED`.
   - Each carries the `remediation` text from `18 §6`.
2. A const slice/`enum` listing all codes; a test asserting the catalog is the
   exact set in `18 §6` (closed set — no silent additions).
3. Serde for wire transport (`{code, message, remediation}`).

## Deliverables
- `error.rs` with the full `CALYX_*` catalog + remediations + a catalog-
  completeness test.

## FSV gate
`cargo test -p calyx-core` green; a test enumerates the codes and asserts
**exact equality** with PRD `18 §6` (no missing/extra); each error serializes to
`{code,message,remediation}`. Print the catalog and diff against `18 §6`.

## Done
The closed error vocabulary exists, matches the PRD, and every error is
structured + actionable.

## Refs
PRD `18 §6`, A16, A17.
