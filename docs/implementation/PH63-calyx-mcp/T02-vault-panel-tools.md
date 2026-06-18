# PH63 · T02 — Vault/panel tool group

| Field | Value |
|---|---|
| **Phase** | PH63 — calyx-mcp (stdio embedded tool surface) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-mcp` |
| **Files** | `crates/calyx-mcp/src/tools/vault.rs` (≤500) |
| **Depends on** | T01 (server scaffold), PH62·T02 (vault/lens engine API wired) |
| **Axioms** | A4, A5, A17 |
| **PRD** | `dbprdplans/14 §2` (vault/panel group) |

## Goal

Register the six vault-and-panel tools with typed schemas and one-line "use when"
descriptions. These are the setup tools an agent calls once to configure a vault.
`add_lens` is "the one call that replaces a whole pipeline" (PRD 14 §2).

## Build (checklist of concrete, code-level steps)

- [ ] `tools/vault.rs`: define a `struct` for each tool, each implementing `Tool`;
  register all six via `McpServer::register`

- [ ] **`calyx.create_vault`** schema and impl:
  - Schema: `{"name": string(required), "panel_template": string(optional,
    enum:["text-default","code-default","civic-default","media-default"])}`
  - Use when: `"start a new database; picks text/code/civic/media-default panel"`
  - Returns: `{"vault_id":"<ulid>","name":"…","panel_template":"…"}`
  - Default: `panel_template` defaults to `"text-default"` if omitted

- [ ] **`calyx.add_lens`** schema and impl:
  - Schema: `{"vault": string(required), "name": string(required), "runtime":
    string(required, enum:["tei-http","onnx","candle","algorithmic"]), "endpoint":
    string(optional), "weights": string(optional), "shape": string(optional,
    e.g. "Dense(768)"), "modality": string(optional,
    enum:["text","code","image","audio","video","structured","mixed"])}`
  - Use when: `"add a measurement axis — the one call that replaces a whole pipeline"`
  - Returns: `{"lens_id":"<hex16>","slot_id":<u16>,"name":"…","state":"active"}`
  - `CALYX_LENS_DIM_MISMATCH` → error with `data.remediation: "fix lens or slot shape"`

- [ ] **`calyx.retire_lens`** and **`calyx.park_lens`** schemas and impls:
  - Schema: `{"vault": string(required), "slot": integer(required)}`
  - Use when retire: `"drop a low-signal lens permanently"`;
    use when park: `"sideline a lens without deleting its data"`
  - Returns: `{"status":"retired"|"parked","slot":<u16>}`

- [ ] **`calyx.list_panel`** schema and impl:
  - Schema: `{"vault": string(required)}`
  - Use when: `"see lenses, their bits signal, and state"`
  - Returns: `{"slots":[{"slot":0,"name":"…","state":"active",
    "modality":"text","bits":0.72,"ci":[0.61,0.83],"lens_id":"…"}]}`

- [ ] **`calyx.profile_lens`** schema and impl:
  - Schema: `{"runtime": string(required), "endpoint": string(optional),
    "weights": string(optional), "probe": string(optional, JSONL path),
    "modality": string(optional)}`
  - Use when: `"get a capability card before committing to a lens"`
  - Returns: `CapabilityCard` JSON with `signal`, `spread`, `separation`, `cost`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `tools/call create_vault {"name":"t","panel_template":"code-default"}`
  → result contains `"vault_id"` (non-empty ULID string) and
  `"panel_template":"code-default"`
- [ ] unit: `tools/call add_lens` with missing `vault` field → JSON-RPC error
  `-32602` (invalid params) with message naming the missing field
- [ ] unit: `tools/call add_lens` with correct params → result contains `lens_id`
  (16-byte hex), `slot_id` (u16), `state:"active"`
- [ ] unit: `tools/call list_panel` after `add_lens` → `slots` array has one entry
  with the correct `name` and `state:"active"`
- [ ] edge: `panel_template: "unknown-panel"` → JSON-RPC `-32602` error;
  `retire_lens` on slot that does not exist → `CALYX_VAULT_ACCESS_DENIED` in
  `error.data.calyx_code`
- [ ] fail-closed: `add_lens` with `runtime:"tei-http"` but no `endpoint` →
  `CALYX_LENS_UNREACHABLE` with `remediation:"restore lens service"` in `data`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the vault manifest file at `<vault.calyx>/manifest/CURRENT` after
  `create_vault` + `add_lens` tool calls
- **Readback:** pipe the two-call sequence to `calyx-mcp` on aiwonder and capture
  the stdout JSON; then `calyx readback --hex <vault.calyx>/manifest/CURRENT` to
  read the manifest bytes; the manifest bytes contain the ULID from the
  `create_vault` response
- **Prove:** the `vault_id` in the `create_vault` MCP response matches the ULID
  embedded in the manifest bytes; the `lens_id` from `add_lens` appears in the
  manifest's lens registry; `list_panel` after `add_lens` returns the correct slot

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH63 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
