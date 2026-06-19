use std::collections::BTreeMap;
use std::path::Path;

use calyx_assay::{
    AssayCacheKey, AssayRow, AssaySubject, EstimatorKind, MiEstimate, TrustTag, admit_lens,
};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::dedup::EpochSecs;
use calyx_aster::recurrence::RetentionPolicy;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    AbsentReason, Anchor, AnchorKind, AnchorValue, CxFlags, CxId, Lens, LensId, Modality,
    SlotVector, VaultId,
};
use calyx_ledger::{EntryKind, SubjectId};
use calyx_loom::recurrence::SeriesStore;
use calyx_registry::{AlgorithmicLens, Input, Registry};
use serde_json::{Value, json};

use super::living_concert_data::{
    CorpusDoc, CorpusSource, display, list_files, load_corpus, readback_bundle, reset_dir,
    write_blake3_manifest, write_json, write_readback_files,
};
use super::living_concert_edges::{
    anneal_heal_edge, budget_edge, conflicting_anchor_edge, lens_unreachable_edge, objective_rows,
    oracle_step, ward_step,
};
use super::living_concert_store::{
    append_event, constellation, ctx, online_row, slot, vault_id, write_constellation,
    write_event_with_rows,
};

const INITIAL_PANEL: u32 = 70;
pub(crate) const GROWN_PANEL: u32 = 71;
pub(crate) const START_TS: u64 = 1_786_000_000_000;
pub(crate) const WEEK_SECS: i64 = 604_800;
pub(crate) const BASE_EVENT_SECS: i64 = 1_704_204_000;

pub fn run_living_concert(root: &Path, source: CorpusSource, calyx_bin: &Path) -> Value {
    reset_dir(root);
    let fixture = load_corpus(&source, 4);
    let vault_dir = root.join("vault");
    let vault_id = vault_id();
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id,
        b"issue641-living".to_vec(),
        VaultOptions::default(),
    )
    .expect("open living concert vault");
    let mut registry = registry();
    let mut growth_lens_id = None;
    let mut ingested = Vec::new();
    let mut recurrence_cx = None;
    let mut edge_lens = Value::Null;
    let mut admission = Value::Null;

    for (index, doc) in fixture.docs.iter().enumerate() {
        if index == 2 {
            let (id, value) = admit_growth_lens(&vault, &mut registry, vault_id);
            growth_lens_id = Some(id);
            admission = value;
        }
        let record = ingest_doc(&vault, &registry, doc, index, growth_lens_id, vault_id);
        if index == 1 {
            edge_lens = lens_unreachable_edge(&vault, &registry, &record.input);
        }
        if recurrence_cx.is_none() {
            recurrence_cx = Some(record.cx_id);
        }
        append_recurrence(&vault, recurrence_cx.unwrap(), index);
        append_measure_event(&vault, record.cx_id, index, &record.slot_names);
        ingested.push(record.readback);
    }

    let recurrence_cx = recurrence_cx.expect("fixture has docs");
    let heal = anneal_heal_edge(&vault);
    let prediction = oracle_step(&vault, recurrence_cx);
    let ward = ward_step(&vault, recurrence_cx);
    let conflict = conflicting_anchor_edge(&vault, &registry, vault_id);
    let budget = budget_edge(&vault, recurrence_cx);
    let objective = objective_rows(&vault, &admission, &prediction);
    vault.flush().expect("flush living concert vault");
    let ledger_end = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .expect("scan ledger")
        .len() as u64;
    let readbacks = readback_bundle(calyx_bin, &vault_dir, recurrence_cx, ledger_end);
    write_readback_files(root, &readbacks);

    let summary = json!({
        "issue": 641,
        "source_of_truth": {
            "vault_dir": display(&vault_dir),
            "base_cf": display(&vault_dir.join("cf/base")),
            "ledger_cf": display(&vault_dir.join("cf/ledger")),
            "wal": display(&vault_dir.join("wal")),
        },
        "corpus": fixture.readback,
        "expected": {
            "docs_ingested": fixture.docs.len(),
            "growth_panel_version": GROWN_PANEL,
            "assay_bits": 0.12,
            "assay_corr": 0.25,
            "oracle_t_hat": BASE_EVENT_SECS + (fixture.docs.len() as i64) * WEEK_SECS,
            "ward_injection_pass": false,
            "budget_error": "CALYX_ANNEAL_BUDGET_EXHAUSTED",
        },
        "loop": {
            "ingested": ingested,
            "admission": admission,
            "anneal_heal": heal,
            "oracle": prediction,
            "ward": ward,
            "objective": objective,
        },
        "edges": {
            "lens_endpoint_killed": edge_lens,
            "conflicting_anchor_recurrence": conflict,
            "over_budget_background_work": budget,
        },
        "ledger": {
            "range": format!("0..{ledger_end}"),
            "entries": ledger_end,
            "kinds_expected": ["ingest", "measure", "assay", "guard", "answer", "anneal", "admin"],
        },
        "readbacks": readbacks,
        "files_after": list_files(root),
    });
    write_json(&root.join("living-concert-readback.json"), &summary);
    write_blake3_manifest(root);
    summary
}

