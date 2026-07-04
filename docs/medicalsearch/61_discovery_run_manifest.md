# Discovery Run Manifest

Issue: #1202

Lodestar now has a ledger-sealable discovery-run manifest for binding a
biomedical discovery atlas build into one reproducible hash chain.

## Model

`DiscoveryRunManifest` records:

- schema version
- run id
- corpus vault id
- panel manifest SHA-256
- ordered discovery stages

Each `DiscoveryRunStage` records:

- stage id
- command
- args
- optional upstream stage id
- input SHA-256
- output SHA-256
- git SHA

The chain is fail-closed. For every stage after the first, the stage input hash
must match either the explicitly named upstream stage output hash or the
previous stage output hash. A mismatch returns
`CALYX_DISCOVERY_RUN_MANIFEST_CHAIN_BROKEN`.

## Ledger Seal

`seal_discovery_run_manifest` appends the manifest under `EntryKind::Assay` in
the Calyx ledger. The payload stores the manifest SHA-256 and the ordered stage
hashes, so normal ledger verification covers the discovery-run seal.

## Reproduction Check

`reproduce_discovery_run_manifest` compares observed stage output hashes against
the manifest. Missing or changed outputs fail closed with
`CALYX_DISCOVERY_RUN_MANIFEST_DRIFT`.

## Verification

Focused tests:

```text
cargo test -p calyx-lodestar --test issue1202_discovery_run_manifest_tests -- --nocapture
cargo check -p calyx-lodestar
git diff --check
bash scripts/linecount.sh
```

Physical FSV:

```text
target/fsv/issue1202-discovery-run-manifest-20260704T062200Z/issue1202_discovery_run_manifest_readback.json
```

The readback observed five chained stages, a ledger payload with
`stage_count=5`, `verify_chain=Intact { count: 1 }`, manifest SHA-256
`ff50eb0b0b44d49f13726914e823bdf81d8052a7bc6b2dadd3bed08e692f4a79`, and
readback artifact SHA-256
`79D5A3440DD00216D6F09D938598D9A6972F5C0882F45EB2DD72992DFCBA1309`.

## Remaining Operator Surface

The core manifest and ledger seal are available in Lodestar. Public CLI
wrappers and per-stage preflight enforcement should be tracked separately so
the individual stage commands can adopt the manifest contract without a broad,
high-risk edit.
