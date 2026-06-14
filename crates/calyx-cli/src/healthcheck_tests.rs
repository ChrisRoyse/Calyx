use crate::dispatch::run;

#[test]
fn healthcheck_writes_pass_json_for_rendered_secret_env() {
    let root =
        std::env::temp_dir().join(format!("calyx-cli-healthcheck-pass-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let repo = home.join("repo");
    let secret = root.join("calyx.env");
    let out = root.join("latest.json");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::write(&secret, "HF_HUB_TOKEN='redacted'\nHF_TOKEN='redacted'\n")
        .expect("write secret env");
    set_secret_mode(&secret);

    run(vec![
        "healthcheck".into(),
        "--out".into(),
        out.display().to_string(),
        "--secret-env".into(),
        secret.display().to_string(),
        "--calyx-home".into(),
        home.display().to_string(),
    ])
    .expect("healthcheck pass");

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out).expect("read latest"))
            .expect("parse latest");
    assert_eq!(json["status"], "pass");
    assert_eq!(json["failure_count"], 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn healthcheck_fails_closed_and_writes_json_for_missing_secret_var() {
    let root = std::env::temp_dir().join(format!(
        "calyx-cli-healthcheck-missing-var-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    let repo = home.join("repo");
    let secret = root.join("calyx.env");
    let out = root.join("latest.json");
    std::fs::create_dir_all(&repo).expect("create repo dir");
    std::fs::write(&secret, "HF_HUB_TOKEN='redacted'\n").expect("write secret env");
    set_secret_mode(&secret);

    let error = run(vec![
        "healthcheck".into(),
        "--out".into(),
        out.display().to_string(),
        "--secret-env".into(),
        secret.display().to_string(),
        "--calyx-home".into(),
        home.display().to_string(),
    ])
    .expect_err("missing HF_TOKEN must fail");

    assert_eq!(error.code(), "CALYX_HEALTHCHECK_FAILED");
    let text = std::fs::read_to_string(&out).expect("read latest");
    assert!(text.contains("CALYX_HEALTH_SECRET_ENV_VAR_MISSING"));
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
fn set_secret_mode(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)
        .expect("secret metadata")
        .permissions();
    permissions.set_mode(0o400);
    std::fs::set_permissions(path, permissions).expect("chmod secret");
}

#[cfg(not(unix))]
fn set_secret_mode(_path: &std::path::Path) {}
