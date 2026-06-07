# PH71 · T03 — Read-flip to Calyx + multi-lens panel/kernel/guard enable (V1)

| Field | Value |
|---|---|
| **Phase** | PH71 — V0 shadow → V1 flip → V2 calyx-only |
| **Stage** | S19 — Leapable Vault Swap |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/leapable/read_flip.rs` (≤500), `crates/calyx-cli/src/leapable/panel_guard_enable.rs` (≤500) |
| **Depends on** | T02 (dual-write + parity gate passing), PH33 (kernel index), PH38 (guard τ calibration), PH22 (default panels) |
| **Axioms** | A18, A4, A11, A12, A16 |
| **PRD** | `dbprdplans/15 §5 V1`, `15 §3`, `15 §4` |

## Goal

Flip the Vault's read path from `sqlite-vec` to Calyx: after this card, Ask reads
from Calyx (authoritative), `sqlite-vec` is demoted to shadow (write-only for
rollback safety). Simultaneously enable the multi-lens panel, kernel-grounded
answers (`kernel_answer` from PH33), and the `Gτ` guard (from PH38) on the live
Vault. The `ShadowVault` mode transitions from `Shadow → Calyx`. The PostgreSQL
control plane must observe no behavioral difference — same `database_name`, same
`chunk_id` responses, same query interface.

## Build (checklist of concrete, code-level steps)

- [ ] `ReadFlip::execute(vault: &mut ShadowVault) -> Result<FlipReceipt, CalyxError>`:
      atomically transitions `VaultMode` from `Shadow → Calyx` by writing the new
      mode byte to `vault.calyx/MANIFEST` via `rename(2)` (atomic swap, same ZFS
      dataset). Any partial failure → `CALYX_VAULT_FLIP_FAILED`, mode remains
      `Shadow`, `sqlite-vec` path unaffected.
- [ ] After flip, `ShadowVault::ask(query_vec: &[f32], top_k: usize)` routes to
      Calyx for retrieval; `sqlite-vec` is kept open for shadow-write only. Returns
      `AskResult { hits: Vec<Hit>, mode: VaultMode::Calyx }` where each `Hit`
      carries `chunk_id` (verbatim), `LedgerRef`, and per-lens provenance.
- [ ] `PanelGuardEnable::enable(vault: &mut ShadowVault, panel_spec: &PanelSpec)
      -> Result<(), CalyxError>`: activates the multi-lens panel on the Vault (calls
      PH22 panel instantiation), triggers lazy backfill for any slots not yet
      embedded. Enforces **never-mix-vectors**: each lens slot is keyed by its
      `LensId`; a `LensId` mismatch on backfill → `CALYX_LENS_FROZEN_VIOLATION`.
- [ ] `PanelGuardEnable::enable_kernel(vault: &mut ShadowVault) -> Result<(), CalyxError>`:
      wires PH33's `kernel_answer` into the Ask path. Kernel-only recall must be ≥
      0.95 · full recall (PH33 contract); if not, logs WARN but does not block flip.
- [ ] `PanelGuardEnable::enable_guard(vault: &mut ShadowVault, tau: f32) ->
      Result<(), CalyxError>`: installs the calibrated `Gτ` guard (PH38) on every
      slot in the panel. `tau` must be the value calibrated on the injection corpus
      (PH38 contract: injection blocked ≥99% at calibrated FAR). Guard failure
      mode: `CALYX_GUARD_TAU_NOT_CALIBRATED` if `tau = 0.0`.
- [ ] `FlipReceipt` carries: `database_name` (verbatim), `flipped_at_seq: u64`
      (the Aster MVCC sequence at the flip moment), `panel_lens_count: usize`,
      `kernel_enabled: bool`, `guard_enabled: bool`. Written to Ledger (A15).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: on a 5-chunk fixture Vault in `Shadow` mode, call `ReadFlip::execute()`
      → mode becomes `Calyx`; re-read `MANIFEST` → mode byte = 0x01; `sqlite-vec`
      still open and writable.
- [ ] unit: after flip, `ask()` on the fixture returns the same `chunk_id` values
      as the pre-flip `sqlite-vec` path (same query vector, same k=3, same ordered
      result — proves contract preservation).
- [ ] unit: `enable_guard` with `tau = 0.0` → `CALYX_GUARD_TAU_NOT_CALIBRATED`.
- [ ] proptest: for any `FlipReceipt`, `receipt.database_name` equals the
      `database_name` stored in the SQLite metadata row byte-exact (seed
      0xFLIP_CAFE; 50 iterations over fixture Vaults with varied names).
- [ ] edge (≥3):
      (a) flip on a Vault already in `Calyx` mode → `Ok(())` (idempotent, no
          double-flip);
      (b) flip with incomplete backfill (some slots missing vectors) → flip
          succeeds; missing slots logged as gaps; kernel remains enabled but reports
          `grounding_gaps`;
      (c) `LensId` mismatch on backfill during `enable()` → `CALYX_LENS_FROZEN_VIOLATION`,
          panel partial-enable is rolled back.
- [ ] fail-closed: `ReadFlip::execute()` on a Vault with corrupted `MANIFEST` →
      `CALYX_MANIFEST_CORRUPT`; mode unchanged.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `vault.calyx/MANIFEST` mode byte after flip; `Ask` results from the
  Calyx path on a real Vault on aiwonder; `FlipReceipt` Ledger entry at
  `flipped_at_seq`.
- **Readback:**
  ```
  # Confirm mode flip:
  calyx readback --vault vault.calyx --show-manifest
  # must print: mode=Calyx, database_name=<expected>, kernel=enabled, guard=enabled

  # Confirm Ask routes through Calyx and returns LedgerRef-cited hits:
  calyx leapable ask --vault vault.calyx --query "test query" --top-k 5
  # must print: 5 hits, each with chunk_id (verbatim from source .db), LedgerRef

  # Confirm sqlite-vec shadow still intact (rollback safety):
  xxd real_vault_copy.db | head -2   # SQLite magic bytes present
  ```
- **Prove:** before this card: `MANIFEST` mode byte = 0x00 (`Shadow`), Ask routes
  to `sqlite-vec`. After: mode byte = 0x01 (`Calyx`), Ask returns Calyx hits with
  `LedgerRef`. The `sqlite-vec` `.db` is still intact (shadow demoted, not deleted).
  PostgreSQL-side: run control-plane SQL queries — responses identical to V0 state.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence: `calyx readback --show-manifest` output showing `mode=Calyx` +
      Ask result with `chunk_id` + `LedgerRef` attached to the PH71 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
- [ ] `CALYX_LENS_FROZEN_VIOLATION` fires on any `LensId` mismatch (grep for the
      error code in module confirms it is the only exit path for that case)
