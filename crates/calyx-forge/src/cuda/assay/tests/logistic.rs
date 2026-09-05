use super::*;

#[test]
fn logistic_summaries_match_cpu_oracle() -> Result<()> {
    let _guard = test_lock();
    let ctx = init_cuda(0, false)?;
    let (samples, labels, n, dim, train_offsets, train_indices, test_offsets, test_indices) =
        logistic_fixture();
    let expected = cpu_logistic_summaries(
        &samples,
        &labels,
        n,
        dim,
        &train_offsets,
        &train_indices,
        &test_offsets,
        &test_indices,
        96,
        0.35,
        1.0e-4,
    );
    let block_value_offsets = [0, samples.len() as i32];
    let block_feature_offsets = [0, dim as i32];
    let block_dims = [dim as i32];
    let block_scales = vec![1.0; train_offsets.len() - 1];
    let actual = logistic_summaries_host(
        &ctx,
        CudaLogisticDataset {
            samples: &samples,
            labels: &labels,
            rows: n,
            dim,
            block_value_offsets: &block_value_offsets,
            block_feature_offsets: &block_feature_offsets,
            block_dims: &block_dims,
            block_scales: &block_scales,
        },
        CudaLogisticSplits {
            train_offsets: &train_offsets,
            train_indices: &train_indices,
            test_offsets: &test_offsets,
            test_indices: &test_indices,
        },
        CudaLogisticConfig {
            max_iterations: 2_048,
            convergence_check_interval: 16,
            relative_parameter_tolerance: 1.0e-4,
            learning_rate: 0.35,
            l2_penalty: 1.0e-4,
        },
    )?;

    assert_close_vec("logistic bits", &actual.bits, &expected.bits, 1e-5);
    assert_close_vec(
        "logistic accuracy",
        &actual.accuracy,
        &expected.accuracy,
        0.0,
    );
    println!(
        "FORGE_LOGISTIC_SOT source=logistic_summaries_host_readback fits={} actual_bits={:?} actual_accuracy={:?} expected_bits={:?} expected_accuracy={:?}",
        actual.bits.len(),
        actual.bits,
        actual.accuracy,
        expected.bits,
        expected.accuracy
    );
    Ok(())
}

#[test]
fn logistic_summaries_fail_loud_on_edges() -> Result<()> {
    let _guard = test_lock();
    let ctx = init_cuda(0, false)?;
    let (samples, labels, n, dim, train_offsets, train_indices, test_offsets, test_indices) =
        logistic_fixture();
    let block_value_offsets = [0, samples.len() as i32];
    let block_feature_offsets = [0, dim as i32];
    let block_dims = [dim as i32];
    let block_scales = vec![1.0; train_offsets.len() - 1];

    let empty = logistic_summaries_host(
        &ctx,
        CudaLogisticDataset {
            samples: &[],
            labels: &[],
            rows: 0,
            dim,
            block_value_offsets: &[0, 0],
            block_feature_offsets: &[0, dim as i32],
            block_dims: &[dim as i32],
            block_scales: &[1.0],
        },
        CudaLogisticSplits {
            train_offsets: &[0, 0],
            train_indices: &[],
            test_offsets: &[0, 0],
            test_indices: &[],
        },
        CudaLogisticConfig {
            max_iterations: 2_048,
            convergence_check_interval: 16,
            relative_parameter_tolerance: 1.0e-4,
            learning_rate: 0.35,
            l2_penalty: 1.0e-4,
        },
    )
    .expect_err("empty logistic input must fail");
    assert!(matches!(empty, ForgeError::ShapeMismatch { .. }));
    println!("FORGE_LOGISTIC_EDGE empty before=n0_dim{dim} after={empty}");

    let mut nonfinite = samples.clone();
    nonfinite[5] = f32::NAN;
    let nan = logistic_summaries_host(
        &ctx,
        CudaLogisticDataset {
            samples: &nonfinite,
            labels: &labels,
            rows: n,
            dim,
            block_value_offsets: &block_value_offsets,
            block_feature_offsets: &block_feature_offsets,
            block_dims: &block_dims,
            block_scales: &block_scales,
        },
        CudaLogisticSplits {
            train_offsets: &train_offsets,
            train_indices: &train_indices,
            test_offsets: &test_offsets,
            test_indices: &test_indices,
        },
        CudaLogisticConfig {
            max_iterations: 2_048,
            convergence_check_interval: 16,
            relative_parameter_tolerance: 1.0e-4,
            learning_rate: 0.35,
            l2_penalty: 1.0e-4,
        },
    )
    .expect_err("non-finite logistic sample must fail");
    assert!(matches!(nan, ForgeError::NumericalInvariant { .. }));
    println!("FORGE_LOGISTIC_EDGE nonfinite before=nan_at_flat_5 after={nan}");

    let mut bad_labels = labels.clone();
    bad_labels[3] = 2;
    let invalid_label = logistic_summaries_host(
        &ctx,
        CudaLogisticDataset {
            samples: &samples,
            labels: &bad_labels,
            rows: n,
            dim,
            block_value_offsets: &block_value_offsets,
            block_feature_offsets: &block_feature_offsets,
            block_dims: &block_dims,
            block_scales: &block_scales,
        },
        CudaLogisticSplits {
            train_offsets: &train_offsets,
            train_indices: &train_indices,
            test_offsets: &test_offsets,
            test_indices: &test_indices,
        },
        CudaLogisticConfig {
            max_iterations: 2_048,
            convergence_check_interval: 16,
            relative_parameter_tolerance: 1.0e-4,
            learning_rate: 0.35,
            l2_penalty: 1.0e-4,
        },
    )
    .expect_err("invalid binary label must fail");
    assert!(matches!(invalid_label, ForgeError::ShapeMismatch { .. }));
    println!("FORGE_LOGISTIC_EDGE invalid_label before=row3_label2 after={invalid_label}");

    let over_dim = vec![0.0_f32; 1025];
    let over_dim_err = logistic_summaries_host(
        &ctx,
        CudaLogisticDataset {
            samples: &over_dim,
            labels: &[0],
            rows: 1,
            dim: 1025,
            block_value_offsets: &[0, 1025],
            block_feature_offsets: &[0, 1025],
            block_dims: &[1025],
            block_scales: &[1.0],
        },
        CudaLogisticSplits {
            train_offsets: &[0, 1],
            train_indices: &[0],
            test_offsets: &[0, 1],
            test_indices: &[0],
        },
        CudaLogisticConfig {
            max_iterations: 2_048,
            convergence_check_interval: 16,
            relative_parameter_tolerance: 1.0e-4,
            learning_rate: 0.35,
            l2_penalty: 1.0e-4,
        },
    )
    .expect_err("dim > 1024 must fail before launch");
    assert!(matches!(over_dim_err, ForgeError::ShapeMismatch { .. }));
    println!("FORGE_LOGISTIC_EDGE max_dim before=1025 after={over_dim_err}");
    Ok(())
}
