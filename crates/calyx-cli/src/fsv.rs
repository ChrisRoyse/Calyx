use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::sst::arrow::{decode_column_chunk, encode_column_chunk};
use calyx_aster::sst::level::SstLevel;
use calyx_aster::sst::{SstReader, write_sst};
use calyx_core::SlotId;
use std::fs;
use std::path::{Path, PathBuf};

pub fn arrow_demo(vault: &Path) -> Result<(), String> {
    let cf = ColumnFamily::slot(SlotId::new(0));
    let cf_dir = vault.join("cf").join(cf.name());
    fs::create_dir_all(&cf_dir).map_err(|error| error.to_string())?;
    let rows = [[1.0_f32, 2.0, 3.5, 4.25], [5.0, 6.0, 7.0, 8.0]];
    let refs: Vec<_> = rows.iter().map(|row| row.as_slice()).collect();
    let chunk = encode_column_chunk(&refs).map_err(|error| error.to_string())?;
    let decoded = decode_column_chunk(&chunk).map_err(|error| error.to_string())?;
    if decoded.n_rows() != 2 || decoded.dim() != 4 {
        return Err("arrow demo decoded unexpected shape".to_string());
    }
    let path = cf_dir.join("00000000000000000001.sst");
    let summary = write_sst(&path, [(b"arrow-key".as_slice(), chunk.as_slice())])
        .map_err(|error| error.to_string())?;
    let stored = SstReader::open(&path)
        .and_then(|reader| reader.get(b"arrow-key"))
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "arrow demo SST row missing".to_string())?;
    let stored = decode_column_chunk(&stored).map_err(|error| error.to_string())?;
    println!(
        "ARROW_DEMO\tCF\t{}\tSST\t{}\tKEY\t{}\tVALUE_MAGIC\t{}\tROWS\t{}\tDIM\t{}\tBYTES\t{}",
        cf.name(),
        summary.path.display(),
        hex_bytes(b"arrow-key"),
        hex_bytes(&stored.raw_bytes()[0..4]),
        stored.n_rows(),
        stored.dim(),
        summary.bytes
    );
    Ok(())
}

pub fn cf_demo(vault: &Path) -> Result<(), String> {
    let mut router = CfRouter::open(vault, 1024).map_err(|error| error.to_string())?;
    router
        .put(ColumnFamily::Base, b"k1", b"base-old")
        .map_err(|error| error.to_string())?;
    router
        .flush_cf(ColumnFamily::Base)
        .map_err(|error| error.to_string())?;
    router
        .put(ColumnFamily::Base, b"k1", b"base-new")
        .map_err(|error| error.to_string())?;
    router
        .put(ColumnFamily::Base, b"k2", b"base-two")
        .map_err(|error| error.to_string())?;
    router
        .put(ColumnFamily::slot(SlotId::new(0)), b"k1", b"slot-zero")
        .map_err(|error| error.to_string())?;
    router
        .flush_cf(ColumnFamily::Base)
        .map_err(|error| error.to_string())?;
    router
        .flush_cf(ColumnFamily::slot(SlotId::new(0)))
        .map_err(|error| error.to_string())?;
    drop(router);

    let reopened = CfRouter::open(vault, 1024).map_err(|error| error.to_string())?;
    assert_value(&reopened, ColumnFamily::Base, b"k1", b"base-new")?;
    assert_value(&reopened, ColumnFamily::Base, b"k2", b"base-two")?;
    assert_value(
        &reopened,
        ColumnFamily::slot(SlotId::new(0)),
        b"k1",
        b"slot-zero",
    )?;
    let base_rows = reopened
        .range(ColumnFamily::Base, b"", b"\xff")
        .map_err(|error| error.to_string())?;
    println!(
        "CF_DEMO\tVAULT\t{}\tBASE_FILES\t{}\tSLOT_FILES\t{}\tBASE_ROWS\t{}\tBASE_DIR\t{}\tSLOT_DIR\t{}",
        vault.display(),
        reopened.level_file_count(ColumnFamily::Base),
        reopened.level_file_count(ColumnFamily::slot(SlotId::new(0))),
        base_rows.len(),
        vault.join("cf/base").display(),
        vault.join("cf/slot_00").display()
    );
    Ok(())
}

pub fn readback_level(cf_name: &str, level_dir: &Path) -> Result<(), String> {
    let cf = parse_cf(cf_name)?;
    let files = list_sst_files(level_dir)?;
    let level = SstLevel::from_oldest_first(files.clone());
    for row in level
        .range(b"", b"\xff")
        .map_err(|error| error.to_string())?
    {
        println!(
            "LEVEL\tCF\t{}\tFILES\t{}\tKEY\t{}\tVALUE\t{}",
            cf.name(),
            files.len(),
            hex_bytes(&row.key),
            hex_bytes(&row.value)
        );
    }
    Ok(())
}

fn assert_value(
    router: &CfRouter,
    cf: ColumnFamily,
    key: &[u8],
    expected: &[u8],
) -> Result<(), String> {
    let got = router
        .get(cf, key)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("missing {} key {}", cf.name(), hex_bytes(key)))?;
    if got != expected {
        return Err(format!("{} key {} mismatch", cf.name(), hex_bytes(key)));
    }
    Ok(())
}

fn list_sst_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn parse_cf(value: &str) -> Result<ColumnFamily, String> {
    match value {
        "base" => Ok(ColumnFamily::Base),
        "slot_00" => Ok(ColumnFamily::slot(SlotId::new(0))),
        _ => Err(format!("unsupported FSV column family: {value}")),
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_bytes_matches_lowercase_plain_hex() {
        assert_eq!(hex_bytes(b"k1"), "6b31");
    }

    #[test]
    fn fsv_cf_parser_names_supported_demo_cfs() {
        assert_eq!(parse_cf("base").unwrap(), ColumnFamily::Base);
        assert_eq!(
            parse_cf("slot_00").unwrap(),
            ColumnFamily::slot(SlotId::new(0))
        );
        assert!(parse_cf("slot_01").is_err());
    }
}
