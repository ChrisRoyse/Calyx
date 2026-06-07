# PH63 · T03 — Ingest/measure tool group

| Field | Value |
|---|---|
| **Phase** | PH63 — calyx-mcp (stdio embedded tool surface) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-mcp` |
| **Files** | `crates/calyx-mcp/src/tools/ingest.rs` (≤500) |
| **Depends on** | T02, PH62·T03 (ingest/anchor engine API) |
| **Axioms** | A1, A15, A16, A17 |
| **PRD** | `dbprdplans/14 §2` (ingest/measure group), `dbprdplans/14 §5` (ergonomic guarantees) |

## Goal

Register the `ingest`, `anchor`, and `measure` tools. `ingest` is content-addressed
and idempotent: retries are safe (PRD 14 §5). Every ingest writes a Ledger entry
(A15). The agent never needs to know vector dimensions or lens internals — just
call `ingest` with text/bytes.

## Build (checklist of concrete, code-level steps)

- [ ] **`calyx.ingest`** schema and impl:
  - Schema: `{"vault": string(required), "input": string(required,
    description:"text to ingest"), "batch": array(optional, items:string,
    description:"batch of texts; mutually exclusive with input")}`
  - Use when: `"store data → constellation (auto multi-lens, idempotent)"`
  - Single item returns: `{"cx_id":"<hex16>","new":true,"ledger_seq":<u64>}`
  - Batch returns: `{"results":[{"cx_id":"…","new":true|false,"ledger_seq":<u64>}]}`
  - `new: false` when content-addressed CxId already exists (idempotent)
  - `CALYX_LENS_UNREACHABLE` when a lens runtime is down during embed

- [ ] **`calyx.anchor`** schema and impl:
  - Schema: `{"vault": string(required), "cx_id": string(required),
    "kind": string(required, enum:["test_pass","thumbs_up","thumbs_down",
    "speaker_match","style_hold","label"]), "label": string(optional,
    required when kind=label), "value": boolean or number(required),
    "confidence": number(optional, 0.0–1.0), "source": string(optional)}`
  - Use when: `"attach a grounded outcome (test pass, thumbs, label)"`
  - Returns: `{"status":"anchored","cx_id":"…","ledger_seq":<u64>}`
  - `CALYX_VAULT_ACCESS_DENIED` when `cx_id` not found in vault

- [ ] **`calyx.measure`** schema and impl:
  - Schema: `{"vault": string(required), "input": string(required)}`
  - Use when: `"get the constellation without storing (for guarding a candidate)"`
  - Returns: constellation JSON with all slot vectors; `SlotVector::Absent` →
    `{"absent":{"reason":"…"}}` (never zero-filled, A16)
  - The result is ephemeral — not written to the vault, no Ledger entry

- [ ] Idempotent `ingest` contract: identical input to the same vault in the same
  panel version → same `cx_id`, `new: false` on second call; verified by the
  content-addressed `CxId` derivation (blake3(input ‖ panel_ver ‖ salt))

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `ingest {"vault":"t","input":"hello"}` twice → first call `"new":true`,
  second call `"new":false`, both return the same `cx_id` hex
- [ ] unit: `anchor {"vault":"t","cx_id":"…","kind":"test_pass","value":true}` →
  result contains `"status":"anchored"` and a non-zero `ledger_seq`
- [ ] unit: `measure {"vault":"t","input":"hello"}` → result JSON has `slots` array;
  `SlotVector::Absent` for a down lens appears as `{"absent":{"reason":"…"}}`,
  not `{"dense":[0,0,0,…]}` (A16 no zero-fill)
- [ ] unit: `ingest` batch of 3 items → result has 3 entries with distinct `cx_id`s
- [ ] edge: `ingest` with both `input` and `batch` set → JSON-RPC `-32602` error
  "mutually exclusive"; `anchor` with `kind:"label"` and no `label` field →
  `-32602` error; `anchor` with unknown `cx_id` → `CALYX_VAULT_ACCESS_DENIED`
- [ ] fail-closed: `ingest` when all lens runtimes return `CALYX_LENS_UNREACHABLE` →
  MCP error with `calyx_code:"CALYX_LENS_UNREACHABLE"` and `remediation:
  "restore lens service"`; partial success (some lenses down) → `SlotVector::Absent`
  for the down lenses, `cx_id` still returned with `CxFlags::degraded` set

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the base CF row for the constellation at `<vault.calyx>/cf/base/<cx_id_hex>`
  and the anchor CF row at `<vault.calyx>/cf/anchors/<cx_id_hex>` after the
  MCP ingest + anchor sequence
- **Readback:** after the `ingest` + `anchor` tool calls, run on aiwonder:
  `calyx readback --cf-row <vault.calyx> --cf base --key <cx_id_hex>` →
  non-empty hex dump; `calyx readback --cf-row <vault.calyx> --cf anchors
  --key <cx_id_hex>` → anchor bytes present with the `test_pass` kind
- **Prove:** base CF row is present and `cx_id` is embedded in its bytes; anchor
  row is present after `anchor` call; second `ingest` of same text returns
  `"new":false` and the CF row is unchanged (idempotent: same bytes)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH63 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
