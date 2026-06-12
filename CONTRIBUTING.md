# Contributing

## Tests

Every test must answer two questions before it lands:

- **Fails when wrong:** what specific defect would make this test fail?
- **Passes when right:** what source-of-truth behavior proves the code is correct?

Follow FIRST:

- **Fast:** unit and property tests should stay small and local.
- **Independent:** no hidden ordering, shared mutable process state, or mystery fixtures.
- **Repeatable:** seed RNGs with `StdRng::seed_from_u64`; inject `Clock`; do not read wall time in logic.
- **Self-validating:** use clear assertions, not log inspection.
- **Timely:** add regression tests with the fix that needs them.

Forbidden in committed tests:

- `sleep()` as synchronization; use polling with a bounded timeout when waiting is unavoidable.
- Lingering `#[ignore]`; fix it, delete it, or file a dated issue before merging.
- Assertion roulette; every assertion should make the failed invariant obvious.
- Wall-clock or locale dependence in logic tests.

Useful tools on aiwonder:

```bash
cargo test --workspace
cargo fuzz --help
cargo mutants --check
```

## Fuzz And Mutation Cadence

`cargo-fuzz` targets live under `fuzz/` and cover the PRD `28 §6c`
untrusted-input boundaries:

```bash
cargo fuzz list
cargo fuzz run aster_sst_decode fuzz/corpus/aster_sst_decode -- -runs=1000
cargo fuzz run aster_wal_replay fuzz/corpus/aster_wal_replay -- -runs=1000
cargo fuzz run aster_manifest_decode fuzz/corpus/aster_manifest_decode -- -runs=1000
cargo fuzz run query_parse fuzz/corpus/query_parse -- -runs=1000
cargo fuzz run lens_output_decode fuzz/corpus/lens_output_decode -- -runs=1000
cargo fuzz run mcp_jsonrpc_decode fuzz/corpus/mcp_jsonrpc_decode -- -runs=1000
```

Use bounded smoke runs on every touched untrusted boundary and longer sessions
when changing parser/decoder logic. Seed Aster corpus directories with real SST,
WAL, and MANIFEST bytes copied from aiwonder evidence vaults. A crash artifact is
not "handled" until a GitHub issue and regression test exist.

Mutation testing is agent-invoked on aiwonder, not hosted CI:

```bash
cargo mutants --in-diff origin/main...HEAD --check --output /home/croyse/calyx/data/mutants/diff-check
cargo mutants --in-diff origin/main...HEAD --output /home/croyse/calyx/data/mutants/diff-run
cargo mutants --package calyx-core --package calyx-aster --output /home/croyse/calyx/data/mutants/core-aster-periodic
```

Every survived mutant is a test-gap issue with the `cargo-mutants` report path
attached. The report artifact is the test-usefulness source of truth; passing
tests are still only a claim until FSV reads the relevant bytes.

Tests are the fast claim. FSV is the verdict: read persisted bytes on aiwonder
and attach the evidence to the relevant GitHub issue.
