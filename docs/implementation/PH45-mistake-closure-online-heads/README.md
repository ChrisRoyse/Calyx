# PH45 — Mistake-Closure + Online Heads + Replay Buffer

**Stage:** S10 — Anneal + Intelligence Objective J  ·  **Crate:** `calyx-anneal`  ·
**PRD roadmap:** `12 §3`, `03 §6`  ·  **Axioms:** A4, A14

## Objective

Implement the JEPA "wrong only once" loop as a database service: when an
observed outcome contradicts a trusted prediction, log it as a `MistakeEntry`,
add the constellation to a surprise-prioritized `ReplayBuffer`, and in a bounded
background "sleep" pass update small `OnlineHeadState` structures (predictor,
calibrator, fusion weights) via an EWC++-style continual update — without ever
touching frozen lens weights (A4). A regression re-assert ensures the same
mistake does not recur on replay. Every update is reversible + tripwire-guarded
+ Ledger-logged via the PH43 substrate. Only derived structures learn; base data
and frozen lenses are immutable (A15).

## Dependencies

- **Phases:** PH44 (self-heal substrate; a healed, non-degraded state is the
  prerequisite for correct mistake-closure; `DegradeRegistry` consulted before
  updating heads)
- **Provides for:** PH46 (autotune bandit uses fusion weights from
  `OnlineHeadState`), PH48 (`mistake_rate` term in `J` sourced from `MistakeLog`)

## Current state (build off what exists)

`calyx-anneal` crate: PH43 + PH44 complete. No mistake-closure, replay buffer,
or online-head structures exist. Greenfield. Heritage: ContextGraph `mejepa`
`mistake_log` / `replay_buffer` / `online_head_state` / `heal` modules — logic
absorbed into Calyx, source copied into `CALYX_HOME`.

**Anneal invariants (binding):**
- Only DERIVED structures learn (predictor/calibrator/fusion weights).
- Frozen lens weights (A4): hash-stable throughout; never touched.
- Persisted constellations (A15): never modified by mistake-closure.
- Every head update reversible + tripwire-guarded + Ledger-logged.
- Bounded background budget — yields to serving + TEI.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/learn/mistake_log.rs` | `MistakeLog` + `MistakeEntry`; append-only CF; `mistake_rate` metric |
| `src/learn/replay_buffer.rs` | `ReplayBuffer`: surprise-prioritized (`|predicted−observed|`) queue; fixed-capacity; evicts lowest-surprise |
| `src/learn/online_head.rs` | `OnlineHeadState`: predictor/calibrator/fusion weight tensors; EWC++-style update; Fisher-diagonal regularizer |
| `src/learn/regression_assert.rs` | Regression re-assert: replay the mistake after head update; assert it does not recur |
| `src/learn/frozen_guard.rs` | `FrozenLensGuard`: checks lens hash before and after any update; `CALYX_LENS_FROZEN_VIOLATION` if changed |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | MistakeLog (append, rate metric) | — |
| T02 | ReplayBuffer (surprise-prioritized, fixed-capacity) | T01 |
| T03 | OnlineHeadState (EWC++ update, head types) | T02 |
| T04 | FrozenLensGuard (hash-stable invariant) | — |
| T05 | Regression re-assert (replay + no-recurrence check) | T01, T02, T03 |
| T06 | Integration FSV: contradiction → update → no recurrence, frozen unchanged | T01–T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Feed a contradicting outcome (predicted label A, observed label B) → Ledger has
`MistakeEntry`; `ReplayBuffer` contains the constellation (prioritized by
`|A−B|`); background pass updates `OnlineHeadState`; replay the original input
→ prediction is now correct (no recurrence) → Ledger has `HeadUpdate` entry.
Read the frozen lens weight hash before and after — must be byte-identical.

## Risks / landmines

- **EWC++ Fisher diagonal** requires storing per-parameter importance weights;
  these can be large if heads are large. Keep online heads small (≤1024 params
  per head type) and use quantized Fisher storage.
- **Replay buffer capacity**: surprise-priority eviction means low-surprise
  mistakes are dropped. This is correct behavior — but test that mistakes above
  threshold always survive eviction.
- **Frozen lens hash** must be computed over the entire weight tensor, not just
  metadata; use SHA-256 of the serialized weight bytes.
- **EWC++ update is approximate** (online Fisher); the regularizer prevents
  catastrophic forgetting but does not guarantee it. The regression re-assert is
  the binding check, not the math alone.
- **No recursion**: the replay does not feed back into the `ReplayBuffer` (would
  cause infinite loops on borderline cases); replay is read-only.
