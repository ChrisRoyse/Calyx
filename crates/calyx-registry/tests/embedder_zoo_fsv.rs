use std::fs;

use calyx_assay::{AssayGate, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{SlotState, VaultStore};
use calyx_loom::{
    AbundanceReport, CeilingEstimate, LoomStore, NeffEstimate, cross_term_upper_bound,
};
use calyx_registry::{Registry, SwapController, apply_capability_gate, persist_vault_panel_state};
use serde_json::json;

#[path = "embedder_zoo_fsv/fsv_io.rs"]
mod fsv_io;
#[path = "embedder_zoo_fsv/model.rs"]
mod model;
#[path = "embedder_zoo_fsv/samples.rs"]
mod samples;

const FSV_TS: u64 = 1_785_500_792;
const ROWS: usize = 96;
const RSS_BUDGET_KIB: u64 = 4 * 1024 * 1024;

#[test]
fn ph74_embedder_zoo_stage_exit_fsv() {
    let root = fsv_io::fsv_root();
    fsv_io::reset_dir(&root);
    let vault_dir = root.join("vault");
    let artifact_dir = root.join("factory-artifacts");
    let xterm_dir = root.join("xterm-cf");
    let cards_dir = root.join("capability-cards");
    fs::create_dir_all(&cards_dir).unwrap();

    let mut registry = Registry::new();
    let converted = model::convert_register_targets(&mut registry, &artifact_dir);
    let mut slots = model::target_slots(&converted);
    let duplicate_slot = model::duplicate_slot(&slots[0], 99, "duplicate_bge_text");
    slots.push(duplicate_slot.clone());

    let (left_samples, right_samples, labels) = samples::cross_modal_samples();
    let image_slot = samples::slot_by_axis(&slots, "image_visual");
    let protein_slot = samples::slot_by_axis(&slots, "protein_sequence");
    let gate = AssayGate { min_samples: 50 };
    let pair_gain = gate
        .pair_gain(&left_samples, &right_samples, &labels)
        .expect("cross-modal pair gain");
    assert!(pair_gain.gain_bits >= 0.05, "{pair_gain:?}");

    let cache_key = model::cache_key();
    let mut assay = model::assay_store(&cache_key, &slots, &converted, &pair_gain, &labels);
    assay.put(
        cache_key.clone(),
        AssaySubject::Pair {
            a: image_slot.slot_id,
            b: protein_slot.slot_id,
        },
        MiEstimate::new(
            pair_gain.gain_bits,
            pair_gain.ci_low,
            pair_gain.ci_high,
            pair_gain.n_samples,
            EstimatorKind::PairGain,
            TrustTag::Trusted,
        ),
        "issue792 image/protein cross-modal pair gain",
        792_200,
    );

    let mut controller = SwapController::new(calyx_core::Panel {
        version: 1,
        slots: slots.clone(),
        created_at: FSV_TS,
        kernel_ref: None,
        guard_ref: None,
    });
    let evaluations = model::evaluate_all(
        &registry,
        &slots,
        &assay,
        &cache_key,
        &duplicate_slot,
        Default::default(),
    );
    let outcomes = apply_all(&mut controller, &evaluations);
    let decisions = model::decision_counts(&evaluations);
    assert_eq!(decisions["admit"], 7);
    assert_eq!(decisions["park"], 1);
    assert_eq!(decisions["retire"], 1);

    for evaluated in &evaluations {
        let path = cards_dir.join(format!(
            "slot-{}-{}.json",
            evaluated.slot_id.get(),
            evaluated.evaluation.card.lens_id
        ));
        fsv_io::write_json(&path, evaluated);
    }

    let vault = AsterVault::new_durable(
        &vault_dir,
        fsv_io::vault_id(),
        b"issue792-stage-exit".to_vec(),
        VaultOptions {
            panel: Some(controller.panel().clone()),
            ..VaultOptions::default()
        },
    )
    .unwrap();
    persist_vault_panel_state(&vault_dir, controller.panel(), &registry).unwrap();

    let active_slots = controller
        .panel()
        .slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active)
        .cloned()
        .collect::<Vec<_>>();
    let mut loom = LoomStore::new(512);
    for index in 0..ROWS {
        let slot_map = samples::slot_map_for(index, &active_slots, &left_samples, &right_samples);
        vault
            .put(samples::constellation(index, &slot_map, fsv_io::vault_id()))
            .unwrap();
        loom.weave(samples::cx(index), &slot_map).unwrap();
    }
    vault.flush().unwrap();

    let (assay_rows, loaded_assay, raw_assay_rows) = persist_and_read_assay(&vault, &assay);
    let (xterm_rows, loaded_xterms) = persist_and_read_xterms(&xterm_dir, &loom);
    let rank = samples::neff_for_active(active_slots.len());
    let naive_duplicate_expectation = active_slots.len() as f32 - 1.0;
    assert!(rank.n_eff > naive_duplicate_expectation, "{rank:?}");

    let abundance = AbundanceReport::new(
        active_slots.len(),
        ROWS,
        loaded_xterms.xterm_count(),
        NeffEstimate::Computed {
            value: rank.n_eff,
            ci_low: rank.n_eff,
            ci_high: active_slots.len() as f32,
        },
        CeilingEstimate::Computed {
            bits: calyx_assay::entropy_bits(&labels),
        },
        loaded_xterms.measured_count(),
        loaded_xterms.xterm_count(),
    );
    let intelligence_dir = vault_dir.join("intelligence");
    fs::create_dir_all(&intelligence_dir).unwrap();
    fsv_io::write_json(&intelligence_dir.join("abundance.json"), &abundance);

    let ledger_rows = fsv_io::append_decision_ledger(&root, &evaluations);
    let footprint = fsv_io::footprint_readback();
    assert!(footprint["rss"]["within_budget"].as_bool().unwrap());

    fsv_io::write_json(
        &root.join("conversion-readback.json"),
        &model::conversion_readback(&converted),
    );
    fsv_io::write_json(
        &root.join("assay-readback.json"),
        &json!({
            "assay_rows": assay_rows,
            "raw_assay_cf_rows": raw_assay_rows,
            "loaded_rows": loaded_assay.rows(),
            "cross_modal_pair": {
                "left_slot": image_slot.slot_id,
                "right_slot": protein_slot.slot_id,
                "gain_bits": pair_gain.gain_bits,
                "pair_bits": pair_gain.pair_bits,
                "left_bits": pair_gain.left_bits,
                "right_bits": pair_gain.right_bits,
                "n_samples": pair_gain.n_samples
            }
        }),
    );
    fsv_io::write_json(
        &root.join("xterm-readback.json"),
        &json!({
            "xterm_cf_root": xterm_dir.join("cf/xterm"),
            "persisted_rows": xterm_rows,
            "loaded_rows": loaded_xterms.xterm_count(),
            "agreement_graph": loaded_xterms.agreement_graph(),
            "expected_upper_bound_per_cx": cross_term_upper_bound(active_slots.len()),
            "sample_rows": loaded_xterms.xterm_rows().into_iter().take(8).collect::<Vec<_>>()
        }),
    );
    fsv_io::write_json(
        &root.join("panel-decisions.json"),
        &json!({
            "outcomes": outcomes,
            "decisions": evaluations,
            "decision_counts": decisions,
            "active_slots": active_slots,
            "panel_after": controller.panel()
        }),
    );
    fsv_io::write_json(&root.join("footprint.json"), &footprint);
    fsv_io::write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 792,
            "source_of_truth": "durable manual Aster vault plus Assay CF, XTerm CF, capability ledger, commissioned artifacts, and vault/intelligence/abundance.json",
            "vault": vault_dir,
            "converted_models": converted.len(),
            "registered_lenses": registry.lens_snapshots().len(),
            "modalities": model::modality_counts(&converted),
            "decision_counts": decisions,
            "active_slot_count": active_slots.len(),
            "xterm_rows": xterm_rows,
            "assay_rows": assay_rows,
            "abundance": abundance,
            "n_eff": rank,
            "naive_duplicate_expectation": naive_duplicate_expectation,
            "cross_modal_gain_bits": pair_gain.gain_bits,
            "footprint": footprint,
            "ledger_rows": ledger_rows,
            "trigger": "convert/register/measure/capability-gate/assay/loom/keep-retire stage-exit workflow"
        }),
    );
    fsv_io::write_physical_files(&root.join("physical-files.txt"), &root);
    fsv_io::write_manifest(&root);
    println!("ISSUE792_FSV_ROOT={}", root.display());

    if !fsv_io::keep_root() {
        fs::remove_dir_all(root).unwrap();
    }
}

