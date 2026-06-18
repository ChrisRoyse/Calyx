use super::*;
use calyx_core::Input;
use proptest::prelude::*;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn known_sentence_is_byte_exact() {
    let fixture = Fixture::new("known", &[[0, 0, 0], [3, 0, 0], [0, 4, 0], [0, 0, 5]]);
    let lens = fixture.lens(None).unwrap();

    let vector = lens
        .measure(&Input::new(Modality::Text, b"alpha beta".to_vec()))
        .unwrap();

    let SlotVector::Dense { dim, data } = vector else {
        panic!("expected dense vector");
    };
    assert_eq!(dim, 3);
    assert_eq!(
        data.iter().map(|value| value.to_bits()).collect::<Vec<_>>(),
        vec![0.6_f32.to_bits(), 0.8_f32.to_bits(), 0.0_f32.to_bits()]
    );
}

proptest! {
    #[test]
    fn mean_pool_is_permutation_invariant(words in prop::collection::vec(0_u8..3, 1..12)) {
        let fixture = Fixture::new("permutation", &[
            [0, 0, 0],
            [3, 0, 0],
            [0, 4, 0],
            [0, 0, 5],
        ]);
        let lens = fixture.lens(None).unwrap();
        let forward = sentence(&words);
        let mut reversed_words = words.clone();
        reversed_words.reverse();
        let reversed = sentence(&reversed_words);

        let left = lens.measure(&Input::new(Modality::Text, forward.into_bytes())).unwrap();
        let right = lens.measure(&Input::new(Modality::Text, reversed.into_bytes())).unwrap();

        prop_assert_eq!(left, right);
    }
}

#[test]
fn all_oov_and_empty_return_zero_safe_unit_vector() {
    let fixture = Fixture::new("fallback", &[[0, 0, 0], [3, 0, 0], [0, 4, 0], [0, 0, 5]]);
    let lens = fixture.lens(None).unwrap();

    let oov = lens
        .measure(&Input::new(Modality::Text, b"doesnotexist".to_vec()))
        .unwrap();
    let empty = lens
        .measure(&Input::new(Modality::Text, b"  ".to_vec()))
        .unwrap();

    let expected = SlotVector::Dense {
        dim: 3,
        data: vec![1.0, 0.0, 0.0],
    };
    assert_eq!(oov, expected);
    assert_eq!(empty, expected);
}

#[test]
fn from_lens_spec_rejects_dim_mismatch() {
    let fixture = Fixture::new(
        "dim-mismatch",
        &[[0, 0, 0], [3, 0, 0], [0, 4, 0], [0, 0, 5]],
    );
    let spec = LensSpec {
        name: "bad-static".to_string(),
        runtime: LensRuntime::StaticLookup {
            embeddings_file: fixture.matrix.clone(),
            tokenizer: fixture.tokenizer.clone(),
            dim: 2,
        },
        output: SlotShape::Dense(2),
        modality: Modality::Text,
        weights_sha256: hash_files(&[fixture.matrix.clone(), fixture.tokenizer.clone()]).unwrap(),
        corpus_hash: sha256_digest(&[b"test"]),
        norm_policy: NormPolicy::unit(),
        max_batch: None,
        axis: None,
        asymmetry: calyx_core::Asymmetry::None,
        quant_default: calyx_core::QuantPolicy::turboquant_default(),
        truncate_dim: None,
        recall_delta: crate::spec::default_recall_delta(),
        retrieval_only: false,
        excluded_from_dedup: false,
    };

    let error = StaticLookupLens::from_lens_spec(&spec).unwrap_err();

    assert_eq!(error.code, "CALYX_LENS_DIM_MISMATCH");
}

#[test]
fn frozen_contract_hashes_matrix_and_tokenizer() {
    let fixture = Fixture::new("hash", &[[0, 0, 0], [3, 0, 0], [0, 4, 0], [0, 0, 5]]);
    let lens = fixture.lens(None).unwrap();
    let expected = hash_files(&[fixture.matrix.clone(), fixture.tokenizer.clone()]).unwrap();

    assert_eq!(lens.contract().weights_sha256(), expected);
}

