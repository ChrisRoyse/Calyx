#include <math.h>

extern "C" __global__ __launch_bounds__(256) void cosine_batch_f32(
    const float *a,
    const float *b,
    int dim,
    int pairs,
    float *out) {
    // DETERMINISM: block reduce with a fixed stride order, fixed 256-thread launch, no atomics.
    __shared__ float dot_shared[256];
    __shared__ float norm_a_shared[256];
    __shared__ float norm_b_shared[256];

    const int pair = blockIdx.x;
    const int tid = threadIdx.x;
    if (pair >= pairs || dim <= 0) {
        return;
    }

    float dot = 0.0f;
    float norm_a = 0.0f;
    float norm_b = 0.0f;
    const int base = pair * dim;

    for (int i = tid; i < dim; i += blockDim.x) {
        const float av = a[base + i];
        const float bv = b[base + i];
        dot += av * bv;
        norm_a += av * av;
        norm_b += bv * bv;
    }

    dot_shared[tid] = dot;
    norm_a_shared[tid] = norm_a;
    norm_b_shared[tid] = norm_b;
    __syncthreads();

    for (int stride = 128; stride > 0; stride >>= 1) {
        if (tid < stride) {
            dot_shared[tid] += dot_shared[tid + stride];
            norm_a_shared[tid] += norm_a_shared[tid + stride];
            norm_b_shared[tid] += norm_b_shared[tid + stride];
        }
        __syncthreads();
    }

    if (tid == 0) {
        const float denom = sqrtf(norm_a_shared[0]) * sqrtf(norm_b_shared[0]);
        out[pair] = denom > 0.0f ? dot_shared[0] / denom : 0.0f;
    }
}
