use super::*;

#[test]
fn shape_parser_accepts_dense_sparse_and_multi() {
    assert_eq!(parse_shape("Dense(768)").unwrap(), SlotShape::Dense(768));
    assert_eq!(
        parse_shape("Sparse(30522)").unwrap(),
        SlotShape::Sparse(30522)
    );
    assert_eq!(
        parse_shape("Multi(32)").unwrap(),
        SlotShape::Multi { token_dim: 32 }
    );
}

#[test]
fn shape_parser_rejects_zero_and_unknown_kind() {
    assert_eq!(
        parse_shape("Dense(0)").unwrap_err().code(),
        "CALYX_CLI_USAGE_ERROR"
    );
    assert_eq!(
        parse_shape("Tensor(32)").unwrap_err().code(),
        "CALYX_CLI_USAGE_ERROR"
    );
}

#[test]
fn algorithmic_sparse_and_multi_lenses_measure_real_vectors() {
    let sparse = build_lens(
        "keywords",
        "algorithmic:sparse-keywords",
        None,
        None,
        Some("Sparse(64)"),
        Some("text"),
    )
    .unwrap();
    let multi = build_lens(
        "tokens",
        "algorithmic:token-hash",
        None,
        None,
        Some("Multi(4)"),
        Some("text"),
    )
    .unwrap();
    let mut registry = Registry::new();
    let sparse_id = sparse.lens_id;
    let multi_id = multi.lens_id;
    sparse.register(&mut registry).unwrap();
    multi.register(&mut registry).unwrap();

    assert!(matches!(
        registry
            .measure(
                sparse_id,
                &Input::new(Modality::Text, b"alpha beta".to_vec())
            )
            .unwrap(),
        SlotVector::Sparse { dim: 64, .. }
    ));
    assert!(matches!(
        registry
            .measure(
                multi_id,
                &Input::new(Modality::Text, b"alpha beta".to_vec())
            )
            .unwrap(),
        SlotVector::Multi { token_dim: 4, .. }
    ));
}

#[test]
fn algorithmic_shape_mismatch_is_calyx_dim_error() {
    let err = build_lens(
        "gte",
        "algorithmic",
        None,
        None,
        Some("Dense(8)"),
        Some("text"),
    )
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_LENS_DIM_MISMATCH");
}
