# PH59 · T06 — Hazards 22–25: secret leakage, nondeterminism, whole-host loss, upgrade skew

| Field | Value |
|---|---|
| **Phase** | PH59 — 25-hazard register FSV + soak |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-hazard-soak` |
| **Files** | `crates/calyx-hazard-soak/src/hazards/security.rs` (≤500) |
| **Depends on** | PH35/PH36 (Ledger hash-chain — hazard 22), PH66 (restic DR — hazard 24), PH18 (frozen lens — hazard 25) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §7` hazards 22–25 |

## Goal

Drive hazards 22 (secret leakage / candidate-text persistence), 23 (numerical nondeterminism
in replay / determinism mode), 24 (whole-host loss — restic DR drill), and 25 (upgrade/format
skew — old shards readable after format version bump), read the SoT bytes, prove each
mitigation. Hazard 22 uses a synthetic secret token injected into a reranker request; the
FSV is a byte scan of all persisted files. Hazard 23 uses `CALYX_DETERMINISM=1` mode. Hazard
24 coordinates with PH66 (restic restore); stub if PH66 is not yet complete.

## Build (checklist of concrete, code-level steps)

**Hazard 22 — Secret leakage:**
- [ ] `fn probe_h22_secret_leakage(vault: &mut Vault) -> HazardResult`:
  - Inject a synthetic secret token `CALYX_TEST_SECRET_ABCD1234` as candidate reranker text in a request
  - Process the request; let it complete
  - Scan all persisted bytes: WAL segments, SST files, Ledger CF, manifest — using `grep -r --include='*.sst' --include='*.wal' --include='*.manifest'` equivalent in Rust (`walkdir` + `memchr`)
  - Verify: the secret token does NOT appear in any persisted file (Ledger stores hashes; reranker text is request-scoped)
  - Verify: the Ledger entry for this request contains only a hash of the content (not the content itself)
  - Record `secret_scan_violations == 0`

**Hazard 23 — Numerical nondeterminism:**
- [ ] `fn probe_h23_nondeterminism(forge: &Forge, vault: &Vault) -> HazardResult`:
  - Set `CALYX_DETERMINISM=1`
  - Run a deterministic query (seeded RNG, fixed reduction order, no nondeterministic atomics)
  - Record the result (embedding output + search results)
  - Run the identical query again with the same seed
  - Verify: results are bit-parity identical within ≤ 1e-3 tolerance (DOCTRINE §0 standard)
  - Record `determinism_replay_max_delta`; assert ≤ 1e-3

**Hazard 24 — Whole-host loss:**
- [ ] `fn probe_h24_whole_host_loss(vault: &Vault) -> HazardResult`:
  - Create a restic snapshot of `$CALYX_HOME` (requires PH66 restic infra; if absent, mark as `skipped_pending_ph66`)
  - Restore to a temp directory `/tmp/calyx_dr_restore_test`
  - Open the restored vault; verify all constellations and anchors readable byte-exact
  - Verify Ledger hash-chain intact (`verify_chain()` passes)
  - Record `dr_restore_verified == true` (or `skipped_pending_ph66`)
  - Clean up `/tmp/calyx_dr_restore_test`

**Hazard 25 — Upgrade / format skew:**
- [ ] `fn probe_h25_upgrade_skew(vault_path: &Path) -> HazardResult`:
  - Open a vault written by a previous format version (use a golden fixture file with `format_version=1`)
  - Verify all CxIds readable (old shards readable by current code)
  - Write a new CxId; verify it is written in the new format version
  - Attempt to open with `format_version=99` (unknown major) → verify `CALYX_FORMAT_VERSION_UNSUPPORTED` returned (refuse unknown-major, fail closed)
  - Record `old_shards_readable == true`; `unknown_major_rejected == true`

- [ ] Aggregate into `target/ph59_hazards_22_25.json`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] H22: `secret_scan_violations == 0`; `grep -r CALYX_TEST_SECRET_ABCD1234 /hotpool/vault_test/` returns no output (read via `std::process::Command` in the test)
- [ ] H23: `determinism_replay_max_delta ≤ 1e-3`; result bytes identical on two runs with same seed (assert via `assert_eq!` on the result vector after rounding to `≤1e-3` tolerance)
- [ ] H24: if restic available — all CxIds readable post-restore; Ledger chain intact; else `skipped_pending_ph66` recorded in JSON (not a failure)
- [ ] H25: `old_shards_readable == true`; `unknown_major_rejected == true`; `CALYX_FORMAT_VERSION_UNSUPPORTED` returned for `format_version=99`
- [ ] edge: H22 — inject secret into 3 different request types (rerank, embed, search); verify none persist in any of the 3 code paths
- [ ] edge: H23 — with `CALYX_DETERMINISM=0` (nondeterministic mode), results may differ; verify the flag is respected (nondeterministic mode does not claim determinism)
- [ ] fail-closed: H25 — `format_version=99` → `CALYX_FORMAT_VERSION_UNSUPPORTED` (not a silent reinterpret of bytes as a different format)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `target/ph59_hazards_22_25.json`; output of `grep -r CALYX_TEST_SECRET_ABCD1234 /hotpool/vault_test/`; `calyx readback --chain-verify` on the restored vault (H24)
- **Readback:**
  ```
  grep -r 'CALYX_TEST_SECRET_ABCD1234' /hotpool/vault_test/ || echo "CLEAN: no secret found"
  calyx readback --chain-verify /tmp/calyx_dr_restore_test/vault_test   # H24
  cat target/ph59_hazards_22_25.json | python3 -c "import json,sys; d=json.load(sys.stdin); print('passed:', all(h['passed'] or h.get('skipped') for h in d))"
  ```
- **Prove:** `grep` returns nothing (CLEAN); `chain-verify` passes on the restored vault; all four hazards report `passed: true` or `skipped_pending_ph66: true`. Attach JSON + grep output + chain-verify output to PH59 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH59 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
