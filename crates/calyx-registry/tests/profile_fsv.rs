use calyx_core::{Input, Lens, LensId, Modality, Result, SlotShape, SlotVector, content_address};
use calyx_registry::{AlgorithmicLens, ProfileProbe, Registry, profile_lens};
use std::path::PathBuf;

#[test]
#[ignore = "manual aiwonder FSV test for PH21 capability cards"]
fn ph21_profile_card_aiwonder_fsv() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).expect("create fsv root");

    let mut registry = Registry::new();
    let algorithmic_id = registry
        .register(AlgorithmicLens::byte_features(
            "ph21-profile-fsv",
            Modality::Text,
        ))
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
    println!("PH21_SIGNAL={:.8}", readback.signal);
    println!("PH21_DIFFERENTIATION={:.8}", readback.differentiation);
    println!("PH21_SPREAD_PR={:.8}", readback.spread.participation_ratio);
    println!(
        "PH21_SPREAD_NORM={:.8}",
        readback.spread.normalized_participation_ratio
    );
    println!("PH21_SEPARATION={:.8}", readback.separation.score);
    println!("PH21_COST_MS_PER_INPUT={:.8}", readback.cost.ms_per_input);
    println!("PH21_COVERAGE_RATE={:.8}", readback.coverage.rate);
    assert_eq!(readback.coverage.failed, 0);
    assert!(readback.spread.participation_ratio > 0.0);

    let collapsed_id = registry
        .register(CollapsedLens)
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

    let empty_error = profile_lens(&registry, algorithmic_id, &[]).expect_err("empty rejected");
    println!("PH21_EDGE_EMPTY_PROBES_ERROR={}", empty_error.code);
    assert_eq!(empty_error.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");

    let mixed = vec![
        ProfileProbe::new(Input::new(Modality::Text, b"valid".to_vec())),
        ProfileProbe::new(Input::new(Modality::Image, vec![1, 2, 3])),
    ];
    let mixed_card = profile_lens(&registry, algorithmic_id, &mixed).expect("mixed coverage");
    println!(
        "PH21_EDGE_MIXED_COVERAGE_RATE={:.8}",
        mixed_card.coverage.rate
    );
    println!("PH21_EDGE_MIXED_FAILED={}", mixed_card.coverage.failed);
    assert_eq!(mixed_card.coverage.measured, 1);
    assert_eq!(mixed_card.coverage.failed, 1);
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

struct CollapsedLens;

impl Lens for CollapsedLens {
    fn id(&self) -> LensId {
        LensId::from_bytes([0x21; 16])
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
