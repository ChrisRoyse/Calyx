use calyx_core::SparseEntry;

pub(super) fn project_sparse(
    lens: &str,
    row_idx: usize,
    sparse_dim: u32,
    entries: Vec<SparseEntry>,
) -> Result<Vec<f32>, String> {
    let mut data = vec![0.0_f32; sparse_dim as usize];
    for entry in entries {
        let Some(value) = data.get_mut(entry.idx as usize) else {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_SPARSE_INDEX_OUT_OF_RANGE: lens={lens} row={row_idx} idx={} dim={sparse_dim}",
                entry.idx
            ));
        };
        *value = entry.val;
    }
    Ok(data)
}

pub(super) fn project_multi(
    lens: &str,
    row_idx: usize,
    token_dim: u32,
    tokens: Vec<Vec<f32>>,
) -> Result<Vec<f32>, String> {
    let token_dim = token_dim as usize;
    if token_dim == 0 || tokens.is_empty() {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_EMPTY_MULTI: lens={lens} row={row_idx} token_dim={token_dim} tokens={}",
            tokens.len()
        ));
    }
    let mut out = vec![0.0_f32; token_dim];
    for (token_idx, token) in tokens.iter().enumerate() {
        if token.len() != token_dim {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_MULTI_DIM_MISMATCH: lens={lens} row={row_idx} token={token_idx} len={} expected={token_dim}",
                token.len()
            ));
        }
        for (axis, value) in token.iter().enumerate() {
            if !value.is_finite() {
                return Err(format!(
                    "CALYX_FSV_ASSAY_CORPUS_BUILD_MULTI_NON_FINITE: lens={lens} row={row_idx} token={token_idx} axis={axis}"
                ));
            }
            out[axis] += *value;
        }
    }
    let denom = tokens.len() as f32;
    for value in &mut out {
        *value /= denom;
    }
    Ok(out)
}
