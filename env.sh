#!/usr/bin/env bash
# Calyx session entrypoint - source this every session on aiwonder.
. "$HOME/.cargo/env"

export CALYX_HOME=/home/croyse/calyx
export CARGO_TARGET_DIR="$CALYX_HOME/target"
export HF_HOME="$CALYX_HOME/.hf-cache"
export CUDA_HOME=/usr/local/cuda
export CUDA_PATH="$CUDA_HOME"
export CUDA_ROOT="$CUDA_HOME"
export NVCC="$CUDA_HOME/bin/nvcc"
export PATH="$CALYX_HOME/bin:$HOME/.local/bin:$CUDA_HOME/bin:$PATH"
export LD_LIBRARY_PATH="$CUDA_HOME/lib64:${LD_LIBRARY_PATH:-}"
for calyx_nvidia_lib in "$CALYX_HOME"/.venv-cudnn/lib/python*/site-packages/nvidia/*/lib; do
    if [ -d "$calyx_nvidia_lib" ]; then
        export LD_LIBRARY_PATH="$calyx_nvidia_lib:$LD_LIBRARY_PATH"
    fi
done
unset calyx_nvidia_lib

# Secrets are optional here and wired in T-017. Keep values out of the repo.
if [ -f "$HOME/.config/calyx/secrets.env" ]; then
    . "$HOME/.config/calyx/secrets.env"
fi
