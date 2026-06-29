use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::{Value, json};

#[test]
fn readme_image_contract_uses_assets_as_the_single_source_of_truth() {
    let root = workspace_root();
    let before = inspect_source_of_truth(&root);
    assert_contract(&before);

    let after = inspect_source_of_truth(&root);
    assert_contract(&after);

    let readback = json!({
        "issue": 966,
        "source_of_truth": {
            "readme": "README.md",
            "canonical_image_dir": "assets/",
            "contract": "docs/readme/README.md",
        },
        "before": before,
        "after": after,
        "edge_case_audit": {
            "empty_local_image_set": {
                "observed_count": after["local_image_refs"].as_array().unwrap().len(),
                "expected": "non-empty; otherwise README would carry no local visual contract",
                "passed": !after["local_image_refs"].as_array().unwrap().is_empty(),
            },
            "maximum_current_asset_set": {
                "observed_count": after["resolved_assets"].as_array().unwrap().len(),
                "expected": "every current README local image is resolved and hashed",
                "passed": after["missing_assets"].as_array().unwrap().is_empty(),
            },
            "invalid_mirror_location": {
                "observed_docs_readme_images": after["docs_readme_image_files"],
                "observed_docs_readme_refs": after["docs_readme_refs"],
                "expected": "no README images are stored in or referenced from docs/readme/",
                "passed": after["docs_readme_image_files"].as_array().unwrap().is_empty()
                    && after["docs_readme_refs"].as_array().unwrap().is_empty(),
            },
        },
    });

    let fsv_root = fsv_root(&root);
    fs::create_dir_all(&fsv_root).expect("create FSV root");
    let readback_path = fsv_root.join("issue966-readme-assets-readback.json");
    fs::write(
        &readback_path,
        serde_json::to_vec_pretty(&readback).expect("serialize readback"),
    )
    .expect("write readback");
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
}

fn inspect_source_of_truth(root: &Path) -> Value {
    let readme_path = root.join("README.md");
    let readme = fs::read_to_string(&readme_path).expect("read README.md");
    let refs = extract_image_refs(&readme);
    let local_refs = refs
        .iter()
        .filter(|image| !is_external(&image.path))
        .collect::<Vec<_>>();

    let mut invalid_local_refs = Vec::new();
    let mut docs_readme_refs = Vec::new();
    let mut missing_assets = Vec::new();
    let mut resolved_assets = Vec::new();

    for image in &local_refs {
        if has_path_escape(&image.path) || !image.path.starts_with("assets/") {
            invalid_local_refs.push(image.to_json());
        }
        if image.path.starts_with("docs/readme/") {
            docs_readme_refs.push(image.to_json());
        }

        let path = root.join(&image.path);
        if !path.is_file() {
            missing_assets.push(image.to_json());
            continue;
        }

        let bytes = fs::read(&path).expect("read resolved asset");
        resolved_assets.push(json!({
            "line": image.line,
            "path": image.path,
            "bytes": bytes.len(),
            "blake3": blake3::hash(&bytes).to_hex().to_string(),
        }));
    }

    let docs_readme = root.join("docs").join("readme");
    let contract_path = docs_readme.join("README.md");
    let contract = fs::read_to_string(&contract_path).expect("read docs/readme/README.md");

    json!({
        "readme_exists": readme_path.is_file(),
        "docs_readme_exists": docs_readme.is_dir(),
        "contract_exists": contract_path.is_file(),
        "contract_declares_assets_canonical": contract.contains("`assets/` is the canonical directory"),
        "contract_rejects_png_mirror": contract.contains("Do not mirror") && contract.contains("PNG assets"),
        "all_image_refs": refs.iter().map(ImageRef::to_json).collect::<Vec<_>>(),
        "local_image_refs": local_refs.iter().map(|image| image.to_json()).collect::<Vec<_>>(),
        "invalid_local_refs": invalid_local_refs,
        "docs_readme_refs": docs_readme_refs,
        "missing_assets": missing_assets,
        "resolved_assets": resolved_assets,
        "docs_readme_image_files": collect_image_files(&docs_readme),
    })
}

fn assert_contract(state: &Value) {
    assert_eq!(state["readme_exists"], true);
    assert_eq!(state["docs_readme_exists"], true);
    assert_eq!(state["contract_exists"], true);
    assert_eq!(state["contract_declares_assets_canonical"], true);
    assert_eq!(state["contract_rejects_png_mirror"], true);
    assert!(
        !state["local_image_refs"].as_array().unwrap().is_empty(),
        "README.md must contain local image references"
    );
    assert_eq!(state["invalid_local_refs"], json!([]));
    assert_eq!(state["docs_readme_refs"], json!([]));
    assert_eq!(state["missing_assets"], json!([]));
    assert_eq!(state["docs_readme_image_files"], json!([]));
}

#[derive(Debug)]
struct ImageRef {
    line: usize,
    path: String,
}

impl ImageRef {
    fn to_json(&self) -> Value {
        json!({
            "line": self.line,
            "path": self.path,
        })
    }
}

fn extract_image_refs(markdown: &str) -> Vec<ImageRef> {
    let mut refs = Vec::new();
    for (line_index, line) in markdown.lines().enumerate() {
        let line_number = line_index + 1;
        refs.extend(extract_html_sources(line, line_number));
        refs.extend(extract_markdown_images(line, line_number));
    }
    refs
}

fn extract_html_sources(line: &str, line_number: usize) -> Vec<ImageRef> {
    let mut refs = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("src=\"") {
        let value = &rest[start + 5..];
        let Some(end) = value.find('"') else {
            break;
        };
        refs.push(ImageRef {
            line: line_number,
            path: clean_image_path(&value[..end]),
        });
        rest = &value[end + 1..];
    }
    refs
}

fn extract_markdown_images(line: &str, line_number: usize) -> Vec<ImageRef> {
    let mut refs = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("![") {
        let Some(label_end) = rest[start..].find("](") else {
            break;
        };
        let value = &rest[start + label_end + 2..];
        let Some(end) = value.find(')') else {
            break;
        };
        refs.push(ImageRef {
            line: line_number,
            path: clean_image_path(&value[..end]),
        });
        rest = &value[end + 1..];
    }
    refs
}

fn clean_image_path(raw: &str) -> String {
    raw.split(['?', '#'])
        .next()
        .unwrap_or(raw)
        .trim_matches(['<', '>'])
        .to_string()
}

fn is_external(path: &str) -> bool {
    path.starts_with("http://")
        || path.starts_with("https://")
        || path.starts_with("data:")
        || path.starts_with("mailto:")
}

fn has_path_escape(path: &str) -> bool {
    Path::new(path).components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

fn collect_image_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_image_files_inner(root, root, &mut files);
    files.sort();
    files
}

fn collect_image_files_inner(root: &Path, dir: &Path, files: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("read dir entry").path();
        if path.is_dir() {
            collect_image_files_inner(root, &path, files);
        } else if is_image_file(&path) {
            files.push(
                path.strip_prefix(root)
                    .expect("relative image path")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "gif" | "jpg" | "jpeg" | "png" | "svg" | "webp"
            )
        })
        .unwrap_or(false)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("canonical workspace root")
}

fn fsv_root(root: &Path) -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("target")
                .join("fsv")
                .join("issue966-readme-assets")
        })
}
