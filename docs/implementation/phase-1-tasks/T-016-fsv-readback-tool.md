# T-016 — FSV readback tool skeleton + Synapse note

**Phase:** PH04 · **Dep:** T-008 · **Sudo:** no

## Objective
Stand up the **readback-tool** pattern early: a `calyx readback`-style command
that prints the **actual persisted bytes** for a human/agent to judge — never a
green-checkmark harness (FSV harnesses are banned, DOCTRINE §0). Document how FSV
is perceived via Synapse.

## Preconditions
- T-008 (workspace; `calyx-cli` skeleton exists).

## Steps
1. In `calyx-cli`, add a `readback` subcommand skeleton that (later phases fill
   in) dumps raw bytes/structured rows from a target: a CF row, a WAL record, a
   Ledger entry, a vault dir listing, a metric. For now: a `--hex <file>` and a
   `--vault-tree <dir>` that print real bytes/listing from disk.
2. Establish the convention: every FSV gate names the readback command that
   proves it; tools **print**, they never assert "passed".
3. Write `repo/docs/implementation/FSV_NOTES.md` mapping the 5-step FSV protocol
   to Synapse abilities (PRD `28 §2c`): `reality_baseline` → `act_run_shell`
   readback → `observe_delta`/`reality_audit` → `find`/`read_text` →
   `capture_screenshot`/`audit_export_bundle`; note `reflex_register` for async
   ops and screenshot+AI-vision for Grafana/`J`-curve.
4. Verify the skeleton prints real bytes on aiwonder.

## Deliverables
- `calyx readback` skeleton (hex/vault-tree); `FSV_NOTES.md` (FSV→Synapse map);
  the readback-tool convention documented.

## FSV gate
`calyx readback --hex <somefile>` on aiwonder prints the file's real bytes
matching `xxd`; `--vault-tree <dir>` lists the real tree; the tool **never**
emits a pass/fail verdict (grep the output — no "PASS"). Proves we built a
perception tool, not a harness.

## Done
The readback pattern exists and is adopted; FSV-via-Synapse documented.

## Refs
DOCTRINE §0, PRD `28 §2/§2c`, `31`, `../02_WORKING_AGREEMENT.md §2`.
