# PH71 · T05 — Remove `sqlite-vec` shadow + default-panels-per-vault (V2)

| Field | Value |
|---|---|
| **Phase** | PH71 — V0 shadow → V1 flip → V2 calyx-only |
| **Stage** | S19 — Leapable Vault Swap |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/leapable/shadow_removal.rs` (≤500) |
| **Depends on** | T04 (V1 gate passing), PH22 (default panels + temporal lenses) |
| **Axioms** | A18, A4, A5, A16 |
| **PRD** | `dbprdplans/15 §5 V2`, `15 §3`, `15 §4` |

## Goal

Execute the V2 transition: remove the `sqlite-vec` shadow handle (the `.db` file
is no longer written to and is archived, not deleted), transition `VaultMode` to
`CalyxOnly`, and install default panels per Vault type (text/code/civic/media per
PH22). After this card, every new Ask, ingest, and anchor operation goes through
Calyx exclusively. The `sqlite-vec` path is gone from the hot code path; its data
file is preserved as a cold archive for rollback-safety until V2 gate passes.

## Build (checklist of concrete, code-level steps)

- [ ] `ShadowRemoval::execute(vault: &mut ShadowVault) -> Result<RemovalReceipt, CalyxError>`:
      (1) verifies `VaultMode::Calyx` (will not proceed from `Shadow` — must flip
      first → `CALYX_VAULT_FLIP_REQUIRED`);
      (2) closes the `sqlite-vec` write handle;
      (3) renames the `.db` file to `.db.archive` (atomic `rename(2)` within the
      same ZFS dataset — same EXDEV guard as Aster manifest swap);
      (4) writes `mode = CalyxOnly` to `vault.calyx/MANIFEST` via atomic rename;
      (5) returns `RemovalReceipt { archived_path: PathBuf, calyx_only_at_seq: u64 }`.
      Any step failure → `CALYX_SHADOW_REMOVAL_FAILED`; prior state preserved.
- [ ] `DefaultPanels::install(vault: &mut ShadowVault, vault_type: VaultType) ->
      Result<PanelReceipt, CalyxError>`: instantiates the correct default panel from
      PH22 based on `VaultType` (`Text | Code | Civic | Media`). Each panel type
      includes the appropriate lens set (text: GTE 768-d + BM25 sparse; code: code
      lens + BM25; civic: civic lens; media: media lens). Lazy backfill is triggered
      for any slots not yet embedded. `PanelReceipt { lens_count: usize,
      backfill_pending: usize }`.
- [ ] Enforce `LensId` content-addressing throughout backfill: if the same model
      weights produce a different hash than the `LensId` stored in a slot →
      `CALYX_LENS_FROZEN_VIOLATION` (never silently overwrite a frozen slot).
- [ ] `ShadowRemoval::rollback(receipt: &RemovalReceipt) -> Result<(), CalyxError>`:
      if called before the V2 gate passes, renames `.db.archive` back to `.db` and
      writes `mode = Calyx` to MANIFEST (back to V1 state). After V2 gate passes
      and evidence is recorded, `rollback()` → `CALYX_ROLLBACK_GATE_ALREADY_PASSED`.
- [ ] `calyx leapable remove-shadow` CLI subcommand calling `ShadowRemoval::execute`
      + `DefaultPanels::install`; prints `RemovalReceipt` and `PanelReceipt`;
      exits non-zero on any error.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `ShadowRemoval::execute()` on a fixture Vault in `Calyx` mode →
      MANIFEST mode byte = 0x02 (`CalyxOnly`); `.db.archive` file exists;
      original `.db` path does not exist; `RemovalReceipt.calyx_only_at_seq > 0`.
- [ ] unit: `DefaultPanels::install` for `VaultType::Text` on the fixture →
      `PanelReceipt.lens_count == 2` (GTE 768-d + sparse); MANIFEST records panel
      spec; re-install is idempotent (`Ok(())`).
- [ ] unit: `rollback()` before gate → MANIFEST returns to `Calyx` mode; `.db`
      restored; `.db.archive` removed.
- [ ] proptest: for any `VaultType`, `DefaultPanels::install` is idempotent (50
      iterations, seed 0xPANEL_FF; second call = same `PanelReceipt`).
- [ ] edge (≥3):
      (a) `execute()` on a Vault still in `Shadow` mode → `CALYX_VAULT_FLIP_REQUIRED`;
      (b) `.db` file already `.db.archive` (prior partial run) → `execute()` detects
          existing archive, skips rename, logs INFO, proceeds to MANIFEST update;
      (c) `LensId` hash mismatch during backfill → `CALYX_LENS_FROZEN_VIOLATION`,
          panel install rolled back, mode stays `Calyx` (not `CalyxOnly`).
- [ ] fail-closed: `execute()` with insufficient disk space for `.db.archive`
      rename → `CALYX_SHADOW_REMOVAL_FAILED`; MANIFEST not updated; `.db` intact.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `vault.calyx/MANIFEST` mode byte = 0x02 (`CalyxOnly`); existence of
  `.db.archive` on aiwonder; absence of the original `.db` at its prior path;
  `PanelReceipt` showing correct lens count for the Vault type.
- **Readback:**
  ```
  # 1. Execute shadow removal:
  calyx leapable remove-shadow --vault vault_v1.calyx --sqlite real_vault_copy.db \
      --vault-type text

  # 2. Confirm mode:
  calyx readback --vault vault_v1.calyx --show-manifest
  # must print: mode=CalyxOnly, panel_lens_count=2, backfill_pending=0 (after full backfill)

  # 3. Confirm archive exists, original gone:
  ls -la real_vault_copy.db.archive   # must exist
  ls real_vault_copy.db               # must NOT exist (ls returns non-zero)

  # 4. Confirm sqlite-vec path is dead in the Calyx binary:
  calyx leapable ask --vault vault_v1.calyx --query "test" --top-k 3
  # must return results (Calyx-only path); no mention of sqlite-vec in output
  ```
- **Prove:** before this card: MANIFEST mode = `Calyx`, `.db` exists and is still
  the shadow. After: MANIFEST mode = `CalyxOnly`, `.db` is archived, default panel
  installed with correct lens count. This establishes the entry state for T06 (the
  full V2 production gate).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence: `calyx readback --show-manifest` output showing `mode=CalyxOnly`
      + `ls` output confirming `.db.archive` present and `.db` absent, attached to
      the PH71 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
- [ ] `rollback()` path tested and confirmed to restore `.db` byte-exact (file
      hash of restored `.db` matches original `.db` hash from before `execute()`)
