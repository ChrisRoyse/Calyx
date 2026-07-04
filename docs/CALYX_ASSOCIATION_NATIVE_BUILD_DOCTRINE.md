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
