use std::path::PathBuf;

use calyx_gatebrokerd::config::{BrokerConfig, ConfigError, validate};
use calyx_gatebrokerd::protocol::{MAX_FRAME_BYTES, ProtocolError, decode_request};

const REQUEST_ID: &str = "123e4567-e89b-12d3-a456-426614174000";
const RUN_ID: &str = "11111111111111111111111111111111";
const RUN_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[test]
fn deployed_example_parses_and_validates_without_schema_drift() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config.toml.example");
    let text = std::fs::read_to_string(&path).expect("read deployed config example");
    let parsed: BrokerConfig = toml::from_str(&text).expect("strict TOML schema must parse");
    let validated = validate(parsed).expect("secure deployed example must validate");
    assert_eq!(validated.roots().len(), 2);
    assert_eq!(validated.execution_roots().len(), 1);
    assert_eq!(
        validated.raw().state.anchor,
        PathBuf::from("/var/lib/calyx-gatebrokerd")
    );
    assert!(
        validated
            .raw()
            .journal_path
            .starts_with(&validated.raw().state.private)
    );
    assert_eq!(validated.raw().max_rpc_frame_bytes, MAX_FRAME_BYTES);
}

#[test]
fn protocol_accepts_descriptor_relative_exec_and_rejects_authority_ambiguity() {
    let valid = format!(
        r#"{{"version":1,"request":{{"verb":"exec_stage","params":{{"request_id":"{REQUEST_ID}","run_id":"{RUN_ID}","run_token":"{RUN_TOKEN}","label":"build","cwd_root":"source","cwd":"job/src","argv":["/usr/bin/true"],"env":[]}}}}}}"#
    );
    let decoded = decode_request(valid.as_bytes()).expect("valid request");
    assert_eq!(decoded.request.request_id().as_str(), REQUEST_ID);

    for cwd in ["/etc", "job/../etc", "job//src", "job/src/"] {
        let invalid = valid.replace("job/src", cwd);
        assert!(
            matches!(
                decode_request(invalid.as_bytes()),
                Err(ProtocolError::InvalidJson(_))
            ),
            "cwd {cwd:?} must fail during bounded deserialization"
        );
    }
    let non_uuid = valid.replace(REQUEST_ID, "request-1");
    assert!(matches!(
        decode_request(non_uuid.as_bytes()),
        Err(ProtocolError::InvalidJson(_))
    ));
    let unknown = valid.replace("\"env\":[]", "\"env\":[],\"uid\":0");
    assert!(matches!(
        decode_request(unknown.as_bytes()),
        Err(ProtocolError::InvalidJson(_))
    ));
}

#[test]
fn frame_and_collection_boundaries_fail_closed() {
    assert!(matches!(
        decode_request(&[]),
        Err(ProtocolError::EmptyFrame)
    ));
    let oversized = vec![b' '; MAX_FRAME_BYTES + 1];
    assert!(matches!(
        decode_request(&oversized),
        Err(ProtocolError::OversizedFrame { .. })
    ));
    let empty_argv = format!(
        r#"{{"version":1,"request":{{"verb":"exec_stage","params":{{"request_id":"{REQUEST_ID}","run_id":"{RUN_ID}","run_token":"{RUN_TOKEN}","label":"build","cwd_root":"source","cwd":".","argv":[],"env":[]}}}}}}"#
    );
    assert!(matches!(
        decode_request(empty_argv.as_bytes()),
        Err(ProtocolError::InvalidCollection { field: "argv", .. })
    ));
}

#[test]
fn unsafe_config_setting_is_not_silently_normalized() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config.toml.example");
    let text = std::fs::read_to_string(path).expect("read config");
    let mut parsed: BrokerConfig = toml::from_str(&text).expect("parse config");
    parsed.containment.allow_same_uid_stage = true;
    let error = validate(parsed).expect_err("same-UID stage must fail closed");
    assert!(matches!(error, ConfigError::UnsafeSetting { .. }));
}

#[test]
fn fixed_worker_uid_forces_single_run_serialization() {
    let mut parsed = deployed_config();
    parsed.max_active_runs = 2;
    let error = validate(parsed).expect_err("fixed worker UID cannot isolate concurrent runs");
    assert!(matches!(
        error,
        ConfigError::UnsafeSetting { field, .. } if field == "max_active_runs"
    ));
}

fn deployed_config() -> BrokerConfig {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/config.toml.example");
    toml::from_str(&std::fs::read_to_string(path).expect("read config")).expect("parse config")
}

