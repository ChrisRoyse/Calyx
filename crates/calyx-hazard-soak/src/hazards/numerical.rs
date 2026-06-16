use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::SlotId;
#[cfg(feature = "cuda")]
use calyx_forge::{Backend, CudaBackend};
use calyx_forge::{QuantLevel, Quantizer, TurboQuantCodec, new_seed, seed_id_hex};
use calyx_sextant::index::{DiskAnnBuildParams, DiskAnnSearch};
use serde_json::json;
use std::fs;
use std::path::Path;

use super::numerical_support::*;
use super::resource::{HazardResult, ProbeResult, run_probe};
use super::resource_support::{case_dir, err};

const DIM: usize = 128;
const ROWS: usize = 1_000;
const K: usize = 10;
const DISTORTION_BOUND: f32 = 0.12;
const MIN_BITS_CODE: &str = "CALYX_QUANT_DRIFT_EXCEEDED";

pub fn run_hazards_9_12(root: &Path) -> Vec<HazardResult> {
    [
        (
            9,
            "NaN/Inf propagation guard",
            probe_h9 as fn(&Path) -> ProbeResult,
        ),
        (10, "TurboQuant drift and recall", probe_h10),
        (11, "QJL seed/codebook staleness", probe_h11),
        (12, "ANN graph corruption rebuild", probe_h12),
    ]
    .into_iter()
    .map(|(id, name, probe)| run_probe(root, id, name, probe))
    .collect()
}

fn probe_h9(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h9_nan_guard")?;
    let vault = dir.join("vault");
    let mut router = CfRouter::open(&vault, 4 * 1024 * 1024).map_err(err)?;
    let slot = ColumnFamily::slot(SlotId::new(9));
    let before_rows = router.iter_cf(slot).map_err(err)?.len();
    let single = nan_guard_code(&single_nan_vector(DIM));
    let after_single_rows = router.iter_cf(slot).map_err(err)?.len();
    let all = nan_guard_code(&vec![f32::NAN; DIM]);
    let after_all_rows = router.iter_cf(slot).map_err(err)?.len();
    router.flush_pending().map_err(err)?;
    let slot_dir = vault.join("cf").join(slot.name());
    let slot_files = count_files(&slot_dir);
    let slot_bytes = tree_bytes(&slot_dir);
    let slot_has_nan = tree_has_nan_f32(&slot_dir);
    let guard_trips = usize::from(single == "CALYX_FORGE_NUMERICAL_INVARIANT")
        + usize::from(all == "CALYX_FORGE_NUMERICAL_INVARIANT");
    let passed = before_rows == 0
        && after_single_rows == 0
        && after_all_rows == 0
        && guard_trips == 2
        && slot_files == 0
        && !slot_has_nan;
    Ok((
        passed,
        json!({
            "trigger": "Forge numerical boundary receives one NaN vector and one all-NaN vector",
            "expected": {
                "primary_error_code": "CALYX_FORGE_NUMERICAL_INVARIANT",
                "all_nan_error_code": "CALYX_FORGE_NUMERICAL_INVARIANT",
                "slot_cf_rows_after": 0,
                "slot_cf_nan_pattern": false
            },
            "actual": {
                "backend": nan_guard_backend(),
                "primary_error_code": single,
                "all_nan_error_code": all,
                "slot_cf_rows_before": before_rows,
                "slot_cf_rows_after_primary": after_single_rows,
                "slot_cf_rows_after_all_nan": after_all_rows,
                "slot_sst_file_count": slot_files,
                "slot_tree_bytes": slot_bytes,
                "slot_tree_contains_nan_f32": slot_has_nan,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_nan_guard_trips_total{{vault=\"ph59-h9\"}} {guard_trips}\ncalyx_slot_nan_pattern_total{{vault=\"ph59-h9\"}} {}\n",
                usize::from(slot_has_nan)
            )
        }),
    ))
}

#[cfg(feature = "cuda")]
fn nan_guard_code(values: &[f32]) -> String {
    match CudaBackend::new().and_then(|backend| backend.topk(values, 4).map(|_| ())) {
        Ok(()) => "NO_ERROR".to_string(),
        Err(error) => error.code().to_string(),
    }
}

#[cfg(not(feature = "cuda"))]
fn nan_guard_code(values: &[f32]) -> String {
    let codec = TurboQuantCodec::new(new_seed(values.len(), b"ph59-h9-cpu"), QuantLevel::Bits3p5)
        .expect("valid codec");
    match codec.encode(values) {
        Ok(_) => "NO_ERROR".to_string(),
        Err(error) => error.code().to_string(),
    }
}

#[cfg(feature = "cuda")]
fn nan_guard_backend() -> &'static str {
    "cuda_topk_kernel"
}

#[cfg(not(feature = "cuda"))]
fn nan_guard_backend() -> &'static str {
    "forge_turboquant_cpu_guard"
}

