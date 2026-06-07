#include <math.h>

__device__ bool forge_higher_priority(float left_score, int left_index, float right_score, int right_index) {
    if (left_score > right_score) {
        return true;
    }
    if (left_score < right_score) {
        return false;
    }
    return left_index < right_index;
}

extern "C" __global__ __launch_bounds__(256) void bitonic_topk_f32(
    const float *scores,
    int count,
    int k,
    int *out_indices,
    float *out_scores) {
    // DETERMINISM: fixed compare-exchange schedule, stable lower-index tie-break, no atomics.
    __shared__ float values[256];
    __shared__ int indices[256];

    const int tid = threadIdx.x;
    if (blockIdx.x != 0 || count <= 0 || k <= 0) {
        return;
    }

    if (tid < count && tid < 256) {
        values[tid] = scores[tid];
        indices[tid] = tid;
    } else {
        values[tid] = -INFINITY;
        indices[tid] = 2147483647;
    }
    __syncthreads();

    for (unsigned int size = 2; size <= 256; size <<= 1) {
        for (unsigned int stride = size >> 1; stride > 0; stride >>= 1) {
            const unsigned int partner = tid ^ stride;
            if (partner > tid) {
                const bool descending = (tid & size) == 0;
                const bool left_wins = forge_higher_priority(values[tid], indices[tid], values[partner], indices[partner]);
                const bool should_swap = descending ? !left_wins : left_wins;

                if (should_swap) {
                    const float tmp_value = values[tid];
                    const int tmp_index = indices[tid];
                    values[tid] = values[partner];
                    indices[tid] = indices[partner];
                    values[partner] = tmp_value;
                    indices[partner] = tmp_index;
                }
            }
            __syncthreads();
        }
    }

    if (tid < k && tid < count && tid < 256) {
        out_indices[tid] = indices[tid];
        out_scores[tid] = values[tid];
    }
}
