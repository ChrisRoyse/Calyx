# Contributing to Calyx

Thanks for your interest in Calyx! Contributions are welcome.

## Getting started

```bash
cargo build --workspace
cargo test --workspace
```

Calyx pins its toolchain in `rust-toolchain.toml` (Rust `1.95`, edition 2024).
The GPU (CUDA) backend is behind the `cuda` feature and is off by default, so
the workspace builds and tests CPU-only on any machine.

## Before you open a PR

Run the same checks CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Tests

Every test should answer two questions:

- **Fails when wrong:** what specific defect would make this test fail?
- **Passes when right:** what behavior proves the code is correct?

Follow FIRST:

- **Fast:** unit and property tests stay small and local.
- **Independent:** no hidden ordering or shared mutable process state.
- **Repeatable:** seed RNGs with `StdRng::seed_from_u64`; inject `Clock` — never
  read wall-clock time in logic, and don't depend on locale.
- **Self-validating:** clear assertions, not log inspection.
- **Timely:** add a regression test alongside the fix that needs it.

Avoid `sleep()` as synchronization (poll with a bounded timeout instead) and
don't leave `#[ignore]` behind without a tracking issue.

## Conventions

Calyx uses a controlled vocabulary — please keep it consistent:

- a frozen embedder is a **lens** (not "model"/"encoder")
- a record is a **constellation** (not "row"/"doc"/"point")
- information-about-an-outcome is **signal** (not "score"/"weight")

## License

By contributing, you agree that your contributions are licensed under the
project's [Business Source License 1.1](LICENSE).
