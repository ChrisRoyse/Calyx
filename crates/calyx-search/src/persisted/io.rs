use super::*;

pub(super) fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> CliResult {
    write_bytes_atomic(path, &serde_json::to_vec_pretty(value)?)
}

pub(super) fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(())
}

/// Stream `value` as compact JSON straight to `path` (atomic temp + rename), hashing
/// the bytes as they pass through. This avoids materializing the whole serialized
/// sidecar in memory: `to_vec_pretty` over a full multi-vector (ColBERT) corpus builds
/// a multi-gigabyte buffer and pins one core for many minutes formatting billions of
/// floats — the post-ingest finalization hang. Returns the lowercase-hex SHA-256 of the
/// written bytes (computed in the same single pass, no second walk over the buffer).
pub(super) fn write_json_atomic_hashed<T: Serialize>(path: &Path, value: &T) -> CliResult<String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let sha256 = {
        let file = File::create(&tmp)?;
        let mut writer = HashingWriter::new(BufWriter::new(file));
        serde_json::to_writer(&mut writer, value).inspect_err(|_| {
            let _ = fs::remove_file(&tmp);
        })?;
        let (buf_writer, sha256) = writer.into_parts();
        let file = buf_writer
            .into_inner()
            .map_err(|err| CliError::io(format!("flush index sidecar {}: {err}", tmp.display())))?;
        file.sync_all()?;
        sha256
    };
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })?;
    Ok(sha256)
}

/// `io::Write` adapter that folds every written byte into a SHA-256 digest as it passes
/// through to the inner writer, so a sidecar's hash is produced during the streaming
/// write instead of a separate pass over a fully materialized buffer.
pub(super) struct HashingWriter<W: Write> {
    inner: W,
    hasher: Sha256,
}

impl<W: Write> HashingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    fn into_parts(self) -> (W, String) {
        let digest = self.hasher.finalize();
        let mut hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            hex.push_str(&format!("{byte:02x}"));
        }
        (self.inner, hex)
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = self.inner.write(buf)?;
        self.hasher.update(&buf[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

pub(super) fn rel(root: &Path, path: &Path) -> CliResult<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|err| CliError::usage(format!("index path is outside vault root: {err}")))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

pub(super) fn stale(message: impl Into<String>) -> CliError {
    CalyxError::stale_derived(message).into()
}
