#include <stdint.h>
#include <math.h>

#define CALYX_MAXSIM_MAX_K 1024
#define CALYX_MAXSIM_THREADS 256

__device__ __forceinline__ bool id_less(
    uint64_t left_hi,
    uint64_t left_lo,
    uint64_t right_hi,
    uint64_t right_lo) {
  return left_hi < right_hi || (left_hi == right_hi && left_lo < right_lo);
}

__device__ __forceinline__ bool better(
    float left_score,
    uint64_t left_hi,
    uint64_t left_lo,
    float right_score,
    uint64_t right_hi,
    uint64_t right_lo) {
  return left_score > right_score ||
         (left_score == right_score &&
          id_less(left_hi, left_lo, right_hi, right_lo));
}

extern "C" __global__ void maxsim_score_rows(
    const float* query_tokens,
    const float* query_norms,
    int query_count,
    const float* doc_tokens,
    const float* doc_norms,
    const uint32_t* row_offsets,
    const uint8_t* candidate_mask,
    int row_count,
    int dim,
    float* row_scores) {
  const int row = (int)blockIdx.x;
  if (row >= row_count) return;
  if (candidate_mask[row] == 0) {
    if (threadIdx.x == 0) row_scores[row] = -INFINITY;
    return;
  }
  const uint32_t token_start = row_offsets[row];
  const uint32_t token_end = row_offsets[row + 1];
  if (query_count == 0) {
    if (threadIdx.x == 0) row_scores[row] = 0.0f;
    return;
  }
  __shared__ float partial[CALYX_MAXSIM_THREADS];
  float score = 0.0f;
  for (int q = 0; q < query_count; ++q) {
    const float* query = query_tokens + ((int64_t)q * dim);
    const float query_norm = query_norms[q];
    float best = -INFINITY;
    for (uint32_t token = token_start + threadIdx.x; token < token_end;
         token += blockDim.x) {
      const float* doc = doc_tokens + ((int64_t)token * dim);
      float dot = 0.0f;
      for (int col = 0; col < dim; ++col) {
        dot += query[col] * doc[col];
      }
      const float doc_norm = doc_norms[token];
      const float cosine =
          (query_norm == 0.0f || doc_norm == 0.0f)
              ? 0.0f
              : dot / (query_norm * doc_norm);
      best = fmaxf(best, cosine);
    }
    partial[threadIdx.x] = best;
    __syncthreads();
    for (int stride = blockDim.x / 2; stride > 0; stride >>= 1) {
      if (threadIdx.x < stride) {
        partial[threadIdx.x] = fmaxf(partial[threadIdx.x], partial[threadIdx.x + stride]);
      }
      __syncthreads();
    }
    if (threadIdx.x == 0) score += partial[0];
    __syncthreads();
  }
  if (threadIdx.x == 0) row_scores[row] = score;
}

extern "C" __global__ void maxsim_chunk_topk(
    const float* row_scores,
    const uint64_t* row_id_hi,
    const uint64_t* row_id_lo,
    const uint8_t* candidate_mask,
    int row_count,
    int k,
    uint64_t* out_hi,
    uint64_t* out_lo,
    float* out_scores,
    uint32_t* out_count) {
  if (threadIdx.x != 0 || blockIdx.x != 0) return;
  int count = 0;
  for (int row = 0; row < row_count; ++row) {
    if (candidate_mask[row] == 0) continue;
    const float score = row_scores[row];
    const uint64_t hi = row_id_hi[row];
    const uint64_t lo = row_id_lo[row];
    int insert = count;
    while (insert > 0 &&
           better(score, hi, lo, out_scores[insert - 1], out_hi[insert - 1],
                  out_lo[insert - 1])) {
      if (insert < k) {
        out_scores[insert] = out_scores[insert - 1];
        out_hi[insert] = out_hi[insert - 1];
        out_lo[insert] = out_lo[insert - 1];
      }
      --insert;
    }
    if (insert < k) {
      out_scores[insert] = score;
      out_hi[insert] = hi;
      out_lo[insert] = lo;
    }
    if (count < k) ++count;
  }
  *out_count = (uint32_t)count;
}

