# T-008 — Scaffold the cargo workspace + crate skeletons

**Phase:** PH01 · **Dep:** T-003 · **Sudo:** no

## Objective
A compiling cargo workspace with **every** Calyx crate as a skeleton, so all
later phases land code into a structure that already builds.

## Preconditions
- T-003 (`env.sh`, toolchain). Repo at `CALYX_HOME/repo` (T-006 may run in
  parallel; the workspace can be created first and committed by T-006).

## Steps
1. `repo/rust-toolchain.toml` pinning the channel chosen in T-003.
2. `repo/Cargo.toml` workspace with members + `[workspace.dependencies]`
   (shared `serde`, `blake3`, `ulid`, `thiserror`, `tracing`, `proptest`, …):
   ```
   [workspace]
   resolver = "2"
   members = ["crates/*"]
   ```
3. Create each crate skeleton under `repo/crates/` (lib unless noted):
   `calyx-core calyx-aster calyx-forge calyx-registry calyx-loom calyx-assay
   calyx-lodestar calyx-mincut calyx-paths calyx-ward calyx-sextant
   calyx-ledger calyx-anneal calyx-oracle calyx-mcp calyx-cli(bin)
   calyxd(bin)`. Each gets a doc-header `lib.rs`/`main.rs` + one trivial
   `#[test]`.
4. `.cargo/config.toml` if needed (target tuning).
5. Build + test:
   ```bash
   source /home/croyse/calyx/repo/env.sh
   cd /home/croyse/calyx/repo && cargo check --workspace && cargo test --workspace
   ```

## Deliverables
- A green workspace: 17 crate skeletons, shared deps, pinned toolchain.

## FSV gate
`cargo check --workspace` + `cargo test --workspace` **green on aiwonder**;
`cargo metadata` lists all members; build output is under `CALYX_HOME/target`
(verify path). No `.rs` file exceeds 500 lines (trivially true; T-009 enforces).

## Done
The workspace compiles + tests clean; every engine has a home crate.

## Refs
PRD `18 §1`, `../00_README.md §5`, A34.
