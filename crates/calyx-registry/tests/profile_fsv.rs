use calyx_assay::estimate::{EstimatorKind, MiEstimate, TrustTag};
use calyx_assay::store::{AssayCacheKey, AssayStore, AssaySubject};
use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_core::{
    AnchorKind, Asymmetry, ConfidenceInterval, Input, Lens, LensId, Modality, Panel, QuantPolicy,
    Result, Signal, Slot, SlotId, SlotKey, SlotShape, SlotState, SlotVector, VaultId,
    content_address,
};
use calyx_registry::frozen::sha256_digest;
use calyx_registry::{
    AlgorithmicLens, FrozenLensContract, LensDType, NormPolicy, ProfileProbe, Registry,
    list_panel_with_assay, profile_lens, profile_slot_with_assay,
};
use std::path::PathBuf;

#[test]
#[ignore = "manual FSV test for PH21 capability cards"]
fn ph21_profile_card_manual_fsv() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).expect("create fsv root");

    let mut registry = Registry::new();
    let algorithmic = AlgorithmicLens::byte_features("ph21-profile-fsv", Modality::Text);
    let algorithmic_id = registry
        .register_frozen(algorithmic.clone(), algorithmic.contract().clone())
        .expect("register algorithmic lens");
    let probes = probe_set();
    let card = profile_lens(&registry, algorithmic_id, &probes).expect("profile algorithmic");
    let card_path = root.join("algorithmic-card.json");
    write_card(&card_path, &card);
    let card_bytes = std::fs::read(&card_path).expect("read algorithmic card");
    let readback: calyx_registry::CapabilityCard =
        serde_json::from_slice(&card_bytes).expect("parse algorithmic readback");

    println!("PH21_FSV_ROOT={}", root.display());
    println!("PH21_ALGORITHMIC_CARD={}", card_path.display());
    println!("PH21_ALGORITHMIC_CARD_SHA={}", digest_hex(&card_bytes));
    println!("PH21_SIGNAL_NULL={}", readback.signal.is_none());
    println!("PH21_SIGNAL_SOURCE={:?}", readback.signal_source);
    println!("PH21_PROXY_SIGNAL={:.8}", readback.proxy_signal);
    println!(
        "PH21_DIFFERENTIATION_NULL={}",
        readback.differentiation.is_none()
    );
    println!(
        "PH21_DIFFERENTIATION_SOURCE={:?}",
        readback.differentiation_source
    );
    println!(
        "PH21_PROXY_DIFFERENTIATION={:.8}",
        readback.proxy_differentiation
    );
    println!("PH21_SPREAD_PR={:.8}", readback.spread.participation_ratio);
    println!(
        "PH21_SPREAD_NORM={:.8}",
        readback.spread.normalized_participation_ratio
    );
    println!("PH21_SEPARATION={:.8}", readback.separation.score);
    println!("PH21_COST_MS_PER_INPUT={:.8}", readback.cost.ms_per_input);
    println!("PH21_COVERAGE_RATE={:.8}", readback.coverage.rate);
    assert_eq!(readback.coverage.failed, 0);
    assert!(readback.signal.is_none());
    assert_eq!(
        readback.signal_source,
        calyx_registry::MetricSource::AssayPending
    );
    assert!(readback.proxy_signal.is_finite());
    assert!(readback.differentiation.is_none());
    assert_eq!(
        readback.differentiation_source,
        calyx_registry::MetricSource::AssayPending
    );
    assert!(readback.proxy_differentiation.is_finite());
    assert!(readback.spread.participation_ratio > 0.0);

    assay_backed_profile_readback(&root, &registry, algorithmic_id, &probes);

    let collapsed_lens = CollapsedLens::new();
    let collapsed_id = registry
        .register_frozen(collapsed_lens.clone(), collapsed_lens.contract.clone())
        .expect("register collapsed");
    let collapsed = profile_lens(&registry, collapsed_id, &probes).expect("profile collapsed");
    let collapsed_path = root.join("collapsed-card.json");
    write_card(&collapsed_path, &collapsed);
    let collapsed_bytes = std::fs::read(&collapsed_path).expect("read collapsed card");
    let collapsed_readback: calyx_registry::CapabilityCard =
        serde_json::from_slice(&collapsed_bytes).expect("parse collapsed readback");
    println!("PH21_COLLAPSED_CARD={}", collapsed_path.display());
    println!("PH21_COLLAPSED_CARD_SHA={}", digest_hex(&collapsed_bytes));
    println!(
        "PH21_COLLAPSED_LOW_SPREAD={}",
        collapsed_readback.low_spread
    );
    println!(
        "PH21_COLLAPSED_SPREAD_PR={:.8}",
        collapsed_readback.spread.participation_ratio
    );
    assert!(collapsed_readback.low_spread);
    assert_eq!(collapsed_readback.spread.participation_ratio, 0.0);
    assert!(collapsed_readback.signal.is_none());
    assert!(collapsed_readback.differentiation.is_none());

    let empty_error = profile_lens(&registry, algorithmic_id, &[]).expect_err("empty rejected");
    let empty_error_path = root.join("edge-empty-error.txt");
    std::fs::write(&empty_error_path, empty_error.code.as_bytes()).expect("write empty error");
    let empty_error_bytes = std::fs::read(&empty_error_path).expect("read empty error");
    println!("PH21_EDGE_EMPTY_PROBES_ERROR={}", empty_error.code);
    println!(
        "PH21_EDGE_EMPTY_PROBES_ERROR_FILE={}",
        empty_error_path.display()
    );
    println!(
        "PH21_EDGE_EMPTY_PROBES_ERROR_SHA={}",
        digest_hex(&empty_error_bytes)
    );
    assert_eq!(empty_error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    assert_eq!(
        String::from_utf8(empty_error_bytes).expect("empty error utf8"),
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );

    let mixed = vec![
        ProfileProbe::new(Input::new(Modality::Text, b"valid".to_vec())),
        ProfileProbe::new(Input::new(Modality::Image, vec![1, 2, 3])),
    ];
    let mixed_card = profile_lens(&registry, algorithmic_id, &mixed).expect("mixed coverage");
    let mixed_path = root.join("edge-mixed-coverage-card.json");
    write_card(&mixed_path, &mixed_card);
    let mixed_bytes = std::fs::read(&mixed_path).expect("read mixed card");
    let mixed_readback: calyx_registry::CapabilityCard =
        serde_json::from_slice(&mixed_bytes).expect("parse mixed readback");
    println!("PH21_EDGE_MIXED_CARD={}", mixed_path.display());
    println!("PH21_EDGE_MIXED_CARD_SHA={}", digest_hex(&mixed_bytes));
    println!(
        "PH21_EDGE_MIXED_COVERAGE_RATE={:.8}",
        mixed_readback.coverage.rate
    );
    println!("PH21_EDGE_MIXED_FAILED={}", mixed_readback.coverage.failed);
    assert_eq!(mixed_readback.coverage.measured, 1);
    assert_eq!(mixed_readback.coverage.failed, 1);
    assert!(mixed_readback.signal.is_none());
    assert!(mixed_readback.differentiation.is_none());
}

