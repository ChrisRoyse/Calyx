#include <math.h>

#define TOPK_BLOCK 1024

__device__ __forceinline__ bool forge_higher_priority(
    float left_score,
    int left_index,
    float right_score,
    int right_index) {
    if (left_score > right_score) {
        return true;
    }
    if (left_score < right_score) {
        return false;
    }
    return left_index < right_index;
}

extern "C" __global__ __launch_bounds__(TOPK_BLOCK) void bitonic_topk_f32(
    const float *scores,
    int count,
    int k,
    int *out_indices,
    float *out_scores) {
    __shared__ float values[TOPK_BLOCK];
    __shared__ int indices[TOPK_BLOCK];
    __shared__ int bad[TOPK_BLOCK];

    const int tid = threadIdx.x;
    const int chunk = blockIdx.x;
    const int chunk_start = chunk * TOPK_BLOCK;
    const int global_index = chunk_start + tid;
    const int chunk_count = max(0, min(TOPK_BLOCK, count - chunk_start));
    const int out_base = chunk * k;

    if (count <= 0 || k <= 0 || chunk_count <= 0) {
        return;
    }

    if (tid < chunk_count) {
        const float score = scores[global_index];
        values[tid] = score;
        indices[tid] = global_index;
        bad[tid] = isnan(score) ? 1 : 0;
    } else {
        values[tid] = -INFINITY;
        indices[tid] = 2147483647;
        bad[tid] = 0;
    }
    __syncthreads();

    for (int stride = TOPK_BLOCK >> 1; stride > 0; stride >>= 1) {
        if (tid < stride) {
            bad[tid] |= bad[tid + stride];
        }
        __syncthreads();
    }
    if (bad[0]) {
        if (tid == 0) {
            out_indices[out_base] = -1;
            out_scores[out_base] = -2.0f;
        }
        return;
    }

    for (unsigned int size = 2; size <= TOPK_BLOCK; size <<= 1) {
        for (unsigned int stride = size >> 1; stride > 0; stride >>= 1) {
            const unsigned int partner = tid ^ stride;
            if (partner > tid) {
                const bool descending = (tid & size) == 0;
                const bool left_wins =
                    forge_higher_priority(values[tid], indices[tid], values[partner], indices[partner]);
                // DETERMINISM: ties broken by index (lower index wins); no warp-divergent paths on index comparison.
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

    if (tid < k && tid < chunk_count) {
        out_indices[out_base + tid] = indices[tid];
        out_scores[out_base + tid] = values[tid];
    }
}
