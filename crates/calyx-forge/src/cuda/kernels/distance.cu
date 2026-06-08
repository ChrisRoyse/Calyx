#include <math.h>

__device__ __forceinline__ bool finite2(float a, float b) {
    return isfinite(a) && isfinite(b);
}

__device__ __forceinline__ void reduce_sums(
    float *sum0,
    float *sum1,
    int *bad,
    int tid) {
    for (int stride = 128; stride > 0; stride >>= 1) {
        if (tid < stride) {
            sum0[tid] += sum0[tid + stride];
            sum1[tid] += sum1[tid + stride];
            bad[tid] |= bad[tid + stride];
        }
        __syncthreads();
    }
}

extern "C" __global__ __launch_bounds__(256) void cosine_batch_f32(
    const float *query,
    const float *candidates,
    int dim,
    int n_cands,
    float *out) {
    __shared__ float dot_shared[256];
    __shared__ float norm_q_shared[256];
    __shared__ float norm_c_shared[256];
    __shared__ int bad_shared[256];

    const int cand = blockIdx.x;
    const int tid = threadIdx.x;
    if (cand >= n_cands) {
        return;
    }

    float dot = 0.0f;
    float norm_q = 0.0f;
    float norm_c = 0.0f;
    int bad = dim <= 0;
    const int base = cand * dim;

    for (int i = tid; i < dim; i += blockDim.x) {
        const float q = query[i];
        const float c = candidates[base + i];
        bad |= !finite2(q, c);
        dot += q * c;
        norm_q += q * q;
        norm_c += c * c;
    }

    dot_shared[tid] = dot;
    norm_q_shared[tid] = norm_q;
    norm_c_shared[tid] = norm_c;
    bad_shared[tid] = bad;
    __syncthreads();

    reduce_sums(dot_shared, norm_q_shared, bad_shared, tid);
    for (int stride = 128; stride > 0; stride >>= 1) {
        if (tid < stride) {
            norm_c_shared[tid] += norm_c_shared[tid + stride];
        }
        __syncthreads();
    }

    if (tid == 0) {
        const float denom = sqrtf(norm_q_shared[0]) * sqrtf(norm_c_shared[0]);
        if (bad_shared[0]) {
            out[cand] = NAN;
        } else {
            out[cand] = denom > 0.0f ? dot_shared[0] / denom : -2.0f;
        }
    }
}

extern "C" __global__ __launch_bounds__(256) void dot_batch_f32(
    const float *query,
    const float *candidates,
    int dim,
    int n_cands,
    float *out) {
    __shared__ float dot_shared[256];
    __shared__ float unused_shared[256];
    __shared__ int bad_shared[256];

    const int cand = blockIdx.x;
    const int tid = threadIdx.x;
    if (cand >= n_cands) {
        return;
    }

    float dot = 0.0f;
    int bad = dim < 0;
    const int base = cand * dim;

    for (int i = tid; i < dim; i += blockDim.x) {
        const float q = query[i];
        const float c = candidates[base + i];
        bad |= !finite2(q, c);
        dot += q * c;
    }

    dot_shared[tid] = dot;
    unused_shared[tid] = 0.0f;
    bad_shared[tid] = bad;
    __syncthreads();
    reduce_sums(dot_shared, unused_shared, bad_shared, tid);

    if (tid == 0) {
        out[cand] = bad_shared[0] ? NAN : dot_shared[0];
    }
}

extern "C" __global__ __launch_bounds__(256) void l2_batch_f32(
    const float *query,
    const float *candidates,
    int dim,
    int n_cands,
    float *out) {
    __shared__ float l2_shared[256];
    __shared__ float unused_shared[256];
    __shared__ int bad_shared[256];

    const int cand = blockIdx.x;
    const int tid = threadIdx.x;
    if (cand >= n_cands) {
        return;
    }

    float l2 = 0.0f;
    int bad = dim < 0;
    const int base = cand * dim;

    for (int i = tid; i < dim; i += blockDim.x) {
        const float q = query[i];
        const float c = candidates[base + i];
        const float diff = q - c;
        bad |= !finite2(q, c);
        l2 += diff * diff;
    }

    l2_shared[tid] = l2;
    unused_shared[tid] = 0.0f;
    bad_shared[tid] = bad;
    __syncthreads();
    reduce_sums(l2_shared, unused_shared, bad_shared, tid);

    if (tid == 0) {
        out[cand] = bad_shared[0] ? NAN : l2_shared[0];
    }
}

extern "C" __global__ __launch_bounds__(256) void normalize_rows_f32(
    float *vecs,
    int dim,
    int rows) {
    __shared__ float norm_shared[256];
    __shared__ int bad_shared[256];
    __shared__ float scale_shared;

    const int row = blockIdx.x;
    const int tid = threadIdx.x;
    if (row >= rows) {
        return;
    }

    const int base = row * dim;
    float norm_sq = 0.0f;
    int bad = dim <= 0;

    for (int i = tid; i < dim; i += blockDim.x) {
        const float value = vecs[base + i];
        bad |= !isfinite(value);
        norm_sq += value * value;
    }

    norm_shared[tid] = norm_sq;
    bad_shared[tid] = bad;
    __syncthreads();

    for (int stride = 128; stride > 0; stride >>= 1) {
        if (tid < stride) {
            norm_shared[tid] += norm_shared[tid + stride];
            bad_shared[tid] |= bad_shared[tid + stride];
        }
        __syncthreads();
    }

    if (tid == 0) {
        const float norm = sqrtf(norm_shared[0]);
        scale_shared = (bad_shared[0] || !(norm > 0.0f) || !isfinite(norm))
            ? NAN
            : 1.0f / norm;
    }
    __syncthreads();

    for (int i = tid; i < dim; i += blockDim.x) {
        vecs[base + i] = isfinite(scale_shared) ? vecs[base + i] * scale_shared : NAN;
    }
}
