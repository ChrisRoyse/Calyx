# PH63 · T04 — Search/navigate tool group (search, kernel_answer, neighbors)

| Field | Value |
|---|---|
| **Phase** | PH63 — calyx-mcp (stdio embedded tool surface) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-mcp` |
| **Files** | `crates/calyx-mcp/src/tools/search.rs` (≤500) |
| **Depends on** | T03, PH24 (RRF fusion + provenance), PH26 (explain), PH33 (kernel_answer) |
| **Axioms** | A10, A15, A17 |
| **PRD** | `dbprdplans/14 §2` (search/navigate group), `dbprdplans/14 §5` |

## Goal

Register the primary `search` tool — the agent's everyday retrieval call — plus
`kernel_answer` and `neighbors`. `search` must work with one argument (`vault` +
`query`); all other fields have sensible defaults (k=10, fusion=rrf, provenance=
true, guard=off). Every hit carries provenance by default (A15). `kernel_answer`
answers via the grounded kernel skeleton.

## Build (checklist of concrete, code-level steps)

- [ ] **`calyx.search`** schema and impl:
  - Schema: `{"vault": string(required), "query": string(required),
    "k": integer(optional, default:10, min:1, max:1000),
    "fusion": string(optional, enum:["rrf","weighted_rrf","single_lens",
    "kernel_first","pipeline"], default:"rrf"),
    "guard": string(optional, enum:["off","in_region"], default:"off"),
    "explain": boolean(optional, default:false),
    "fresh": boolean(optional, default:false),
    "filter": object(optional, description:"JSON predicate")}`
  - Use when: `"the everyday multi-lens search (RRF default, provenance attached)"`
  - Returns: `{"hits":[{"rank":1,"cx_id":"…","score":0.83,"provenance":
    {"ledger_seq":42,"chain_hash":"…"},"per_lens":[…only if explain],"guard":
    {…only if guard≠off}}]}`
  - `provenance` is always present in every hit (not optional, A15)
  - `CALYX_GUARD_OOD` in `error.data` when a guarded search blocks all results

- [ ] **`calyx.kernel_answer`** schema and impl:
  - Schema: `{"vault": string(required), "query": string(required),
    "anchor": string(optional, description:"anchor kind to ground the kernel"),
    "explain": boolean(optional, default:false)}`
  - Use when: `"answer via the grounded kernel skeleton"`
  - Returns: `{"answer":"…","kernel_cx_ids":["…"],"recall":0.97,"gaps":["…"]}`
  - `CALYX_KERNEL_UNGROUNDED` → `error.data.remediation: "add anchors
    (grounding_gaps)"` (verbatim from PRD 18 §6)

- [ ] **`calyx.neighbors`** schema and impl:
  - Schema: `{"vault": string(required), "cx_id": string(required),
    "slot": integer(optional, description:"per-lens neighborhood; all slots
    if omitted"), "k": integer(optional, default:10)}`
  - Use when: `"per-lens neighborhood of a known constellation"`
  - Returns: `{"neighbors":[{"cx_id":"…","score":0.91,"slot":0}]}`

- [ ] Default sensibility contract (binding): a `search` call with only `vault`
  and `query` set must succeed on any vault that has been ingested into; no
  additional config required from the agent

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `tools/call search {"vault":"t","query":"hello"}` (minimal args) →
  result has `hits` array; each hit contains `provenance` field; no error
- [ ] unit: `search` with `explain:true` → each hit's `per_lens` array is non-empty
  and contains `slot`, `rank`, `raw`, `weight`, `contribution` fields
- [ ] unit: `kernel_answer` before vault has anchors → `CALYX_KERNEL_UNGROUNDED`
  in `error.data.calyx_code`; `error.data.remediation` = `"add anchors
  (grounding_gaps)"`
- [ ] unit: `neighbors {"vault":"t","cx_id":"<id>","k":5}` → at most 5 neighbors,
  each with valid `cx_id` and `score` in [0.0, 1.0]
- [ ] edge: `search` on empty vault → `{"hits":[]}`, exit without error (empty is
  not an error); `k:0` → JSON-RPC `-32602`; `fusion:"unknown"` → `-32602`
- [ ] fail-closed: `search` with `guard:"in_region"` before guard is calibrated →
  `CALYX_GUARD_PROVISIONAL` in `error.data`; never silently returns unguarded
  results when `guard:"in_region"` is specified

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the raw JSON-RPC response bytes written to stdout by `calyx-mcp` for
  a `search` tool call on aiwonder
- **Readback:** pipe the search request to `calyx-mcp` and capture stdout:
  `echo '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":
  "calyx.search","arguments":{"vault":"mcp-test","query":"fail under load"}}}' |
  ./calyx-mcp 2>/dev/null` → stdout is the JSON-RPC response; inspect with
  `python3 -m json.tool` to verify `hits[0].provenance` is present
- **Prove:** the `provenance.ledger_seq` in the top hit is a non-zero integer;
  the `cx_id` in the hit matches a constellation that exists in the vault (verified
  by `calyx readback --cf-row … --key <cx_id_hex>` returning non-empty bytes);
  search with `explain:true` includes `per_lens` in the response

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH63 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
