# PH71 · T04 — Byte-exact `.db` round-trip migration verifier (V1 gate)

| Field | Value |
|---|---|
| **Phase** | PH71 — V0 shadow → V1 flip → V2 calyx-only |
| **Stage** | S19 — Leapable Vault Swap |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/leapable/round_trip_verifier.rs` (≤500) |
| **Depends on** | T03 (read-flip + panel/guard enabled), PH64 (migration tool) |
| **Axioms** | A18, A15, A16 |
| **PRD** | `dbprdplans/15 §5 V1`, `15 §4` |

## Goal

Prove the **V1 FSV gate**: `calyx migrate vault <real.db> <vault.calyx>` round-trips
a real `.db` byte-exact on content (every chunk's `chunk_id`, `database_name`, and
`text_hash` matches the source SQLite rows verbatim). Additionally prove the A/B
recall win (Calyx recall ≥ `sqlite-vec` recall) and no latency regression on the
same real Vault. This card produces the `RoundTripVerifier` that is the mechanical
gate: it is not a harness (a harness would be a fake test — it must run against a
real `.db` on aiwonder, FSV only).

## Build (checklist of concrete, code-level steps)

- [ ] `RoundTripVerifier::verify(sqlite_path: &Path, calyx_dir: &Path) ->
      Result<VerifyReport, CalyxError>`: opens both the source SQLite and the
      migrated Calyx Vault; for every SQLite chunk row, reads the corresponding
      Calyx `Constellation` by `chunk_id`; compares `text_hash`, `database_name`,
      and vector bytes. Any mismatch → `CALYX_ROUND_TRIP_MISMATCH { chunk_id, field,
      expected_hash, actual_hash }`. Returns `VerifyReport { total: usize,
      matched: usize, mismatches: Vec<MismatchDetail> }`.
- [ ] `VerifyReport::gate_passes(&self) -> bool`: `self.mismatches.is_empty() &&
      self.matched == self.total`. False → `CALYX_ROUND_TRIP_GATE_FAILED`.
- [ ] `RoundTripVerifier::benchmark_recall(sqlite_path, calyx_dir, queries:
      &[QueryVec], top_k: usize) -> Result<RecallBenchmark, CalyxError>`: runs the
      same `queries` against both paths; computes per-query recall@k and aggregate
      mean. `RecallBenchmark { sqlite_mean_recall, calyx_mean_recall, latency_sqlite_p99_us,
      latency_calyx_p99_us }`. Gate: `calyx_mean_recall >= sqlite_mean_recall`
      (A/B win) and `latency_calyx_p99_us <= latency_sqlite_p99_us * 1.05` (≤5%
      regression allowed). Violations → `CALYX_AB_RECALL_BELOW_BASELINE` /
      `CALYX_LATENCY_REGRESSION`.
- [ ] Enforce `database_name` preservation in `VerifyReport`: if any Calyx
      constellation's `database_name` differs from the SQLite row's verbatim value
      → `CALYX_CONTRACT_NAME_MISMATCH` (separate error code from vector mismatch).
- [ ] `calyx leapable verify-round-trip` CLI subcommand that calls `RoundTripVerifier`
      and prints the `VerifyReport` in human-readable and JSON form; exits non-zero
      if `!gate_passes()`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: migrate a 10-chunk fixture `.db` (known `chunk_id` = `"c001"`–`"c010"`,
      known `database_name = "fixture_db"`, known 32-byte text hashes from seed
      0xBEEF_CAFE) → `RoundTripVerifier::verify()` → `VerifyReport { total: 10,
      matched: 10, mismatches: [] }` → `gate_passes() == true`.
- [ ] unit: inject a deliberate text-hash corruption in one Calyx constellation →
      `mismatches.len() == 1`, mismatch carries the correct `chunk_id` and both
      hashes.
- [ ] unit: `benchmark_recall` on the 10-chunk fixture with 5 known query vectors →
      both mean recalls return 1.0 (exact-match fixture); latencies logged; gate
      passes.
- [ ] proptest: for any permutation of chunk insertion order (seed 0xROUND_TRIP,
      50 iterations), `VerifyReport.matched == total` (order-independent).
- [ ] edge (≥3):
      (a) empty `.db` (0 chunks) → `VerifyReport { total: 0, matched: 0 }` →
          `gate_passes() == true` (vacuously correct);
      (b) `database_name` stored differently in Calyx (e.g. trimmed whitespace) →
          `CALYX_CONTRACT_NAME_MISMATCH`;
      (c) missing Calyx constellation for a chunk that exists in SQLite →
          `CALYX_ROUND_TRIP_MISMATCH` with `field = "missing"`.
- [ ] fail-closed: corrupted Calyx manifest during verify → `CALYX_MANIFEST_CORRUPT`;
      `VerifyReport` not returned (fail-hard, not partial).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the output of `calyx leapable verify-round-trip` run against a real
  Leapable Vault copy on aiwonder; the `VerifyReport.json` written to disk; and the
  `RecallBenchmark` showing A/B recall win.
- **Readback:**
  ```
  # 1. Run migration (PH64 tool):
  calyx migrate vault real_vault_copy.db vault_v1.calyx

  # 2. Run round-trip verifier:
  calyx leapable verify-round-trip \
      --sqlite real_vault_copy.db \
      --calyx vault_v1.calyx \
      --output verify_report.json
  # must exit 0 and print: total=N, matched=N, mismatches=0, gate=PASS

  # 3. A/B recall benchmark:
  calyx leapable verify-round-trip \
      --sqlite real_vault_copy.db \
      --calyx vault_v1.calyx \
      --benchmark --queries test_queries.jsonl
  # must print: calyx_mean_recall >= sqlite_mean_recall, latency_p99 within 5%

  # 4. Confirm database_name verbatim:
  cat verify_report.json | jq '.database_name'
  # must match the exact string from the source .db metadata row
  ```
- **Prove:** `verify_report.json` shows `matched == total` (byte-exact on all
  `chunk_id`, `database_name`, `text_hash` fields) and `mismatches == []`.
  `RecallBenchmark` shows Calyx recall ≥ sqlite recall. This is the **V1 FSV gate**.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence: `verify_report.json` (`gate=PASS`, `mismatches=[]`) + recall
      benchmark output attached to the PH71 GitHub issue — proven on a real Vault
      on aiwonder
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
- [ ] `CALYX_CONTRACT_NAME_MISMATCH` is a distinct error code from
      `CALYX_ROUND_TRIP_MISMATCH` (grep confirms two separate enum variants)
