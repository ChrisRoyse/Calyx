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

Tests are the fast claim. FSV is the verdict: read persisted bytes on aiwonder
and attach the evidence to the relevant GitHub issue.
