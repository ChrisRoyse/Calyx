use std::path::Path;

use super::super::args::Args;

pub(super) fn lens_prefix(slot: usize, name: &str) -> String {
    format!("slot_{slot:02}_{}", safe_name(name))
}

pub(super) fn display_final(args: &Args, rel: &str) -> String {
    display(&args.out_dir.join(rel))
}

pub(super) fn display(path: &Path) -> String {
    path.display().to_string()
}

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '_',
        })
        .collect()
}
