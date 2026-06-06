# T-015 — Test scaffolding (seeded RNG, injected Clock, proptest)

**Phase:** PH03 · **Dep:** T-008 · **Sudo:** no

## Objective
Establish the test discipline (FIRST + properties) as reusable scaffolding so
every later phase writes useful tests by default, and prove the toolchain
(proptest/fuzz/mutants/criterion) works on aiwonder.

## Preconditions
- T-008 (workspace).

## Steps
1. Add dev-deps to `[workspace.dependencies]`: `proptest`, `rand` (seeded), and
   document `cargo-fuzz`, `cargo-mutants`, `criterion` (install via `cargo
   install` — userspace):
   ```bash
   source /home/croyse/calyx/repo/env.sh
   cargo install cargo-fuzz cargo-mutants
   ```
2. A `calyx-testkit` dev helper (or a `tests/common` module): seeded RNG
   (`StdRng::seed_from_u64`), a `FixedClock`, proptest strategies for core types
   (IDs, enums, a small `Constellation`).
3. First real property tests (land with T-010/T-013): `parse∘display == id`,
   serde round-trip, `Absent` stays absent.
4. Document the **two questions** (fails-when-wrong / passes-when-right) and the
   FIRST rules in the repo `CONTRIBUTING`/working-agreement; forbid `sleep()`,
   shared state, lingering `#[ignore]`.
5. Smoke a `cargo-mutants --check` on `calyx-core` to confirm it runs.

## Deliverables
- Seeded-RNG + injected-clock + proptest scaffolding; `cargo-fuzz`/`-mutants`
  installed; the test doctrine documented.

## FSV gate
`cargo test --workspace` runs the property tests green on aiwonder; a proptest
shrinks a deliberately-broken invariant to a minimal counterexample (demonstrate,
then fix); `cargo-mutants --check` runs. Determinism: re-running a seeded test
gives identical results (no wall-clock/RNG nondeterminism).

## Done
The test discipline is scaffolded, the tooling proven on aiwonder, the first
property tests green.

## Refs
PRD `28 §6c`, A34 (free OSS), `../02_WORKING_AGREEMENT.md §3`.
