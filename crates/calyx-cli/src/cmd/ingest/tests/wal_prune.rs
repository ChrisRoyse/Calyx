//! Issue #1992: the ingest WAL-prune env config resolves through a pure
//! resolver that fails closed on garbage, without mutating process-global
//! environment (the #1985 a1b9d6e0 lesson — env mutation races every parallel
//! ingest test).

use super::super::command::wal_prune::resolve_ingest_wal_prune_config_from;

#[test]
fn rejects_garbage_wal_prune_round_max_segments_env() {
    let key = "CALYX_INGEST_WAL_PRUNE_ROUND_MAX_SEGMENTS";
    for garbage in ["not-a-number", "0", "-3", ""] {
        let error = resolve_ingest_wal_prune_config_from(Some(garbage.to_string()), None)
            .expect_err("garbage segment cap must fail resolution closed");
        assert!(
            error.message().contains(key),
            "unexpected error for {garbage:?}: {}",
            error.message()
        );
    }
}

#[test]
fn rejects_garbage_wal_prune_round_max_bytes_env() {
    let key = "CALYX_INGEST_WAL_PRUNE_ROUND_MAX_BYTES";
    for garbage in ["not-a-number", "0", ""] {
        let error = resolve_ingest_wal_prune_config_from(None, Some(garbage.to_string()))
            .expect_err("garbage byte cap must fail resolution closed");
        assert!(
            error.message().contains(key),
            "unexpected error for {garbage:?}: {}",
            error.message()
        );
    }
}

#[test]
fn unset_overrides_resolve_to_validated_defaults() {
    let config = resolve_ingest_wal_prune_config_from(None, None).expect("defaults must resolve");
    assert!(config.max_segments >= 1);
    assert!(config.max_bytes >= 1);
    config.validate().expect("defaults must validate");
}

#[test]
fn valid_overrides_are_honored() {
    let config =
        resolve_ingest_wal_prune_config_from(Some("4".to_string()), Some("1024".to_string()))
            .expect("valid overrides resolve");
    assert_eq!(config.max_segments, 4);
    assert_eq!(config.max_bytes, 1024);
}
