# 12 - probe matrix

- **Issue:** #879   **Phase:** P0 discovery   **Date (UTC):** 2026-06-29   **Vault/panel:** physical Calyx vaults with persisted search indexes
- **Goal:** run a reusable physical probe matrix over fusion x phrasing x length x lens emphasis, persist the complete matrix as a source-of-truth artifact, and log combinations that surface unique grounded hits.

## Root cause
The previous #879 slice only proved the Lodestar in-memory planner. It did not provide a physical CLI harness that opened a real vault, measured real selected lenses, searched the persisted index, and wrote a durable readback artifact.

While wiring the physical harness, three related root causes showed up:

- Weighted-RRF profiles and explicit single-lens slots were not expressible through the physical search API, so matrix rows could not preserve their exact fusion intent.
- Search measurement and persisted-index boundedness checks were global to all active slots. A scoped probe could be blocked by unrelated active slots and sidecars.
- The anchored large corpus has two separate defects outside #879: a corrupted slot-15 frozen registry contract (#1000) and excessive large-corpus matrix materialization/search state (#1001).

## Research used
Best-practice research was done with Exa plus browser search before implementation.

- Cormack, Clarke, and Buettcher, SIGIR 2009, "Reciprocal Rank Fusion outperforms Condorcet and individual rank learning methods." This supports keeping RRF-style rank evidence explainable per contributing run.
- Azure AI Search hybrid/RRF docs:
  - https://learn.microsoft.com/en-us/azure/search/hybrid-search-overview
  - https://learn.microsoft.com/en-us/azure/search/hybrid-search-ranking
- Elastic search/retriever docs:
  - https://www.elastic.co/docs/api/doc/elasticsearch/v9/operation/operation-search

Implementation choices taken from that research:

- Preserve exact source provenance for every fusion row: fusion mode, selected RRF profile, selected slot, rank, ledger sequence, ledger hash, and per-lens contribution.
- Fail closed when a selected target is unavailable instead of silently broadening the query.
- Scope selected fields/slots explicitly so a targeted probe matrix does not measure unrelated active lenses.
- Persist a durable matrix artifact and independently read it back instead of treating command success as proof.

## What changed
Implemented a physical `calyx probe-matrix <vault>` command:

- `crates/calyx-cli/src/cmd/probe_matrix.rs`
- `crates/calyx-cli/src/cmd/probe_matrix/parse.rs`
- `crates/calyx-cli/src/cmd/probe_matrix/tests.rs`
- `crates/calyx-cli/src/cmd/mod.rs`
- `crates/calyx-cli/src/cmd/tests.rs`
- `crates/calyx-cli/src/cmd/tests/token_roundtrip.rs`
- `crates/calyx-cli/src/usage.rs`
- `crates/calyx-search/src/engine.rs`
- `crates/calyx-search/src/lib.rs`
- `crates/calyx-search/src/persisted.rs`
- `crates/calyx-search/src/persisted/mixed_tests.rs`

The command now:

- Opens a real Calyx vault and persisted panel/registry.
- Validates requested slots are present, active, and text modality.
- Runs the probe matrix through physical search with explicit slot scoping.
- Preserves `WeightedRrfProfile(profile)` and `SingleLensSlot(slot)` fusion choices.
- Writes `<vault>/idx/probe_matrix/<blake3>/matrix.json` with an atomic write, byte readback, JSON decode, and SHA256 report.
- Fails closed if the matrix has no records, no grounded accepted hits, or no productive rows.

## Local verification
Commands run on the authoring checkout:

```bash
cargo fmt --all -- --check
git diff --check
cargo test -p calyx-search boundedness_check_is_scoped_to_selected_slots --target-dir target/issue879-search --jobs 32 -- --nocapture
cargo test -p calyx-search explicit_fusion_choices_preserve_profile_and_slot --target-dir target/issue879-search --jobs 32 -- --nocapture
cargo test -p calyx-cli cmd::probe_matrix::tests --target-dir target/issue879-cli-probe --jobs 32 -- --nocapture
cargo test -p calyx-lodestar --test issue879_probe_matrix_tests --target-dir target/issue879-lodestar --jobs 32 -- --nocapture
cargo test -p calyx-cli vault_subcommands_round_trip --target-dir target/issue879-cli-roundtrip --jobs 32 -- --nocapture
```

