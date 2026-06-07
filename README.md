# Calyx

Calyx is the universal association-native database described by the PRDs in
`docs/dbprdplans/` and the implementation plan in `docs/implementation/`.

All build, test, and verification work happens on aiwonder under
`/home/croyse/calyx`. A local checkout is for authoring only.

## Per-Merge Gate

Run the gate on aiwonder before every merge:

```bash
cd /home/croyse/calyx/repo
source ./env.sh
bash scripts/check.sh
```

`scripts/check.sh` runs `cargo fmt`, `cargo check`, `cargo clippy -D warnings`,
`cargo test`, and the `scripts/linecount.sh` gate. There is no hosted CI for
Calyx; FSV evidence in GitHub Issues is the release gate.

Every `.rs` source/test file must stay at or below 500 lines. If a file exceeds
the limit, open a `type:task` issue and modularize it per
`docs2/modulateprompt.md` before the gate can pass.