struct IngestedDoc {
    cx_id: CxId,
    input: Input,
    slot_names: Vec<&'static str>,
    readback: Value,
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    let probe = Input::new(Modality::Text, b"issue641 deterministic probe".to_vec());
    let byte = AlgorithmicLens::byte_features("issue641-byte-features", Modality::Text);
    registry
        .register_frozen_with_probe(byte.clone(), byte.contract().clone(), &probe)
        .expect("register byte lens");
    let scalar = AlgorithmicLens::scalar("issue641-scalar", Modality::Text);
    registry
        .register_frozen_with_probe(scalar.clone(), scalar.contract().clone(), &probe)
        .expect("register scalar lens");
    registry
}

fn ingest_doc(
    vault: &AsterVault,
    registry: &Registry,
    doc: &CorpusDoc,
    index: usize,
    growth_lens_id: Option<LensId>,
    vault_id: VaultId,
) -> IngestedDoc {
    let bytes = format!(
        "query:{}\ntitle:{}\ntext:{}",
        doc.query, doc.title, doc.text
    )
    .into_bytes();
    let input =
        Input::new(Modality::Text, bytes.clone()).with_pointer(format!("scifact://{}", doc.doc_id));
    let panel = growth_lens_id.map_or(INITIAL_PANEL, |_| GROWN_PANEL);
    let cx_id = vault.cx_id_for_input(&bytes, panel);
    let mut slots = BTreeMap::new();
    let byte_id = AlgorithmicLens::byte_features("issue641-byte-features", Modality::Text).id();
    let scalar_id = AlgorithmicLens::scalar("issue641-scalar", Modality::Text).id();
    slots.insert(
        slot(0),
        registry.measure(byte_id, &input).expect("byte lens"),
    );
    slots.insert(
        slot(1),
        registry.measure(scalar_id, &input).expect("scalar lens"),
    );
    let mut slot_names = vec!["slot_00", "slot_01"];
    if let Some(id) = growth_lens_id {
        slots.insert(slot(2), registry.measure(id, &input).expect("growth lens"));
        slot_names.push("slot_02");
    }
    if index == 1 {
        slots.insert(
            slot(3),
            SlotVector::Absent {
                reason: AbsentReason::LensUnavailable,
            },
        );
        slot_names.push("slot_03");
    }
    let anchor = Anchor {
        kind: anchor_kind(),
        value: AnchorValue::Number(doc.score),
        source: format!("beir-scifact:qrels/test.tsv#query={}", doc.query_id),
        observed_at: START_TS + index as u64,
        confidence: 1.0,
    };
    let flags = CxFlags {
        ungrounded: false,
        degraded: index == 1,
        ..CxFlags::default()
    };
    let cx = constellation(vault_id, cx_id, panel, &input, slots, vec![anchor], flags);
    let payload = json!({"tag":"living_ingest_v1","doc_id":doc.doc_id,"query_id":doc.query_id});
    write_constellation(vault, cx, payload, START_TS + index as u64);
    IngestedDoc {
        cx_id,
        input,
        slot_names: slot_names.clone(),
        readback: json!({
            "cx_id": cx_id.to_string(),
            "doc_id": doc.doc_id,
            "query_id": doc.query_id,
            "panel_version": panel,
            "slots": slot_names,
            "anchor_score": doc.score,
        }),
    }
}

