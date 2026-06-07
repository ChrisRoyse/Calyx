# PH61 · T05 — `supply_chain.rs`: SBOM + `cargo audit` + content-addressed lens weight check

| Field | Value |
|---|---|
| **Phase** | PH61 — Crypto-shred erasure + STRIDE FSV + secret-scan |
| **Stage** | S14 — Security & Privacy by Construction |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/supply_chain.rs` (≤500) |
| **Depends on** | — (standalone; depends on PH18 `LensId` content-addressing) |
| **Axioms** | A33, A16 |
| **PRD** | `dbprdplans/30 §2` (Dependency / supply chain axis — pin crate versions + `cargo audit`, content-addressed lens weights, SBOM) |

## Goal

Implement the supply-chain integrity layer: (1) an SBOM manifest generator that
records every crate name + version + checksum locked in `Cargo.lock`; (2) a
`run_cargo_audit()` function that invokes `cargo audit` locally and returns a
structured result; (3) a lens-weight integrity check that verifies a loaded
`LensId`'s weights match the content-addressed hash stored at registration time,
detecting any silent model swap. Together these implement the `Dependency / supply
chain` row of the STRIDE hardening axes (`30 §2`).

## Build (checklist of concrete, code-level steps)

- [ ] `struct SbomEntry { crate_name: String, version: String, checksum: Option<String>, source: String }` —
  parsed from `Cargo.lock`; `serde`.
- [ ] `struct Sbom { generated_at: Timestamp, entries: Vec<SbomEntry> }` — `serde`.
- [ ] `fn generate_sbom(cargo_lock_path: &Path, now: Timestamp) -> Result<Sbom>` —
  parses `Cargo.lock` (TOML) and extracts all `[[package]]` entries; returns
  `CALYX_SBOM_PARSE_ERROR` if `Cargo.lock` cannot be parsed; never panics on
  missing fields (skip entries with parse errors, log a warning).
- [ ] `enum CargoAuditResult { Clean, Vulnerabilities(Vec<AuditVulnEntry>), ToolNotFound, ParseError(String) }`.
- [ ] `struct AuditVulnEntry { package: String, version: String, advisory_id: String, description: String }`.
- [ ] `fn run_cargo_audit(cargo_lock_path: &Path) -> CargoAuditResult` — invokes
  `cargo audit --json --file <path>` via `std::process::Command`; parses JSON
  stdout; if `cargo audit` is not found returns `ToolNotFound` (not a panic);
  if exit 0 → `Clean`; if exit 1 with JSON vulnerabilities → `Vulnerabilities(...)`.
- [ ] `fn assert_audit_clean(result: &CargoAuditResult) -> Result<()>` — returns
  `Ok(())` for `Clean` and `ToolNotFound` (dev machine without tool is not a hard
  failure); returns `CALYX_SUPPLY_CHAIN_VULN` for `Vulnerabilities`; returns
  `Err` for `ParseError`.
- [ ] `fn verify_lens_weight_hash(lens_id: &LensId, weights_path: &Path) -> Result<()>` —
  reads the weights file, computes `blake3` of its contents, compares to the
  `LensId`'s content-address component; returns `CALYX_LENS_WEIGHT_TAMPERED` if
  they differ (a swapped model = a new `LensId`, detectable — `30 §2`).
- [ ] Add `CALYX_SBOM_PARSE_ERROR`, `CALYX_SUPPLY_CHAIN_VULN`, `CALYX_LENS_WEIGHT_TAMPERED`
  to `calyx-core/src/error.rs`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `generate_sbom` on a synthetic minimal `Cargo.lock` string (3 packages) →
  `sbom.entries.len() == 3`; entry fields match the hard-coded expected values.
- [ ] unit: `verify_lens_weight_hash` with a temp file whose `blake3` matches a
  synthetic `LensId`'s content address → `Ok(())`; flip one byte → `CALYX_LENS_WEIGHT_TAMPERED`.
- [ ] unit: `assert_audit_clean(Clean)` → `Ok(())`; `assert_audit_clean(ToolNotFound)` → `Ok(())`
  (tool absence is not a block); `assert_audit_clean(Vulnerabilities([one_vuln]))` →
  `CALYX_SUPPLY_CHAIN_VULN`.
- [ ] unit: `run_cargo_audit` on a machine without `cargo audit` installed → returns
  `ToolNotFound`, never panics.
- [ ] edge (≥3): `Cargo.lock` with a package missing the `checksum` field → entry
  still included with `checksum: None` (not skipped entirely); empty `Cargo.lock`
  → `sbom.entries.len() == 0`; weights file does not exist → `CALYX_LENS_WEIGHT_TAMPERED`
  (not `io::Error` panic — fail closed, A16).
- [ ] fail-closed: `verify_lens_weight_hash` returns the structured error on any
  hash mismatch; never silently loads a tampered model.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** actual `Cargo.lock` in `CALYX_HOME` on aiwonder; actual `cargo audit`
  tool installed on aiwonder.
- **Readback:**
  1. `cargo test -p calyx-aster supply_chain -- --nocapture 2>&1` prints SBOM entry
     count for the synthetic lock file and the `CALYX_LENS_WEIGHT_TAMPERED` output
     for the flipped-byte case.
  2. On aiwonder: `cargo audit 2>&1` run from `CALYX_HOME`; confirm exit 0 (clean)
     or list known advisories. `assert_audit_clean` result printed.
  3. `verify_lens_weight_hash` run against the actual TEI model weights on aiwonder
     (`:8088` resident model file); confirm `Ok(())`.
- **Prove:** before: no supply-chain checks; after: SBOM generated with correct entry
  count; `cargo audit` result is structured (not raw stderr); tampered weights
  detected before any use of the model.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH61 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
