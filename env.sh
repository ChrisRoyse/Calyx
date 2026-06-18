#!/usr/bin/env bash
# Calyx session entrypoint - source this every session on aiwonder.
. "$HOME/.cargo/env"

export CALYX_HOME=/home/croyse/calyx
export HF_HOME="$CALYX_HOME/.hf-cache"
export CALYX_REPO="$CALYX_HOME/repo"
export GPU_PY="$CALYX_HOME/.venv-gpu/bin/python"
export CUDA_HOME=/usr/local/cuda
export CUDA_PATH="$CUDA_HOME"
export CUDA_ROOT="$CUDA_HOME"
export NVCC="$CUDA_HOME/bin/nvcc"
export PATH="$CALYX_HOME/bin:$HOME/.local/bin:$CUDA_HOME/bin:$PATH"
export LD_LIBRARY_PATH="$CUDA_HOME/lib64:${LD_LIBRARY_PATH:-}"
for calyx_nvidia_lib in "$CALYX_HOME"/.venv-gpu/lib/python*/site-packages/nvidia/*/lib; do
    if [ -d "$calyx_nvidia_lib" ]; then
        export LD_LIBRARY_PATH="$calyx_nvidia_lib:$LD_LIBRARY_PATH"
    fi
done
for calyx_nvidia_lib in "$CALYX_HOME"/.venv-cudnn/lib/python*/site-packages/nvidia/*/lib; do
    if [ -d "$calyx_nvidia_lib" ]; then
        export LD_LIBRARY_PATH="$calyx_nvidia_lib:$LD_LIBRARY_PATH"
    fi
done
unset calyx_nvidia_lib

export CALYX_CUVS_VENV="$CALYX_HOME/.venv-cuvs"
for calyx_cuvs_site in "$CALYX_CUVS_VENV"/lib/python*/site-packages; do
    if [ -d "$calyx_cuvs_site/libcuvs/lib64/cmake/cuvs" ]; then
        export CMAKE_PREFIX_PATH="$calyx_cuvs_site/libcuvs/lib64/cmake/cuvs:$calyx_cuvs_site/libraft/lib64/cmake/raft:$calyx_cuvs_site/librmm/lib64/cmake/rmm:$calyx_cuvs_site/rapids_logger/lib64/cmake/rapids_logger:$calyx_cuvs_site/librmm/lib64/cmake/nvtx3:$calyx_cuvs_site/libraft/lib64/rapids/cmake:$calyx_cuvs_site/librmm/lib64/rapids/cmake${CMAKE_PREFIX_PATH:+:$CMAKE_PREFIX_PATH}"
        export LD_LIBRARY_PATH="$calyx_cuvs_site/libcuvs/lib64:$calyx_cuvs_site/libraft/lib64:$calyx_cuvs_site/librmm/lib64:$calyx_cuvs_site/rapids_logger/lib64:$LD_LIBRARY_PATH"
        for calyx_nvidia_lib in "$CALYX_CUVS_VENV"/lib/python*/site-packages/nvidia/*/lib; do
            if [ -d "$calyx_nvidia_lib" ]; then
                export LD_LIBRARY_PATH="$calyx_nvidia_lib:$LD_LIBRARY_PATH"
            fi
        done
        unset calyx_nvidia_lib
        break
    fi
done
unset calyx_cuvs_site

# Do not set CARGO_TARGET_DIR here and do not cd from a sourced env file.
# Worktrees must keep their own target directories unless a command explicitly
# opts into a shared target. Use scripts/build-verified-calyx.sh for FSV builds.

# Secrets are optional here and wired in T-017. Keep values out of the repo.
if [ -f "$HOME/.config/calyx/secrets.env" ]; then
    . "$HOME/.config/calyx/secrets.env"
fi
