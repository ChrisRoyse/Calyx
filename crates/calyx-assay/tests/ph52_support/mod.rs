use std::fs;
use std::path::PathBuf;

pub(crate) fn write_readback(label: &str, name: &str, value: serde_json::Value) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let path = PathBuf::from(root).join(name);
    fs::create_dir_all(path.parent().expect("readback parent")).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    println!("{label}={}", path.display());
}
