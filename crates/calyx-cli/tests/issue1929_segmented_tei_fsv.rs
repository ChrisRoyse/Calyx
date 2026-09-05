//! Issue #1929 FSV — native multi-segment (`SegmentedMulti`) representation.
//!
//! This is an opt-in real-service full-state verification. It exercises the
//! production long-input measurement path (`TeiSegmentedLens`) against a live
//! frozen E5 TEI service, persists each measured slot into a durable
//! `AsterVault`, then reopens the vault from disk and reads the typed slot back
//! (DB-native readback, not in-process state). It proves that long inputs keep
//! every ordered segment as a distinct vector with exact UTF-8 byte and token
//! ranges, that the segmentation policy identity is preserved, and that no
//! consumer can silently receive a pooled/flattened surrogate.
//!
//! Run it explicitly against the frozen E5 service:
//! ```text
//! CALYX_FSV_1929_TEI=http://127.0.0.1:18190/embed \
//! CALYX_FSV_1929_OPINION=/path/to/opinion.bin \
//! CALYX_FSV_1929_OUT=/tmp/issue1929-fsv \
//! cargo test -p calyx-cli --test issue1929_segmented_tei_fsv -- --nocapture
//! ```
//! Without `CALYX_FSV_1929_TEI` the test is skipped (CI has no TEI service).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use calyx_aster::vault::encode::{decode_slot_vector, encode_slot_vector};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, CxId, Input, InputRef, LedgerRef, Lens, Modality, SlotId, SlotVector,
    VaultStore, slot_similarity,
};
use calyx_registry::{DEFAULT_TEI_SEGMENTATION_POLICY, TeiSegmentedLens};
use serde_json::{Value, json};

const DIM: u32 = 768;
const CAPACITY: u32 = 510; // max_input_tokens(512) - special_tokens(2)
const OVERLAP: u32 = 32;
const SALT: &[u8] = b"issue1929-segmented-fsv-salt";

fn endpoint() -> Option<String> {
    std::env::var("CALYX_FSV_1929_TEI").ok()
}