Focused tests prove:

- Probe-matrix command parsing and token round trips.
- Source-of-truth matrix persistence and readback.
- Missing slots fail before artifact creation.
- Explicit weighted-RRF profiles and single-lens slots preserve exact search intent.
- Boundedness checks are scoped to selected slots and fail closed when the selected sidecar is corrupt.

## Manual full state verification
Source of truth:

```text
/home/croyse/calyx/vaults/01KW9Q13GMN0DJEGRP820YE5AQ/idx/probe_matrix/6c6b38f256484066e9ab3ad7ef5a0c183daf1cbaaf4129acbd38c7dff021f933/matrix.json
```

Manual FSV root:

```text
/home/croyse/calyx/fsv/issue879-manual-vault-20260629T125251Z
```

Physical setup used the real CLI:

```bash
calyx create-vault issue879-fsv-125251 --panel-template text-default
calyx add-lens issue879-fsv-125251 --name issue879_sparse --runtime algorithmic:sparse-keywords:64 --shape 'Sparse(64)'
calyx add-lens issue879-fsv-125251 --name issue879_scalar --runtime algorithmic:scalar --shape 'Dense(1)'
calyx ingest issue879-fsv-125251 --text alpha
calyx ingest issue879-fsv-125251 --text omega
calyx ingest issue879-fsv-125251 --text 'type diabetes alpha pathway'
calyx anchor issue879-fsv-125251 <cx> --kind label:issue879 --value <name> --confidence 1.0 --source issue879-manual-fsv
calyx rebuild-search-index issue879-fsv-125251
```

Selected physical slots:

```json
{
  "issue879_sparse": {"slot_id": 8, "lens_id": "5b813a24c36f4fe0c1fb77ca9836e050"},
  "issue879_scalar": {"slot_id": 9, "lens_id": "dc28852f0b38c142057ba04682c3e90b"}
}
```

Happy-path command:

```bash
RAYON_NUM_THREADS=32 /home/croyse/calyx/target/issue879-probe/debug/calyx probe-matrix issue879-fsv-125251 \
  --frontier alpha \
  --slot 8 --slot 9 \
  --weighted-profile bridge \
  --phrasing terse \
  --length entity \
  --top-k 1
```

Before state:

```json
{"vault":"issue879-fsv-125251","vault_dir":"/home/croyse/calyx/vaults/01KW9Q13GMN0DJEGRP820YE5AQ","artifact_count":0}
```

After state:

```json
{"vault":"issue879-fsv-125251","vault_dir":"/home/croyse/calyx/vaults/01KW9Q13GMN0DJEGRP820YE5AQ","artifact_count":1,"exit":0}
```

Readback from the matrix file itself:

```json
{
  "source_of_truth_bytes": 6435,
  "source_of_truth_sha256": "bc84f978b20d30af2825b95a84e0460adb83f47fa5eddc66f828bebbede2c11d",
  "schema_version": 1,
  "vault": "issue879-fsv-125251",
  "frontier": "alpha",
  "axis_counts": {"slots": 2, "weighted_profiles": 1, "phrasings": 1, "lengths": 1, "records": 6},
  "active_slots": [8, 9],
  "productive_count": 1,
  "accepted_hit_count": 6,
  "refusal_count": 0,
  "productive_rows": [
    {
      "accepted_hit_count": 1,
      "fusion": "single_lens",
      "length": "entity",
      "lens_emphasis": {"kind": "slot", "value": 8},
      "phrasing": "terse",
      "refusal_count": 0,
      "unique_hit_count": 1,
      "variant_id": 3
    }
  ]
}
```

Productive hit read from the artifact:

```json
{
  "cx_id": "efab2164502b7693780f33df2a2dafe8",
  "status": "accepted",
  "grounded": true,
  "score": 0.5908617377281189,
  "provenance": [
    "rank=1",
    "ledger_seq=3",
    "ledger_hash=798da76bbfbb627607d8bd6ebde779080d52b57f886b63f3223c337800661787",
    "provenance_source=Stored",
    "lens:8 rank=1 contribution=0.59086174"
  ]
}
```

## Boundary and edge case audit
Source of truth for edge cases: artifact count under the vault's `idx/probe_matrix`.