extern "C" __global__ void maxsim_merge_topk(
    const uint64_t* chunk_hi,
    const uint64_t* chunk_lo,
    const float* chunk_scores,
    const uint32_t* chunk_count,
    uint64_t* global_hi,
    uint64_t* global_lo,
    float* global_scores,
    uint32_t* global_count,
    int k) {
  if (threadIdx.x != 0 || blockIdx.x != 0) return;

  __shared__ uint64_t old_hi[CALYX_MAXSIM_MAX_K];
  __shared__ uint64_t old_lo[CALYX_MAXSIM_MAX_K];
  __shared__ float old_scores[CALYX_MAXSIM_MAX_K];

  const int old_count = (int)(*global_count);
  const int take_chunk = (int)(*chunk_count);
  for (int i = 0; i < old_count; ++i) {
    old_hi[i] = global_hi[i];
    old_lo[i] = global_lo[i];
    old_scores[i] = global_scores[i];
  }

  const int output_count = min(k, old_count + take_chunk);
  int old_pos = 0;
  int chunk_pos = 0;
  for (int out = 0; out < output_count; ++out) {
    const bool use_old =
        old_pos < old_count &&
        (chunk_pos >= take_chunk ||
         better(old_scores[old_pos], old_hi[old_pos], old_lo[old_pos],
                chunk_scores[chunk_pos], chunk_hi[chunk_pos],
                chunk_lo[chunk_pos]));
    if (use_old) {
      global_hi[out] = old_hi[old_pos];
      global_lo[out] = old_lo[old_pos];
      global_scores[out] = old_scores[old_pos];
      ++old_pos;
    } else {
      global_hi[out] = chunk_hi[chunk_pos];
      global_lo[out] = chunk_lo[chunk_pos];
      global_scores[out] = chunk_scores[chunk_pos];
      ++chunk_pos;
    }
  }
  *global_count = (uint32_t)output_count;
}

extern "C" __global__ void maxsim_score_rows_batch(
    const float* query_tokens,
    const float* query_norms,
    const uint32_t* query_offsets,
    int query_rows,
    const float* doc_tokens,
    const float* doc_norms,
    const uint32_t* row_offsets,
    const uint8_t* candidate_mask,
    int row_count,
    int dim,
    float* row_scores) {
  const int row = (int)blockIdx.x;
  const int query_row = (int)blockIdx.y;
  if (row >= row_count || query_row >= query_rows) return;
  float* output = row_scores + ((int64_t)query_row * row_count);
  if (candidate_mask[row] == 0) {
    if (threadIdx.x == 0) output[row] = -INFINITY;
    return;
  }
  const uint32_t token_start = row_offsets[row];
  const uint32_t token_end = row_offsets[row + 1];
  const uint32_t query_start = query_offsets[query_row];
  const uint32_t query_end = query_offsets[query_row + 1];
  __shared__ float partial[CALYX_MAXSIM_THREADS];
  float score = 0.0f;
  for (uint32_t q = query_start; q < query_end; ++q) {
    const float* query = query_tokens + ((int64_t)q * dim);
    const float query_norm = query_norms[q];
    float best = -INFINITY;
    for (uint32_t token = token_start + threadIdx.x; token < token_end;
         token += blockDim.x) {
      const float* doc = doc_tokens + ((int64_t)token * dim);
      float dot = 0.0f;
      for (int col = 0; col < dim; ++col) {
        dot += query[col] * doc[col];
      }
      const float doc_norm = doc_norms[token];
      const float cosine =
          (query_norm == 0.0f || doc_norm == 0.0f)
              ? 0.0f
              : dot / (query_norm * doc_norm);
      best = fmaxf(best, cosine);
    }
    partial[threadIdx.x] = best;
    __syncthreads();
    for (int stride = blockDim.x / 2; stride > 0; stride >>= 1) {
      if (threadIdx.x < stride) {
        partial[threadIdx.x] = fmaxf(partial[threadIdx.x], partial[threadIdx.x + stride]);
      }
      __syncthreads();
    }
    if (threadIdx.x == 0) score += partial[0];
    __syncthreads();
  }
  if (threadIdx.x == 0) output[row] = score;
}

