use super::super::*;
use super::tokens;

#[test]
fn parse_ingest_status_command() {
    let parsed = parse(&tokens([
        "ingest-status",
        "mydb",
        "--session",
        "issue1065-session",
    ]))
    .unwrap();
    assert_eq!(
        parsed,
        Subcommand::IngestStatus(IngestStatusArgs {
            vault: "mydb".to_string(),
            session_id: "issue1065-session".to_string(),
        })
    );
}

#[test]
fn parse_ingest_session_id_requires_batch() {
    let err = parse(&tokens([
        "ingest",
        "mydb",
        "--text",
        "hello",
        "--session-id",
        "not-for-text",
    ]))
    .unwrap_err();
    assert_eq!(err.code(), "CALYX_CLI_USAGE_ERROR");
    assert!(err.message().contains("--batch"));
}
