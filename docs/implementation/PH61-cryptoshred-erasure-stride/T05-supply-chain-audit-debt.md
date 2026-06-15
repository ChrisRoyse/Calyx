# PH61 T05 Supply-Chain Audit Debt

Issue #710 tracks informational `cargo audit` unmaintained warnings observed
during PH61 supply-chain FSV. The gate must distinguish vulnerabilities from
maintenance debt without hiding either.

## 2026-06-13 Readback

Evidence root:
`/home/croyse/calyx/data/secure/fsv-issue710-audit-final-20260613T191924Z`

- `cargo audit --json --file Cargo.lock` exits 0 and reports
  `vulnerabilities.found=false`.
- The previous direct `bincode 2.0.1` warning (`RUSTSEC-2025-0141`) is removed
  by using the maintained `bincode_reloaded 3.1.6` package under the dependency
  name `bincode`; Aster and Ledger source imports remain unchanged.
- `cargo tree -i bincode@2.0.1` reports no matching package.
- One warning remains: `RUSTSEC-2024-0436` for `paste 1.0.15`.
- `tree-paste-normal-build.txt` proves `paste` is still present on normal/build
  paths through current upstream `candle-core 0.10.2`, `gemm 0.19.0`, and
  `tokenizers 0.22.2`.
- `pastey-patch-trial-*` records that `[patch.crates-io] paste = { package =
  "pastey", version = "0.2.3" }` is not a transparent replacement: Cargo treats
  it as a patch for package `pastey`, and rejects same-source patches.
- `latest-transitive-trial-*` records that trying current `tokenizers 0.23.1`
  and `fastembed 5.16.1` still leaves `paste 1.0.15` on normal/build paths,
  including the `candle-core 0.10.2` -> `gemm 0.19.0` path.

## Constraint

`candle-core 0.10.2` and `gemm 0.19.0` are the current crates.io releases at the
time of readback. The advisory alternatives found during FSV are `pastey 0.2.3`
and `with_builtin_macros 0.1.0`, but they are different package names, so they
are not safe transparent transitive patches for dependencies that import
`paste` directly.

Do not suppress `RUSTSEC-2024-0436` silently. Re-evaluate when Candle, gemm,
tokenizers, or their direct macro dependencies publish a release that removes
`paste`, or when Calyx replaces those upstream dependencies.