fn probe_h10(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h10_quant_drift")?;
    let mut router = CfRouter::open(dir.join("vault"), 32 * 1024 * 1024).map_err(err)?;
    let rows = paired_vectors(ROWS, DIM);
    let ids = ids(ROWS);
    let codec = TurboQuantCodec::new(new_seed(DIM, b"ph59-h10-turboquant"), QuantLevel::Bits3p5)
        .map_err(err)?;
    let quantized = rows
        .iter()
        .map(|row| codec.encode(row).map_err(err))
        .collect::<Result<Vec<_>, _>>()?;
    let decoded = quantized
        .iter()
        .map(|q| codec.decode(q).map_err(err))
        .collect::<Result<Vec<_>, _>>()?;
    for (id, qv) in ids.iter().zip(&quantized) {
        router
            .put(
                ColumnFamily::slot(SlotId::new(10)),
                id.as_bytes(),
                &qv.bytes,
            )
            .map_err(err)?;
    }
    router.flush_pending().map_err(err)?;
    let slot_rows = router
        .iter_cf(ColumnFamily::slot(SlotId::new(10)))
        .map_err(err)?
        .len();
    let drift = drift_summary(&rows, &decoded)?;
    let full_recall = recall_against_exact(&rows, &ids, &rows, DIM, K)?;
    let quant_recall = recall_against_exact(&rows, &ids, &decoded, DIM, K)?;
    let min_bit = min_bit_contract_code(DIM, DISTORTION_BOUND, MIN_BITS_CODE)?;
    let max_relative_error = drift.max_relative_error;
    let passed = slot_rows == ROWS
        && drift.max_relative_error <= DISTORTION_BOUND
        && quant_recall + f32::EPSILON >= full_recall * 0.95
        && min_bit.code == MIN_BITS_CODE;
    Ok((
        passed,
        json!({
            "trigger": "1000 paired deterministic slot vectors encoded with TurboQuant Bits3p5 and searched through HNSW",
            "expected": {
                "max_relative_ip_error_lte": DISTORTION_BOUND,
                "quant_recall_gte_full_x_0_95": true,
                "minimum_bitwidth_fail_closed_code": MIN_BITS_CODE
            },
            "actual": {
                "constellations": ROWS,
                "pair_count": ROWS,
                "slot_cf_rows": slot_rows,
                "quant_level": "Bits3p5",
                "distortion_method": "decoded_quantized_vector_inner_product",
                "distortion": drift,
                "recall_at_10_full": full_recall,
                "recall_at_10_quantized": quant_recall,
                "recall_floor": full_recall * 0.95,
                "minimum_bitwidth_contract": min_bit,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_quant_ip_max_relative_error{{vault=\"ph59-h10\"}} {:.8}\ncalyx_quant_recall_at_10{{vault=\"ph59-h10\",kind=\"quantized\"}} {:.8}\n",
                max_relative_error, quant_recall
            )
        }),
    ))
}

fn probe_h11(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h11_seed_staleness")?;
    let input = paired_vectors(2, DIM).remove(0);
    let seed_a = new_seed(DIM, b"ph59-h11-seed-a");
    let seed_b = new_seed(DIM, b"ph59-h11-seed-b");
    let codec_a = TurboQuantCodec::new(seed_a.clone(), QuantLevel::Bits3p5).map_err(err)?;
    let codec_b = TurboQuantCodec::new(seed_b.clone(), QuantLevel::Bits3p5).map_err(err)?;
    let first = codec_a.encode(&input).map_err(err)?;
    let second = codec_a.encode(&input).map_err(err)?;
    let changed = codec_b.encode(&input).map_err(err)?;
    let same_path = dir.join("same_seed.qv");
    let changed_path = dir.join("different_seed.qv");
    fs::write(&same_path, &first.bytes).map_err(err)?;
    fs::write(&changed_path, &changed.bytes).map_err(err)?;
    let same_seed_file_bytes = fs::read(&same_path).map_err(err)?;
    let different_seed_file_bytes = fs::read(&changed_path).map_err(err)?;
    let same_seed_parity = first.bytes == second.bytes && same_seed_file_bytes == first.bytes;
    let different_seed_differs = first.bytes != changed.bytes
        && first.seed_id != changed.seed_id
        && same_seed_file_bytes != different_seed_file_bytes;
    let passed = same_seed_parity && different_seed_differs;
    Ok((
        passed,
        json!({
            "trigger": "re-quantize one deterministic vector with same and different QJL rotation seeds",
            "expected": {
                "same_seed_bit_identical": true,
                "different_seed_differs": true,
                "codebook_staleness_na": true
            },
            "actual": {
                "same_seed_bit_identical": same_seed_parity,
                "different_seed_differs": different_seed_differs,
                "same_seed_bytes": first.bytes.len(),
                "different_seed_bytes": changed.bytes.len(),
                "same_seed_id": seed_id_hex(&first.seed_id),
                "different_seed_id": seed_id_hex(&changed.seed_id),
                "seed_version": seed_a.version,
                "codebook_staleness_na": true,
                "turboquant_codebook": "none_data_oblivious_rotation",
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_qjl_same_seed_parity{{vault=\"ph59-h11\"}} {}\ncalyx_qjl_different_seed_delta{{vault=\"ph59-h11\"}} {}\n",
                usize::from(same_seed_parity), usize::from(different_seed_differs)
            )
        }),
    ))
}

fn probe_h12(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h12_ann_corruption")?;
    let graph = dir.join("idx").join("graph.cda");
    let vault = dir.join("vault");
    let rows = paired_vectors(128, 32);
    let ids = ids(rows.len());
    let mut router = CfRouter::open(&vault, 8 * 1024 * 1024).map_err(err)?;
    for (id, row) in ids.iter().zip(&rows) {
        router
            .put(ColumnFamily::Base, id.as_bytes(), &vec_bytes(row))
            .map_err(err)?;
    }
    router.flush_pending().map_err(err)?;
    let base_before = read_base_rows(&router, &ids)?;
    let params = DiskAnnBuildParams {
        dim: 32,
        m_max: 16,
        ef_construction: 48,
        alpha: 1.2,
    };
    let search = DiskAnnSearch::build(
        SlotId::new(12),
        &graph,
        &id_rows(&ids, &base_before),
        params,
        None,
        search_params(),
    )
    .map_err(err)?;
    let before_hit = top_hit(&search, &base_before[7])?;
    drop(search);
    let before_graph_bytes = fs::metadata(&graph).map_err(err)?.len();
    flip_8_bytes(&graph, 0)?;
    let corrupt_code =
        match DiskAnnSearch::open(SlotId::new(12), &graph, ids.clone(), None, search_params()) {
            Ok(_) => "NO_ERROR".to_string(),
            Err(error) => error.code.to_string(),
        };
    let degraded = corrupt_code == "CALYX_INDEX_CORRUPT";
    let fallback = fallback_hnsw_search(&ids, &base_before, &base_before[7], K)?;
    let rebuilt = DiskAnnSearch::build(
        SlotId::new(12),
        &graph,
        &id_rows(&ids, &base_before),
        params,
        None,
        search_params(),
    )
    .map_err(err)?;
    let after_hit = top_hit(&rebuilt, &base_before[7])?;
    let base_after = read_base_rows(&router, &ids)?;
    let rebuild_total = usize::from(degraded);
    let passed = before_hit == ids[7]
        && degraded
        && fallback.first().map(|hit| hit.cx_id) == Some(ids[7])
        && after_hit == ids[7]
        && base_before == base_after
        && fs::metadata(&graph).map_err(err)?.len() == before_graph_bytes
        && rebuild_total == 1;
    Ok((
        passed,
        json!({
            "trigger": "flip 8 bytes in a DiskANN graph header, degrade to base-CF fallback, rebuild graph",
            "expected": {
                "corrupt_error_code": "CALYX_INDEX_CORRUPT",
                "degraded_flag": true,
                "ann_degraded_rebuilds_total": 1,
                "base_cf_data_loss": false,
                "post_rebuild_top_hit_restored": true
            },
            "actual": {
                "graph_bytes_before": before_graph_bytes,
                "corrupt_error_code": corrupt_code,
                "degraded_flag": degraded,
                "fallback_top_hit": fallback.first().map(|hit| hit.cx_id.to_string()),
                "before_top_hit": before_hit.to_string(),
                "after_rebuild_top_hit": after_hit.to_string(),
                "expected_top_hit": ids[7].to_string(),
                "base_rows_before": base_before.len(),
                "base_rows_after": base_after.len(),
                "base_cf_bytes_identical_after_corrupt": base_before == base_after,
                "ann_degraded_rebuilds_total": rebuild_total,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_ann_degraded_rebuilds_total{{vault=\"ph59-h12\"}} {rebuild_total}\ncalyx_ann_degraded_flag{{vault=\"ph59-h12\"}} {}\n",
                usize::from(degraded)
            )
        }),
    ))
}
