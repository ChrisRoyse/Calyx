use calyx_core::{Input, Lens, Modality};
use calyx_registry::{
    DecayFunction, E2RecencyConfig, E2RecencyLens, E3PeriodicConfig, E3PeriodicLens,
    E4PositionalConfig, E4PositionalLens, PeriodicOptions, SequenceOptions, TEMPORAL_FLAGS,
    civic_default, code_default, instantiate_panel, media_default, text_default,
};
use serde_json::json;
use std::path::PathBuf;

#[test]
#[ignore = "manual FSV test for PH22 default panels and temporal lenses"]
fn ph22_panels_temporal_manual_fsv() {
    let root = fsv_root();
    std::fs::create_dir_all(&root).expect("create fsv root");

    let templates = [
        text_default(),
        code_default(),
        civic_default(),
        media_default(),
    ];
    let mut panel_lines = Vec::new();
    let mut flag_readback = Vec::new();
    for template in &templates {
        let instantiated = instantiate_panel(template, 22);
        let names = instantiated
            .panel
            .slots
            .iter()
            .map(|slot| slot.slot_key.key().to_string())
            .collect::<Vec<_>>();
        let line = format!(
            "{}:{}:{}",
            instantiated.template_name,
            instantiated.panel.slots.len(),
            names.join(",")
        );
        println!("PH22_PANEL={line}");
        panel_lines.push(line);
        assert!(names.ends_with(&[
            "E2_recency".to_string(),
            "E3_periodic".to_string(),
            "E4_positional".to_string()
        ]));
        let spec_flags = template
            .slots
            .iter()
            .rev()
            .take(3)
            .all(|slot| slot.retrieval_only && slot.excluded_from_dedup);
        let core_flags = instantiated
            .panel
            .slots
            .iter()
            .rev()
            .take(3)
            .all(|slot| slot.retrieval_only && slot.excluded_from_dedup);
        flag_readback.push(json!({
            "template": instantiated.template_name,
            "spec_temporal_flags": spec_flags,
            "core_slot_temporal_flags": core_flags,
        }));
        assert!(spec_flags);
        assert!(core_flags);
    }
    std::fs::write(root.join("panel-readback.txt"), panel_lines.join("\n"))
        .expect("write panel readback");
    std::fs::write(
        root.join("temporal-core-flags-readback.json"),
        serde_json::to_vec_pretty(&flag_readback).unwrap(),
    )
    .expect("write temporal flags readback");

    let e2 = E2RecencyLens::new(E2RecencyConfig {
        decay: DecayFunction::Linear {
            max_age_secs: 200_000,
        },
        reference_time: 1_000_000,
    });
    let e2_score = dense1(e2.measure(&ts(900_000)).expect("measure e2"));
    println!("PH22_E2_LINEAR_SCORE={e2_score:.8}");
    assert_eq!(e2_score, 0.5);

    let e2_exp = E2RecencyLens::new(E2RecencyConfig {
        decay: DecayFunction::Exponential {
            half_life_secs: 86_400,
        },
        reference_time: 86_400,
    });
    let e2_exp_score = dense1(e2_exp.measure(&ts(0)).expect("measure e2 exp"));
    println!("PH22_E2_EXP_SCORE={e2_exp_score:.8}");
    assert!((e2_exp_score - 0.5).abs() < 1e-6);

    let e3_hour = E3PeriodicLens::new(E3PeriodicConfig {
        options: PeriodicOptions {
            target_hour: Some(14),
            ..PeriodicOptions::default()
        },
        reference_time: 0,
    });
    let e3_hour_score = dense(e3_hour.measure(&ts(14 * 3_600 + 30 * 60)).unwrap())[0];
    println!("PH22_E3_HOUR_SCORE={e3_hour_score:.8}");
    assert_eq!(e3_hour_score, 1.0);

    let e3_dow = E3PeriodicLens::new(E3PeriodicConfig {
        options: PeriodicOptions {
            target_day_of_week: Some(3),
            ..PeriodicOptions::default()
        },
        reference_time: 0,
    });
    let monday = 4 * 86_400;
    let e3_dow_score = dense(e3_dow.measure(&ts(monday)).unwrap())[1];
    println!("PH22_E3_DOW_SCORE={e3_dow_score:.8}");
    assert!((e3_dow_score - (1.0 - 3.0 / 3.5)).abs() < 1e-6);

    let e4 = E4PositionalLens::new(E4PositionalConfig {
        options: SequenceOptions::default(),
    });
    let e4_mid = dense(e4.measure(&pos(50, 100)).expect("measure e4"));
    println!("PH22_E4_MIDPOINT={e4_mid:?}");
    assert_close(&e4_mid, &[1.0, 0.0, 1.0, 0.0], 1e-6);

    println!(
        "PH22_TEMPORAL_FLAGS=retrieval_only:{} excluded_from_dedup:{}",
        TEMPORAL_FLAGS.retrieval_only, TEMPORAL_FLAGS.excluded_from_dedup
    );
    let text_template = text_default();
    assert!(
        text_template
            .temporal_specs()
            .all(|slot| slot.retrieval_only && slot.excluded_from_dedup)
    );

    let bad_e2 = e2
        .measure(&Input::new(Modality::Structured, vec![1]))
        .unwrap_err();
    println!("PH22_EDGE_E2_BAD_INPUT={}", bad_e2.code);
    let e4_end = dense(e4.measure(&pos(100, 100)).expect("measure e4 end"));
    println!("PH22_EDGE_E4_END={e4_end:?}");
    assert_close(&e4_end, &[0.0, -1.0, 0.0, 1.0], 1e-5);
}

fn fsv_root() -> PathBuf {
    if let Ok(root) = std::env::var("CALYX_FSV_ROOT") {
        return PathBuf::from(root);
    }
    let home = std::env::var("CALYX_HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join("data")
        .join(format!("fsv-issue108-test-{}", std::process::id()))
}

fn ts(value: i64) -> Input {
    Input::new(Modality::Structured, value.to_le_bytes().to_vec())
}

fn pos(position: u64, total: u64) -> Input {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&position.to_le_bytes());
    bytes.extend_from_slice(&total.to_le_bytes());
    Input::new(Modality::Structured, bytes)
}

fn dense(vector: calyx_core::SlotVector) -> Vec<f32> {
    vector.as_dense().expect("dense vector").to_vec()
}

fn dense1(vector: calyx_core::SlotVector) -> f32 {
    dense(vector)[0]
}

fn assert_close(actual: &[f32], expected: &[f32], tolerance: f32) {
    for (actual, expected) in actual.iter().zip(expected) {
        assert!((*actual - *expected).abs() <= tolerance);
    }
}
