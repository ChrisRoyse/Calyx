use std::fs;
use std::io;
use std::path::Path;

use crate::error::CliResult;

pub(crate) fn readback_vault_tree(path: &Path) -> CliResult {
    for line in vault_tree_lines(path)? {
        println!("{line}");
    }
    Ok(())
}

fn vault_tree_lines(root: &Path) -> io::Result<Vec<String>> {
    let root = root.canonicalize()?;
    let mut lines = vec![format!("DIR\t{}", display_relative(&root, &root))];
    collect_tree(&root, &root, &mut lines)?;
    Ok(lines)
}

fn collect_tree(root: &Path, dir: &Path, lines: &mut Vec<String>) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let metadata = entry.metadata()?;
        let relative = display_relative(root, &path);
        if metadata.is_dir() {
            lines.push(format!("DIR\t{relative}"));
            collect_tree(root, &path, lines)?;
        } else {
            lines.push(format!("FILE\t{relative}\tbytes={}", metadata.len()));
        }
    }

    Ok(())
}

pub(crate) fn display_relative(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    if relative.as_os_str().is_empty() {
        ".".to_string()
    } else {
        normalize_path(relative)
    }
}

fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
