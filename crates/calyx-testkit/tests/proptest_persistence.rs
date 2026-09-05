use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn top_level_integration_proptests_configure_failure_persistence() {
    let workspace = workspace_root();
    let crates_dir = workspace.join("crates");
    let mut violations = Vec::new();
    visit_rs_files(&crates_dir, &mut |path| {
        if !is_top_level_integration_test(path) {
            return;
        }
        let body = fs::read_to_string(path).expect("read rust source");
        if has_proptest_macro(&body)
            && !body.contains("integration_proptest_config")
            && !body.contains("failure_persistence")
        {
            violations.push(path.strip_prefix(&workspace).unwrap().display().to_string());
        }
    });

    assert!(
        violations.is_empty(),
        "top-level integration proptests must set source-adjacent failure persistence: {violations:#?}"
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn visit_rs_files(root: &Path, visit: &mut impl FnMut(&Path)) {
    for entry in fs::read_dir(root).expect("read crates dir") {
        let path = entry.expect("read dir entry").path();
        if path.is_dir() {
            visit_rs_files(&path, visit);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            visit(&path);
        }
    }
}

fn is_top_level_integration_test(path: &Path) -> bool {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    components
        .windows(3)
        .any(|window| window[0] == "crates" && window[2] == "tests")
}

fn has_proptest_macro(body: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("proptest! {") || trimmed.starts_with("proptest::proptest! {")
    })
}
