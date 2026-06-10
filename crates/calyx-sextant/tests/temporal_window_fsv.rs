use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CALYX_TEMPORAL_INVALID_WINDOW, CxId, LedgerRef};
use calyx_sextant::{
    FreshnessTag, Hit, ProvenanceSource, TemporalFixedClock, TimeWindow, filter_hits_by_window,
};
use serde_json::json;

#[test]
fn temporal_window_fsv_writes_filter_readbacks() {
    let (root, keep_root) = fsv_root();
    reset_dir(&root);
    let output_path = root.join("temporal-window-readback.json");
    let before_output_exists = output_path.exists();

    let clock = TemporalFixedClock::new(1_000_000);
    let window = TimeWindow::last_hours(2, &clock).expect("last two hours");
    let hits = vec![
        hit(1, 996_400, 0.90),
        hit(2, 989_200, 0.80),
        hit(3, 998_200, 0.70),
    ];
    let expected_ids = vec![id_hex(1), id_hex(3)];
    write_json(
        &root.join("temporal-window-input.json"),
        &json!({
            "clock_secs": clock.secs,
            "window": window,
            "hand_expected": {
                "kept_ids": expected_ids,
                "reason": "996400 and 998200 are in [992800,1000000); 989200 is below start"
            },
            "input_hits": hit_readback(&hits),
        }),
    );

    let filtered = filter_hits_by_window(hits.clone(), &window);
    let empty_filtered = filter_hits_by_window(Vec::new(), &window);
    let all_window_hits = vec![hit_without_time(4), hit(5, 10, 0.50)];
    let all_window_filtered = filter_hits_by_window(all_window_hits.clone(), &TimeWindow::all());

    let zero_error = TimeWindow::last_hours(0, &clock).expect_err("zero window fails closed");
    let reversed_error = TimeWindow::new(200, 100).expect_err("reversed window fails closed");
    let overflow_error =
        TimeWindow::last_hours(u64::MAX, &clock).expect_err("overflow fails closed");

    let readback = json!({
        "before_output_exists": before_output_exists,
        "window": window,
        "actual_kept_ids": ids(&filtered),
        "actual_kept_times": filtered.iter().map(|hit| hit.event_time_secs).collect::<Vec<_>>(),
        "expected_kept_ids": vec![id_hex(1), id_hex(3)],
        "matches_expected": ids(&filtered) == vec![id_hex(1), id_hex(3)],
        "order_preserved": filtered.first().map(|hit| hit.rank) == Some(1)
            && filtered.get(1).map(|hit| hit.rank) == Some(3),
        "out_of_window_absent": !ids(&filtered).contains(&id_hex(2)),
        "empty_edge": {
            "before_count": 0,
            "after_count": empty_filtered.len()
        },
        "all_window_edge": {
            "before_ids": ids(&all_window_hits),
            "after_ids": ids(&all_window_filtered),
            "missing_time_retained": all_window_filtered.iter().any(|hit| hit.event_time_secs.is_none())
        },
        "invalid_edges": {
            "zero_hours": zero_error.code,
            "reversed": reversed_error.code,
            "overflow": overflow_error.code,
            "expected": CALYX_TEMPORAL_INVALID_WINDOW
        }
    });
    write_json(&output_path, &readback);
    write_blake3_sums(&root);

    println!("temporal_window_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(ids(&filtered), vec![id_hex(1), id_hex(3)]);
    assert!(empty_filtered.is_empty());
    assert_eq!(all_window_filtered, all_window_hits);
    assert_eq!(zero_error.code, CALYX_TEMPORAL_INVALID_WINDOW);
    assert_eq!(reversed_error.code, CALYX_TEMPORAL_INVALID_WINDOW);
    assert_eq!(overflow_error.code, CALYX_TEMPORAL_INVALID_WINDOW);

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn hit(seed: u8, event_time_secs: i64, score: f32) -> Hit {
    let mut hit = hit_without_time(seed);
    hit.event_time_secs = Some(event_time_secs);
    hit.score = score;
    hit
}

fn hit_without_time(seed: u8) -> Hit {
    Hit {
        cx_id: CxId::from_bytes([seed; 16]),
        score: 0.0,
        rank: seed as usize,
        event_time_secs: None,
        temporal_scores: None,
        causal_confidence: calyx_sextant::CausalConfidence::Absent,
        causal_gate: None,
        per_lens: Vec::new(),
        cross_terms_used: false,
        guard: None,
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        provenance_source: ProvenanceSource::Stub,
        freshness: FreshnessTag::fresh(0),
        explain: None,
    }
}

fn ids(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.cx_id.to_string()).collect()
}

fn id_hex(seed: u8) -> String {
    CxId::from_bytes([seed; 16]).to_string()
}

fn hit_readback(hits: &[Hit]) -> Vec<serde_json::Value> {
    hits.iter()
        .map(|hit| {
            json!({
                "cx_id": hit.cx_id.to_string(),
                "rank": hit.rank,
                "score": hit.score,
                "event_time_secs": hit.event_time_secs,
            })
        })
        .collect()
}

fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

fn write_blake3_sums(root: &Path) {
    let mut entries = fs::read_dir(root)
        .expect("read root")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    entries.sort();
    let mut lines = String::new();
    for path in entries {
        if path.file_name().and_then(|name| name.to_str()) == Some("BLAKE3SUMS.txt") {
            continue;
        }
        let bytes = fs::read(&path).expect("read artifact");
        let name = path.file_name().expect("file name").to_string_lossy();
        lines.push_str(&format!("{}  {}\n", blake3_hex(&bytes), name));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines).expect("write checksums");
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn fsv_root() -> (PathBuf, bool) {
    if let Ok(root) = std::env::var("CALYX_TEMPORAL_WINDOW_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    (
        std::env::temp_dir().join(format!("calyx-temporal-window-fsv-{}", std::process::id())),
        false,
    )
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv root");
}
