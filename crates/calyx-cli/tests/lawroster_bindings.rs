//! Refs #1913 — freeze test for the six structured lens bindings + frozen
//! vocabularies of the `courtlistener-opinion-v1` schema (roster-rebuild wave).
//!
//! Every committed binding under `tests/lawroster/bindings/` is loaded through the
//! REAL `calyx_registry::StructuredBinding` parser (the same parser
//! `calyx lens add --runtime algorithmic --modality structured --weights <b>.json`
//! feeds). No mocks: the parser validates the vocab, the shape is the runtime's
//! own `StructuredBinding::shape()`, and each binding is round-tripped through the
//! canonical runtime-kind freeze path used at `lens add` time. Corrupted copies
//! must fail closed.

use std::path::{Path, PathBuf};

use calyx_core::SlotShape;
use calyx_registry::{StructuredBinding, StructuredEncoder};
use serde_json::Value;

const SCHEMA: &str = "courtlistener-opinion-v1";

fn lawroster_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("lawroster")
}

fn binding_bytes(name: &str) -> Vec<u8> {
    let path = lawroster_dir().join("bindings").join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn load_binding(name: &str) -> StructuredBinding {
    StructuredBinding::from_json_bytes(&binding_bytes(name))
        .unwrap_or_else(|error| panic!("frozen binding {name} rejected by real parser: {error:?}"))
}

fn vocab_tokens(name: &str) -> Vec<String> {
    let path = lawroster_dir().join("vocabs").join(name);
    let value: Value = serde_json::from_slice(
        &std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("vocab {name} is not JSON: {error}"));
    value["tokens"]
        .as_array()
        .unwrap_or_else(|| panic!("vocab {name} has no tokens array"))
        .iter()
        .map(|token| {
            token
                .as_str()
                .unwrap_or_else(|| panic!("vocab {name} token is not a string"))
                .to_owned()
        })
        .collect()
}

fn dim(shape: SlotShape) -> u32 {
    match shape {
        SlotShape::Dense(value) | SlotShape::Sparse(value) => value,
        SlotShape::Multi { token_dim } => token_dim,
    }
}

/// (file, expected shape) for every committed lane binding.
fn frozen_lanes() -> Vec<(&'static str, SlotShape)> {
    vec![
        ("lane0-court-onehot.binding.json", SlotShape::Dense(8)),
        ("lane1-date-cyclic-month.binding.json", SlotShape::Dense(2)),
        (
            "lane2-judge-panel-multihot.binding.json",
            SlotShape::Sparse(80),
        ),
        (
            "lane3-docket-stablehash.binding.json",
            SlotShape::Sparse(1024),
        ),
        ("lane4a-status-onehot.binding.json", SlotShape::Dense(2)),
        (
            "lane4b-per-curiam-boolean.binding.json",
            SlotShape::Dense(1),
        ),
        ("lane5-citation-log1p.binding.json", SlotShape::Dense(1)),
    ]
}

#[test]
fn every_frozen_binding_loads_and_has_expected_shape() {
    for (name, expected) in frozen_lanes() {
        let binding = load_binding(name);
        assert_eq!(
            binding.schema_id(),
            SCHEMA,
            "{name} declares a foreign schema id"
        );
        assert_eq!(
            binding.shape(),
            expected,
            "{name} emits {:?}, expected {expected:?}",
            binding.shape()
        );
    }
}

#[test]
fn every_frozen_binding_round_trips_through_the_canonical_freeze_path() {
    // `lens add` freezes a binding into its runtime kind and re-decodes it; a
    // binding that is not canonical would be rejected at register time.
    for (name, _) in frozen_lanes() {
        let binding = load_binding(name);
        let kind = binding.runtime_kind().unwrap_or_else(|error| {
            panic!("{name} could not be frozen to a runtime kind: {error:?}")
        });
        assert!(
            StructuredBinding::is_runtime_kind(&kind),
            "{name} kind prefix"
        );
        let decoded = StructuredBinding::from_runtime_kind(&kind)
            .unwrap_or_else(|error| panic!("{name} runtime kind did not decode: {error:?}"));
        assert_eq!(
            decoded, binding,
            "{name} did not survive a freeze round-trip"
        );
    }
}

#[test]
fn lane0_court_onehot_dim_equals_frozen_court_vocab_len() {
    let binding = load_binding("lane0-court-onehot.binding.json");
    let tokens = vocab_tokens("court_vocab.json");
    assert_eq!(tokens.len(), 8, "frozen court vocab size drifted");
    match binding.encoder() {
        StructuredEncoder::OneHot { categories } => {
            assert_eq!(
                categories, &tokens,
                "lane0 OneHot categories disagree with the frozen court vocab file"
            );
            assert_eq!(dim(binding.shape()), categories.len() as u32);
        }
        other => panic!("lane0 is not a OneHot encoder: {other:?}"),
    }
}

#[test]
fn lane2_judge_multihot_dim_equals_frozen_judge_vocab_len() {
    let binding = load_binding("lane2-judge-panel-multihot.binding.json");
    let tokens = vocab_tokens("judge_vocab.json");
    assert_eq!(
        tokens.len(),
        80,
        "frozen judge roster size drifted from #1466"
    );
    match binding.encoder() {
        StructuredEncoder::SetMultiHot { vocabulary } => {
            assert_eq!(
                vocabulary, &tokens,
                "lane2 SetMultiHot vocabulary disagrees with the frozen judge vocab file"
            );
            assert_eq!(dim(binding.shape()), vocabulary.len() as u32);
        }
        other => panic!("lane2 is not a SetMultiHot encoder: {other:?}"),
    }
}

#[test]
fn lane4a_status_onehot_dim_equals_frozen_status_vocab_len() {
    let binding = load_binding("lane4a-status-onehot.binding.json");
    let tokens = vocab_tokens("status_vocab.json");
    assert_eq!(
        tokens,
        vec!["Published".to_owned(), "Unpublished".to_owned()]
    );
    match binding.encoder() {
        StructuredEncoder::OneHot { categories } => {
            assert_eq!(categories, &tokens);
            assert_eq!(dim(binding.shape()), 2);
        }
        other => panic!("lane4a is not a OneHot encoder: {other:?}"),
    }
}

#[test]
fn corrupted_binding_copies_fail_closed() {
    // A committed binding is only trustworthy if the parser refuses tampered bytes.
    // Start from the real lane0 file and mutate it three independent ways.
    let mut value: Value =
        serde_json::from_slice(&binding_bytes("lane0-court-onehot.binding.json"))
            .expect("lane0 is valid JSON");

    // 1. Duplicate category — violates the OneHot uniqueness invariant.
    let mut duplicated = value.clone();
    let cats = duplicated["encoder"]["categories"]
        .as_array()
        .unwrap()
        .clone();
    let mut dup = cats.clone();
    dup.push(cats[0].clone());
    duplicated["encoder"]["categories"] = Value::Array(dup);
    assert!(
        StructuredBinding::from_json_bytes(&serde_json::to_vec(&duplicated).unwrap()).is_err(),
        "parser accepted a OneHot vocab with a duplicate category"
    );

    // 2. Unknown field — the binding struct is deny_unknown_fields.
    value["injected_extra_field"] = Value::Bool(true);
    assert!(
        StructuredBinding::from_json_bytes(&serde_json::to_vec(&value).unwrap()).is_err(),
        "parser accepted a binding with an unknown field"
    );

    // 3. Truncated/garbage bytes.
    let mut truncated = binding_bytes("lane0-court-onehot.binding.json");
    truncated.truncate(truncated.len() / 2);
    assert!(
        StructuredBinding::from_json_bytes(&truncated).is_err(),
        "parser accepted truncated binding bytes"
    );
}
