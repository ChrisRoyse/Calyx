# Calyx Association-Native Build Doctrine

This doctrine applies to Calyx project work in any domain. It is the operating
translation of the builder handbook into engineering rules for issues, docs,
code, and FSV.

## Foundational Rule

Build from first principles:

1. Decompose the project into atomic, independently measurable units.
2. Store those atoms and their provenance in Calyx.
3. Compute every base association that the current scope makes possible.
4. Differentiate associations against grounded outcomes in bits.
5. Distill the minimal kernel.
6. Compose search, guard, prediction, imputation, mining, and reporting from
   that kernel.

Records are not the intelligence. The association structure among measured
atoms is the intelligence substrate. A feature, field, molecule, protein, DNA
sequence, paper, trial, drug, disease, endpoint, safety signal, and outcome is
useful only after it is represented as a Calyx atom with provenance and linked
by typed associations that can be read back.

The binding method is:

```text
atoms -> all base associations -> differentiate -> kernel -> compose
```

Do not skip any step. If a run cannot cover all atoms or all base associations
in a scope, the run must narrow and name the scope instead of implying complete
coverage. "All associations" means all base relationships the declared scope
and current Calyx capabilities make measurable, with deficits routed to new
data-acquisition or capability issues.

## Modeling Rules

- Use Calyx as the source of truth. Do not create a side store for data that
  Calyx must reason over; add the missing Calyx capability instead.
- Embed latent content: free text, images, audio, video, code, scientific
  sequences, descriptions, and generated summaries.
- Encode explicit values: numbers, categories, booleans, ordinals, timestamps,
  identifiers, relationships, and measurements.
- For hybrid records, do both. Keep exact structured encoders and semantic
  embedding slots separate in the same constellation.
- Never flatten a panel. Slots stay typed and independent through search,
  guard, bits, kernel, and report surfaces.
- Every derived artifact must preserve enough source ids, hashes, and row
  counts to trace it back to the physical Calyx rows or source bytes.

Structured values are first-class intelligence inputs. Numbers, categories,
booleans, ordinals, timestamps, identifiers, references, relations, graph
position, recurrence, and measured outcomes must be represented with frozen
deterministic encoders when they are explicit. Text labels, descriptions,
notes, abstracts, and generated row summaries should be embedded as additional
slots when their meaning is latent. Hybrid records should carry both families
in the same constellation, still no-flatten.

Every missing data class is an engineering task, not a reason to infer around
the gap. If a biomedical claim needs trials, safety, mechanisms, perturbation
signatures, targets, variants, disease ontology, outcome labels, or negative
controls, acquire those source bytes, ingest them into Calyx with provenance,
and compute their associations with the rest of the substrate.

## Association Rules

- Count all base associations within the bounded scope, not only convenient
  or hand-picked pairs.
- Make scope explicit. If a run is bounded, partial, sampled, or incomplete,
  the artifact must say so and must not be described as full-corpus proof.
- Associations are ranked evidence and hypotheses until differentiated against
  grounded outcomes and validated by the required gates.
- Do not claim derived `C(N,2)` signal past the data-processing ceiling of the
  measured panel/outcome information.
- Refusals, gaps, and deficits are findings. Persist them and route them to
  the next issue or lens/data acquisition task.
- Use typed association surfaces. A drug-disease co-mention, drug-target edge,
  target-disease validation, transcriptomic reversal, adverse-event signal,
  clinical-trial intervention-condition row, and literature citation are
  different evidence instruments and must not be collapsed into one generic
  "association" without preserving type and provenance.
- Build composition from the kernel. Search, answer, guard, hypothesis mining,
  imputation, consequence prediction, atlas publishing, and lowered artifacts
  must name the kernel or graph generation they were derived from.

## Grounding Rules

- Grounding means a real anchored outcome or power-proven instrument, not a
  plausible association.
- Bits, sufficiency, known-positive/negative, time-split, safety, and
  counter-evidence gates are required before promoting a biomedical association
  beyond hypothesis status.
- A clinical, treatment, cure, or actionability claim requires all applicable
  validation, safety, and counter-evidence gates to pass with physical
  readback. Association-only inference can prioritize leads; it cannot by
  itself become a clinical recommendation.
- Missing data does not justify a fallback. Acquire the data, add the Calyx
  storage/ingest/readback capability, or fail closed with a structured error.

Biomedical claim ladder:

1. Association-only output is a ranked research lead.
2. Evidence-backed output is a provenance-backed hypothesis.
3. Falsification-retained output is a stronger hypothesis that survived the
   available counter-evidence gates.
4. Mechanism/safety/outcome-validated output may become a preclinical or
   clinical-review candidate.
5. Clinical actionability or cure language requires the relevant clinical,
   safety, regulatory, and human-review gates. Calyx must refuse that claim
   until the source-of-truth artifacts prove those gates.

## Execution Rules

- GitHub issues are live state. Claim, update, split, and close issues with
  artifact-backed evidence.
- Every task must produce or update a durable source of truth: Calyx rows,
  ledger entries, graph lifecycle rows, report files, docs, or issues.
- Full State Verification reads the persisted source of truth after the write.
  Command success, logs, and tests are supporting evidence, not proof.
- If a prerequisite is missing, create the atomic issue and wire it into the
  dependency order before doing downstream mining.
- Prefer root-cause Calyx capability work over one-off scripts. Scripts are
  acceptable as bounded FSV/prototype tools only when their outputs are
  imported into Calyx or tracked as source artifacts.
- Keep the issue tree atomic. If completing one issue reveals missing source
  data, a missing Calyx storage/readback primitive, a missing gate, or a
  missing validator, file or update the specific issue before downstream
  claims depend on it.
- For substantial discovery runs, persist a machine-readable result pack,
  human-readable findings doc, and readback summary. The result pack is the
  substrate for future mining; the doc is for operators; the readback summary
  is the proof layer.

## Biomedical Discovery Implication

For the #867 biomedical program, the required shape is:

1. Evidence atoms: diseases, drugs, targets, genes, variants, pathways,
   phenotypes, trials, transcriptomic signatures, outcomes, safety signals,
   assays, literature claims, molecules, proteins, and DNA.
2. Calyx storage: every atom, source row, outcome, and validation signal belongs
   in Calyx with provenance and graph lifecycle state.
3. Association substrate: typed edges and collection-local CSR/readback over
   accepted graph generations.
4. Differentiation: known-positive/negative, time-split, bits, sufficiency,
   reversal, target/outcome, safety, and counter-evidence gates.
5. Composition: all-pair typed miners, domain hunts, synergy miners, human
   review atlas, and Oracle/kernel prediction surfaces.

The goal is maximum grounded healing intelligence: find, rank, falsify, and
explain human-healing hypotheses. The system must not skip the gates needed to
turn a hypothesis into a stronger claim.

## Default Constants To Preserve Unless Recalibrated

- Association yield per input: `N + C(N,2) + 1`.
- Bit floor: about `0.05` bits.
- Correlation ceiling: about `0.6`.
- Minimum paired samples for MI/calibration: about `50`, unless a small-sample
  posterior or explicit provisional status is used.
- Sufficiency: `I(panel; anchor) >= H(anchor)`.
- Kernel recall target: about `0.95`.
- Reciprocal-rank fusion default: `K = 60`.

Changing these constants is allowed only when the new value is calibrated,
documented, and verified against persisted artifacts.
