use calyx_core::{Input, Lens, LensId, Modality, Result, SlotShape, SlotVector, content_address};
use calyx_registry::frozen::sha256_digest;
use calyx_registry::{
    AlgorithmicLens, FrozenLensContract, LensDType, NormPolicy, ProfileProbe, Registry,
    profile_lens,
};
use std::path::PathBuf;

#[test]
#[ignore = "manual aiwonder FSV test for PH21 capability cards"]
fn ph21_profile_card_aiwonder_fsv() {
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

fn digest_hex(bytes: &[u8]) -> String {
    content_address([bytes])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
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
