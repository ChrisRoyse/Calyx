# T-014 — calyx-core: traits + Clock

**Phase:** PH04 · **Dep:** T-013 · **Sudo:** no

## Objective
The engine trait boundaries (`Lens`, `Index`, `VaultStore`, `Estimator`) and the
injected `Clock` — the seams every later crate implements, with determinism
baked in.

## Preconditions
- T-013 (structs).

## Steps
1. `crates/calyx-core/src/traits.rs` (signatures from PRD `18 §3`, all returning
   `Result<_, CalyxError>`):
   - `Lens` — `id`, `shape`, `modality`, `measure`, `measure_batch` (frozen,
     deterministic).
   - `Index` — `insert`, `search(q,k,ef)`, `rebuild`.
   - `VaultStore` — `put`, `get(id,snapshot)`, `anchor`, `snapshot()->Seq`.
   - `Estimator` — `mi`, `redundancy`.
2. `crates/calyx-core/src/time.rs`:
   - `trait Clock { fn now(&self) -> Ts; }` (inject everywhere; **never**
     `SystemTime::now()` in logic — PRD `28 §6c`), a real `SystemClock`, and a
     `FixedClock` for tests; a monotonic server-stamp `Seq`/`Ts` type.
3. Ensure object-safety where dynamic dispatch is needed; doc each trait with a
   one-line "implemented by".
4. A `#[test]` that a `FixedClock` makes a timestamped operation deterministic.

## Deliverables
- `traits.rs` + `time.rs`; the four engine traits + injected `Clock`.

## FSV gate
`cargo test -p calyx-core` green; the traits compile + are object-safe where
required; a `FixedClock`-driven operation is byte-deterministic across runs
(assert), proving the clock is injected (no hidden `SystemTime::now()`).

## Done
Trait boundaries + injected clock exist; `calyx-core` is complete (PH04) and
green; Stage 1+ can implement against these.

## Refs
PRD `18 §3`, `28 §6c`, A4, A15.
