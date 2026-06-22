use std::path::PathBuf;

use calyx_assay::{AssayCacheKey, AssayStore, AssaySubject, EstimatorKind, MiEstimate, TrustTag};
use calyx_core::{
    AnchorKind, Asymmetry, FixedClock, Input, LensId, Modality, Panel, QuantPolicy, Slot, SlotId,
    SlotKey, SlotShape, SlotState, VaultId, content_address,
};
use calyx_ledger::{ActorId, DirectoryLedgerStore, LedgerAppender, LedgerCfStore, decode};
use calyx_registry::{
    CapabilityGateDecision, CapabilityGateEvaluation, CapabilityGateThresholds, MetricSource,
    ProfileProbe, Registry, StaticLookupLens, append_capability_gate_ledger, apply_capability_gate,
    evaluate_capability_gate, lens_spec_from_manifest_path, max_panel_pairwise_correlation,
    profile_slot_with_assay,
};
use serde_json::json;

#[test]
#[ignore = "manual FSV for #787 capability-card gating"]
fn issue787_capability_gate_fsv_writes_readback_artifacts() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).expect("create fsv root");
    let manifest = manifest_path();
    let probes = probes();
    let mut registry = Registry::new();

    let admit_slot = register_static_slot(&mut registry, &manifest, "issue787-admit", 0);
    let park_slot = register_static_slot(&mut registry, &manifest, "issue787-park", 1);
    let baseline_slot = register_static_slot(&mut registry, &manifest, "issue787-baseline", 2);
    let retire_slot = register_static_slot(&mut registry, &manifest, "issue787-retire", 3);
    let mut controller = calyx_registry::SwapController::new(Panel {
        version: 1,
        slots: vec![admit_slot.clone(), park_slot.clone(), retire_slot.clone()],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    });
    let baseline_panel = Panel {
        version: 1,
        slots: vec![baseline_slot],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    };
    let cache_key = AssayCacheKey::scoped(1, "issue787-fsv", vault_id(), AnchorKind::Reward);
    let assay = assay_store(&cache_key, &admit_slot, &park_slot, &retire_slot);
    let before_ledger = ledger_rows(&root);
    assert!(before_ledger.is_empty());

    let thresholds = CapabilityGateThresholds::default();
    let admit = eval(
        &registry,
        &admit_slot,
        &probes,
        &assay,
        &cache_key,
        0.0,
        thresholds,
    );
    let park = eval(
        &registry, &park_slot, &probes, &assay, &cache_key, 0.0, thresholds,
    );
    let retire_corr = max_panel_pairwise_correlation(
        &registry,
        &baseline_panel,
        retire_slot.lens_id,
        None,
        &probes,
    )
    .expect("duplicate static lens correlation");
    let retire = eval(
        &registry,
        &retire_slot,
        &probes,
        &assay,
        &cache_key,
        retire_corr,
        thresholds,
    );

    assert_eq!(admit.decision, CapabilityGateDecision::Admit);
    assert_eq!(park.decision, CapabilityGateDecision::Park);
    assert_eq!(retire.decision, CapabilityGateDecision::Retire);
    assert!((retire.max_pairwise_corr - 1.0).abs() <= 1e-6);

    let outcomes = vec![
        apply_capability_gate(&mut controller, admit_slot.slot_id, &admit, 10).unwrap(),
        apply_capability_gate(&mut controller, park_slot.slot_id, &park, 11).unwrap(),
        apply_capability_gate(&mut controller, retire_slot.slot_id, &retire, 12).unwrap(),
    ];
    let ledger_refs = append_ledger(&root, &[&admit, &park, &retire]);
    let after_ledger = ledger_rows(&root);

    write_json(root.join("admit-card.json"), &admit);
    write_json(root.join("park-card.json"), &park);
    write_json(root.join("retire-card.json"), &retire);
    write_json(
        root.join("panel-states.json"),
        &json!({
            "outcomes": outcomes,
            "slots": controller.panel().slots,
        }),
    );
    write_json(root.join("ledger-readback.json"), &after_ledger);
    write_json(root.join("ledger-refs.json"), &ledger_refs);

    println!("ISSUE787_FSV_ROOT={}", root.display());
    println!("ISSUE787_MANIFEST={}", manifest.display());
    println!("ISSUE787_ADMIT_DECISION={:?}", admit.decision);
    println!("ISSUE787_PARK_DECISION={:?}", park.decision);
    println!("ISSUE787_RETIRE_DECISION={:?}", retire.decision);
    println!("ISSUE787_RETIRE_CORR={:.8}", retire.max_pairwise_corr);
    println!("ISSUE787_LEDGER_ROWS={}", after_ledger.len());
    println!(
        "ISSUE787_PANEL_STATES_SHA={}",
        file_digest(&root.join("panel-states.json"))
    );
    assert_eq!(after_ledger.len(), 3);
    assert_eq!(controller.panel().slots[0].state, SlotState::Active);
    assert_eq!(controller.panel().slots[1].state, SlotState::Parked);
    assert_eq!(controller.panel().slots[2].state, SlotState::Retired);

    let empty_error =
        max_panel_pairwise_correlation(&registry, &baseline_panel, admit_slot.lens_id, None, &[])
            .unwrap_err();
    std::fs::write(root.join("edge-empty-error.txt"), empty_error.code).unwrap();
    assert_eq!(empty_error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let edge_card = profile_slot_with_assay(&registry, &admit_slot, &probes, &assay, &cache_key)
        .expect("edge source card");
    let mut missing_signal = edge_card.clone();
    missing_signal.signal = None;
    missing_signal.signal_source = MetricSource::AssayPending;
    let missing_signal_eval =
        evaluate_capability_gate(missing_signal, 0.0, thresholds).expect("missing signal edge");
    write_json(
        root.join("edge-missing-signal-card.json"),
        &missing_signal_eval,
    );
    assert_eq!(missing_signal_eval.decision, CapabilityGateDecision::Park);

    let mut collapsed = edge_card;
    collapsed.low_spread = true;
    collapsed.spread.participation_ratio = 0.0;
    collapsed.spread.normalized_participation_ratio = 0.0;
    collapsed.spread.stable_rank = 0.0;
    collapsed.spread.total_variance = 0.0;
    collapsed.spread.mean_pairwise_distance = 0.0;
    let collapsed_eval =
        evaluate_capability_gate(collapsed, 0.0, thresholds).expect("collapsed edge");
    write_json(root.join("edge-collapsed-card.json"), &collapsed_eval);
    assert_eq!(collapsed_eval.decision, CapabilityGateDecision::Park);
}

