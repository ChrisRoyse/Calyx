//! Calyx command-line entry point.

mod fsv;
mod ops;

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
        [command, flag, cf, vault_flag, vault]
            if command == "readback" && flag == "--cf" && vault_flag == "--vault" =>
        {
            ops::readback_cf(Path::new(vault), cf)
        }
        [command, flag, vault_flag, vault]
            if command == "readback" && flag == "--wal" && vault_flag == "--vault" =>
        {
            ops::readback_wal(Path::new(vault))
        }
        [command, flag, cf, level_flag, level_dir]
            if command == "readback" && flag == "--cf" && level_flag == "--level" =>
        {
            fsv::readback_level(cf, Path::new(level_dir))
        }
        [command, vault_flag, vault, cf_flag, cf]
            if command == "compact" && vault_flag == "--vault" && cf_flag == "--cf" =>
        {
            ops::compact(Path::new(vault), cf)
        }
        [command, vault_flag, vault, duration_flag, duration]
            if command == "compact-watch"
                && vault_flag == "--vault"
                && duration_flag == "--duration" =>
        {
            ops::compact_watch(Path::new(vault), ops::parse_duration(duration)?)
        }
        [
            command,
            vault_flag,
            vault,
            ops_flag,
            ops,
            threads_flag,
            threads,
        ] if command == "soak"
            && vault_flag == "--vault"
            && ops_flag == "--ops"
            && threads_flag == "--threads" =>
        {
            let ops = ops
                .parse::<usize>()
                .map_err(|error| format!("invalid --ops: {error}"))?;
            let threads = threads
                .parse::<usize>()
                .map_err(|error| format!("invalid --threads: {error}"))?;
            ops::soak(Path::new(vault), ops, threads)
        }
        [command, vault_flag, vault, cf_flag, cf, output_flag, output]
            if command == "tier"
                && vault_flag == "--vault"
                && cf_flag == "--cf"
                && output_flag == "--output" =>
        {
            ops::tier(Path::new(vault), cf, output)
        }
        [command, vault_flag, vault] if command == "vault-demo" && vault_flag == "--vault" => {
            ops::vault_demo(Path::new(vault))
        }
        [command, vault_flag, vault] if command == "arrow-demo" && vault_flag == "--vault" => {
            fsv::arrow_demo(Path::new(vault))
        }
        [command, vault_flag, vault] if command == "cf-demo" && vault_flag == "--vault" => {
            fsv::cf_demo(Path::new(vault))
        }
        [command, vault_flag, vault] if command == "mvcc-demo" && vault_flag == "--vault" => {
            fsv::mvcc_demo(Path::new(vault))
        }
        [command, vault_flag, vault, records_flag, records]
            if command == "wal-drill" && vault_flag == "--vault" && records_flag == "--records" =>
        {
            let records = records
                .parse::<usize>()
                .map_err(|error| format!("invalid --records: {error}"))?;
            fsv::wal_drill(Path::new(vault), records)
        }
        [command, wal_dir] if command == "wal-replay" => fsv::wal_replay(Path::new(wal_dir)),
        [command, vault_flag, vault, cf_flag, cf, offset_flag, offset]
            if command == "corrupt-shard"
                && vault_flag == "--vault"
                && cf_flag == "--cf"
                && offset_flag == "--byte-offset" =>
        {
            let offset = offset
                .parse::<u64>()
                .map_err(|error| format!("invalid --byte-offset: {error}"))?;
            fsv::corrupt_shard(Path::new(vault), cf, offset)
        }
        [command, vault_flag, vault, requests_flag, requests]
            if command == "wal-batch-demo"
                && vault_flag == "--vault"
                && requests_flag == "--requests" =>
        {
            let requests = requests
                .parse::<usize>()
                .map_err(|error| format!("invalid --requests: {error}"))?;
            ops::wal_batch_demo(Path::new(vault), requests)
        }
        [] | [_]
            if args
                .first()
                .is_none_or(|arg| arg == "--help" || arg == "-h") =>
        {
            print_usage();
            Ok(())
        }
        _ => Err(usage().to_string()),
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
    println!("{}", usage());
    println!("prints source-of-truth bytes or listings for manual FSV inspection");
}

fn usage() -> &'static str {
    "usage: calyx readback (--hex <file> | --vault-tree <dir> | --cf <name> --vault <dir> | --cf <name> --level <dir> | --wal --vault <dir>)\n       calyx compact --vault <dir> --cf <name>\n       calyx compact-watch --vault <dir> --duration <30s|500ms>\n       calyx soak --vault <dir> --ops <n> --threads <n>\n       calyx tier --vault <dir> --cf <name> --output <hot|cold>\n       calyx vault-demo --vault <dir>\n       calyx arrow-demo --vault <dir>\n       calyx cf-demo --vault <dir>\n       calyx mvcc-demo --vault <dir>\n       calyx wal-drill --vault <dir> --records <n>\n       calyx wal-replay <wal-dir>\n       calyx corrupt-shard --vault <dir> --cf <name> --byte-offset <n>\n       calyx wal-batch-demo --vault <dir> --requests <n>"
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