fn out_dir() -> PathBuf {
    std::env::var("CALYX_FSV_1929_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("issue1929-segmented-fsv"))
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into()
}

/// Content tokens (start, stop) byte offsets from the real service, exactly as
/// the lens sees them (`add_special_tokens=false`).
fn content_tokens(endpoint: &str, text: &str) -> Vec<(usize, usize)> {
    let tok = endpoint
        .strip_suffix("/embed")
        .map(|base| format!("{base}/tokenize"))
        .expect("endpoint ends in /embed");
    let mut resp = agent()
        .post(&tok)
        .header("Accept", "application/json")
        .send_json(json!({"inputs": text, "add_special_tokens": false}))
        .expect("tokenize request");
    let value: Value = resp.body_mut().read_json().expect("tokenize json");
    value[0]
        .as_array()
        .expect("token array")
        .iter()
        .filter(|t| !t["special"].as_bool().unwrap_or(false))
        .map(|t| {
            (
                t["start"].as_u64().expect("start") as usize,
                t["stop"].as_u64().expect("stop") as usize,
            )
        })
        .collect()
}

/// Builds ASCII text that tokenizes to exactly `target` content tokens by
/// growing single-letter words; fails closed if it cannot land exactly.
fn text_with_exact_tokens(endpoint: &str, target: usize) -> String {
    let mut words = target;
    for _ in 0..32 {
        let text = vec!["a"; words].join(" ");
        let count = content_tokens(endpoint, &text).len();
        if count == target {
            return text;
        }
        if count < target {
            words += target - count;
        } else {
            words -= (count - target).min(words.saturating_sub(1));
        }
    }
    panic!("could not build text with exactly {target} content tokens");
}

fn lens(endpoint: &str) -> TeiSegmentedLens {
    TeiSegmentedLens::new(
        "issue1929-e5-segmented",
        endpoint,
        Modality::Text,
        DIM,
        DEFAULT_TEI_SEGMENTATION_POLICY,
    )
    .expect("segmented lens")
}

fn source_hash(sv: &SlotVector) -> [u8; 32] {
    match sv {
        SlotVector::SegmentedMulti { provenance, .. } => provenance.source_hash,
        other => panic!("expected SegmentedMulti, got {other:?}"),
    }
}

/// Persists the measured slot into a durable vault, reopens from disk, and
/// returns the DB-native read-back slot alongside the encode/decode round-trip.
fn persist_and_read_back(source: &[u8], measured: &SlotVector) -> SlotVector {
    let dir = out_dir().join(format!("vault-{}", hex8(&source_hash(measured))));
    let _ = std::fs::remove_dir_all(&dir);
    let vault_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap();
    let slot = SlotId::new(9);
    let cx_id = CxId::from_input(source, 1, SALT);
    let cx = Constellation {
        cx_id,
        vault_id,
        panel_version: 1,
        created_at: 1_785_500_000,
        input_ref: InputRef {
            hash: source_hash(measured),
            pointer: None,
            redacted: false,
        },
        modality: Modality::Text,
        slots: BTreeMap::from([(slot, measured.clone())]),
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0u8; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    };

    let stored_id;
    {
        let vault = AsterVault::new_durable(&dir, vault_id, SALT.to_vec(), VaultOptions::default())
            .unwrap();
        stored_id = vault.put(cx).expect("durable put of segmented slot");
        vault.flush().expect("flush");
    }
    // Cold reopen from disk: any in-process state is gone.
    let reopened =
        AsterVault::new_durable(&dir, vault_id, SALT.to_vec(), VaultOptions::default()).unwrap();
    let read = reopened
        .get(stored_id, reopened.snapshot())
        .expect("read back constellation from disk");
    let slot_vec = read
        .slots
        .get(&slot)
        .expect("segmented slot present")
        .clone();

    // Independent byte-level codec round-trip proof.
    let encoded = encode_slot_vector(measured).expect("encode");
    let decoded = decode_slot_vector(&encoded).expect("decode");
    assert_eq!(&decoded, measured, "encode/decode must be byte-lossless");
    slot_vec
}

fn hex8(bytes: &[u8; 32]) -> String {
    bytes[..4].iter().map(|b| format!("{b:02x}")).collect()
}

fn write_evidence(name: &str, value: &Value) {
    let dir = out_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    println!("ISSUE1929_FSV_EVIDENCE={}", path.display());
}

/// Verifies the structural invariants of a measured segmented slot against an
/// independent full-source tokenization from the same live service and returns
/// a compact JSON summary of the persisted state.
fn verify_segmented(
    label: &str,
    endpoint: &str,
    source: &[u8],
    measured: &SlotVector,
    read_back: &SlotVector,
    expected_segments: Option<usize>,
) -> Value {
    assert_eq!(
        read_back, measured,
        "[{label}] DB-native readback must equal the measured slot byte-for-byte"
    );
    let text = std::str::from_utf8(source).expect("utf8 source");
    let full = content_tokens(endpoint, text);
    let (token_dim, provenance, segments) = match measured {
        SlotVector::SegmentedMulti {
            token_dim,
            provenance,
            segments,
        } => (*token_dim, provenance, segments),
        other => panic!("[{label}] expected SegmentedMulti, got {other:?}"),
    };
    assert_eq!(token_dim, DIM, "[{label}] token_dim");
    assert_eq!(
        provenance.source_byte_len as usize,
        source.len(),
        "[{label}] source byte length"
    );
    assert_eq!(
        provenance.source_token_count as usize,
        full.len(),
        "[{label}] source content-token count matches independent /tokenize"
    );
    assert_eq!(
        provenance.max_input_tokens,
        CAPACITY + 2,
        "[{label}] policy"
    );
    assert_eq!(
        provenance.overlap_tokens as u32, OVERLAP,
        "[{label}] overlap"
    );
    assert_eq!(
        provenance.policy_hash,
        DEFAULT_TEI_SEGMENTATION_POLICY.content_hash(),
        "[{label}] frozen segmentation-policy identity is preserved"
    );
    if let Some(expected) = expected_segments {
        assert_eq!(segments.len(), expected, "[{label}] segment count");
    }

    // First/last coverage and per-segment byte/token exactness.
    let first = &segments[0];
    assert_eq!(first.byte_start, 0, "[{label}] first byte_start");
    assert_eq!(first.token_start, 0, "[{label}] first token_start");
    let last = segments.last().unwrap();
    assert_eq!(
        last.byte_end as usize,
        source.len(),
        "[{label}] last segment covers to end of source"
    );
    assert_eq!(
        last.token_end as usize,
        full.len(),
        "[{label}] last token_end covers all content tokens"
    );

    let mut prev_end: Option<u32> = None;
    for (idx, seg) in segments.iter().enumerate() {
        assert!(
            text.is_char_boundary(seg.byte_start as usize)
                && text.is_char_boundary(seg.byte_end as usize),
            "[{label}] seg {idx} byte bounds land on UTF-8 char boundaries"
        );
        assert!(
            seg.byte_start < seg.byte_end,
            "[{label}] seg {idx} nonempty"
        );
        assert_eq!(
            seg.data.len(),
            DIM as usize,
            "[{label}] seg {idx} vector dim"
        );
        // Byte offsets must match the independent full-source token offsets.
        let expect_start = if seg.token_start == 0 {
            0
        } else {
            full[seg.token_start as usize].0
        };
        let expect_end = if seg.token_end as usize == full.len() {
            source.len()
        } else {
            full[seg.token_end as usize].0
        };
        assert_eq!(
            seg.byte_start as usize, expect_start,
            "[{label}] seg {idx} byte_start matches token offset"
        );
        assert_eq!(
            seg.byte_end as usize, expect_end,
            "[{label}] seg {idx} byte_end matches token offset"
        );
        if let Some(prev_end) = prev_end {
            assert_eq!(
                seg.token_start,
                prev_end - OVERLAP,
                "[{label}] seg {idx} overlaps the previous segment by exactly {OVERLAP} tokens"
            );
        }
        prev_end = Some(seg.token_end);
    }

    // Native downstream consumer: per-segment late-interaction evidence with no
    // flattening. Self-similarity keeps every block and the segmentation.
    let evidence = slot_similarity(measured, measured).expect("native block similarity");
    assert_eq!(
        evidence.left_block_count as usize,
        segments.len(),
        "[{label}] similarity retains every segment block"
    );
    assert!(
        evidence.left_segmentation.is_some(),
        "[{label}] similarity carries segmentation provenance"
    );
    assert!(
        evidence.score > 0.999,
        "[{label}] self MaxSim ~1.0, got {}",
        evidence.score
    );
    // Anti-silent-fallback: a dense consumer can never get a pooled surrogate.
    assert!(
        measured.as_dense().is_none(),
        "[{label}] as_dense() must be None for a typed segment block"
    );

    json!({
        "label": label,
        "source_byte_len": provenance.source_byte_len,
        "source_token_count": provenance.source_token_count,
        "segment_count": segments.len(),
        "policy_hash_hex": hex8(&provenance.policy_hash),
        "source_hash_hex": hex8(&provenance.source_hash),
        "as_dense_is_none": measured.as_dense().is_none(),
        "similarity_block_count": evidence.left_block_count,
        "similarity_score": evidence.score,
        "segments": segments.iter().take(4).map(|s| json!({
            "byte_start": s.byte_start,
            "byte_end": s.byte_end,
            "token_start": s.token_start,
            "token_end": s.token_end,
            "measured_tokens": s.measured_tokens,
        })).collect::<Vec<_>>(),
    })
}

#[test]
fn issue1929_segmented_tei_native_representation_fsv() {
    let Some(endpoint) = endpoint() else {
        eprintln!("SKIP issue1929 FSV: set CALYX_FSV_1929_TEI=http://127.0.0.1:18190/embed");
        return;
    };
    let lens = lens(&endpoint);
    let mut report = serde_json::Map::new();

    // (a) Short input, well under one context -> exactly one segment.
    let short = b"Native late-interaction segmentation FSV short input.".to_vec();
    let m = lens
        .measure(&Input::new(Modality::Text, short.clone()))
        .unwrap();
    let rb = persist_and_read_back(&short, &m);
    report.insert(
        "short".into(),
        verify_segmented("short", &endpoint, &short, &m, &rb, Some(1)),
    );

    // (b) Exactly at the 510-content-token boundary -> still one segment.
    let boundary = text_with_exact_tokens(&endpoint, CAPACITY as usize).into_bytes();
    let m = lens
        .measure(&Input::new(Modality::Text, boundary.clone()))
        .unwrap();
    let rb = persist_and_read_back(&boundary, &m);
    report.insert(
        "boundary_510".into(),
        verify_segmented("boundary_510", &endpoint, &boundary, &m, &rb, Some(1)),
    );

    // (c) One token over the boundary -> two segments with exact overlap.
    let over = text_with_exact_tokens(&endpoint, CAPACITY as usize + 1).into_bytes();
    let m = lens
        .measure(&Input::new(Modality::Text, over.clone()))
        .unwrap();
    let rb = persist_and_read_back(&over, &m);
    report.insert(
        "over_511".into(),
        verify_segmented("over_511", &endpoint, &over, &m, &rb, Some(2)),
    );

    // (d) Multibyte UTF-8 content long enough to exceed one context.
    let unit = "café ☕ مرحبا 😀 Ohio v. Cuyahoga — appellÉe held. ";
    let mut multibyte = String::from(unit);
    while content_tokens(&endpoint, &multibyte).len() <= CAPACITY as usize + OVERLAP as usize {
        multibyte.push_str(unit);
    }
    let multibyte = multibyte.into_bytes();
    let m = lens
        .measure(&Input::new(Modality::Text, multibyte.clone()))
        .unwrap();
    let rb = persist_and_read_back(&multibyte, &m);
    report.insert(
        "multibyte".into(),
        verify_segmented("multibyte", &endpoint, &multibyte, &m, &rb, None),
    );

    // (e) Very long real CourtListener opinion (optional path).
    if let Ok(path) = std::env::var("CALYX_FSV_1929_OPINION") {
        let opinion = std::fs::read(&path).expect("read opinion bytes");
        let m = lens
            .measure(&Input::new(Modality::Text, opinion.clone()))
            .unwrap();
        let rb = persist_and_read_back(&opinion, &m);
        report.insert(
            "real_opinion".into(),
            verify_segmented("real_opinion", &endpoint, &opinion, &m, &rb, None),
        );
    }

    // (f) Empty input must be a structured rejection, never a zero vector.
    let empty_err = lens
        .measure(&Input::new(Modality::Text, Vec::new()))
        .expect_err("empty input must fail closed");
    assert_eq!(empty_err.code, "CALYX_LENS_SEGMENTATION_INVALID");
    report.insert(
        "empty_rejection".into(),
        json!({"code": empty_err.code, "message": empty_err.message}),
    );

    write_evidence("issue1929-segmented-tei-fsv.json", &Value::Object(report));
}
