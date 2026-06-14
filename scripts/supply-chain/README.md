# Supply-chain integrity tooling (`scripts/supply-chain/`)

Implements the **reproducible-builds** and **SBOM** halves of PRD `30 §2`
(Dependency / supply chain). The ZFS bit-rot half of `30 §1` lives under
`infra/aiwonder/` (`bin/verify-zfs-integrity.sh`, `ops/enable-zfs-scrub.sh`,
`ops/fsv-zfs-bitrot.sh`). Issue #596.

## Reproducible builds

| File | Purpose |
|---|---|
| `repro-build-env.sh` | **Source** it to set the canonical reproducible build env: pinned `SOURCE_DATE_EPOCH` (commit time), `CARGO_INCREMENTAL=0`, `LC_ALL=C`, `TZ=UTC`, and `--remap-path-prefix` for `$WS_ROOT` and `$CARGO_HOME`. |
| `verify-reproducible-build.sh [pkg]` | Builds `pkg` (default `calyx-core`) **twice** into separate target dirs and compares a sha256 manifest of every artifact. Exits `CALYX_REPRO_BUILD_MISMATCH` (10) on any difference. |

```bash
scripts/supply-chain/verify-reproducible-build.sh calyx-core
```

Why these knobs (see `repro-build-env.sh` header for the full rationale):
toolchain is already pinned by `rust-toolchain.toml`; `--locked` makes
`Cargo.lock` authoritative; the remaining non-determinism we control is
incremental compilation, embedded build paths, locale/timezone, and
compile-time timestamps.

The remap flags deliberately live in this script (applied at build time) and
**not** in `.cargo/config.toml`, which must stay free of absolute machine paths
(see that file's header).

## SBOM (CycloneDX 1.6)

| File | Purpose |
|---|---|
| `emit-sbom.py` | Self-contained (stdlib only) deterministic CycloneDX 1.6 emitter parsing `Cargo.lock`. No external `cargo-cyclonedx` dependency. |
| `emit-sbom.sh [out.json]` | Wrapper: deterministic timestamp, runs the emitter, then **independently re-reads** the SBOM and asserts component count == `Cargo.lock` `[[package]]` count (`CALYX_SBOM_COUNT_MISMATCH` / 4). Default output `dist/sbom/calyx.cdx.json`. |

```bash
scripts/supply-chain/emit-sbom.sh
```

The SBOM is itself reproducible: components are sorted, the BOM `timestamp` comes
from `SOURCE_DATE_EPOCH`, and `serialNumber` is a UUIDv5 of the component set, so
identical `Cargo.lock` → byte-identical SBOM.

## Error taxonomy

| Code | Exit | Meaning |
|---|---|---|
| `CALYX_REPRO_BUILD_MISMATCH` | 10 | artifacts differ across two builds |
| `CALYX_REPRO_BUILD_FAILED` | 11 | a `cargo build` failed |
| `CALYX_REPRO_NO_ARTIFACTS` | 12 | nothing to compare |
| `CALYX_SBOM_PARSE_ERROR` | 2 | `Cargo.lock` missing / invalid / package missing name+version |
| `CALYX_SBOM_EMPTY` | 3 | zero packages parsed |
| `CALYX_SBOM_COUNT_MISMATCH` | 4 | SBOM count != lockfile package count |

All paths fail closed — no silent fallback or partial artifact.
