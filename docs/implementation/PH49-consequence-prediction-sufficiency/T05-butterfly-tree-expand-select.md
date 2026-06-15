# PH49 · T05 — Butterfly tree: `expand` + `select` (hop-attenuated)

| Field | Value |
|---|---|
| **Phase** | PH49 — Consequence prediction + sufficiency gate |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/butterfly.rs` (≤500) |
| **Depends on** | T04 (`oracle_predict`, `Consequence` type), PH42 (grounded recurrence edges) |
| **Axioms** | A20, A2, A29 |
| **PRD** | `dbprdplans/21 §2` (`expand(consequence)`, `select(branch)`) |

## Goal

Implement `expand(consequence) -> Vec<Consequence>` and `select(branch, desired_outcome) -> ConsequenceTree`
— the butterfly tree API from `21 §2`. `expand` walks the grounded recurrence graph one
hop deeper, attenuating confidence per hop. `select` returns the branch whose terminal
consequences best match a desired outcome. Together they let a caller "choose the pathway
with the consequences you want" — the Oracle's consequence-inversion / agency interface.
Depth is bounded (`MAX_DEPTH = 4`); cyclic graphs are detected (visited-set); attenuation
factor default 0.7 per hop (configurable). Only grounded edges are traversed; ungrounded
paths are labeled `provisional`.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn expand(vault: &Vault, consequence: &Consequence, clock: &dyn Clock) -> Result<Vec<Consequence>, OracleError>` — queries PH42 recurrence graph for outgoing edges from `consequence.action_or_event` in `consequence.domain`; returns up to `MAX_DEPTH` hops
- [ ] Per-hop confidence attenuation: `child.confidence = parent.confidence * HOP_ATTENUATION` (default `0.7`); clamp to `[0.0, 1.0]`
- [ ] `hop` counter incremented per level; stop expansion when `hop >= MAX_DEPTH` (default `4`) or confidence < `MIN_CONFIDENCE_THRESHOLD` (default `0.05`)
- [ ] Cycle detection: maintain a `visited: HashSet<(domain, action_or_event)>` per expansion call; skip already-visited nodes silently (log to Ledger as structural note, not error)
- [ ] Ungrounded edges (no recurrence backing): mark child `Consequence.provenance` as provisional (`LedgerRef::provisional()`); do not inflate confidence
- [ ] `pub fn select(tree: &ConsequenceTree, desired_outcome: &AnchorValue) -> Option<&ConsequenceTree>` — depth-first search for the branch with terminal `outcome` closest to `desired_outcome` (cosine similarity via Forge, or exact match for discrete anchors); returns the best-match subtree
- [ ] `pub fn build_tree(vault: &Vault, root: Consequence, clock: &dyn Clock) -> Result<ConsequenceTree, OracleError>` — convenience: recursively expands from root to `MAX_DEPTH`, returns full `ConsequenceTree`
- [ ] `MAX_DEPTH`, `HOP_ATTENUATION`, `MIN_CONFIDENCE_THRESHOLD` as named constants (not magic numbers)
- [ ] All new `Consequence` nodes carry a `LedgerRef` (A15); write one ledger entry per `expand` call

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: linear chain A→B→C→D in synthetic vault; `expand(A)` depth-first to 3 hops returns B, C, D; confidence at hop 3 = `seed_confidence * 0.7^3 ± 1e-4`
- [ ] unit: `select` on tree with three branches, one ending in target outcome → returns the correct subtree
- [ ] unit: cyclic graph A→B→A → `expand` terminates without infinite loop; B's children do not include A again
- [ ] proptest: for any tree built by `build_tree`, all `consequence.hop ≤ MAX_DEPTH`; all `consequence.confidence ≤ parent.confidence`
- [ ] edge (≥3): no outgoing edges from `consequence` → `expand` returns empty `Vec` (not error); `hop = MAX_DEPTH` already → expansion returns empty; all branches below `MIN_CONFIDENCE_THRESHOLD` → prune, return empty
- [ ] fail-closed: PH42 recurrence query failure → `Err(OracleError::NoRecurrence)` with code; not silently empty

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the `ConsequenceTree` JSON printed by `calyx readback oracle_expand <consequence_id>`; hop count and confidence values in each `Consequence` node
- **Readback:** `calyx readback oracle_expand <consequence_id> --depth 3` prints the tree JSON; inspect `hop` and `confidence` fields at each depth; `grep provisional` shows which edges lacked grounding
- **Prove:** confidence at depth k = `root_confidence * 0.7^k ± 1e-4`; no hop exceeds `MAX_DEPTH = 4`; cycle in real data does not cause stack overflow or infinite loop

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH49 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
