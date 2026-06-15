# PH50 · T04 — `reverse_query`: back-edge traversal + provisional tagging

| Field | Value |
|---|---|
| **Phase** | PH50 — Super-intelligence predicate + reverse_query |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/reverse_query.rs` (≤500) |
| **Depends on** | T01 (Cause type), PH42 (grounded recurrence — asymmetric causal back-edges), PH33 (Lodestar kernel — kernel-toward-antecedents traversal) |
| **Axioms** | A23, A2, A20 |
| **PRD** | `dbprdplans/21 §5` (epistemic symmetry Q↔A), `dbprdplans/21 §9` (`reverse_query` API) |

## Goal

Implement `reverse_query(vault, answer) -> Vec<Cause>` — the epistemic-symmetry
operation from *The Symmetry of Knowing* (A23). Given an answer/outcome, traverse the
grounded association/causal graph **backwards** (asymmetric back-edges + kernel-toward-
antecedents) to recover the likely questions/causes. Only grounded edges (backed by
recurrence) are traversed as `provisional: false`; ungrounded edges are labeled
`provisional: true` and included but flagged. Powers abductive reasoning ("what would
cause this outcome?") and grounding-gap discovery. The graph is navigable both ways
(`21 §5`): forward associations and asymmetric/causal back-edges.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn reverse_query(vault: &Vault, answer: &AnchorValue, domain: DomainId, clock: &dyn Clock) -> Result<Vec<Cause>, OracleError>`
- [ ] Find the constellation(s) that match `answer` in the domain (ANN search on outcome anchor, PH24)
- [ ] For each matching constellation, enumerate **asymmetric back-edges**: edges in the causal/recurrence graph where this outcome is the *effect* end (PH42 causal direction tracking); these represent candidate causes
- [ ] For each back-edge: retrieve the antecedent action/event; create `Cause { action_or_event, domain, confidence, provisional, provenance }`
- [ ] **Grounded back-edge:** backed by grounded recurrence (`21 §5`): `provisional = false`; confidence from recurrence cadence (same Bayesian posterior as T04 in PH49)
- [ ] **Ungrounded back-edge:** edge exists in the association graph but no grounded recurrence: `provisional = true`; confidence = structural similarity only (lower bound); included in result but clearly flagged
- [ ] **Kernel-toward-antecedents:** also query Lodestar (PH33) kernel traversal backward: from the matched constellation, walk toward antecedent kernel nodes; these are typically higher-confidence causes
- [ ] Sort results: grounded causes first (by descending confidence), then provisional causes
- [ ] Cycle detection via visited-set (same pattern as butterfly.rs); default depth limit `MAX_REVERSE_DEPTH = 3`
- [ ] Each returned `Cause` carries a `LedgerRef` (A15); write one Ledger entry per `reverse_query` call

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: vault with planted `cause_A → effect_B` grounded recurrence (15 co-occurrences); `reverse_query(effect_B)` returns a `Cause` with `action_or_event = "cause_A"` and `provisional = false`
- [ ] unit: vault with structural association A↔B but no recurrence; `reverse_query(B)` returns A with `provisional = true`
- [ ] unit: results sorted — all `provisional = false` causes appear before `provisional = true` causes
- [ ] proptest: if a cause is grounded (has recurrence backing), it is never labeled `provisional = true`
- [ ] edge (≥3): answer not found in vault → `Err(OracleError::DomainNotFound)`; multiple causes with equal confidence → stable sort (by `action_or_event` lexicographic as tiebreak); cycle in back-edges → terminates, no duplicate in result
- [ ] fail-closed: PH42 recurrence query failure → `Err(OracleError::NoRecurrence)` with code; ungrounded path never marked `provisional = false`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the `Vec<Cause>` JSON returned by `calyx readback reverse_query <answer_id> --domain <domain>`; `provisional` field on each `Cause`
- **Readback:**
  ```
  calyx readback reverse_query --answer <known_effect_id> --domain <domain>
  # First result: action_or_event = <planted_cause>, provisional = false
  calyx readback reverse_query --answer <ungrounded_effect_id> --domain <domain>
  # Results labeled: provisional = true
  ```
- **Prove:** planted cause recovers as first result with `provisional = false`; an ungrounded back-edge appears with `provisional = true`; no duplicate causes in the list

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH50 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
