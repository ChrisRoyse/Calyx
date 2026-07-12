use std::fs;
use std::path::{Path, PathBuf};

use calyx_assay::{EnsembleDecision, LINEAR_CKA_REDUNDANCY_METHOD};

use super::{evaluate, request_for, temp_root, write_outputs, write_rows};

const LENS_COUNT: usize = 10;
const ROWS: usize = 160;
const ORTHOGONAL_DIM: usize = 16;

#[test]
fn i8bin_card_preserves_orthogonal_redundancy_through_persisted_decisions() {
    let root = temp_root("i8bin-ensemble-card-orthogonal-redundancy");
    fs::create_dir_all(&root).unwrap();
    let rows = root.join("rows.jsonl");
    write_rows(&rows, ROWS);
    let plan = write_orthogonal_plan(&root);
    let request = request_for(&root, plan, rows, None, ROWS, Some(ROWS));

    let report = evaluate(&request).unwrap();

    assert_eq!(report.matrix.pairs.len(), 45);
    assert_eq!(
        report.matrix.redundancy_metric,
        LINEAR_CKA_REDUNDANCY_METHOD
    );
    assert!(
        report.matrix.pairs.iter().all(|pair| pair.corr > 0.99),
        "full-matrix representation similarity lost orthogonal redundancy: {:?}",
        report
            .matrix
            .pairs
            .iter()
            .map(|pair| pair.corr)
            .collect::<Vec<_>>()
    );
    assert!(report.matrix.n_eff < 1.01, "matrix={:?}", report.matrix);
    assert!(report.card.n_eff < 1.01, "card={:?}", report.card);
    assert!(report.card.pairs.iter().all(|pair| pair.corr > 0.99));
    assert!(report.card.pairs.iter().all(|pair| {
        pair.redundancy.as_ref().is_some_and(|estimate| {
            estimate.raw_signed_point > 0.99
                && estimate.redundancy_point > 0.99
                && estimate.mc_gate_upper_estimate > 0.99
        })
    }));
    assert!(report.card.lenses.iter().all(|lens| {
        lens.max_pairwise_corr > 0.99
            && lens.decision != EnsembleDecision::Keep
            && lens.decision_reason.contains("redundancy > 0.600000")
    }));
    for pair in &report.card.pairs {
        let matrix_pair = report
            .matrix
            .pairs
            .iter()
            .find(|candidate| {
                candidate.slot_a == pair.slot_a.get() && candidate.slot_b == pair.slot_b.get()
            })
            .unwrap();
        assert_eq!(pair.corr, matrix_pair.corr);
        assert_eq!(pair.nmi, matrix_pair.nmi);
        let estimate = pair.redundancy.as_ref().unwrap();
        assert_eq!(estimate.raw_signed_point, matrix_pair.raw_linear_cka);
        assert_eq!(estimate.redundancy_point, matrix_pair.linear_cka_point);
        assert_eq!(
            estimate.mc_standard_error,
            matrix_pair.linear_cka_mc_standard_error
        );
        assert_eq!(
            estimate.mc_gate_upper_estimate,
            matrix_pair.linear_cka_gate_score
        );
    }

    let evidence = write_outputs(&request, &report).unwrap();
    assert!(evidence.ensemble_card_payload_readback);
    let persisted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&evidence.ensemble_card_path).unwrap()).unwrap();
    assert!(persisted["n_eff"].as_f64().unwrap() < 1.01);
    assert_eq!(
        persisted["redundancy_method"]["metric"],
        LINEAR_CKA_REDUNDANCY_METHOD
    );
    assert!(persisted["pairs"].as_array().unwrap().iter().all(|pair| {
        pair["corr"].as_f64().unwrap() > 0.99
            && pair["redundancy"]["raw_signed_point"].as_f64().unwrap() > 0.99
            && pair["redundancy"]["mc_gate_upper_estimate"]
                .as_f64()
                .unwrap()
                > 0.99
    }));

    let _ = fs::remove_dir_all(root);
}

fn write_orthogonal_plan(root: &Path) -> PathBuf {
    let vector_dir = root.join("orthogonal-i8bin");
    fs::create_dir_all(&vector_dir).unwrap();
    let mut slots = Vec::with_capacity(LENS_COUNT);
    for slot in 0..LENS_COUNT {
        let corpus = vector_dir.join(format!("slot_{slot:02}_orthogonal_corpus.i8bin"));
        let queries = vector_dir.join(format!("slot_{slot:02}_orthogonal_queries.i8bin"));
        write_orthogonal_i8bin(&corpus, ROWS, slot);
        write_orthogonal_i8bin(&queries, 2, slot);
        slots.push(format!(
            "{{\"slot\":{slot},\"name\":\"semantic-fastembed-orthogonal-{slot}\",\"lens_id\":\"{:032x}\",\"weights_sha256\":\"{:064x}\",\"bits_about\":0.5,\"corpus\":\"{}\",\"queries\":\"{}\",\"vault\":\"{}\"}}",
            slot + 1,
            slot + 1,
            json_path(&corpus),
            json_path(&queries),
            json_path(&root.join(format!("vault-{slot}")))
        ));
    }
    let plan = root.join("orthogonal_partitioned_rrf_plan.json");
    fs::write(&plan, format!("{{\"slots\":[{}]}}", slots.join(","))).unwrap();
    plan
}

fn write_orthogonal_i8bin(path: &Path, rows: usize, lens: usize) {
    let mut bytes = Vec::with_capacity(8 + rows * ORTHOGONAL_DIM);
    bytes.extend_from_slice(&(rows as u32).to_le_bytes());
    bytes.extend_from_slice(&(ORTHOGONAL_DIM as u32).to_le_bytes());
    for row in 0..rows {
        for dim in 0..ORTHOGONAL_DIM {
            let base = hadamard_sign(row % ORTHOGONAL_DIM, dim);
            let transform = hadamard_sign(lens, dim);
            bytes.push((24_i8 * base * transform) as u8);
        }
    }
    fs::write(path, bytes).unwrap();
}

fn hadamard_sign(row: usize, column: usize) -> i8 {
    if (row & column).count_ones().is_multiple_of(2) {
        1
    } else {
        -1
    }
}

fn json_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}
