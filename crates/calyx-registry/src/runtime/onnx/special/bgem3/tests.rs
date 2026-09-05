#[cfg(feature = "cuda")]
use std::fs;
#[cfg(feature = "cuda")]
use std::time::Instant;

#[cfg(feature = "cuda")]
use calyx_core::Lens;
#[cfg(feature = "cuda")]
use serde_json::json;

#[cfg(not(feature = "cuda"))]
use super::SharedBgem3Backend;
#[cfg(feature = "cuda")]
use super::*;
#[cfg(feature = "cuda")]
use crate::Registry;

#[test]
fn shared_backend_layout_stays_indirect() {
    let actual = std::mem::size_of::<SharedBgem3Backend>();
    let maximum = 2 * std::mem::size_of::<usize>();
    println!("BGE_M3_BACKEND_LAYOUT source=size_of actual_bytes={actual} maximum_bytes={maximum}");
    assert!(
        actual <= maximum,
        "SharedBgem3Backend grew to {actual} bytes; keep heavyweight sessions behind deliberate indirection (maximum {maximum} bytes)"
    );
}

/// Real-model full-state verification for #1497 and #1545. The ignored test
/// is run manually against the immutable joint BGE-M3 ONNX-CUDA artifact.
#[test]
#[ignore = "requires CUDA, CALYX_BGE_M3_ARTIFACT_ROOT, and CALYX_FSV_ROOT"]
#[cfg(feature = "cuda")]
fn issue1497_issue1545_real_bgem3_group_is_one_session_and_one_forward() -> Result<()> {
    let fsv_root = PathBuf::from(
        std::env::var("CALYX_FSV_ROOT")
            .expect("CALYX_FSV_ROOT must name the durable evidence directory"),
    );
    let artifact_root = PathBuf::from(
        std::env::var("CALYX_BGE_M3_ARTIFACT_ROOT")
            .expect("CALYX_BGE_M3_ARTIFACT_ROOT must name the immutable joint artifact"),
    );
    let dense_spec =
        crate::lens_spec_from_manifest_path(artifact_root.join("manifest-dense.json"))?;
    let sparse_spec =
        crate::lens_spec_from_manifest_path(artifact_root.join("manifest-sparse.json"))?;
    let colbert_spec =
        crate::lens_spec_from_manifest_path(artifact_root.join("manifest-colbert.json"))?;
    let dense = FastembedBgem3Lens::from_lens_spec(&dense_spec)?;
    let sparse = FastembedBgem3Lens::from_lens_spec(&sparse_spec)?;
    let colbert = FastembedBgem3Lens::from_lens_spec(&colbert_spec)?;
    assert_eq!(dense.runtime_name(), "onnx-bgem3-dense");
    assert_eq!(sparse.runtime_name(), "onnx-bgem3-sparse");
    assert_eq!(colbert.runtime_name(), "onnx-bgem3-colbert");
    let group_key = dense.measurement_group_key()?.expect("dense group key");
    assert_eq!(sparse.measurement_group_key()?, Some(group_key));
    assert_eq!(colbert.measurement_group_key()?, Some(group_key));
    assert_eq!(dense.runtime_stats().session_initializations, 1);
    assert_eq!(sparse.runtime_stats().forward_calls, 0);

    let mut registry = Registry::new();
    let dense_id = registry.register_frozen(dense.clone(), dense.contract().clone())?;
    let sparse_id = registry.register_frozen(sparse.clone(), sparse.contract().clone())?;
    let colbert_id = registry.register_frozen(colbert.clone(), colbert.contract().clone())?;
    let lens_ids = [dense_id, sparse_id, colbert_id];

    // Edge 1: empty input is a no-op and does not execute the graph.
    eprintln!("ISSUE1545_EDGE empty before={:?}", dense.runtime_stats());
    let empty = registry.measure_grouped_batch(&lens_ids, &[])?;
    assert!(empty.values().all(Vec::is_empty));
    assert_eq!(dense.runtime_stats().forward_calls, 0);
    eprintln!("ISSUE1545_EDGE empty after={:?}", dense.runtime_stats());

    let inputs = vec![
        Input::new(Modality::Text, b"red fox crosses a frozen river".to_vec()),
        Input::new(
            Modality::Text,
            b"graph storage validates durable state".to_vec(),
        ),
    ];
    eprintln!(
        "ISSUE1545_HAPPY before={:?} inputs={}",
        dense.runtime_stats(),
        inputs.len()
    );
    let measured = registry.measure_grouped_batch(&lens_ids, &inputs)?;
    let grouped_after = dense.runtime_stats();
    assert_eq!(
        grouped_after.forward_calls, 1,
        "three outputs require one forward"
    );
    assert_eq!(
        sparse.runtime_stats(),
        grouped_after,
        "lenses share one runtime"
    );
    assert_eq!(
        colbert.runtime_stats(),
        grouped_after,
        "lenses share one runtime"
    );
    eprintln!("ISSUE1545_HAPPY after={grouped_after:?}");

    // Compare against the previous three-forward behavior using the same
    // real session and inputs. Exact equality is expected because only
    // output selection changed; the model graph and conversion code did not.
    let independent = [
        (dense_id, dense.measure_batch(&inputs)?),
        (sparse_id, sparse.measure_batch(&inputs)?),
        (colbert_id, colbert.measure_batch(&inputs)?),
    ];
    for (lens_id, vectors) in independent {
        assert_eq!(
            measured.get(&lens_id),
            Some(&vectors),
            "fused output must equal the prior independent output for {lens_id}"
        );
    }
    let parity_after = dense.runtime_stats();
    assert_eq!(parity_after.forward_calls, 4);
    eprintln!(
        "ISSUE1545_PARITY grouped_forward_calls={} independent_forward_calls=3 final_forward_calls={} outputs_exact=true",
        grouped_after.forward_calls, parity_after.forward_calls
    );

    // Every non-empty requested subset performs exactly one forward and
    // converts only the requested real model heads.
    let subsets = [
        vec![dense_id],
        vec![sparse_id],
        vec![colbert_id],
        vec![dense_id, sparse_id],
        vec![dense_id, colbert_id],
        vec![sparse_id, colbert_id],
        vec![dense_id, sparse_id, colbert_id],
    ];
    let mut subset_evidence = Vec::new();
    for subset in &subsets {
        let before = dense.runtime_stats();
        let outputs = registry.measure_grouped_batch(subset, &inputs)?;
        let after = dense.runtime_stats();
        assert_eq!(outputs.len(), subset.len());
        assert!(subset.iter().all(|lens_id| outputs.contains_key(lens_id)));
        assert_eq!(after.forward_calls, before.forward_calls + 1);
        assert_eq!(after.tokenization_calls, before.tokenization_calls + 1);
        assert_eq!(
            after.dense_conversions,
            before.dense_conversions + u64::from(subset.contains(&dense_id))
        );
        assert_eq!(
            after.sparse_conversions,
            before.sparse_conversions + u64::from(subset.contains(&sparse_id))
        );
        assert_eq!(
            after.colbert_conversions,
            before.colbert_conversions + u64::from(subset.contains(&colbert_id))
        );
        subset_evidence.push(json!({
            "lens_ids": subset.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "result_heads": outputs.len(),
            "forward_delta": 1,
            "dense_conversion_delta": u64::from(subset.contains(&dense_id)),
            "sparse_conversion_delta": u64::from(subset.contains(&sparse_id)),
            "colbert_conversion_delta": u64::from(subset.contains(&colbert_id)),
        }));
    }

    // A runtime batch limit of one preserves the two exact inputs and
    // turns them into exactly two fused forwards, not six head forwards.
    let before_chunked = dense.runtime_stats();
    let chunked =
        crate::measure_registry_group_with_runtime_limit(&registry, &lens_ids, &inputs, Some(1))?;
    let after_chunked = dense.runtime_stats();
    assert_eq!(
        after_chunked.forward_calls,
        before_chunked.forward_calls + 2
    );
    assert_eq!(
        after_chunked.tokenization_calls,
        before_chunked.tokenization_calls + 2
    );
    assert_eq!(
        after_chunked.dense_conversions,
        before_chunked.dense_conversions + 2
    );
    assert_eq!(
        after_chunked.sparse_conversions,
        before_chunked.sparse_conversions + 2
    );
    assert_eq!(
        after_chunked.colbert_conversions,
        before_chunked.colbert_conversions + 2
    );
    let mut independent_chunked = BTreeMap::new();
    for lens_id in lens_ids {
        let mut vectors = Vec::with_capacity(inputs.len());
        for input in &inputs {
            vectors.extend(registry.measure_batch(lens_id, std::slice::from_ref(input))?);
        }
        independent_chunked.insert(lens_id, vectors);
    }
    assert!(
        chunked == independent_chunked,
        "two fused one-item forwards must exactly match six independent one-item forwards"
    );

    // The complete runtime identity splits otherwise-identical projections
    // when their effective batch boundaries differ, without loading a
    // second model merely to inspect the key.
    let LensRuntime::FastembedBgem3 {
        model_id, files, ..
    } = &dense_spec.runtime
    else {
        return Err(CalyxError::lens_unreachable(
            "dense BGE-M3 manifest did not produce a BGE-M3 runtime",
        ));
    };
    let mut incompatible_spec = dense_spec.clone();
    incompatible_spec.max_batch = Some(1);
    let incompatible = device::DeviceBgem3Runtime::prepare(&incompatible_spec, model_id, files)?;
    let incompatible_key = device_runtime_key(&incompatible)?;
    assert_ne!(incompatible_key, group_key);

    // Invalid modality and UTF-8 fail before the session runs.
    let before_invalid = dense.runtime_stats();
    let modality_error = registry
        .measure_grouped_batch(&lens_ids, &[Input::new(Modality::Image, vec![1, 2, 3])])
        .unwrap_err();
    assert_eq!(dense.runtime_stats(), before_invalid);
    let utf8_error = registry
        .measure_grouped_batch(&lens_ids, &[Input::new(Modality::Text, vec![0xff, 0xfe])])
        .unwrap_err();
    assert_eq!(dense.runtime_stats(), before_invalid);

    // Two simultaneous callers share the same session. The mutex keeps
    // ORT runs at one active invocation while both callers receive exact
    // output parity and neither deadlocks.
    let before_concurrent = dense.runtime_stats();
    let (concurrent_left, concurrent_right) = std::thread::scope(|scope| -> Result<_> {
        let left = scope.spawn(|| registry.measure_grouped_batch(&lens_ids, &inputs));
        let right = scope.spawn(|| registry.measure_grouped_batch(&lens_ids, &inputs));
        let left = left
            .join()
            .map_err(|_| CalyxError::lens_unreachable("left BGE-M3 caller panicked"))??;
        let right = right
            .join()
            .map_err(|_| CalyxError::lens_unreachable("right BGE-M3 caller panicked"))??;
        Ok((left, right))
    })?;
    let concurrent_after = dense.runtime_stats();
    assert_eq!(concurrent_left, measured);
    assert_eq!(concurrent_right, measured);
    assert_eq!(
        concurrent_after.forward_calls,
        before_concurrent.forward_calls + 2
    );
    assert_eq!(concurrent_after.max_concurrent_runs, 1);

    // Real short/long batches provide docs/s and exact output hashes for
    // the requested 1/8/32 scale points without changing batch semantics.
    let mut benchmark_evidence = Vec::new();
    let mut short_throughput = BTreeMap::new();
    for (batch, input_kind) in [
        (1_usize, "short"),
        (8, "short"),
        (32, "short"),
        (1, "long"),
        (8, "long"),
        (32, "long"),
    ] {
        let body = if input_kind == "long" {
            "legal precedent contract evidence retrieval ".repeat(80)
        } else {
            "legal contract evidence".to_string()
        };
        let benchmark_inputs = (0..batch)
            .map(|index| Input::new(Modality::Text, format!("{index}:{body}").into_bytes()))
            .collect::<Vec<_>>();
        let before = dense.runtime_stats();
        let started = Instant::now();
        let outputs = registry.measure_grouped_batch(&lens_ids, &benchmark_inputs)?;
        let elapsed = started.elapsed();
        let after = dense.runtime_stats();
        assert_eq!(after.forward_calls, before.forward_calls + 1);
        assert_eq!(outputs.len(), 3);
        let encoded = serde_json::to_vec(&outputs).map_err(|error| {
            CalyxError::lens_unreachable(format!("encode BGE-M3 benchmark output: {error}"))
        })?;
        let docs_per_second = batch as f64 / elapsed.as_secs_f64();
        if input_kind == "short" {
            short_throughput.insert(batch, docs_per_second);
        }
        benchmark_evidence.push(json!({
            "batch": batch,
            "input_kind": input_kind,
            "elapsed_us": elapsed.as_micros(),
            "docs_per_second": docs_per_second,
            "output_blake3": blake3::hash(&encoded).to_hex().to_string(),
            "output_bytes": encoded.len(),
            "forward_delta": 1,
        }));
    }
    let short_batch_1 = short_throughput[&1];
    let short_batch_32 = short_throughput[&32];
    let short_batch_32_speedup = short_batch_32 / short_batch_1;
    assert!(
        short_batch_32_speedup > 1.0,
        "32-item CUDA batch throughput {short_batch_32} docs/s did not exceed scalar {short_batch_1} docs/s"
    );
    let final_stats = dense.runtime_stats();

    // Edge 2: an invalid duplicate request fails before graph execution.
    eprintln!("ISSUE1545_EDGE duplicate before={final_stats:?}");
    let duplicate = registry
        .measure_grouped_batch(&[dense_id, dense_id], &inputs)
        .unwrap_err();
    assert_eq!(duplicate.code, "CALYX_LENS_DIM_MISMATCH");
    assert_eq!(dense.runtime_stats(), final_stats);
    eprintln!(
        "ISSUE1545_EDGE duplicate after={:?} error={}:{}",
        dense.runtime_stats(),
        duplicate.code,
        duplicate.message
    );

    // Edge 3: zero runtime chunk limits fail before graph execution.
    eprintln!(
        "ISSUE1545_EDGE zero_limit before={:?}",
        dense.runtime_stats()
    );
    let zero_limit =
        crate::measure_registry_group_with_runtime_limit(&registry, &lens_ids, &inputs, Some(0))
            .unwrap_err();
    assert_eq!(zero_limit.code, "CALYX_LENS_UNREACHABLE");
    assert_eq!(dense.runtime_stats(), final_stats);
    eprintln!(
        "ISSUE1545_EDGE zero_limit after={:?} error={}:{}",
        dense.runtime_stats(),
        zero_limit.code,
        zero_limit.message
    );

    let evidence = json!({
        "source_of_truth": "real BGE-M3 output vectors written by this test",
        "artifact_root": artifact_root,
        "runtime_names": [dense.runtime_name(), sparse.runtime_name(), colbert.runtime_name()],
        "runtime": {
            "session_initializations": grouped_after.session_initializations,
            "grouped_forward_calls": grouped_after.forward_calls,
            "parity_final_forward_calls": parity_after.forward_calls,
            "final_stats": {
                "forward_calls": final_stats.forward_calls,
                "tokenization_calls": final_stats.tokenization_calls,
                "dense_conversions": final_stats.dense_conversions,
                "sparse_conversions": final_stats.sparse_conversions,
                "colbert_conversions": final_stats.colbert_conversions,
                "max_concurrent_runs": final_stats.max_concurrent_runs,
            },
            "group_key": group_key
                .as_bytes()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        },
        "inputs": inputs,
        "subsets": subset_evidence,
        "runtime_limit_one": {
            "inputs": 2,
            "forward_delta": after_chunked.forward_calls - before_chunked.forward_calls,
        },
        "incompatible_batch_group_key": incompatible_key
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "fail_closed": {
            "invalid_modality": { "code": modality_error.code, "message": modality_error.message },
            "invalid_utf8": { "code": utf8_error.code, "message": utf8_error.message },
        },
        "concurrency": {
            "callers": 2,
            "forward_delta": concurrent_after.forward_calls - before_concurrent.forward_calls,
            "max_concurrent_runs": concurrent_after.max_concurrent_runs,
            "outputs_exact": true,
        },
        "throughput": {
            "short_batch_1_docs_per_second": short_batch_1,
            "short_batch_32_docs_per_second": short_batch_32,
            "short_batch_32_speedup": short_batch_32_speedup,
        },
        "benchmarks": benchmark_evidence,
        "results": measured
            .iter()
            .map(|(lens_id, vectors)| json!({
                "lens_id": lens_id.to_string(),
                "vectors": vectors,
            }))
            .collect::<Vec<_>>(),
    });
    let evidence_path = fsv_root.join("issue1545-bgem3-grouped-results.json");
    fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).expect("serialize real BGE-M3 vectors"),
    )
    .expect("write BGE-M3 FSV evidence");
    let persisted: serde_json::Value = serde_json::from_slice(
        &fs::read(&evidence_path).expect("read BGE-M3 FSV evidence back from disk"),
    )
    .expect("parse BGE-M3 FSV evidence after readback");
    assert_eq!(persisted["runtime"]["grouped_forward_calls"], 1);
    assert_eq!(persisted["results"].as_array().map(Vec::len), Some(3));
    eprintln!(
        "ISSUE1545_SOURCE_OF_TRUTH path={} persisted_results={} persisted_grouped_forward_calls={}",
        evidence_path.display(),
        persisted["results"].as_array().map(Vec::len).unwrap_or(0),
        persisted["runtime"]["grouped_forward_calls"]
    );
    Ok(())
}
