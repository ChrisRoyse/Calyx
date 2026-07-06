use super::*;

#[test]
fn byte_features_are_bit_deterministic() {
    let lens = AlgorithmicLens::byte_features("byte-fsv", Modality::Text);
    let input = Input::new(Modality::Text, b"Calyx PH17: 2+2=4\n".to_vec());

    let first = lens.measure(&input).unwrap();
    let second = lens.measure(&input).unwrap();

    assert_eq!(first, second);
}

#[test]
fn empty_input_emits_real_dense_vector() {
    let lens = AlgorithmicLens::byte_features("byte-empty", Modality::Text);
    let input = Input::new(Modality::Text, Vec::new());
    let vector = lens.measure(&input).unwrap();
    let bytes = serde_json::to_vec(&vector).unwrap();

    println!(
        "ALGORITHMIC_EMPTY_BYTES={}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(
        vector,
        SlotVector::Dense {
            dim: BYTE_FEATURE_DIM,
            data: {
                let mut data = vec![0.0; BYTE_FEATURE_DIM as usize];
                data[0] = 1.0;
                data
            }
        }
    );
}

#[test]
fn scalar_feature_is_centered_for_cosine_assay() {
    let lens = AlgorithmicLens::scalar("scalar-fsv", Modality::Structured);
    let low = Input::new(Modality::Structured, b"!!!!!!!!!!!!!!!!".to_vec());
    let high = Input::new(Modality::Structured, b"zzzzzzzzzzzzzzzz".to_vec());

    let low = lens.measure(&low).unwrap();
    let high = lens.measure(&high).unwrap();

    assert!(matches!(low, SlotVector::Dense { data, .. } if data[0] < 0.0));
    assert!(matches!(high, SlotVector::Dense { data, .. } if data[0] > 0.0));
}

#[test]
fn algorithmic_fsv_determinism_probe() {
    let lens = AlgorithmicLens::byte_features("byte-fsv", Modality::Text);
    let input = Input::new(Modality::Text, b"Calyx registry manual FSV".to_vec());
    let first = lens.measure(&input).unwrap();
    let second = lens.measure(&input).unwrap();
    let first_bytes = serde_json::to_vec(&first).unwrap();
    let second_bytes = serde_json::to_vec(&second).unwrap();

    println!("ALGORITHMIC_FSV_DIGEST={}", digest_hex(&first_bytes));
    println!(
        "ALGORITHMIC_FSV_BYTES={}",
        String::from_utf8_lossy(&first_bytes)
    );
    assert_eq!(first_bytes, second_bytes);
}

fn digest_hex(bytes: &[u8]) -> String {
    calyx_core::content_address([bytes])
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
