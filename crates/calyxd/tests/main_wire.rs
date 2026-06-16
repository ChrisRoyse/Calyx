use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn validate_config_defaults_to_calyx_toml_and_prints_no_secret_words() {
    let dir = test_dir("validate-default");
    let vault = dir.join("vault");
    let logs = dir.join("logs");
    fs::create_dir_all(&vault).unwrap();
    let config = dir.join("calyx.toml");
    write_config(&config, "127.0.0.1:0", &vault, &logs);

    let output = Command::new(env!("CARGO_BIN_EXE_calyxd"))
        .current_dir(&dir)
        .arg("--validate-config")
        .output()
        .expect("run calyxd --validate-config");

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("bind_addr"));
    assert!(text.contains("vault_path"));
    assert!(text.contains("vram_budget_mib"));
    let lowered = text.to_lowercase();
    for secret_word in ["password", "token", "key"] {
        assert!(
            !lowered.contains(secret_word),
            "validate output leaked forbidden word {secret_word}: {text}"
        );
    }
    cleanup(dir);
}

#[test]
fn validate_config_rejects_non_loopback_bind() {
    let dir = test_dir("validate-non-loopback");
    let vault = dir.join("vault");
    let logs = dir.join("logs");
    fs::create_dir_all(&vault).unwrap();
    let config = dir.join("bad.toml");
    write_config(&config, "0.0.0.0:7700", &vault, &logs);

    let output = calyxd(&["--config", config.to_str().unwrap(), "--validate-config"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("CALYX_DAEMON_BIND_FAILED"),
        "stderr: {}",
        stderr(&output)
    );
    cleanup(dir);
}

#[test]
fn missing_config_path_is_config_invalid() {
    let dir = test_dir("missing-config");
    let missing = dir.join("missing.toml");

    let output = calyxd(&["--config", missing.to_str().unwrap(), "--validate-config"]);

    assert!(!output.status.success());
    let err = stderr(&output);
    assert!(err.contains("CALYX_DAEMON_CONFIG_INVALID"), "{err}");
    assert!(err.contains("missing.toml"), "{err}");
    cleanup(dir);
}

#[test]
fn forced_cuda_failure_exits_before_socket_bind() {
    let dir = test_dir("forced-cuda");
    let vault = dir.join("vault");
    let logs = dir.join("logs");
    fs::create_dir_all(&vault).unwrap();
    let port = free_loopback_port();
    let bind = format!("127.0.0.1:{port}");
    let config = dir.join("calyx.toml");
    write_config(&config, &bind, &vault, &logs);

    let output = Command::new(env!("CARGO_BIN_EXE_calyxd"))
        .arg("--config")
        .arg(&config)
        .env("CALYX_FORCE_CUDA_FAIL", "1")
        .output()
        .expect("run calyxd");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("CALYX_FORGE_DEVICE_UNAVAILABLE"),
        "stderr: {}",
        stderr(&output)
    );
    assert!(
        TcpStream::connect(&bind).is_err(),
        "server must not bind after forced CUDA failure"
    );
    cleanup(dir);
}

fn write_config(path: &Path, bind_addr: &str, vault: &Path, logs: &Path) {
    let health = logs.join("latest.json");
    let text = format!(
        "bind_addr = \"{bind_addr}\"\n\
         vault_path = \"{}\"\n\
         vram_budget_mib = 8192\n\
         log_dir = \"{}\"\n\
         health_log_path = \"{}\"\n",
        toml_path(vault),
        toml_path(logs),
        toml_path(&health)
    );
    fs::write(path, text).unwrap();
}

fn toml_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}

fn free_loopback_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind free port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    port
}

fn calyxd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_calyxd"))
        .args(args)
        .output()
        .expect("run calyxd")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyxd-main-wire-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).unwrap();
}
