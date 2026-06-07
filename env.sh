#!/usr/bin/env bash
# Calyx session entrypoint - source this every session on aiwonder.
source "$HOME/.cargo/env"

export CALYX_HOME=/home/croyse/calyx
export CARGO_TARGET_DIR="$CALYX_HOME/target"
export HF_HOME="$CALYX_HOME/.hf-cache"
export CUDA_HOME=/usr/local/cuda
export PATH="$CALYX_HOME/bin:$HOME/.local/bin:$CUDA_HOME/bin:$PATH"
export LD_LIBRARY_PATH="$CUDA_HOME/lib64:${LD_LIBRARY_PATH:-}"

# Secrets are optional here and wired in T-017. Keep values out of the repo.
if [ -f "$HOME/.config/calyx/secrets.env" ]; then
    . "$HOME/.config/calyx/secrets.env"
fi
