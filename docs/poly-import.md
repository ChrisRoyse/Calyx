# Poly import note

This checkout imports the useful Calyx-facing pieces from
`ChrisRoyse/Poly`, source checkout `C:\code\poly`, commit `2cac886`.

Imported surface:

- `crates/calyx-poly`: the Poly forecasting/operations crate, wired into the
  Calyx workspace.
- `calyx-assay` estimators and FSV coverage for rank/categorical association,
  distance correlation, HSIC, MIC, Granger, copula tail dependence,
  cross-correlation, conditional MI, CCM, Hawkes, point-process
  co-intensity, PC-stable causal discovery, partial correlation, and partial
  networks.
- Ledger improvements: typed `EntryKind` values for grounding, admission,
  agent forecast, policy, and score entries, plus stable `CALYX_*` error-code
  redaction support for payload `code` fields.
- Aster vault helper for anchoring multiple grounding entries against ledger
  sequence numbers.
- Poly operator runbooks:
  `docs/poly-single-workstation-runbook.md` and
  `docs/poly-daily-jobs-runbook.md`.

The import intentionally keeps Calyx-Dev's newer assay ensemble
leave-one-out baseline behavior instead of overwriting it with the older Poly
copy.

Verification run during import:

- `cargo check -p calyx-poly`
- `cargo test -p calyx-poly`
- `cargo test -p calyx-assay -p calyx-core -p calyx-ledger -p calyx-aster`