fn admit_growth_lens(
    vault: &AsterVault,
    registry: &mut Registry,
    vault_id: VaultId,
) -> (LensId, Value) {
    let decision = admit_lens(0.12, 0.25).expect("admit growth lens");
    let probe = Input::new(Modality::Text, b"issue641 growth probe".to_vec());
    let lens = AlgorithmicLens::one_hot("issue641-growth-one-hot", Modality::Text, 8);
    let lens_id = registry
        .register_frozen_with_probe(lens.clone(), lens.contract().clone(), &probe)
        .expect("register growth lens");
    let cache_key = AssayCacheKey::scoped(GROWN_PANEL, "issue641-scifact", vault_id, anchor_kind());
    let row = AssayRow {
        cache_key,
        subject: AssaySubject::Lens { slot: slot(2) },
        estimate: MiEstimate::point(0.12, 4, EstimatorKind::LogisticProbe, TrustTag::Trusted),
        provenance: "issue641 living concert qrels anchor".to_string(),
        written_at_seq: 0,
        payload: None,
    };
    let payload = json!({"tag":"living_assay_v1","lens_id":lens_id,"decision":decision});
    let mut rows = vec![(
        ColumnFamily::Assay,
        b"issue641:assay:growth-slot".to_vec(),
        serde_json::to_vec(&row).unwrap(),
    )];
    rows.push(online_row(
        100,
        json!({"tag":"registry_admission_v1","lens_id":lens_id,"decision":decision}),
    ));
    write_event_with_rows(
        vault,
        EntryKind::Assay,
        SubjectId::Lens(lens_id),
        payload,
        rows,
        START_TS + 20,
    );
    append_event(
        vault,
        EntryKind::Admin,
        SubjectId::Lens(lens_id),
        json!({
            "tag":"registry_panel_growth_v1","from_panel":INITIAL_PANEL,"to_panel":GROWN_PANEL
        }),
        START_TS + 21,
    );
    (
        lens_id,
        json!({"lens_id":lens_id.to_string(),"decision":decision,"panel_version":GROWN_PANEL}),
    )
}

fn append_recurrence(vault: &AsterVault, cx_id: CxId, index: usize) {
    let time = BASE_EVENT_SECS + index as i64 * WEEK_SECS;
    let store =
        SeriesStore::with_retention(vault, RetentionPolicy::default()).expect("series store");
    store
        .append_occurrence(cx_id, EpochSecs(time), ctx(&format!("loop-{index}")))
        .expect("append recurrence");
    append_event(
        vault,
        EntryKind::Measure,
        SubjectId::Cx(cx_id),
        json!({
            "tag":"living_recurrence_occurrence_v1","occurrence_index":index,"t_k":time
        }),
        START_TS + 40 + index as u64,
    );
}

fn append_measure_event(vault: &AsterVault, cx_id: CxId, index: usize, slots: &[&str]) {
    append_event(
        vault,
        EntryKind::Measure,
        SubjectId::Cx(cx_id),
        json!({
            "tag":"living_measure_v1","loop_index":index,"slots":slots
        }),
        START_TS + 60 + index as u64,
    );
}

fn anchor_kind() -> AnchorKind {
    AnchorKind::Label("scifact_relevance".to_string())
}