fn assay_backed_profile_readback(
    root: &std::path::Path,
    registry: &Registry,
    lens_id: LensId,
    probes: &[ProfileProbe],
) {
    let slot = slot_for_lens(lens_id);
    let panel = panel_for_slot(slot.clone());
    let cache_key = AssayCacheKey::scoped(
        panel.version,
        "ph21-assay-backed-profile",
        vault_id(),
        AnchorKind::Reward,
    );
    let mut store = AssayStore::default();
    store.put(
        cache_key.clone(),
        AssaySubject::Lens { slot: slot.slot_id },
        estimate(0.42, EstimatorKind::Ksg),
        "ph21 lens signal",
        70,
    );
    store.put(
        cache_key.clone(),
        AssaySubject::Pair {
            a: slot.slot_id,
            b: SlotId::new(7),
        },
        estimate(0.08, EstimatorKind::PairGain),
        "ph21 pair gain",
        71,
    );

    let assay_dir = root.join("assay-cf");
    let mut router = CfRouter::open(&assay_dir, 1024).expect("open assay cf");
    let before_rows = router
        .iter_cf(ColumnFamily::Assay)
        .expect("read before assay");
    println!("PH21_ASSAY_CF_BEFORE_ROWS={}", before_rows.len());
    assert!(before_rows.is_empty());
    store
        .persist_to_aster(&mut router)
        .expect("persist assay rows");
    let cf_rows = router.iter_cf(ColumnFamily::Assay).expect("read assay cf");
    let cf_readback = cf_rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "key": bytes_hex(&row.key),
                "value": bytes_hex(&row.value),
            })
        })
        .collect::<Vec<_>>();
    let cf_path = root.join("assay-cf-readback.json");
    std::fs::write(&cf_path, serde_json::to_vec_pretty(&cf_readback).unwrap())
        .expect("write assay cf readback");
    drop(router);

    let reopened = CfRouter::open(&assay_dir, 1024).expect("reopen assay cf");
    let loaded = AssayStore::load_from_aster(&reopened).expect("load assay rows");
    let assay_card = profile_slot_with_assay(registry, &slot, probes, &loaded, &cache_key).unwrap();
    let assay_card_path = root.join("assay-backed-card.json");
    write_card(&assay_card_path, &assay_card);
    let assay_card_bytes = std::fs::read(&assay_card_path).expect("read assay card");
    let assay_readback: calyx_registry::CapabilityCard =
        serde_json::from_slice(&assay_card_bytes).expect("parse assay card");
    let listing = list_panel_with_assay(&panel, registry, &loaded, &cache_key);
    let listing_path = root.join("assay-backed-panel-listing.json");
    std::fs::write(&listing_path, serde_json::to_vec_pretty(&listing).unwrap())
        .expect("write listing");
    let listing_bytes = std::fs::read(&listing_path).expect("read listing");
    let listing_readback: Vec<calyx_registry::PanelSlotListing> =
        serde_json::from_slice(&listing_bytes).expect("parse listing");

    println!("PH21_ASSAY_CF_READBACK={}", cf_path.display());
    println!("PH21_ASSAY_CARD={}", assay_card_path.display());
    println!("PH21_ASSAY_CARD_SHA={}", digest_hex(&assay_card_bytes));
    println!("PH21_ASSAY_SIGNAL={:?}", assay_readback.signal);
    println!(
        "PH21_ASSAY_SIGNAL_SOURCE={:?}",
        assay_readback.signal_source
    );
    println!(
        "PH21_ASSAY_DIFFERENTIATION={:?}",
        assay_readback.differentiation
    );
    println!("PH21_ASSAY_PANEL_LISTING={}", listing_path.display());
    println!("PH21_ASSAY_PANEL_BITS={:?}", listing_readback[0].bits_about);
    assert_eq!(assay_readback.signal, Some(0.42));
    assert_eq!(
        assay_readback.signal_source,
        calyx_registry::MetricSource::AssayStore
    );
    assert_eq!(assay_readback.differentiation, Some(0.08));
    assert_eq!(
        assay_readback.differentiation_source,
        calyx_registry::MetricSource::AssayStore
    );
    assert_eq!(listing_readback[0].bits_about, Some(0.42));
    assert_eq!(cf_readback.len(), 2);
}

