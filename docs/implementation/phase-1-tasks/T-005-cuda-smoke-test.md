# T-005 — CUDA 13.2 / sm_120 GPU build smoke test

**Phase:** PH00 · **Dep:** T-003, T-004 · **Sudo:** no

## Objective
Prove the box can **compile and run** GPU code for the RTX 5090 (sm_120) from
Rust, so Stage 2 (Forge) is unblocked — before writing any real kernels.

## Preconditions
- T-003 (`env.sh`, CUDA on PATH), T-004 (cmake). CUDA 13.2 + nvcc confirmed.

## Steps
1. **nvcc sm_120 smoke** — compile a trivial kernel for the target arch:
   ```bash
   source /home/croyse/calyx/repo/env.sh
   cat > /home/croyse/calyx/tmp/smoke.cu <<'EOF'
   #include <cstdio>
   __global__ void add(float* a){ a[threadIdx.x]+=1.0f; }
   int main(){ float* d; cudaMalloc(&d,16*sizeof(float)); add<<<1,16>>>(d);
     cudaDeviceSynchronize(); printf("cuda ok: %s\n", cudaGetErrorString(cudaGetLastError()));
     return 0; }
   EOF
   nvcc -arch=sm_120 /home/croyse/calyx/tmp/smoke.cu -o /home/croyse/calyx/tmp/smoke && \
     /home/croyse/calyx/tmp/smoke
   ```
2. **Rust GPU smoke** — a throwaway crate depending on `cudarc` (or `candle-core`
   with the `cuda` feature) that allocates device memory + runs one op, built
   with `CARGO_TARGET_DIR` in-root. Confirm it links against `/usr/local/cuda-13.2`.
3. Record the working flags (arch=sm_120, CUDA path) for `calyx-forge`'s build
   script (Stage 2 / PH13).
4. Clean up `/home/croyse/calyx/tmp/*`.

## Deliverables
- A proven sm_120 nvcc compile+run and a Rust-CUDA link smoke; the working build
  flags recorded for Forge.

## FSV gate
`smoke` prints `cuda ok: no error` on aiwonder's RTX 5090; the Rust-CUDA crate
runs a device op and prints a correct result; `nvidia-smi` shows the process
used the GPU (read it). No CPU-only fallback masked the test.

## Done
GPU build path validated end-to-end; flags recorded; Forge unblocked.

## Refs
`../12_STAGE2_FORGE.md` (PH13), `../01_AIWONDER_ENVIRONMENT.md §2`, A13.