extern "C" __global__ void maxsim_chunk_topk_batch(
    const float* row_scores,
    const uint64_t* row_id_hi,
    const uint64_t* row_id_lo,
    const uint8_t* candidate_mask,
    int row_count,
    int query_rows,
    int k,
    uint64_t* out_hi,
    uint64_t* out_lo,
    float* out_scores,
    uint32_t* out_count) {
  if (threadIdx.x != 0) return;
  const int query_row = (int)blockIdx.x;
  if (query_row >= query_rows) return;
  const float* scores = row_scores + ((int64_t)query_row * row_count);
  uint64_t* query_hi = out_hi + ((int64_t)query_row * k);
  uint64_t* query_lo = out_lo + ((int64_t)query_row * k);
  float* query_scores = out_scores + ((int64_t)query_row * k);
  int count = 0;
  for (int row = 0; row < row_count; ++row) {
    if (candidate_mask[row] == 0) continue;
    const float score = scores[row];
    const uint64_t hi = row_id_hi[row];
    const uint64_t lo = row_id_lo[row];
    int insert = count;
    while (insert > 0 &&
           better(score, hi, lo, query_scores[insert - 1], query_hi[insert - 1],
                  query_lo[insert - 1])) {
      if (insert < k) {
        query_scores[insert] = query_scores[insert - 1];
        query_hi[insert] = query_hi[insert - 1];
        query_lo[insert] = query_lo[insert - 1];
      }
      --insert;
    }
    if (insert < k) {
      query_scores[insert] = score;
      query_hi[insert] = hi;
      query_lo[insert] = lo;
    }
    if (count < k) ++count;
  }
  out_count[query_row] = (uint32_t)count;
}

extern "C" __global__ void maxsim_merge_topk_batch(
    const uint64_t* chunk_hi,
    const uint64_t* chunk_lo,
    const float* chunk_scores,
    const uint32_t* chunk_count,
    uint64_t* global_hi,
    uint64_t* global_lo,
    float* global_scores,
    uint32_t* global_count,
    int query_rows,
    int k) {
  if (threadIdx.x != 0) return;
  const int query_row = (int)blockIdx.x;
  if (query_row >= query_rows) return;
  const int64_t offset = (int64_t)query_row * k;
  const uint64_t* chunk_hi_row = chunk_hi + offset;
  const uint64_t* chunk_lo_row = chunk_lo + offset;
  const float* chunk_scores_row = chunk_scores + offset;
  uint64_t* global_hi_row = global_hi + offset;
  uint64_t* global_lo_row = global_lo + offset;
  float* global_scores_row = global_scores + offset;

  __shared__ uint64_t old_hi[CALYX_MAXSIM_MAX_K];
  __shared__ uint64_t old_lo[CALYX_MAXSIM_MAX_K];
  __shared__ float old_scores[CALYX_MAXSIM_MAX_K];
  const int old_count = (int)global_count[query_row];
  const int take_chunk = (int)chunk_count[query_row];
  for (int i = 0; i < old_count; ++i) {
    old_hi[i] = global_hi_row[i];
    old_lo[i] = global_lo_row[i];
    old_scores[i] = global_scores_row[i];
  }
  const int output_count = min(k, old_count + take_chunk);
  int old_pos = 0;
  int chunk_pos = 0;
  for (int out = 0; out < output_count; ++out) {
    const bool use_old =
        old_pos < old_count &&
        (chunk_pos >= take_chunk ||
         better(old_scores[old_pos], old_hi[old_pos], old_lo[old_pos],
                chunk_scores_row[chunk_pos], chunk_hi_row[chunk_pos],
                chunk_lo_row[chunk_pos]));
    if (use_old) {
      global_hi_row[out] = old_hi[old_pos];
      global_lo_row[out] = old_lo[old_pos];
      global_scores_row[out] = old_scores[old_pos];
      ++old_pos;
    } else {
      global_hi_row[out] = chunk_hi_row[chunk_pos];
      global_lo_row[out] = chunk_lo_row[chunk_pos];
      global_scores_row[out] = chunk_scores_row[chunk_pos];
      ++chunk_pos;
    }
  }
  global_count[query_row] = (uint32_t)output_count;
}
