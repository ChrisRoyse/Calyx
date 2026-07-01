use std::fs;

pub(crate) fn write_readback(label: &str, name: &str, value: serde_json::Value) {
    let Some(root) = calyx_fsv::fsv_root("CALYX_FSV_ROOT") else {
        return;
    };
    let path = root.join(name);
    fs::create_dir_all(path.parent().expect("readback parent")).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    println!("{label}={}", path.display());
}
