use calyx_core::{Result, SlotId, SlotVector};
use calyx_forge::QuantLevel;
use calyx_sextant::{CALYX_SEXTANT_VECTOR_SHAPE, HnswIndex, QuantConfig, SextantIndex};

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::cx_u8_fill as cx;

#[test]
fn hnsw_turboquant_prepares_rows_and_exact_reranks() -> Result<()> {
    let mut quantized = HnswIndex::new(SlotId::new(8), 4, 42)
        .with_quant(QuantConfig::turboquant(QuantLevel::Bits3p5));
    let mut exact = HnswIndex::new(SlotId::new(8), 4, 42);
    let rows = rows();
    for (idx, values) in rows.iter().enumerate() {
        quantized.insert(cx(idx as u8), dense(values), idx as u64 + 1)?;
        exact.insert(cx(idx as u8), dense(values), idx as u64 + 1)?;
    }

    assert_eq!(quantized.turboquant_prepared_count(), rows.len());

    let query_values = vec![0.08, 0.93, 0.04, 0.02];
    let query = dense(&query_values);
    let quantized_hits = quantized.search(&query, 4, Some(rows.len()))?;
    let exact_hits = exact.brute_force(&query_values, 4);

    assert_eq!(
        ids(&quantized_hits),
        exact_hits.iter().map(|(id, _)| *id).collect::<Vec<_>>()
    );
    for (hit, (_, exact_score)) in quantized_hits.iter().zip(exact_hits.iter()) {
        assert!((hit.score - exact_score).abs() <= 1e-6);
    }

    quantized.insert(cx(3), dense(&[0.01, 0.99, 0.01, 0.01]), 99)?;
    assert_eq!(quantized.turboquant_prepared_count(), rows.len());
    println!(
        "hnsw_turboquant_prepared PASSED rows={} top={} score={:.6}",
        rows.len(),
        quantized_hits[0].cx_id,
        quantized_hits[0].score
    );
    Ok(())
}

#[test]
fn hnsw_turboquant_nonfinite_input_fails_closed() {
    let mut index = HnswIndex::new(SlotId::new(8), 4, 42)
        .with_quant(QuantConfig::turboquant(QuantLevel::Bits3p5));
    let err = index
        .insert(cx(1), dense(&[1.0, f32::NAN, 0.0, 0.0]), 1)
        .expect_err("non-finite TurboQuant row must fail closed");

    assert_eq!(err.code, CALYX_SEXTANT_VECTOR_SHAPE);
    assert_eq!(index.turboquant_prepared_count(), 0);
    println!("hnsw_turboquant_nonfinite PASSED {err}");
}

fn rows() -> Vec<Vec<f32>> {
    vec![
        vec![1.00, 0.00, 0.00, 0.00],
        vec![0.00, 1.00, 0.00, 0.00],
        vec![0.00, 0.00, 1.00, 0.00],
        vec![0.10, 0.90, 0.05, 0.00],
        vec![0.20, 0.70, 0.20, 0.00],
        vec![0.05, 0.05, 0.90, 0.10],
        vec![0.55, 0.45, 0.00, 0.00],
        vec![0.00, 0.10, 0.10, 0.80],
    ]
}

fn dense(values: &[f32]) -> SlotVector {
    SlotVector::Dense {
        dim: values.len() as u32,
        data: values.to_vec(),
    }
}

fn ids(hits: &[calyx_sextant::IndexSearchHit]) -> Vec<calyx_core::CxId> {
    hits.iter().map(|hit| hit.cx_id).collect()
}
