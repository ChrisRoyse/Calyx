# Issue 752 - Vault Panel And Registry Persistence Unblocker

## Scope

Issue #752 unblocks PH62 T02 and PH63 T02 by adding a vault-bound persistence
path for the panel plus registry state that CLI/MCP vault tools will mutate.
Before this change, Aster manifests referenced a hardcoded `panel/current.bin`
stub, `VaultManifest` had no registry reference, and `Registry` state lived only
in memory.

## Implementation

- `VaultManifest` now has an optional `registry_ref` immutable reference under
  `registry/`, validated and hash-verified with panel and codebook refs.
- Durable Aster vaults opened with `VaultOptions.panel = Some(panel)` write an
  initial manifest immediately, with a JSON panel asset under
  `panel/panel-v########-<hash>.json`.
- Legacy/default durable vault behavior is preserved: no manifest is forced for
  default empty durable opens, and existing no-panel vaults keep the old
  `panel/current.bin` asset path.
- Aster flushes preserve the current `panel_ref`, `codebook_refs`, and
  `registry_ref` when a reopened process has no in-memory panel option.
- `calyx-registry::persistence` writes and loads:
  - the current `Panel` JSON asset,
  - a `VaultRegistrySnapshot` sidecar JSON asset,
  - the manifest pointers that bind both assets into the vault.
- Registry snapshots store the real registered `lens_id`, frozen contract,
  optional `LensSpec`, and determinism proof. Load rebuilds known algorithmic,
  TEI HTTP, and external-command runtimes; unsupported local model runtimes load
  as fail-closed placeholders that preserve panel/listing state and return
  `CALYX_LENS_UNREACHABLE` on measurement.

## Aiwonder Gate

Run from `/home/croyse/calyx/repo` on branch
`issue752-vault-panel-registry-persistence`.

- `cargo fmt --all -- --check`
- `find crates -name '*.rs' -type f -print0 | xargs -0 wc -l | awk '$1 > 500 && $2 != "total" { print }'`
- `cargo check -p calyx-aster`
- `cargo check -p calyx-registry`
- `cargo clippy -p calyx-aster -p calyx-registry --all-targets -- -D warnings`
- `cargo test -p calyx-aster --test panel_manifest_persistence_fsv -- --nocapture`
- `cargo test -p calyx-registry --test issue752_vault_panel_persistence_fsv -- --nocapture`
- `cargo test -p calyx-aster -p calyx-registry --all-targets`

All commands passed on aiwonder.

## Manual FSV

Source of truth: bytes under `/home/croyse/calyx/data`, not test return values.

- Aster panel manifest root:
  `/home/croyse/calyx/data/fsv-issue752-panel-manifest-20260614`
  - `CURRENT` points to `manifest-00000000000000000003.json`.
  - `MANIFEST` has `manifest_seq: 3`, `durable_seq: 2`, and
    `panel_ref.logical_path: panel/panel-v00000007-117bdad5ca4e4c85.json`.
  - The panel asset is JSON with slot `issue752-existing`, lens id
    `07070707070707070707070707070707`, state `active`.
  - `registry_ref` is `null`, proving default reopen/flush preserved the JSON
    panel ref without inventing a registry sidecar.
- Registry happy-path root:
  `/home/croyse/calyx/data/fsv-issue752-panel-registry-20260614/happy`
  - `CURRENT` points to `manifest-00000000000000000003.json`.
  - Final `MANIFEST` points to
    `panel/panel-v00000003-d3dda30a979336b2.json` and
    `registry/registry-63b7f333f8707bb7.json`.
  - The final panel JSON has slot key `issue752-byte`, lens id
    `05c423b9dc88c95b2e6a7edba41ed739`, state `parked`, and dense dim 16.
  - The final registry JSON contains the same lens id, frozen contract, and
    `LensRuntime::Algorithmic { kind: "byte_features" }` spec.
- Edge roots:
  - `edge-no-registry/MANIFEST` has `registry_ref: null`; loader returned an
    empty registry snapshot.
  - `edge-corrupt-registry/MANIFEST` points to
    `registry/registry-66651a9604024b86.json` with expected blake3
    `66651a9604024b8665c8ff3351e2b1d14466bf17de5fe440532cf1c327f18c99`,
    while the actual file bytes are `{"corrupted":"issue752"}` and load fails
    with `CALYX_ASTER_CORRUPT_SHARD`.
  - `edge-unsupported-runtime/registry/registry-e341a8c55eff5a9f.json` contains
    `candle_local` for `issue752-cold-candle`; cold load preserves the lens id
    and measurement fails closed with `CALYX_LENS_UNREACHABLE`.
