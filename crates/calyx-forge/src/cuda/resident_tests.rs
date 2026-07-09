use super::test_lock;
use crate::cuda::{cosine_resident_host, init_cuda, upload_candidate_block};
use crate::{BlockId, ForgeError, Result};

fn row_major(rows: &[[f32; 3]]) -> Vec<f32> {
    rows.iter().flat_map(|row| row.iter().copied()).collect()
}

#[test]
fn resident_candidate_block_reuses_uploaded_corpus_for_two_queries() -> Result<()> {
    let _guard = test_lock();
    let ctx = init_cuda(0, false)?;
    let candidates = row_major(&[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
    let block = upload_candidate_block(&ctx, BlockId(1341), &candidates, 3)?;
    let mut out = vec![0.0; 2];

    cosine_resident_host(&ctx, &[1.0, 0.0, 0.0], &block, &mut out)?;
    assert!((out[0] - 1.0).abs() <= 1e-5);
    assert!(out[1].abs() <= 1e-5);

    cosine_resident_host(&ctx, &[0.0, 1.0, 0.0], &block, &mut out)?;
    assert!(out[0].abs() <= 1e-5);
    assert!((out[1] - 1.0).abs() <= 1e-5);

    println!(
        "CUDA_RESIDENT_CANDIDATES block_id={} dim={} n_cands={} q2={:?}",
        block.block_id().0,
        block.dim(),
        block.n_cands(),
        out
    );
    Ok(())
}

#[test]
fn resident_candidate_block_fail_closed_edges() -> Result<()> {
    let _guard = test_lock();
    let ctx = init_cuda(0, false)?;
    let err = match upload_candidate_block(&ctx, BlockId(1341), &[1.0, f32::NAN], 2) {
        Ok(_) => panic!("non-finite resident corpus must fail at upload"),
        Err(err) => err,
    };
    assert!(matches!(err, ForgeError::NumericalInvariant { .. }));

    let candidates = row_major(&[[1.0, 0.0, 0.0]]);
    let block = upload_candidate_block(&ctx, BlockId(1342), &candidates, 3)?;
    let mut out = vec![0.0; 1];
    let err = cosine_resident_host(&ctx, &[f32::INFINITY, 0.0, 0.0], &block, &mut out)
        .expect_err("non-finite query must fail per request");
    assert!(matches!(err, ForgeError::NumericalInvariant { .. }));
    let err = cosine_resident_host(&ctx, &[1.0, 0.0, 0.0], &block, &mut [])
        .expect_err("output shape mismatch must fail closed");
    assert!(matches!(err, ForgeError::ShapeMismatch { .. }));

    println!("CUDA_RESIDENT_EDGES upload_query_shape_fail_closed");
    Ok(())
}
