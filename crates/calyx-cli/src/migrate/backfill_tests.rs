use super::*;

#[test]
fn real_tei_closed_endpoint_without_offline_mode_fails_closed() {
    let spec = PanelSlotSpec::content(
        "closed_tei",
        PanelLensRuntime::TeiHttp {
            endpoint: "http://127.0.0.1:9".to_string(),
        },
        SlotShape::Dense(3),
        Modality::Text,
    );
    let row = ChunkRow {
        row_num: 1,
        chunk_id: "closed-tei".to_string(),
        database_name: "db".to_string(),
        content: b"alpha beta".to_vec(),
        embedding: vec![0.0; 768],
        event_time_secs: None,
        event_time_raw: None,
    };
    let result = measure_slot(
        &spec,
        &row,
        BackfillMode::RealTei,
        0,
        TemporalContext::from_rows(std::slice::from_ref(&row)),
    );
    let error = match result {
        Ok(_) => panic!("closed TEI endpoint unexpectedly produced a vector"),
        Err(error) => error,
    };

    println!("ISSUE914_CLOSED_TEI_ERROR={}", error.code);
    assert_eq!(error.code, "CALYX_LENS_UNREACHABLE");
}
