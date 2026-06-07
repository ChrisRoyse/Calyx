//! Calyx command-line entry point.

use std::env;
use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    match args.as_slice() {
        [command, flag, value] if command == "readback" && flag == "--hex" => {
            readback_hex(Path::new(value)).map_err(|error| error.to_string())
        }
        [command, flag, value] if command == "readback" && flag == "--vault-tree" => {
            readback_vault_tree(Path::new(value)).map_err(|error| error.to_string())
        }
        [] | [_]
            if args
                .first()
                .is_none_or(|arg| arg == "--help" || arg == "-h") =>
        {
            print_usage();
            Ok(())
        }
        _ => Err("usage: calyx readback (--hex <file> | --vault-tree <dir>)".to_string()),
    }
}

fn readback_hex(path: &Path) -> io::Result<()> {
    let bytes = fs::read(path)?;
    for line in hex_lines(&bytes) {
        println!("{line}");
    }
    Ok(())
}

fn readback_vault_tree(path: &Path) -> io::Result<()> {
    for line in vault_tree_lines(path)? {
        println!("{line}");
    }
    Ok(())
}

fn hex_lines(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(32)
        .map(|chunk| {
            let mut line = String::with_capacity(chunk.len() * 2);
            for byte in chunk {
                line.push(hex_digit(byte >> 4));
                line.push(hex_digit(byte & 0x0f));
            }
            line
        })
        .collect()
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
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

fn display_relative(root: &Path, path: &Path) -> String {
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

fn print_usage() {
    println!("usage: calyx readback (--hex <file> | --vault-tree <dir>)");
    println!("prints source-of-truth bytes or listings for manual FSV inspection");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-cli");
    }

    #[test]
    fn hex_lines_match_xxd_plain_chunks() {
        let bytes: Vec<_> = (0u8..=34).collect();

        assert_eq!(
            hex_lines(&bytes),
            vec![
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "202122",
            ]
        );
    }

    #[test]
    fn display_relative_root_is_dot() {
        let root = PathBuf::from("/tmp/calyx-readback");

        assert_eq!(display_relative(&root, &root), ".");
    }
}
