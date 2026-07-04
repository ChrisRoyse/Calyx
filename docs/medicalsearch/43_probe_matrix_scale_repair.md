# #1192 Probe-Matrix Scale Repair and Real-Vault Readback

## Scope

#1192 verifies that probe-matrix mining can run against the large #869 clinical
association vault without reintroducing the earlier 41 GB resident-set failure
and without silently using stale derived indexes.

This is discovery infrastructure. It does not establish a treatment, cure, or
clinical actionability claim. Any association mined through this path remains a
ranked, traceable hypothesis until it clears outcome, safety, counter-evidence,
and external validation gates.

## Root Causes Found

The original large-RSS probe-matrix issue had already been addressed by #1001 in
commit `da7a883f`, which stopped materializing large provenance/hit structures
per variant.

The current real-vault blocker was earlier in the pipeline:

1. The real vault manifest pointed `panel_ref` at the stage-one placeholder
   `panel/current.bin` and had `registry_ref: null`, so panel loading failed
   with `CALYX_ASTER_CORRUPT_SHARD: decode panel`.
2. After explicit manifest repair, probe-matrix still always requested fresh
   search indexes. That was correct as a default, but unlike regular search it
   had no operator-visible `--stale-ok` policy for cases where the derived
   watermark advanced due unrelated non-search graph/manifest writes.

## Code Changes

- `e8b7fcad` adds `calyx panel manifest-restore --vault ... --panel-asset ...
  --registry-asset ...`.
  - It requires exact operator-supplied asset refs.
  - It validates ref prefixes, hashes, panel decode, registry decode, and
    registry-panel agreement.
  - It writes a new manifest, then reloads through the real panel loader and
    rechecks vault registry contracts.
- `5d0f2991` adds explicit `probe-matrix --stale-ok`.
  - Default behavior remains fail-closed fresh-index checking.
  - `--stale-ok` is an explicit operator policy, matching the regular search
    CLI contract.
  - The persisted progress JSON records `stale_ok`.

## Local Gates

Focused and hygiene gates passed:

```bash
cargo test -p calyx-cli manifest_restore -- --nocapture
cargo test -p calyx-cli cmd::probe_matrix::tests -- --nocapture
pwsh -File scripts/cargo-fmt-workspace.ps1
bash scripts/linecount.sh
git diff --check
cargo check -p calyx-cli
```

`git diff --check` emitted only CRLF normalization warnings for existing test
files.

## Real Vault Manifest Repair FSV

Host: aiwonder.

Repo: `/home/croyse/calyx/repo`.

Vault: `corpus-anchored-869-20260625T080546Z`
(`01KVYX0KYVBQSGVC6N2S00FX6J`).

FSV root:

```text
/home/croyse/calyx/fsv/issue1192-manifest-restore-20260704T013925Z
```

Repair command:

```bash
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx panel manifest-restore \
  --vault corpus-anchored-869-20260625T080546Z \
  --panel-asset panel/panel-v00000009-33427fad14698dd4.json \
  --registry-asset registry/registry-b14e0896f0c88790.json
```

Readback summary:

```json
{
  "status": "manifest_panel_registry_restored",
  "manifest_seq_before": 683838,
  "manifest_seq_after": 683839,
  "durable_seq": 684029,
  "derived_content_seq": 684029,
  "old_panel_ref": "panel/current.bin",
  "old_registry_ref": null,
  "new_panel_ref": "panel/panel-v00000009-33427fad14698dd4.json",
  "new_panel_blake3": "33427fad14698dd49ec43d649307d64f84170b263d3d4fef04692c33be07cf73",
  "new_registry_ref": "registry/registry-b14e0896f0c88790.json",
  "new_registry_blake3": "b14e0896f0c8879011ad785890f942f09979785a383fa674afebd571602c7fa8",
  "manifest_pointer": "manifest-00000000000000683839.json",
  "reloaded_panel_version": 9,
  "reloaded_slot_count": 25,
  "reloaded_registry_lens_count": 14,
  "registry_checked_count": 14
}
```

Independent readback after repair:

```bash
CALYX_HOME=/home/croyse/calyx \
  ./target/release/calyx list-panel corpus-anchored-869-20260625T080546Z
```

Result: 25 slots loaded, slots 8 through 24 active.

## Real Probe-Matrix FSV

FSV root:

```text
/home/croyse/calyx/fsv/issue1192-probe-matrix-stale-ok-20260704T014754Z
```

Release build on aiwonder:

```bash
cd /home/croyse/calyx/repo
git pull --ff-only
cargo build --release -p calyx-cli
```

Result: release build passed at `5d0f2991`.

Happy-path command shape:

```bash
CALYX_HOME=/home/croyse/calyx \
  /usr/bin/time -v ./target/release/calyx probe-matrix \
  corpus-anchored-869-20260625T080546Z \
  --frontier "type 2 diabetes" \
  --slot 21 \
  --weighted-profile bridge \
  --phrasing clinical \
  --length phrase \
  --top-k 3 \
  --guard off \
  --stale-ok \
  --out "$ROOT/happy/probe.json" \
  --search-miss-budget-ms 60000 \
  --search-hit-budget-ms 10000
```

Readback summary:

| Case | Exit | Status | Stop reason | Variants | Records | Accepted hits | Cache | Wall | Max RSS |
|---|---:|---|---|---:|---:|---:|---|---:|---:|
| `happy` | 0 | `ok` | none | 5/5 | 5 | 15 | 1 miss, 4 hits, 64 stored hits | 0:01.90 | 789,840 KB |
| `edge_gpu_resident_required` | 2 | `incomplete` | `resident_required` | 0/5 | 0 | 0 | no search cache use | 0:01.38 | 673,684 KB |
| `edge_variant_budget` | 2 | `incomplete` | `variant_budget_exhausted` | 1/5 | 1 | 3 | 1 miss, 64 stored hits | 0:01.73 | 789,724 KB |

Persisted artifact hashes:

| Case | Artifact | SHA256 |
|---|---|---|
| `happy` | `probe.json` | `a0faf70f421a6179f0f4fb05c14694f233a1d8033136233071a86944b3af1638` |
| `happy` | progress JSON | `a7e78c770b45cfe58f4de6fb786acbff16ab6787e16d4f61974bb22a16f17ff8` |
| `edge_gpu_resident_required` | `probe.json` | `e460f132bd35b8b7ede1711a50322d50b91496c3217295e943b1dc7e6fdfc628` |
| `edge_gpu_resident_required` | progress JSON | `da6fc0fbc9d995d40ee486e9b9b376bc3886208d39d9494d51eeaccd0bf93472` |
| `edge_variant_budget` | `probe.json` | `c5ffa7fe356c0a4fa07460c4651566923e5fca1f2e3f3c194082f4fca824adbc` |
| `edge_variant_budget` | progress JSON | `98f403f74f7aafeabf15e228b253670dae8ea60762a0b2ba5b6c28d77b9ed9f3` |

`readback_summary.json` and `sha256sums.txt` live at the FSV root above.

## Conclusion

#1192 is satisfied:

- the real #869 vault manifest now points to a decodable panel and matching
  registry snapshot;
- panel load and registry contract readback pass after the Calyx DB repair;
- probe-matrix retains fresh-index fail-closed behavior by default;
- explicit `--stale-ok` enables the operator-approved stale-index path;
- the real large-vault happy path completes in under 2 seconds and under 1 GB
  max RSS for the tested slot/frontier;
- edge cases persist diagnostic incomplete matrices and progress artifacts with
  source-of-truth hashes.

The next discovery dependency is broad typed all-pair association mining over
the now-readable large graph and probe substrate.
