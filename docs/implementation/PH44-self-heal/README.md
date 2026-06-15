# PH44 — Self-Heal (Rebuild Derived, Degrade Flags)

**Stage:** S10 — Anneal + Intelligence Objective J  ·  **Crate:** `calyx-anneal`  ·
**PRD roadmap:** `12 §2`, `24 §7` (rows 12, 16)  ·  **Axioms:** A16

## Objective

Implement Calyx's homeostasis loop: continuous fault detection on derived
structures (ANN indexes, kernel indexes, guard profiles) and automatic
background rebuilds when corruption or drift is detected. A corrupt derived
structure raises a `degraded` flag and serves with reduced quality rather than
returning wrong-but-confident results. A corrupt base shard fails reads closed
and triggers a restore from snapshot/restic. A failing lens endpoint is marked
`health=failing` and routing degrades to remaining lenses. A drifted `τ` triggers
Ward recalibration. A lens whose signal decays below `0.05 bits` is auto-parked.
Every heal action is reversible + tripwire-guarded + Ledger-logged (A14/A15 via
the PH43 substrate).

## Dependencies

- **Phases:** PH43 (AnnealSubstrate safety substrate — all heals run through it),
  PH33 (kernel index — kernel rebuild path; `kernel_answer` needed for heal
  health check)
- **Provides for:** PH45 (mistake-closure runs after heal; a healed state is a
  prerequisite for correct replay), PH46 (autotune can't safely tune a degraded
  index)

## Current state (build off what exists)

`calyx-anneal` crate: PH43 substrate is complete (`tripwire.rs`, `shadow.rs`,
`rollback.rs`, `budget.rs`, `ledger_anneal.rs`). PH44 T01 adds the
`heal::degrade` registry, durable `anneal_health` CF, and
`calyx anneal status --health` readback. Remaining self-heal cards build on
that registry for triggers, background rebuild tasks, restore, and recalibration.

**Anneal invariants (binding):**
- Rebuild derived from base+slots, never from another derived or from a
  wrong-but-confident result (A16).
- Every heal action is reversible + tripwire-guarded + Ledger-logged.
- Bounded background budget — yields to serving + TEI.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/heal/triggers.rs` | Fault detectors: corrupt ANN/kernel/guard checksums, drifted τ, decayed lens signal, stale derived |
| `src/heal/degrade.rs` | `DegradeRegistry`: per-component `health` flag (`ok / degraded / failing / parked`); routing logic for failing lenses |
| `src/heal/rebuild.rs` | Background rebuild tasks: ANN from base+slots, kernel from base+lodestar, guard profile from anchors |
| `src/heal/restore.rs` | Base-shard corruption handler: fail reads closed, trigger restic/ZFS restore, alert |
| `src/heal/recalibrate.rs` | τ recalibration trigger: detect FAR creep, call Ward recalibrate, log |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Degrade registry + health flags | — |
| T02 | Fault detectors (corruption / drift / decay) | T01 |
| T03 | Background rebuild (ANN + kernel + guard from base+slots) | T01, T02 |
| T04 | Base-shard restore path (fail-closed + restic alert) | T01 |
| T05 | τ recalibration trigger + lens park on decay | T01, T02 |
| T06 | Integration FSV: corrupt ANN → degraded + rebuild, no data loss | T01, T02, T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Flip a byte in the HNSW ANN index file → `DegradeRegistry` marks component
`degraded` → background rebuild starts → `calyx anneal status` shows
`health=degraded` then transitions to `health=ok` after rebuild → `xxd` the base
CF confirms base bytes are intact (no data loss) → Ledger shows `kind=Anneal,
action=Rebuild` entry. Separately: kill the TEI lens endpoint → search
`health=degraded` for that lens, results still returned from remaining lenses
(no hang).

## Risks / landmines

- **Derived-only rebuilds:** rebuild MUST source from base+slots (WAL-proven
  data), never from the corrupt derived artifact; enforce with a type-level
  distinction (`DerivedSource` vs `BaseSource`).
- **"Never wrong-but-confident"** (A16): while degraded, search must include a
  `degraded: true` flag in results; callers must not strip it.
- **Rebuild races:** concurrent writes during a rebuild must be safe; use the
  MVCC snapshot from Aster (PH08) to rebuild against a consistent snapshot.
- **Restic/ZFS restore** is operator-gated; the self-heal path alerts and fails
  closed; it does NOT silently restore — log + alert, then wait for operator
  confirmation or auto-restore if configured.
- **Lens park vs retire:** park = keep weights, stop searching; retire = tombstone.
  Auto-park on `< 0.05 bits` signal; never auto-retire (irreversible).
