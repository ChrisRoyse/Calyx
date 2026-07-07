# #1277 Biomedical Blindspot Audit Gates

## Scope

#1277 through #1283 add a machine-readable blindspot audit after hypothesis
generation, mechanistic direction gating (#1269), and falsification. This stage
does not claim efficacy, safety, clinical actionability, treatment guidance,
dosing guidance, recommendation, pair-interaction proof, or cure evidence.

The audit closes the report-section blindspots that were still mostly prose:

- germline-versus-somatic context and synthetic-lethality inversion;
- drug lifecycle/viability for discontinued, withdrawn, failed, unavailable, or
  discredited molecules;
- external literature novelty and correlation to internal `novelty_score`;
- repeated-run/corpus/seed stability;
- benchmark-export readiness for Open Targets/Hetionet-style comparison;
- transcriptomic-reversal specificity for LINCS/CMap-style signatures.

## Implementation

Primary module:

```text
crates/calyx-cli/src/cmd/biomedical_blindspot_audit.rs
```

CLI:

```text
calyx biomedical-blindspot-audit \
  --hypotheses-report <typed/hunt report json> \
  --literature-audit <jsonl> \
  --stability-audit <jsonl> \
  --drug-lifecycle <jsonl> \
  --transcriptomic-audit <jsonl> \
  --out-dir <dir>
```

The command requires every source file. Missing files, malformed JSONL, or
source rows missing required identifiers fail closed with a structured
`CALYX_CLI_*` error. Biomedical failures do not disappear into stderr; they are
persisted as blocked or pending hypothesis rows with explicit reason codes.

Persisted outputs:

- `biomedical_blindspot_audit_report.json`
- `audited_hypotheses.jsonl`
- `ready_hypotheses.jsonl`
- `blocked_hypotheses.jsonl`
- `benchmark_export.jsonl`
- `metrics.json`

Representative reason codes:

| Code | Meaning |
|---|---|
| `CALYX_BLINDSPOT_GERMLINE_SYNTHETIC_LETHALITY_RISK` | germline/constitutional disease plus synthetic-lethal cell-killing rationale |
| `CALYX_BLINDSPOT_DRUG_NOT_VIABLE` | drug lifecycle source says discontinued, withdrawn, terminated, suspended, failed, discredited, fraud-tainted, unavailable, or revoked |
| `CALYX_BLINDSPOT_LITERATURE_AUDIT_MISSING` | no external literature audit row matched the candidate |
| `CALYX_BLINDSPOT_PATIENT_CONTEXT_MISSING` | drug-disease candidate lacks patient/disease/variant-origin context |
| `CALYX_BLINDSPOT_REPRODUCIBILITY_LOW` | repeated-run frequency is below the configured stability threshold |
| `CALYX_BLINDSPOT_TRANSCRIPTOMIC_LOW_SPECIFICITY` | transcriptomic reversal is generic mechanism-class signal rather than a specific reproducible signature |
| `CALYX_BLINDSPOT_TRANSCRIPTOMIC_NOT_REPRODUCIBLE_GOLD` | transcriptomic reversal lacks gold/reproducible/self-connected evidence |
| `CALYX_BLINDSPOT_BENCHMARK_FIELDS_MISSING` | row cannot be exported with at least disease plus target or drug fields |

## Source Research

The contract follows source-backed fields rather than free-text optimism:

- Open Targets direction-of-effect evidence separates direction on target and
  direction on trait.
- ChEMBL action and lifecycle fields distinguish positive/negative modulation,
  approved phase, and withdrawn/warning status.
- ClinGen dosage sensitivity separates haploinsufficiency and
  triplosensitivity mechanisms.
- PubMed/NCBI and Europe PMC are appropriate external literature-count sources
  for novelty audits, subject to their API/rate-limit contracts.
- LINCS/iLINCS only treats reproducible, self-connected (`gold`) signatures as
  strong transcriptomic evidence; generic mechanism-class reversals are weak
  priors.

## Real FSV

Final FSV root:

```text
C:\code\Calyx-Dev\target\fsv\biomedical_blindspot_audit_final_20260707_180115
```

Manual FSV log:

```text
C:\code\Calyx-Dev\target\fsv\biomedical_blindspot_audit_final_20260707_180115\manual_fsv_log.txt
```

Source of truth:

```text
CLI persisted JSON/JSONL artifacts under each case directory. Verification
used separate file reads after command execution, not command return values.
The historical 726-row atlas JSONL was not present in this checkout; the docs
reference a prior `/home/croyse/...` vault path, so full 726-row
reclassification was not rerun here.
```

Happy-path persisted readback:

| Row | Status | Reason codes |
|---|---|---|
| `braf-trametinib-cfc` | `ready_for_human_review_after_blindspot_audit` | none |
| `olaparib-fanconi-brca2` | `blocked_by_blindspot_audit` | `CALYX_BLINDSPOT_GERMLINE_SYNTHETIC_LETHALITY_RISK` |
| `tarextumab-cadasil-notch3` | `blocked_by_blindspot_audit` | `CALYX_BLINDSPOT_DRUG_NOT_VIABLE` |
| `generic-hdac-reversal` | `blocked_by_blindspot_audit` | `CALYX_BLINDSPOT_TRANSCRIPTOMIC_LOW_SPECIFICITY`, `CALYX_BLINDSPOT_TRANSCRIPTOMIC_NOT_REPRODUCIBLE_GOLD` |

Happy-path counts:

```text
audited=4
ready=1
blocked=3
pending=0
benchmark_export_rows=4
```

Boundary and edge-case readbacks:

| Case | Expected outcome | Persisted proof |
|---|---|---|
| Missing patient/literature/stability context | pending, not ready | `blocked_hypotheses.jsonl` row has `pending_blindspot_evidence` plus `CALYX_BLINDSPOT_PATIENT_CONTEXT_MISSING`, `CALYX_BLINDSPOT_LITERATURE_AUDIT_MISSING`, `CALYX_BLINDSPOT_STABILITY_AUDIT_MISSING` |
| Low repeated-run stability | blocked | `blocked_hypotheses.jsonl` row has `CALYX_BLINDSPOT_REPRODUCIBILITY_LOW` and stability `0.333333333333333` |
| Malformed lifecycle source row | fail closed | command exit `2`, no report created, stderr JSON says `drug_lifecycle line 1 missing drug_name/name` |

Synapse readback independently reopened the final FSV tree and confirmed:

```text
audited=4
ready=1
blocked=3
pending=0
ready_ids=braf-trametinib-cfc
blocked_ids=generic-hdac-reversal,olaparib-fanconi-brca2,tarextumab-cadasil-notch3
```

## Verification Commands

```text
cargo fmt -p calyx-cli -- --check
cargo check -p calyx-cli
cargo test -p calyx-cli biomedical_blindspot -- --nocapture
cargo clippy -p calyx-cli --all-targets -- -D warnings
```

Manual FSV used the compiled `target\debug\calyx.exe` and then read back the
JSON/JSONL artifacts listed above.

## GitHub State

Issues completed by this entry:

- #1277 epic: biomedical blindspot hardening.
- #1278 germline-versus-somatic and synthetic-lethality context gate.
- #1279 drug viability and lifecycle gate.
- #1280 external literature novelty audit and novelty-score calibration metrics.
- #1281 reproducibility/stability and benchmark export audit surfaces.
- #1282 transcriptomic reversal specificity gate.
- #1283 final FSV and documentation.
- #1284 workspace dependency inheritance fix discovered during verification.

## Findings

- The root cause was that several skeptical-reviewer checks existed only in the
  paper/rejection prose, not as a required persisted contract.
- The fix adds a fail-closed audit surface after generation: missing source
  context becomes `pending_blindspot_evidence`; dangerous or contradicted
  context becomes `blocked_by_blindspot_audit`; only rows clearing every audit
  become `ready_for_human_review_after_blindspot_audit`.
- The audit also prevents generic class labels from being treated as concrete
  drug entities when an explicit drug name is present, avoiding spurious
  lifecycle misses on transcriptomic mechanism-class rows.
