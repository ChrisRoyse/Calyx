# PH63 · T05 — Search/navigate extensions (agree, disagree, define, guard_generate, traverse, skills)

| Field | Value |
|---|---|
| **Phase** | PH63 — calyx-mcp (stdio embedded tool surface) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-mcp` |
| **Files** | `crates/calyx-mcp/src/tools/search.rs` (extend, ≤500) |
| **Depends on** | T04, PH27 (agreement graph), PH37 (guard), PH34 (multi-scope kernel for skills) |
| **Axioms** | A8, A12, A17, A21 |
| **PRD** | `dbprdplans/14 §2` (search/navigate — agree/disagree/define/guard_generate/traverse/skills) |

## Goal

Register the remaining six search/navigate tools that expose the agreement graph,
conceptual definition, identity-locked generation gate, causal traversal, and
hierarchical skill navigation. These are richer navigation tools that an agent
uses when the basic `search` is not sufficient for the task.

## Build (checklist of concrete, code-level steps)

- [ ] **`calyx.agree`** and **`calyx.disagree`** schemas and impls:
  - Schema: `{"vault": string(required), "cx_id": string(required),
    "slot": integer(optional)}`
  - Use when agree: `"find constellations consistent with this one on a given lens"`
  - Use when disagree: `"find constellations that are anomalous relative to this one"`
  - Returns: `{"constellations":[{"cx_id":"…","score":0.88,"slot":0}]}`
  - Backed by `Calyx::neighbors` with agreement/disagreement polarity

- [ ] **`calyx.define`** schema and impl:
  - Schema: `{"vault": string(required), "lens": integer(required,
    description:"lens slot id"), "index": integer(required,
    description:"position within the lens's output space")}`
  - Use when: `"get a term's grounded definition — the constellation other lenses
    form at that index (Gärdenfors conceptual space)"`
  - Returns: `{"definition": Constellation JSON}` — the cross-lens reading at
    the given index (PRD 18 §4 `Calyx::define`)

- [ ] **`calyx.guard_generate`** schema and impl:
  - Schema: `{"vault": string(required), "candidate_text": string(required),
    "identity_cx": string(optional, description:"reference cx for speaker/style
    identity lock")}`
  - Use when: `"identity-locked generation gate: accept only if inside Gτ on
    identity slots (voice/style)"`
  - Returns: `{"verdict":"pass"|"ood","tau":0.12,"distance":0.08,
    "identity_cx":"…"}`
  - `CALYX_GUARD_OOD` → remediation `"new-region or reject per policy"`;
    `CALYX_GUARD_PROVISIONAL` when not calibrated

- [ ] **`calyx.traverse`** schema and impl:
  - Schema: `{"vault": string(required), "cx_id": string(required),
    "direction": string(required, enum:["forward","backward","both"]),
    "hops": integer(required, min:1, max:10)}`
  - Use when: `"causal/asymmetric walk from a constellation"`
  - Returns: `{"path":[{"cx_id":"…","hop":1,"direction":"forward","score":0.71}]}`

- [ ] **`calyx.skills`** and **`calyx.search_skill`** schemas and impls:
  - `skills {"vault": string}` → `{"skill_tree": {…hierarchical skill nodes…}}`
  - `search_skill {"vault": string, "skill": string, "query": string}` →
    `{"hits":[…]}` (search within a skill scope)
  - Use when skills: `"hierarchical-skill navigation"`
  - Use when search_skill: `"search within a specific skill scope"`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `agree` for a constellation with 5 neighbors → result has ≤5 entries,
  all with `score` in [0.0, 1.0]; `disagree` with same cx → different top results
  (anomaly polarity different from agreement)
- [ ] unit: `define` with a known `lens=0, index=42` → returns a constellation
  JSON with `slots` array (may be an empty/stub result before PH34, but not an
  error)
- [ ] unit: `guard_generate` before calibration → `CALYX_GUARD_PROVISIONAL` in
  `error.data`; after calibration with injection corpus → `verdict:"ood"` for an
  injection candidate
- [ ] unit: `traverse {"direction":"forward","hops":2}` → path has ≤2 entries,
  each with increasing `hop` value
- [ ] edge: `traverse hops:0` → JSON-RPC `-32602`; `traverse hops:11` → `-32602`
  (max is 10); `skills` on empty vault → `{"skill_tree":{}}`, not an error;
  `search_skill` for unknown skill name → empty `hits`, not an error
- [ ] fail-closed: `guard_generate` with no vault → `CALYX_VAULT_ACCESS_DENIED`;
  `agree` with non-existent `cx_id` → `CALYX_VAULT_ACCESS_DENIED`; never
  returns agreement results for a cx that doesn't exist

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the agreement graph entries in `<vault.calyx>/cf/loom/` after a vault
  with ≥2 constellations has been ingested
- **Readback:** pipe `agree {"vault":"mcp-test","cx_id":"<id>"}` to `calyx-mcp`
  on aiwonder and capture the stdout JSON; the `constellations` array contains
  at least one entry; each entry's `cx_id` is verifiable via `calyx readback
  --cf-row … --cf base --key <cx_id_hex>` returning non-empty bytes
- **Prove:** `agree` and `disagree` return different sets of `cx_id`s for the same
  input constellation; `guard_generate` with an out-of-domain candidate returns
  `verdict:"ood"` when the vault has a calibrated guard profile

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH63 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