fn eval(
    registry: &Registry,
    slot: &Slot,
    probes: &[ProfileProbe],
    assay: &AssayStore,
    cache_key: &AssayCacheKey,
    corr: f32,
    thresholds: CapabilityGateThresholds,
) -> CapabilityGateEvaluation {
    let card = profile_slot_with_assay(registry, slot, probes, assay, cache_key).unwrap();
    evaluate_capability_gate(card, corr, thresholds).unwrap()
}

fn append_ledger(root: &std::path::Path, evals: &[&CapabilityGateEvaluation]) -> Vec<String> {
    let ledger_dir = root.join("ledger");
    let mut appender = LedgerAppender::open(
        DirectoryLedgerStore::open(&ledger_dir).unwrap(),
        FixedClock::new(50),
    )
    .unwrap();
    evals
        .iter()
        .map(|evaluation| {
            let reference = append_capability_gate_ledger(
                &mut appender,
                evaluation,
                ActorId::Service("issue787-capability-gate-fsv".to_string()),
            )
            .unwrap();
            format!("{}:{}", reference.seq, hex(&reference.hash))
        })
        .collect()
}

fn ledger_rows(root: &std::path::Path) -> Vec<serde_json::Value> {
    let store = DirectoryLedgerStore::open(root.join("ledger")).unwrap();
    store
        .scan()
        .unwrap()
        .into_iter()
        .map(|row| {
            let entry = decode(&row.bytes).unwrap();
            json!({
                "seq": row.seq,
                "kind": entry.kind.to_string(),
                "subject": format!("{:?}", entry.subject),
                "payload_sha256": digest(&entry.payload),
                "payload": serde_json::from_slice::<serde_json::Value>(&entry.payload).unwrap(),
            })
        })
        .collect()
}

fn register_static_slot(
    registry: &mut Registry,
    manifest: &std::path::Path,
    name: &str,
    slot_id: u16,
) -> Slot {
    let mut spec = lens_spec_from_manifest_path(manifest).unwrap();
    spec.name = name.to_string();
    let lens = StaticLookupLens::from_lens_spec(&spec).unwrap();
    let contract = lens.contract().clone();
    let lens_id = registry
        .register_frozen_with_spec(lens, contract, spec)
        .unwrap();
    slot(lens_id, slot_id, name)
}

fn assay_store(cache_key: &AssayCacheKey, admit: &Slot, park: &Slot, retire: &Slot) -> AssayStore {
    let mut store = AssayStore::default();
    put(&mut store, cache_key, admit.slot_id, 0.12, 1);
    put(&mut store, cache_key, park.slot_id, 0.01, 2);
    put(&mut store, cache_key, retire.slot_id, 0.12, 3);
    store
}

fn put(store: &mut AssayStore, cache_key: &AssayCacheKey, slot: SlotId, bits: f32, seq: u64) {
    store.put(
        cache_key.clone(),
        AssaySubject::Lens { slot },
        MiEstimate::point(bits, 96, EstimatorKind::Ksg, TrustTag::Trusted),
        "issue787-fsv grounded outcome",
        seq,
    );
}

fn slot(lens_id: LensId, slot_id: u16, key: &str) -> Slot {
    let slot_id = SlotId::new(slot_id);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, key.to_string()),
        lens_id,
        shape: SlotShape::Dense(256),
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some(key.to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: Default::default(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

fn probes() -> Vec<ProfileProbe> {
    [
        ("contract damages alpha", "legal"),
        ("tort liability beta", "legal"),
        ("glucose pathway gamma", "bio"),
        ("protein folding delta", "bio"),
    ]
    .into_iter()
    .map(|(text, label)| {
        ProfileProbe::labeled(Input::new(Modality::Text, text.as_bytes().to_vec()), label)
    })
    .collect()
}

fn manifest_path() -> PathBuf {
    std::env::var("CALYX_ISSUE787_STATIC_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from("/var/lib/calyx/lenses/semantic-potion-base-8m/model2vec/manifest.json")
        })
}

fn fsv_root() -> PathBuf {
    std::env::var("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/var/lib/calyx/tmp/issue787-capability-gate-fsv"))
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn write_json(path: PathBuf, value: &impl serde::Serialize) {
    std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn file_digest(path: &std::path::Path) -> String {
    digest(&std::fs::read(path).unwrap())
}

fn digest(bytes: &[u8]) -> String {
    hex(&content_address([bytes]))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
