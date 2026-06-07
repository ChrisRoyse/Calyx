#include <math.h>
#include <stdint.h>

static __device__ __forceinline__ float mxfp4_scale(unsigned char scale) {
    return ldexpf(1.0f, ((int)scale) - 127);
}

static __device__ __forceinline__ unsigned char mxfp4_nibble(
    const unsigned char *codes,
    int idx) {
    const unsigned char byte = codes[(idx >> 1)];
    return (idx & 1) == 0 ? (byte & 0x0f) : (byte >> 4);
}

static __device__ __forceinline__ float mxfp4_decode(
    const unsigned char *codes,
    const unsigned char *scales,
    int linear_idx) {
    const int block = linear_idx >> 5;
    const int offset = linear_idx & 31;
    const unsigned char code = mxfp4_nibble(codes + block * 16, offset);
    if (code == 15) {
        return 0.0f;
    }
    const int clamped = code > 14 ? 14 : code;
    const int signed_code = clamped - 7;
    return ((float)signed_code) * mxfp4_scale(scales[block]) * (1.0f / 7.0f);
}

extern "C" __global__ __launch_bounds__(128) void gemm_mxfp4_fp32_accum_kernel(
    const unsigned char *a_codes,
    const unsigned char *a_scales,
    const unsigned char *b_codes,
    const unsigned char *b_scales,
    int m,
    int k,
    int n,
    float *out) {
    const int cell = blockIdx.x * blockDim.x + threadIdx.x;
    const int total = m * n;
    if (cell >= total) {
        return;
    }

    const int row = cell % m;
    const int col = cell / m;
    float sum = 0.0f;
    for (int depth = 0; depth < k; ++depth) {
        const int a_idx = depth * m + row;
        const int b_idx = col * k + depth;
        sum += mxfp4_decode(a_codes, a_scales, a_idx)
            * mxfp4_decode(b_codes, b_scales, b_idx);
    }
    out[cell] = isfinite(sum) ? sum : NAN;
}
