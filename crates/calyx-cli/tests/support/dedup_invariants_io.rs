#[path = "dedup_fsv_io.rs"]
mod dedup_fsv_io;

pub(crate) use dedup_fsv_io::{
    fsv_root, list_tree_files as list_files, reset_dir, write_blake3_sums, write_json,
};
