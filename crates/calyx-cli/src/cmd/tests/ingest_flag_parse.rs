use super::*;
use calyx_core::Modality;

#[test]
fn parse_ingest_allow_cold_gpu_workers_flag() {
    let parsed = parse(&tokens([
        "ingest",
        "mydb",
        "--batch",
        "batch.jsonl",
        "--allow-cold-gpu-workers",
    ]))
    .unwrap();
    let Subcommand::Ingest(args) = parsed else {
        panic!("expected ingest subcommand");
    };
    assert!(args.allow_cold_gpu_workers);
    assert!(args.resident_addr.is_none());
}

#[test]
fn parse_ingest_text_command() {
    let parsed = parse(&tokens(["ingest", "mydb", "--text", "hello"])).unwrap();
    assert_eq!(
        parsed,
        Subcommand::Ingest(IngestArgs {
            vault: "mydb".to_string(),
            text: Some("hello".to_string()),
            batch: None,
            file: None,
            modality: None,
            idempotent: true,
            output: IngestOutput::Summary,
            resident_addr: None,
            allow_cold_gpu_workers: false,
            session_id: None,
        })
    );
}

#[test]
fn parse_ingest_video_file_command() {
    let parsed = parse(&tokens([
        "ingest",
        "media",
        "--file",
        "clip.webm",
        "--modality",
        "video",
    ]))
    .unwrap();
    assert_eq!(
        parsed,
        Subcommand::Ingest(IngestArgs {
            vault: "media".to_string(),
            text: None,
            batch: None,
            file: Some("clip.webm".into()),
            modality: Some(Modality::Video),
            idempotent: true,
            output: IngestOutput::Summary,
            resident_addr: None,
            allow_cold_gpu_workers: false,
            session_id: None,
        })
    );
}

#[test]
fn parse_ingest_rows_output_command() {
    let parsed = parse(&tokens([
        "ingest",
        "mydb",
        "--batch",
        "batch.jsonl",
        "--output",
        "rows",
    ]))
    .unwrap();
    assert_eq!(
        parsed,
        Subcommand::Ingest(IngestArgs {
            vault: "mydb".to_string(),
            text: None,
            batch: Some("batch.jsonl".into()),
            file: None,
            modality: None,
            idempotent: true,
            output: IngestOutput::Rows,
            resident_addr: None,
            allow_cold_gpu_workers: false,
            session_id: None,
        })
    );
}

#[test]
fn parse_ingest_resident_addr_command() {
    let parsed = parse(&tokens([
        "ingest",
        "mydb",
        "--batch",
        "batch.jsonl",
        "--resident-addr",
        "127.0.0.1:8787",
    ]))
    .unwrap();
    assert_eq!(
        parsed,
        Subcommand::Ingest(IngestArgs {
            vault: "mydb".to_string(),
            text: None,
            batch: Some("batch.jsonl".into()),
            file: None,
            modality: None,
            idempotent: true,
            output: IngestOutput::Summary,
            resident_addr: Some("127.0.0.1:8787".parse().unwrap()),
            allow_cold_gpu_workers: false,
            session_id: None,
        })
    );
}

#[test]
fn parse_ingest_rejects_non_loopback_resident_addr() {
    let err = parse(&tokens([
        "ingest",
        "mydb",
        "--batch",
        "batch.jsonl",
        "--resident-addr",
        "10.0.0.10:8787",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_INGEST_RESIDENT_ADDR_REFUSED");
}

#[test]
fn parse_ingest_rejects_unknown_output_mode() {
    let err = parse(&tokens([
        "ingest",
        "mydb",
        "--batch",
        "batch.jsonl",
        "--output",
        "verbose",
    ]))
    .unwrap_err();

    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("summary or rows"));
}