```json
{
  "cases": [
    {
      "case": "empty_frontier",
      "before": {"artifact_count": 1},
      "after": {"artifact_count": 1, "exit": 2},
      "error": "CALYX_CLI_USAGE_ERROR: probe-matrix requires non-empty --frontier <text>"
    },
    {
      "case": "missing_slot",
      "before": {"artifact_count": 1},
      "after": {"artifact_count": 1, "exit": 2},
      "error": "CALYX_CLI_USAGE_ERROR: --slot 123 is not present in the vault panel"
    },
    {
      "case": "zero_top_k",
      "before": {"artifact_count": 1},
      "after": {"artifact_count": 1, "exit": 2},
      "error": "CALYX_CLI_USAGE_ERROR: --top-k must be >= 1"
    }
  ]
}
```

All three edge cases failed closed and did not create or modify a matrix artifact.

## Anchored-corpus findings
Attempts against `corpus-anchored-869-20260625T080546Z` exposed two separate P0 defects:

- #1000: slot 15 has a persisted frozen-contract mismatch. The active slot stores lens id `6f47e637a0c287f78972af35c369ce82`, but reconstructing the registry spec yields `51349ee0af01f5257a648c5ac60a9246`.
- #1001: even after scoped slot selection, large-corpus probe-matrix runs can materialize too much provenance/search state. One selected two-slot run reached about 41 GB RSS and ran more than 11 minutes before artifact creation.

Those blockers are not hidden by #879. The probe matrix harness is complete and verified on a real physical vault; #1000 and #1001 track the remaining corpus-specific failures.

## Conclusion
#879 is complete for the physical probe-matrix harness. The command now runs against real vault state, persists a durable matrix artifact, records productive combinations with grounded provenance, and fails closed on invalid or unproductive states. Large-corpus repair and scaling continue under #1000 and #1001.

## Update 2026-07-01 — #1088 in-region guard calibration
Real-vault runs (calyx15000 issue #51) showed `--guard in-region` silently filtering every
retrieved candidate: the in-region guard applied a hardcoded, uncalibrated cosine tau of 0.999
(a near-duplicate threshold), so the benchmark surfaced a generic
`CALYX_PROBE_MATRIX_INCOMPLETE` / "no grounded accepted hits" indistinguishable from a genuinely
empty benchmark. Observed signal: `guard.prefilter.done count=0 filtered=64 tau=0.999000`.

Best-practice basis (matches the Ward conformal-calibration doctrine): cosine thresholds are not
transferable across models/corpora and must be calibrated to the corpus's actual score
distribution — a fixed global tau is a category error for real corpora.

What changed (no fallback masking, never auto-switches to `--guard off`):

- **Calibration path:** `calyx probe-matrix ... --guard in-region --guard-tau <cosine in (0,1]>`
  supplies a corpus-calibrated threshold; it overrides the documented default
  (`calyx_search::DEFAULT_IN_REGION_GUARD_TAU = 0.999`). Out-of-range tau, or `--guard-tau`
  without `--guard in-region`, fails closed at parse and again in the engine.
- **Fail-closed diagnosis:** when in-region filtered every candidate the search path actually
  retrieved across all completed variants, the run exits with
  `CALYX_PROBE_MATRIX_GUARD_FILTERED_ALL`, naming the applied tau (and whether it was the
  uncalibrated default or operator-supplied), the observed in-region best-cosine range, the
  per-variant zero-hit reasons, and the persisted matrix/progress artifact paths. The observed
  cosine range is the calibration evidence: rerun with `--guard-tau` at or below the observed
  best-cosine max.
- Per-variant guard diagnostics (prefilter input/output, guard tau, best-cosine min/max,
  zero-hit reason) were already persisted in `diagnostics.variant_guard_counts`; the new exit
  reads them back instead of discarding them.

Calibration procedure for a real vault: run once with `--guard in-region` (uncalibrated); read
`observed in-region best-cosine range [min, max]` from the `CALYX_PROBE_MATRIX_GUARD_FILTERED_ALL`
error (or `guard_best_cosine_max` in `diagnostics.variant_guard_counts`); rerun with
`--guard-tau` at or below that max. A follow-up unification with the calibrated per-slot Ward
guard profile (`calyx guard calibrate`, Guard CF `profile\0default`, as the MCP search path
already enforces) is tracked separately.
