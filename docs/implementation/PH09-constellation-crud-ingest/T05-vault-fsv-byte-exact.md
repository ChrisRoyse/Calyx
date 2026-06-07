# PH09 · T05 — Vault put/get/anchor FSV (byte-exact on disk)

| Field | Value |
|---|---|
| **Phase** | PH09 — Constellation CRUD + CxId + idempotent ingest |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/vault.rs` (≤500), `crates/calyx-cli/src/main.rs` |
| **Depends on** | T02 (WAL write), T03 (idempotent), T04 (ledger stub) |
| **Axioms** | A1, A15 |
| **PRD** | `dbprdplans/04 §5`, `dbprdplans/03 §3` |

## Goal

The phase FSV gate: prove on aiwonder by reading real SST bytes that N
constellations written through `put` land in `base` + `slot_*` + `anchors` +
`ledger` CFs with byte-exact values, survive a vault process restart, and that
re-ingest of identical input is idempotent on disk. The `calyx ingest` and
`calyx readback` commands are the verification tools.

## Build (checklist of concrete, code-level steps)

- [ ] Add `calyx ingest --vault <path> --input <text> --slot <dim> <f32...>`
  CLI subcommand: creates an `AsterVault`, calls `put` with a synthetic
  constellation (one dense slot with the specified values), prints `CxId: <hex>`.
- [ ] Add `calyx anchor --vault <path> --cx-id <hex> --kind reward --value 1.0`
  CLI subcommand: calls `vault.anchor(...)`.
- [ ] Write an end-to-end test (spawns CLI processes or calls vault API directly)
  that exercises the full cycle: ingest → flush → cold-open → get → anchor → get.
- [ ] Assert in the test: `get` after cold-open returns the same struct as before
  the process boundary; anchor CF row is present and byte-exact.
- [ ] Assert: re-ingest same input returns same CxId; WAL segment size unchanged.
- [ ] Document the exact `xxd` and `calyx readback` commands in the test output.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit (end-to-end): put 3 constellations; flush; cold-open; get all 3
  byte-exact; anchor one; get anchored → anchors present.
- [ ] unit: idempotent re-ingest after cold-open: WAL count unchanged, CxId same.
- [ ] proptest: for any `n in 1..=10` distinct constellations (seeded RNG), put all
  + flush + cold-open + get all → byte-exact for each.
- [ ] edge (≥3): (1) constellation with 0 slots (only base row); (2) constellation
  with 15 slots; (3) anchor written after cold-open (anchor is appended to
  existing base row + anchors CF).
- [ ] fail-closed: `get` on non-existent CxId after cold-open → `CALYX_STALE_DERIVED`
  (missing row), not panic.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `vault/cf/base/`, `vault/cf/slot_00/`, `vault/cf/anchors/`,
  `vault/cf/ledger/` under `/home/croyse/calyx/test-vault/`.
- **Readback:**
  ```
  calyx ingest --vault /home/croyse/calyx/test-vault --input "fsv-test" --slot 4 0.1 0.2 0.3 0.4
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  calyx readback --cf slot_00 --vault /home/croyse/calyx/test-vault
  calyx readback --cf ledger --vault /home/croyse/calyx/test-vault
  calyx anchor --vault /home/croyse/calyx/test-vault --cx-id <printed-id> --kind reward --value 1.0
  calyx readback --cf anchors --vault /home/croyse/calyx/test-vault
  xxd /home/croyse/calyx/test-vault/cf/base/000001.sst | head -4
  ```
- **Prove:** `base` SST contains a 16-byte CxId key at offset `HEADER_LEN`; the
  decoded value header shows `panel_version` and `modality` matching the input.
  `slot_00` SST contains the 4 f32 values (`0.1, 0.2, 0.3, 0.4`) as raw
  big-endian bytes. `ledger` SST has seq=1 row with 32 zero bytes. `anchors` SST
  has the reward anchor after `calyx anchor`. Evidence screenshot posted to PH09
  GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH09 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
