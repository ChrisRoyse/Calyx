# #1269 Target-Match Fallacy Mechanistic Direction Gate

## Scope

#1269 fixed the target-match fallacy in the biomedical hypothesis engine. The
bug class was mechanistic direction loss: the pipeline could treat a
target-disease match as supportive without first proving whether disease biology
requires target inhibition, activation, or replacement/restoration, and without
distinguishing loss-of-function, gain-of-function, dosage-loss, and dosage-gain
evidence.

This is an engine correctness gate. It does not create a treatment claim,
efficacy claim, safety claim, dosing claim, recommendation, actionability
claim, pair-interaction proof, or cure evidence.

## Implementation

Primary module:

```text
crates/calyx-cli/src/cmd/mechanistic_direction.rs
```

Pipeline surfaces updated:

- `association-validation-gates` now admits Open Targets target-disease rows as
  positive benchmark evidence only when source-backed direction-on-target and
  direction-on-trait imply a required target modulation.
- `typed-association-miner` preserves mechanism-sensitive orientation and
  blocks gene-disease or drug-target candidates whose mechanism direction is
  missing, unrecognized, or internally conflicting.
- `hypothesis-falsification-sweep` treats required-modulation and observed-drug
  action conflicts as counter-evidence instead of support.
- Persisted JSON/JSONL reports now expose mechanistic direction counts,
  blocked rows/candidates, reason codes, inferred required target modulation,
  observed action modulation, mutation consequence, and source fields.

Mechanistic contract:

| Disease mechanism | Trait effect | Required target modulation |
|---|---|---|
| Gain of function / dosage gain | Risk | Inhibit |
| Gain of function / dosage gain | Protective | Activate |
| Loss of function / dosage loss | Risk | Replace or restore |
| Loss of function / dosage loss | Protective | Inhibit |

Drug-target action vocabulary is normalized from ChEMBL/DGIdb-style fields such
as `action_type`, `interactionTypes`, `directionality`, `moa`, and
`mechanism_of_action`. Ambiguous trait wording such as a bare `association`
does not imply risk or protection; it is blocked with an explicit reason code.

## Source Research

The implementation follows source-backed direction models:

- Open Targets direction-of-effect evidence combines direction on target and
  direction on trait.
- ChEMBL action types distinguish activating/agonist/positive-modulator actions
  from inhibiting/antagonist/blocker actions.
- DGIdb interaction directionality groups interactions as activating or
  inhibiting by mechanism.
- ClinGen dosage sensitivity distinguishes haploinsufficiency and
  triplosensitivity evidence.

## Real FSV

Final FSV root:

```text
C:\code\Calyx-Dev\target\fsv\target_match_fallacy_final_20260707_110320
```

Manual FSV log:

```text
C:\code\Calyx-Dev\target\fsv\target_match_fallacy_final_20260707_110320\manual_fsv_log.txt
```

Source of truth:

```text
CLI persisted JSON/JSONL artifacts under the final FSV root. Verification used
separate file reads after command execution, not command return values alone.
```

Live source readbacks used by the final FSV:

| Source | Readback |
|---|---|
| Open Targets | `directionOnTarget=GoF`, `directionOnTrait=risk`, `score=1` |
| ChEMBL | `action_type=INHIBITOR`, `mechanism=TNF-alpha inhibitor` |

Primary readbacks:

| Artifact | Assertion |
|---|---|
| `out_validation/association_validation_report.json` | `gate_passed=true`, `blocked_direction_rows=0`, `inferred_required_direction_rows=1` |
| `out_miner_gene_disease/typed_association_miner_report.json` | one BRAF/cardiofaciocutaneous syndrome hypothesis; `required_target_modulation=inhibit`; `mutation_consequence=gain_of_function` |
| `out_miner_drug_target/typed_association_miner_report.json` | one adalimumab/TNF hypothesis; `observed_target_modulation=inhibit` |
| `out_falsification/falsification_sweep_report.json` | `support_evidence_count=2`, `counter_evidence_count=0` |

Boundary and edge-case readbacks:

| Case | Expected outcome | Persisted proof |
|---|---|---|
| Missing Open Targets direction | fail closed | command exit `2`; `mechanistic_direction_blocked_rows.jsonl` has `CALYX_MECH_TARGET_CONSEQUENCE_MISSING` |
| Invalid miner direction | fail closed | command exit `2`; `blocked_candidates.jsonl` records missing/unrecognized direction reasons |
| Falsification direction conflict | counter-evidence | `counter_evidence.jsonl` has `mechanistic_required_direction_conflict` |

Synapse readback also inspected the final FSV tree and confirmed:

```text
gate=True blocked=0 inferred=1
```

## Verification Commands

```text
cargo build -p calyx-cli
cargo check -p calyx-cli
cargo clippy -p calyx-cli --all-targets -- -D warnings
cargo test -p calyx-cli mechanistic_direction -- --nocapture
cargo test -p calyx-cli association_validation -- --nocapture
cargo test -p calyx-cli typed_association_miner -- --nocapture
cargo test -p calyx-cli hypothesis_falsification -- --nocapture
git diff --check
```

All passed on the local authoring checkout before commit.

## GitHub State

Closed issues:

- #1269 epic: source-backed mechanistic direction gates.
- #1270 schema and report contract.
- #1271 Open Targets direction-of-effect gate.
- #1272 drug-target action normalization.
- #1273 mutation consequence and dosage mechanism screen.
- #1274 direction-aware typed mining and no unsafe reversal.
- #1275 falsification conflict gate.
- #1276 final FSV.

## Findings

- The root cause was not a single bad comparison; it was missing mechanistic
  direction as a required contract between validation, mining, and
  falsification.
- The fix makes mechanistic direction part of the persisted hypothesis surface,
  so later stages cannot silently reinterpret a target match as support.
- Unknown, unrecognized, ambiguous, or conflicting direction now creates a
  durable blocked-row artifact or counter-evidence artifact with reason codes.
- The system remains fail-closed: there are no permissive fallbacks for missing
  target mechanism or drug action direction.

## Conclusion

#1269 is complete. Target-disease and drug-target evidence now has to carry
source-backed mechanistic direction before it can support a hypothesis, and
direction conflicts are surfaced as falsification evidence.