#[test]
fn f16_matrix_decodes_and_normalizes() {
    let fixture = Fixture::new_f16(
        "f16",
        &[[0x0000, 0x0000], [0x3c00, 0x0000], [0x0000, 0x4000]],
    );
    let lens = fixture.lens(Some(2)).unwrap();

    let vector = lens
        .measure(&Input::new(Modality::Text, b"alpha beta".to_vec()))
        .unwrap();

    let SlotVector::Dense { data, .. } = vector else {
        panic!("expected dense vector");
    };
    let norm = data.iter().map(|value| value * value).sum::<f32>().sqrt();
    assert!((norm - 1.0).abs() < 1.0e-6);
}

struct Fixture {
    root: PathBuf,
    matrix: PathBuf,
    tokenizer: PathBuf,
}

impl Fixture {
    fn new(label: &str, rows: &[[i8; 3]]) -> Self {
        let root = temp_root(label);
        let matrix = root.join("embeddings.cslm");
        let tokenizer = root.join("tokenizer.json");
        write_i8_matrix(&matrix, rows, 3);
        write_tokenizer(&tokenizer);
        Self {
            root,
            matrix,
            tokenizer,
        }
    }

    fn new_f16(label: &str, rows: &[[u16; 2]]) -> Self {
        let root = temp_root(label);
        let matrix = root.join("embeddings.cslm");
        let tokenizer = root.join("tokenizer.json");
        write_f16_matrix(&matrix, rows, 2);
        write_tokenizer(&tokenizer);
        Self {
            root,
            matrix,
            tokenizer,
        }
    }

    fn lens(&self, dim: Option<u32>) -> Result<StaticLookupLens> {
        StaticLookupLens::from_files(StaticLookupFileSpec {
            name: "fixture-static".to_string(),
            embeddings_file: self.matrix.clone(),
            tokenizer: self.tokenizer.clone(),
            dim,
            norm_policy: NormPolicy::unit(),
            expected_weights_sha256: None,
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn sentence(ids: &[u8]) -> String {
    ids.iter()
        .map(|id| match id {
            0 => "alpha",
            1 => "beta",
            _ => "gamma",
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn temp_root(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "calyx-static-lookup-{label}-{}-{nanos}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_i8_matrix(path: &Path, rows: &[[i8; 3]], dim: u32) {
    let mut bytes = header(rows.len() as u32, dim, DTYPE_I8, 1.0);
    for row in rows {
        for value in row {
            bytes.push(*value as u8);
        }
    }
    fs::write(path, bytes).unwrap();
}

fn write_f16_matrix(path: &Path, rows: &[[u16; 2]], dim: u32) {
    let mut bytes = header(rows.len() as u32, dim, DTYPE_F16, 1.0);
    for row in rows {
        for value in row {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    fs::write(path, bytes).unwrap();
}

fn header(rows: u32, dim: u32, dtype: u8, scale: f32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&rows.to_le_bytes());
    bytes.extend_from_slice(&dim.to_le_bytes());
    bytes.push(dtype);
    bytes.extend_from_slice(&[0, 0, 0]);
    bytes.extend_from_slice(&scale.to_le_bytes());
    bytes
}

fn write_tokenizer(path: &Path) {
    let value = json!({
        "version": "1.0",
        "truncation": null,
        "padding": null,
        "added_tokens": [],
        "normalizer": null,
        "pre_tokenizer": {"type": "Whitespace"},
        "post_processor": null,
        "decoder": null,
        "model": {
            "type": "WordLevel",
            "vocab": {"[UNK]": 0, "alpha": 1, "beta": 2, "gamma": 3},
            "unk_token": "[UNK]"
        }
    });
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}
