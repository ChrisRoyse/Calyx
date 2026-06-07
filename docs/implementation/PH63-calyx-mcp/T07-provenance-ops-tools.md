# PH63 · T07 — Provenance/ops tool group

| Field | Value |
|---|---|
| **Phase** | PH63 — calyx-mcp (stdio embedded tool surface) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-mcp` |
| **Files** | `crates/calyx-mcp/src/tools/provenance.rs` (≤500) |
| **Depends on** | T06, PH35 (hash-chain Ledger), PH36 (Merkle verify/reproduce), PH45 (Anneal) |
| **Axioms** | A15, A17 |
| **PRD** | `dbprdplans/14 §2` (provenance/ops group), `dbprdplans/18 §4` |

## Goal

Register the five provenance-and-ops tools: `provenance`, `answer_trace`,
`verify_chain`, `reproduce`, and `anneal.status`. These let an agent trace the
lineage of any result, verify tamper-evidence, replay a claim, and inspect self-
optimization state — the "every result is traceable" guarantee from PRD 14 §5.

## Build (checklist of concrete, code-level steps)

- [ ] **`calyx.provenance`** schema and impl:
  - Schema: `{"vault": string(required), "cx_id": string(required)}`
  - Use when: `"full lineage of a constellation"`
  - Returns: `Lineage` JSON: `{"cx_id":"…","ingest_seq":12,
    "ledger_chain_hash":"<hex64>","lens_measures":[{"slot":0,"lens_id":"…",
    "measured_at":"<ts>"}],"anchors":[{"kind":"test_pass","ledger_seq":13}]}`

- [ ] **`calyx.answer_trace`** schema and impl:
  - Schema: `{"answer_id": string(required)}`
  - Use when: `"full lineage of a kernel answer or search result"`
  - Returns: `AnswerTrace` JSON with retrieval steps, kernel cx_ids used, and
    each step's ledger reference

- [ ] **`calyx.verify_chain`** schema and impl:
  - Schema: `{"vault": string(required), "from_seq": integer(optional),
    "to_seq": integer(optional)}`
  - Use when: `"tamper check: verify the Ledger hash-chain over a range"`
  - Returns: `{"status":"ok"|"broken","checked":<n>,"break_at":null|<seq>}`
  - `CALYX_LEDGER_CHAIN_BROKEN` → `error.data.remediation:
    "quarantine range, investigate"` (verbatim)

- [ ] **`calyx.reproduce`** schema and impl:
  - Schema: `{"vault": string(required), "answer_id": string(required)}`
  - Use when: `"replay a claim to verify bit-parity"`
  - Returns: `{"bit_parity":true,"original_hash":"…","reproduced_hash":"…"}`
  - Mismatch → `{"bit_parity":false,…}` with both hashes; both are returned,
    never hidden; exit with error when divergent (fail closed)

- [ ] **`calyx.anneal.status`** schema and impl:
  - Schema: `{"vault": string(required)}`
  - Use when: `"self-optimization state, tripwires, proposals"`
  - Returns: `{"phase":"stable"|"healing"|"tuning","tripwires":[{"name":"…",
    "state":"armed"|"tripped"}],"proposals":[{"type":"add_lens","rationale":"…"}],
    "last_soak_at":"<ts>","p99_latency_ms":42}`

- [ ] All five tools registered in `McpServer` with correct `ToolDef.use_when`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `provenance` for a known `cx_id` → `ledger_chain_hash` is a 64-char
  hex string; `ingest_seq` matches the `ledger_seq` returned by the `ingest` call
- [ ] unit: `verify_chain` on an intact vault → `{"status":"ok","break_at":null}`
- [ ] unit: `verify_chain` after flipping one byte in a Ledger file → `status:
  "broken"` and `break_at` is the tampered sequence number
- [ ] unit: `reproduce` for a deterministic kernel answer → `bit_parity:true`;
  `original_hash == reproduced_hash`
- [ ] unit: `anneal.status` → result contains all required fields and is valid
  JSON (parse round-trip)
- [ ] edge: `provenance` for unknown `cx_id` → `CALYX_VAULT_ACCESS_DENIED`;
  `verify_chain` with `from_seq > to_seq` → JSON-RPC `-32602`; `answer_trace`
  for unknown `answer_id` → `CALYX_VAULT_ACCESS_DENIED`; `reproduce` with
  divergent hash never returns `bit_parity:true`
- [ ] fail-closed: `verify_chain` on a vault where the Ledger CF is missing →
  `CALYX_ASTER_CORRUPT_SHARD`, not a silent `"status":"ok"`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the Ledger entry bytes at `<vault.calyx>/ledger/<seq_hex>` on aiwonder
  after the MCP workflow (ingest + anchor + search) has written entries
- **Readback:** pipe `provenance {"vault":"mcp-test","cx_id":"<id>"}` to
  `calyx-mcp` on aiwonder; extract `ledger_chain_hash` from the JSON response;
  run `calyx readback --ledger <vault.calyx> --seq 1` and compare the
  `entry_hash` in the readback header to the `ledger_chain_hash` in the MCP
  response — they must match byte-for-byte (hex comparison)
- **Prove:** `verify_chain` on the aiwonder vault after the full MCP workflow
  returns `"status":"ok"`; the Ledger entry bytes read by `readback --ledger`
  contain the same chain hash as the `provenance` MCP response; `anneal.status`
  returns a non-empty JSON without error

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH63 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
