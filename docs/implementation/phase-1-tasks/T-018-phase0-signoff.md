# T-018 — Phase-0 exit-gate FSV checklist + sign-off issue

**Phase:** PH04 · **Dep:** all T-001…T-017 · **Sudo:** no

## Objective
Prove Stage 0 is genuinely DONE by FSV — not by checking boxes — and record the
sign-off, then update the context issues and point at Stage 1.

## Preconditions
- T-001…T-017 complete.

## Steps
1. Run the Stage-0 exit checklist (`../10_STAGE0_FOUNDATION.md`), each line
   proven by reading bytes on aiwonder:
   - `CALYX_HOME` self-contained; **no Calyx file outside the root** (spot-check
     `find`); leapable/contextgraph/postgres untouched.
   - `source repo/env.sh && bash scripts/check.sh` → fmt+clippy+test+≤500 all ✅.
   - `cargo test --workspace` green; `calyx-core` IDs/enums/errors/structs/traits
     done; error catalog == PRD `18 §6` (printed diff); serde round-trips
     byte-exact; content-addressing deterministic.
   - Five pinned `type:context` issues live; read-state query returns them.
   - CUDA sm_120 smoke passed (T-005); HF auth 200 (T-017); readback tool prints
     real bytes (T-016).
2. Capture evidence (readback output; `cargo test` log; `gh issue list`;
   screenshots where visual) and attach to a **`[SIGN-OFF] Stage 0`** issue.
3. Update `[CONTEXT] You are here` → "Stage 0 DONE; starting Stage 1 (Aster) /
   PH05 WAL". Run the per-phase context-hygiene pass (re-verify each context-
   issue line; prune stale).
4. Open the Stage 1 `type:task` issues (PH05 first).

## Deliverables
- A `[SIGN-OFF] Stage 0` issue with FSV evidence; updated context issues; Stage 1
  tasks opened.

## FSV gate
Every Stage-0 exit line is proven by a byte-level readback attached to the
sign-off issue (not a return value, not a harness verdict). The
`[CONTEXT] You are here` issue truthfully reflects the new state.

## Done
Stage 0 signed off with evidence; the project is ready to build Aster (Stage 1).

## Refs
`../10_STAGE0_FOUNDATION.md`, `../03_PHASE_MAP.md`, PRD `29 §4`, DOCTRINE §0.