fn apply_all(
    controller: &mut SwapController,
    evaluations: &[model::EvaluatedSlot],
) -> Vec<calyx_registry::PanelCapabilityGateOutcome> {
    evaluations
        .iter()
        .map(|evaluated| {
            apply_capability_gate(
                controller,
                evaluated.slot_id,
                &evaluated.evaluation,
                FSV_TS + 1,
            )
            .unwrap()
        })
        .collect()
}

fn persist_and_read_assay(vault: &AsterVault, assay: &AssayStore) -> (usize, AssayStore, usize) {
    let assay_rows = assay.persist_to_vault(vault).unwrap();
    let loaded_assay = AssayStore::load_from_vault(vault).unwrap();
    let raw_rows = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Assay)
        .unwrap()
        .len();
    assert_eq!(assay_rows, loaded_assay.len());
    assert_eq!(assay_rows, raw_rows);
    (assay_rows, loaded_assay, raw_rows)
}

fn persist_and_read_xterms(xterm_dir: &std::path::Path, loom: &LoomStore) -> (usize, LoomStore) {
    let mut router = CfRouter::open(xterm_dir, 1_048_576).unwrap();
    let rows = loom.persist_xterms_to_aster(&mut router).unwrap();
    let loaded = LoomStore::load_xterms_from_aster(&router, 512).unwrap();
    assert_eq!(rows, loaded.xterm_count());
    (rows, loaded)
}
