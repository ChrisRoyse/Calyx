# Calyx Documentation — Shared Authoring Spec (READ FIRST)

You are one of several documentation agents producing a numbered technical reference
for the **Calyx** project (repo: `ChrisRoyse/Calyx-Dev`, local checkout at
`C:\code\Calyx-Dev`). Calyx is an "association-native database" written in Rust
(workspace, edition 2024, toolchain 1.95). Output `.md` files go in
`C:\Users\hotra\Downloads\calyxdocs\`.

## Iron rules (non-negotiable)

1. **Document WHAT IS, not what should be.** Every statement must be derived directly
   from the source code you read. No assumptions, no aspirations, no marketing.
2. **NEVER invent.** If you cannot determine something from the code, write
   "Not determined from source" rather than guessing.
3. **NEVER describe aspirational behavior.** Document what the code DOES, not what a
   comment says it should do. If a struct/function is a stub or `todo!()`, say so.
4. **NEVER skip "boring" parts.** Document every public struct field, every function
   parameter, every config key, every enum variant, every error type. Completeness is
   the point.
5. **Trace every claim to a source file.** Reference paths inline as
   `crates/calyx-x/src/y.rs` (and the function/struct name) when making a behavioral claim.
6. **Use tables aggressively** for structured data: struct fields, function signatures,
   enum variants, constants, config keys, error types, parameters.
7. **Describe algorithm STEPS, not just names.** "Uses HNSW" is insufficient — give the
   steps, parameters, and complexity where determinable.
8. **Code snippets only when exact syntax matters** (hash preimages, SQL DDL, on-disk
   byte layouts, CLI command formats, key formulas). Keep them short.
9. Keep prose factual and terse. No "cleverly", "efficiently", "powerful".
10. Include a **"What is NOT covered / gaps"** note where relevant (stubs, `todo!()`,
    unimplemented paths, explicit limitations).

## Required document structure

Every doc you write must:
- Start with a top-level `# Heading` matching the topic.
- Immediately list **`**Source files covered:**`** — every source file the doc draws from
  (bulleted list of `crates/.../file.rs` paths).
- Use numbered sections `## 1. ...`, `## 2. ...` with subsections `### 1.1`, `### 1.2`.
- Cross-reference sibling docs by filename, e.g. `See [04_storage_and_schema.md](04_storage_and_schema.md)`.
- End with a short `## Gaps / not covered` section if applicable.

## How to read the code

- Start from the crate's `src/lib.rs` (module-level `//!` doc + `pub mod`/`pub use`).
- Follow `pub` items. Read `mod.rs` files, then submodules.
- Read the crate's `tests/` dir to confirm real behavior and intended contracts.
- Read the crate's `Cargo.toml` for dependencies and features.
- Cross-check the planning doc for the subsystem in `docs/dbprdplans/` (these are the
  ORIGINAL design plans — useful for intent, but if code and plan disagree, document the
  CODE and note the divergence). Do NOT copy aspirational plan text as if it were implemented.

## Full document index (for cross-references)

- 00_INDEX.md — table of contents
- 01_system_overview.md — architecture, stack, subsystem summaries
- 02_source_code_map.md — full file tree + dependency graph
- 03_configuration.md — all config keys, env vars
- 04_storage_and_schema.md — Aster on-disk format + any SQLite schema
- 05_core.md — calyx-core (ids, errors, data model, traits)
- 06_aster_storage_engine.md — calyx-aster (LSM, WAL, MVCC, tiering)
- 07_forge_math_runtime.md — calyx-forge (CPU SIMD / CUDA / quantization)
- 08_registry_lenses.md — calyx-registry (embedder runtimes)
- 09_sextant_search.md — calyx-sextant (ANN, BM25, fusion, planner)
- 10_loom_associations.md — calyx-loom (DDA, agreement graph)
- 11_assay_signal_bits.md — calyx-assay (mutual information, panel sufficiency)
- 12_lodestar_kernel.md — calyx-lodestar (grounding kernel)
- 13_ward_guard.md — calyx-ward (fail-closed guard, conformal calibration)
- 14_ledger_provenance.md — calyx-ledger (hash chain, checkpoints)
- 15_anneal_optimization.md — calyx-anneal (reversible self-tuning)
- 16_oracle_prediction.md — calyx-oracle (consequence prediction, honesty gate)
- 17_graph_mincut_paths.md — calyx-mincut + calyx-paths (graph primitives)
- 18_hazard_soak_and_testkit.md — calyx-hazard-soak + calyx-testkit
- 19_mcp_api_tools_reference.md — calyx-mcp (every tool, params, returns, errors)
- 20_cli_and_daemon_reference.md — calyx-cli + calyxd (commands, flags, metrics)
- 21_test_suite.md — test inventory, categories, how to run
- 22_verification_report.md — counts, metrics, health snapshot
- 23_planning_docs_summary.md — summary of docs/dbprdplans + docs/implementation
- 24_roadmap.md — open GitHub issues + remaining work

## Filename you must write

Your dispatch prompt tells you exactly which filename(s) to produce. Write only those.
Return to the orchestrator a SHORT summary (≤200 words): what you wrote, the key
cross-cutting facts (schema version, notable constants, public tool/command names,
error taxonomy roots) other docs may need, and any stubs/gaps you found.
