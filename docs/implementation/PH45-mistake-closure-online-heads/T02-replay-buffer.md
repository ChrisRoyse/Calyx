# PH45 · T02 — ReplayBuffer (surprise-prioritized, fixed-capacity)

| Field | Value |
|---|---|
| **Phase** | PH45 — Mistake-Closure + Online Heads + Replay Buffer |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/learn/replay_buffer.rs` (≤500) |
| **Depends on** | T01 (MistakeLog provides `MistakeRef` + surprise scores for seeding) |
| **Axioms** | A4, A14 |
| **PRD** | `dbprdplans/12 §3` |

## Goal

Implement `ReplayBuffer`: a fixed-capacity priority queue of constellations
that need to be replayed for online head updates. Priority is `surprise =
|predicted − observed|` — high-surprise mistakes get replayed first. When the
buffer is full, the lowest-surprise entry is evicted. Seeded from `MistakeLog`
on restart. The buffer is read-only during replay (sampling does not remove
entries; entries are eventually evicted only by the capacity rule).

## Build (checklist of concrete, code-level steps)

- [ ] `struct ReplayEntry { cx_id: CxId, surprise: f64, mistake_ref: MistakeRef, added_ts: LogicalTime }`.
- [ ] `struct ReplayBuffer { heap: BinaryHeap<ReplayEntry>, capacity: usize, clock: Arc<dyn Clock> }` — max-heap by `surprise`; fixed `capacity` (default 4096).
- [ ] `fn push(&mut self, entry: ReplayEntry)` — if buffer full and `entry.surprise ≤ min_surprise_in_heap`, discard; else insert and pop the min-surprise entry.
- [ ] `fn sample_batch(&self, n: usize, seed: u64) -> Vec<ReplayEntry>` — returns `n` entries sampled proportional to `surprise`; seeded RNG for determinism; does NOT remove sampled entries from the buffer.
- [ ] `fn len(&self) -> usize`; `fn is_empty(&self) -> bool`.
- [ ] `fn seed_from_log(&mut self, log: &MistakeLog, n: usize)` — reads last `n` entries from `MistakeLog`, pushes them; used on restart.
- [ ] Persist buffer state to `anneal_replay` CF (serialized snapshot) so high-surprise mistakes survive restarts.
- [ ] Replay does NOT feed back into `MistakeLog` (no infinite loops); replay is strictly read + consume for head update.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: push 3 entries with surprises `[0.8, 0.3, 0.1]`; buffer capacity=2; after 3 pushes, buffer contains `[0.8, 0.3]` (lowest `0.1` evicted).
- [ ] unit: `sample_batch(2, seed=42)` on a 5-entry buffer is deterministic — same seed → same result; different seed → potentially different result.
- [ ] proptest: for any sequence of pushes on a capacity-N buffer, `len() ≤ N` always.
- [ ] edge: push to a capacity-1 buffer: higher surprise replaces lower surprise; equal surprise keeps existing entry (stable eviction); empty buffer `sample_batch` → empty vec.
- [ ] fail-closed: `capacity=0` → `CALYX_ANNEAL_INVALID_CAPACITY`; `sample_batch(n > len())` → returns all entries (no panic).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_replay` CF snapshot + in-memory heap state.
- **Readback:** `calyx anneal replay-status` — prints `len`, `capacity`, `top_surprises: [f64; 5]`.
- **Prove:** push 100 entries with random surprises (seeded); call `replay-status`; confirm `len ≤ 4096`; confirm top 5 surprises are all ≥ the 4091st (heap ordering invariant). `sample_batch(10, seed=42)` called twice returns identical results.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH45 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
