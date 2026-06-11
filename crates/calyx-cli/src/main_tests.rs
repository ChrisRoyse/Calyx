use super::*;
use calyx_anneal::TripwireRegistry;
use std::path::PathBuf;

#[test]
fn crate_metadata_is_present() {
    assert_eq!(env!("CARGO_PKG_NAME"), "calyx-cli");
}

#[test]
fn hex_lines_match_xxd_plain_chunks() {
    let bytes: Vec<_> = (0u8..=34).collect();

    assert_eq!(
        hex_lines(&bytes),
        vec![
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "202122",
        ]
    );
}

#[test]
fn display_relative_root_is_dot() {
    let root = PathBuf::from("/tmp/calyx-readback");

    assert_eq!(vault_tree::display_relative(&root, &root), ".");
}

#[test]
fn temporal_search_readback_command_executes() {
    run(vec![
        "readback".into(),
        "temporal_search".into(),
        "--explain".into(),
        "--clock-fixed".into(),
        "1000000".into(),
        "--tz-offset".into(),
        "0".into(),
    ])
    .expect("temporal search readback");
}

#[test]
fn dedup_check_readback_rejects_invalid_cosine_arg() {
    let error = run(vec![
        "readback".into(),
        "dedup-check".into(),
        "--vault".into(),
        "missing".into(),
        "--cx-id".into(),
        "00000000000000000000000000000000".into(),
        "--slot".into(),
        "0".into(),
        "--tau".into(),
        "2.0".into(),
        "--near-cos".into(),
        "0.95".into(),
        "--distinct-cos".into(),
        "0.85".into(),
        "--vault-id".into(),
        "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
        "--salt".into(),
        "s".into(),
    ])
    .expect_err("invalid tau");

    assert!(error.contains("--tau"));
}

#[test]
fn tripwire_config_readback_command_executes() {
    let root = std::env::temp_dir().join(format!("calyx-cli-tripwire-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create tripwire test vault");
    TripwireRegistry::load_from_vault(&root).expect("write default tripwire config");

    run(vec![
        "readback".into(),
        "config".into(),
        "tripwire".into(),
        "--vault".into(),
        root.display().to_string(),
    ])
    .expect("tripwire config readback");

    let _ = std::fs::remove_dir_all(root);
}