fn fsv_root() -> PathBuf {
    if let Ok(root) = std::env::var("CALYX_FSV_ROOT") {
        return PathBuf::from(root);
    }
    let home = std::env::var("CALYX_HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("data")
        .join(format!("fsv-issue107-test-{}", std::process::id()))
}

fn probe_set() -> Vec<ProfileProbe> {
    vec![
        ProfileProbe::labeled(Input::new(Modality::Text, b"alpha words".to_vec()), "words"),
        ProfileProbe::labeled(Input::new(Modality::Text, b"beta phrase".to_vec()), "words"),
        ProfileProbe::labeled(
            Input::new(Modality::Text, b"12345 67890".to_vec()),
            "digits",
        ),
        ProfileProbe::labeled(
            Input::new(Modality::Text, b"98765 43210".to_vec()),
            "digits",
        ),
    ]
}

fn write_card(path: &std::path::Path, card: &calyx_registry::CapabilityCard) {
    let json = serde_json::to_vec_pretty(card).expect("serialize card");
    std::fs::write(path, json).expect("write card");
}

fn slot_for_lens(lens_id: LensId) -> Slot {
    let slot_id = SlotId::new(0);
    let mut bits_about = std::collections::BTreeMap::new();
    bits_about.insert(
        AnchorKind::Reward,
        Signal {
            bits: 0.31,
            ci: ConfidenceInterval {
                low: 0.30,
                high: 0.32,
            },
            n: 64,
            estimator: "fsv-slot-cache".to_string(),
            ts: 1,
        },
    );
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, "ph21-assay-slot".to_string()),
        lens_id,
        shape: SlotShape::Dense(4),
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: None,
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about,
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}

fn panel_for_slot(slot: Slot) -> Panel {
    Panel {
        version: 1,
        slots: vec![slot],
        created_at: 1,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn estimate(bits: f32, estimator: EstimatorKind) -> MiEstimate {
    MiEstimate::point(bits, 96, estimator, TrustTag::Trusted)
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn digest_hex(bytes: &[u8]) -> String {
    content_address([bytes])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bytes_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone)]
struct CollapsedLens {
    contract: FrozenLensContract,
}

impl CollapsedLens {
    fn new() -> Self {
        Self {
            contract: collapsed_contract("ph21-collapsed"),
        }
    }
}

impl Lens for CollapsedLens {
    fn id(&self) -> LensId {
        self.contract.lens_id()
    }

    fn shape(&self) -> SlotShape {
        SlotShape::Dense(4)
    }

    fn modality(&self) -> Modality {
        Modality::Text
    }

    fn measure(&self, _input: &Input) -> Result<SlotVector> {
        Ok(SlotVector::Dense {
            dim: 4,
            data: vec![1.0, 0.0, 0.0, 0.0],
        })
    }
}

fn collapsed_contract(name: &str) -> FrozenLensContract {
    FrozenLensContract::new(
        name,
        sha256_digest(&[name.as_bytes(), b"weights"]),
        sha256_digest(&[name.as_bytes(), b"corpus"]),
        SlotShape::Dense(4),
        Modality::Text,
        LensDType::F32,
        NormPolicy::None,
    )
}
