# 14 — Calyx MCP & Agent Interface

Implements A17. Primary user = AI agent. Goal: **zero hand-written multi-embedder plumbing — ever.** Replace full code with a handful of typed tool calls.

## 1. Design principles

- **Self-describing:** each tool has a typed schema + one-line "use when" (doctrine §7.5). Agent discovers capability without docs.
- **Sensible defaults:** a vault ships default panel/fusion/guard — `search` works with one argument; power comes from optional fields.
- **Constraint over procedure:** agent declares *what* (in-region, fresh, top-5 with provenance), not *how* to fuse/index.
- **Fail closed + structured errors:** every error carries `code`, `message`, `remediation` (`18`).
- **Provenance by default:** results carry lineage; `explain` adds the breakdown.

## 2. The tool surface (grouped)

### Vault & panel
| Tool | Use when |
|---|---|
| `calyx.create_vault(name, panel_template?)` | start a new database; picks `text/code/civic/media-default` panel |
| `calyx.add_lens(vault, lens_spec)` | add a measurement axis — **the one call that replaces a whole pipeline** |
| `calyx.retire_lens / park_lens(vault, slot)` | drop/sideline a low-signal lens |
| `calyx.list_panel(vault)` | see lenses + their bits + state |
| `calyx.profile_lens(lens_spec, probe?)` | get a capability card before committing |

### Ingest & measure
| Tool | Use when |
|---|---|
| `calyx.ingest(vault, input | batch)` | store text data → constellation (auto multi-lens, idempotent) |
| `calyx.ingest_media(vault, file, modality)` | store retained audio/video bytes → constellation |
| `calyx.anchor(vault, cx_id, outcome)` | attach a grounded outcome (test pass, thumbs, label) |
| `calyx.measure(vault, input)` | get the constellation without storing (for guarding a candidate) |

### Search & navigate
| Tool | Use when |
|---|---|
| `calyx.search(vault, query, opts?)` | the everyday multi-lens search (RRF default, provenance attached) |
| `calyx.kernel_answer(vault, query, anchor?)` | answer via the grounded kernel skeleton |
| `calyx.neighbors / agree / disagree(vault, cx, slot?)` | per-lens neighborhood / consistency / anomaly |
| `calyx.define(vault, index_in_lens)` | get a term's grounded definition = the constellation other lenses form at that index |
| `calyx.guard_generate(vault, candidate, identity_cx?)` | identity-locked generation: accept only if inside `Gτ` on identity slots (voice/style) |
| `calyx.traverse(vault, cx, direction, hops)` | causal/asymmetric walk |
| `calyx.skills(vault) / search_skill(vault, skill, query)` | hierarchical-skill navigation |

### Intelligence extraction
| Tool | Use when |
|---|---|
| `calyx.abundance(vault)` | DDA report: N, C(N,2), materialized, n_eff, DPI ceiling |
| `calyx.bits(vault, anchor)` | per-lens signal + panel sufficiency + deficit attribution |
| `calyx.kernel(vault, anchor?)` | build/get the grounding kernel + recall + grounding gaps |
| `calyx.guard.calibrate / guard.check(vault, ...)` | calibrate or apply the `Gτ` boundary |
| `calyx.propose_lens(vault, anchor)` | ask Calyx what lens would close a sufficiency gap |

### Provenance & ops
| Tool | Use when |
|---|---|
| `calyx.provenance(vault, cx_id) / answer_trace(answer_id)` | full lineage |
| `calyx.verify_chain(vault, range) / reproduce(answer_id)` | tamper check / replay a claim |
| `calyx.anneal.status(vault)` | self-optimization state, tripwires, proposals |

## 3. The "build a multi-embedder system" workflow — before vs after

**Before:** load 5 models; write batching, 5 index builders, RRF, MI redundancy checks, kernel finder, guard, provenance; debug dimension mismatches; re-write next project.

**After (Calyx):**
```
v = calyx.create_vault("myproj", panel_template="code-default")   # 15 lenses, ready
calyx.add_lens(v, {name:"my-domain-lens", runtime:"onnx", weights:..., shape:Dense(768)})
calyx.ingest(v, my_inputs)                # multi-lens, idempotent, provenance
calyx.anchor(v, cx, {test_pass:true})     # ground outcomes as they arrive
calyx.bits(v, "test_pass")                # which lenses earn their place
calyx.kernel(v, "test_pass")              # the 1% that explains it
calyx.search(v, "why does X fail under load?")   # multi-lens, guarded, provenanced
```
Every hard part (batching, indexing, fusion, MI, kernel, guard, provenance, autotuning) is inside the database (A17). Agent expresses intent; Calyx does the plumbing.

## 4. Transport & deployment of the interface

- **Embedded:** in-process Rust API (Tauri sidecar links `libcalyx`); MCP over stdio for the local agent — replaces the `sqlite-vec` MCP tools in Leapable's vault sidecar.
- **Server:** `calyxd` exposes MCP over the existing ingress (Cloudflare Access-gated, loopback bind) on aiwonder. `mcp.leapable.ai` is intentionally 503 today (no central MCP upstream); Calyx server MCP is for ops/agent use behind Access, not a per-user public MCP (`16`).
- **Wire format:** MCP JSON-RPC; payloads markdown/JSON per doctrine (markdown for instructions, JSON for tool payloads).

## 5. Ergonomic guarantees (binding)

- No tool requires the agent to know vector dims, index params, or fusion math (defaults + autotune handle it).
- Every result is **explainable** (`explain=true`) and **traceable** (`provenance`).
- Errors are **actionable**: `code` + `remediation` so an agent self-corrects (e.g. `CALYX_ASSAY_INSUFFICIENT_SAMPLES → anchor ≥50 outcomes first`).
- Idempotent ingest (content-addressed) so retries are safe.

**One sentence:** Calyx MCP makes the entire calculus-of-association stack — lenses, DDA, bits, kernel, guard, search, provenance, self-tuning — reachable in a dozen typed, self-describing, provenance-returning tool calls, so a multi-lens system is configured, not coded.
