# Association-Native Doctrine and Context Update

Issues: #863, #860, #867, #1214

Status: complete documentation/context update.

The builder handbook has been folded into the repo-level doctrine at
`docs/CALYX_ASSOCIATION_NATIVE_BUILD_DOCTRINE.md`.

## Added Doctrine

- The binding method is `atoms -> all base associations -> differentiate ->
  kernel -> compose`.
- Bounded runs must declare their scope and must not imply full coverage.
- Explicit structured values use deterministic encoders; latent content uses
  embedders; hybrid records carry both in no-flatten constellations.
- Missing data classes create data-acquisition or Calyx-capability tasks.
- Association evidence remains typed: co-mention, drug-target, target-disease,
  reversal, safety, trial, literature, and other instruments stay distinct.
- Composition surfaces must name the kernel or graph generation they derive
  from.
- Biomedical outputs follow a claim ladder from research lead to clinical
  actionability; association-only inference is not enough for a cure or
  clinical recommendation.
- Substantial discovery runs need a result pack, findings doc, and readback
  summary.

## FSV

Documentation readback:

```text
Get-Content docs/CALYX_ASSOCIATION_NATIVE_BUILD_DOCTRINE.md
Get-Content docs/medicalsearch/65_association_native_doctrine_context_update.md
```

Repo hygiene gates:

```text
cargo fmt --check
bash scripts/linecount.sh
git diff --check
```

## Boundary

This update strengthens operating doctrine and issue context. It does not claim
that any biomedical association is a cure, treatment, or clinical actionability
result.
