# T-003 — Configure Rust toolchain + `env.sh`

**Phase:** PH00 · **Dep:** T-002 · **Sudo:** no

## Objective
Reuse the already-installed rustup toolchain but **isolate all output inside
`CALYX_HOME`**, and create `repo/env.sh` — the single entrypoint every session
sources.

## Preconditions
- T-002 (home exists). Rust present via rustup (`~/.cargo`, `~/.rustup`).

## Steps
1. Confirm the toolchain on aiwonder:
   ```bash
   source ~/.cargo/env && rustc --version && cargo --version && rustup show
   ```
2. Pick a pinned stable channel; record it for `rust-toolchain.toml` (T-008).
   Add components: `rustup component add clippy rustfmt`.
3. Write `/home/croyse/calyx/repo/env.sh`:
   ```bash
   #!/usr/bin/env bash
   # Calyx session entrypoint — source this every session on aiwonder.
   source "$HOME/.cargo/env"
   export CALYX_HOME=/home/croyse/calyx
   export CARGO_TARGET_DIR="$CALYX_HOME/target"     # build output stays in-root
   export HF_HOME="$CALYX_HOME/.hf-cache"
   export CUDA_HOME=/usr/local/cuda                  # 13.2
   export PATH="$CALYX_HOME/bin:$HOME/.local/bin:$CUDA_HOME/bin:$PATH"
   export LD_LIBRARY_PATH="$CUDA_HOME/lib64:${LD_LIBRARY_PATH:-}"
   # secrets (0600, outside the repo) — see T-017
   [ -f "$HOME/.config/calyx/secrets.env" ] && . "$HOME/.config/calyx/secrets.env"
   ```
   `chmod +x env.sh`.
4. Verify isolation: `source env.sh && echo $CARGO_TARGET_DIR` →
   `/home/croyse/calyx/target`.

## Deliverables
- `repo/env.sh`; pinned toolchain + clippy/rustfmt; recorded channel for
  `rust-toolchain.toml`.

## FSV gate
`source repo/env.sh && cargo --version && nvcc --version` succeed; a throwaway
`cargo new /tmp/x && (cd /tmp/x && cargo build)` puts output under
`CARGO_TARGET_DIR` (verify the path), proving build output is in-root.

## Done
`env.sh` is the working entrypoint; toolchain reused; output isolated.

## Refs
`../01_AIWONDER_ENVIRONMENT.md §4`, `../00_README.md §6`.