#[test]
fn managed_roots_cannot_be_moved_beneath_the_caller_owned_checkout() {
    let mut parsed = deployed_config();
    let tmp = parsed
        .roots
        .iter_mut()
        .find(|(alias, _)| alias.as_str() == "tmp")
        .expect("tmp root")
        .1;
    tmp.common_ancestor = PathBuf::from("/home/builder/calyx");
    tmp.shared = PathBuf::from("/home/builder/calyx/tmp");
    tmp.private = PathBuf::from("/home/builder/calyx/.gatebroker/tmp");
    let error = validate(parsed).expect_err("caller-owned managed root must fail closed");
    assert!(
        matches!(error, ConfigError::InvalidField { field, .. } if field == "roots.tmp.common_ancestor")
    );
}

#[test]
fn journal_must_remain_inside_root_only_private_state() {
    let mut parsed = deployed_config();
    parsed.journal_path = PathBuf::from("/var/lib/calyx-gatebrokerd/journal.sqlite");
    let error = validate(parsed).expect_err("public journal path must fail closed");
    assert!(matches!(error, ConfigError::InvalidField { field, .. } if field == "journal_path"));
}

#[test]
fn execution_roots_require_read_only_openat2_no_symlink_resolution() {
    for weaken in ["read_only", "openat2", "beneath", "symlinks", "magiclinks"] {
        let mut parsed = deployed_config();
        let source = parsed
            .execution_roots
            .iter_mut()
            .find(|(alias, _)| alias.as_str() == "source")
            .expect("source root")
            .1;
        match weaken {
            "read_only" => source.read_only = false,
            "openat2" => source.require_openat2 = false,
            "beneath" => source.require_resolve_beneath = false,
            "symlinks" => source.require_no_symlinks = false,
            "magiclinks" => source.require_no_magiclinks = false,
            _ => unreachable!(),
        }
        assert!(
            matches!(validate(parsed), Err(ConfigError::UnsafeSetting { .. })),
            "weak execution-root guarantee {weaken} must fail closed"
        );
    }
}

#[test]
fn execution_and_deletion_authorities_cannot_overlap() {
    let mut parsed = deployed_config();
    parsed
        .execution_roots
        .iter_mut()
        .find(|(alias, _)| alias.as_str() == "source")
        .expect("source root")
        .1
        .path = PathBuf::from("/var/lib/calyx-gatebrokerd/objects");
    assert!(matches!(
        validate(parsed),
        Err(ConfigError::OverlappingPaths { .. })
    ));
}

#[test]
fn configured_modes_preserve_worker_traversal_without_namespace_write() {
    for unsafe_mode in ["0700", "0755", "0731", "0713"] {
        let mut parsed = deployed_config();
        parsed
            .roots
            .iter_mut()
            .find(|(alias, _)| alias.as_str() == "tmp")
            .expect("tmp root")
            .1
            .shared_mode = unsafe_mode.into();
        assert!(
            matches!(validate(parsed), Err(ConfigError::InvalidField { .. })),
            "managed shared mode {unsafe_mode} must fail closed"
        );
    }

    for overbroad_mode in ["0750", "0711", "0770"] {
        let mut parsed = deployed_config();
        parsed
            .roots
            .iter_mut()
            .find(|(alias, _)| alias.as_str() == "tmp")
            .expect("tmp root")
            .1
            .published_mode = overbroad_mode.into();
        assert!(
            matches!(validate(parsed), Err(ConfigError::InvalidField { .. })),
            "published mode {overbroad_mode} must fail closed"
        );
    }

    for inaccessible_mode in ["0700", "0750"] {
        let mut parsed = deployed_config();
        parsed
            .execution_roots
            .iter_mut()
            .find(|(alias, _)| alias.as_str() == "source")
            .expect("source root")
            .1
            .expected_mode = inaccessible_mode.into();
        assert!(
            matches!(validate(parsed), Err(ConfigError::InvalidField { .. })),
            "execution mode {inaccessible_mode} must not strand the worker"
        );
    }
}

#[test]
fn state_anchor_and_private_journal_authority_fail_closed() {
    for weaken in [
        "anchor_owner",
        "anchor_mode",
        "private_mode",
        "journal_owner",
        "journal_mode",
        "path_chain",
        "symlinks",
    ] {
        let mut parsed = deployed_config();
        match weaken {
            "anchor_owner" => parsed.state.anchor_owner = "builder".into(),
            "anchor_mode" => parsed.state.anchor_mode = "0733".into(),
            "private_mode" => parsed.state.private_mode = "0711".into(),
            "journal_owner" => parsed.state.journal_directory_owner = "builder".into(),
            "journal_mode" => parsed.state.journal_directory_mode = "0755".into(),
            "path_chain" => parsed.state.require_root_owned_path_chain = false,
            "symlinks" => parsed.state.require_no_symlinks = false,
            _ => unreachable!(),
        }
        assert!(
            validate(parsed).is_err(),
            "weakened state authority {weaken} must fail closed"
        );
    }
}
