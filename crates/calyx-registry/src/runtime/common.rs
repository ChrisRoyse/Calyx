use std::env;
use std::fs;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Input, Lens, Result};

use crate::frozen::LengthDelimitedSha256;
use crate::lens::ensure_input_modality;

pub const DEFAULT_MAX_TOKENS: usize = 512;
const STREAM_HASH_BUFFER_BYTES: usize = 1024 * 1024;

pub fn default_hf_cache_root() -> PathBuf {
    if let Some(path) = env::var_os("HF_HOME") {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("CALYX_HOME") {
        return PathBuf::from(path).join(".hf-cache");
    }
    PathBuf::from(".hf-cache")
}

pub fn fastembed_cache_root(default_cache: &Path) -> PathBuf {
    env::var_os("HF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| default_cache.to_path_buf())
}

pub fn hash_files(paths: &[PathBuf]) -> Result<[u8; 32]> {
    let mut hasher = LengthDelimitedSha256::new();
    let mut buffer = vec![0_u8; STREAM_HASH_BUFFER_BYTES];
    for path in paths {
        hash_file_into(path, &mut hasher, &mut buffer)?;
    }
    Ok(hasher.finalize())
}

fn hash_file_into(
    path: &Path,
    hasher: &mut LengthDelimitedSha256,
    buffer: &mut [u8],
) -> Result<()> {
    let file = fs::File::open(path).map_err(|err| {
        CalyxError::lens_unreachable(format!(
            "open lens artifact {} for hashing failed: {err}",
            path.display()
        ))
    })?;
    let len = file.metadata().map_err(|err| {
        CalyxError::lens_unreachable(format!(
            "stat lens artifact {} for hashing failed: {err}",
            path.display()
        ))
    })?;
    hasher.begin_part(len.len());
    let mut reader = BufReader::new(file);
    loop {
        let read = reader.read(buffer).map_err(|err| {
            CalyxError::lens_unreachable(format!(
                "read lens artifact {} while hashing failed: {err}",
                path.display()
            ))
        })?;
        if read == 0 {
            return Ok(());
        }
        hasher.update_chunk(&buffer[..read]);
    }
}

pub fn text_from_input<'a>(lens: &dyn Lens, input: &'a Input) -> Result<&'a str> {
    ensure_input_modality(lens, input)?;
    std::str::from_utf8(&input.bytes).map_err(|err| {
        CalyxError::lens_dim_mismatch(format!("lens {} input is not UTF-8: {err}", lens.id()))
    })
}

pub fn normalize_unit(data: &mut [f32]) -> Result<()> {
    if data.iter().any(|value| !value.is_finite()) {
        return Err(CalyxError::lens_numerical_invariant(
            "local neural lens emitted NaN or Inf",
        ));
    }
    let sum = data
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>();
    let norm = sum.sqrt();
    if !norm.is_finite() || norm <= 0.0 {
        return Err(CalyxError::lens_numerical_invariant(
            "local neural lens emitted zero-norm vector",
        ));
    }
    for value in data {
        *value = (*value as f64 / norm) as f32;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frozen::sha256_digest;

    #[test]
    fn normalize_unit_rejects_zero_norm() {
        let mut values = vec![0.0, 0.0];

        let error = normalize_unit(&mut values).unwrap_err();

        assert_eq!(error.code, "CALYX_LENS_NUMERICAL_INVARIANT");
    }

    #[test]
    fn normalize_unit_produces_unit_length() {
        let mut values = vec![3.0, 4.0];

        normalize_unit(&mut values).unwrap();

        let norm = values.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1.0e-6);
    }

    #[test]
    fn hash_files_streams_length_delimited_contract_digest() {
        let root =
            std::env::temp_dir().join(format!("calyx-runtime-hash-stream-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let first = root.join("first.bin");
        let second = root.join("second.bin");
        let first_bytes = vec![0x3c_u8; STREAM_HASH_BUFFER_BYTES + 17];
        let second_bytes = b"second contract part".to_vec();
        fs::write(&first, &first_bytes).unwrap();
        fs::write(&second, &second_bytes).unwrap();

        let actual = hash_files(&[first, second]).unwrap();
        let expected = sha256_digest(&[&first_bytes, &second_bytes]);

        assert_eq!(actual, expected);
        let _ = fs::remove_dir_all(&root);
    }
}
