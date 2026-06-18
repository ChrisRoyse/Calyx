use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct VideoMetadata {
    pub(crate) path: String,
    pub(crate) source_title: Option<String>,
    pub(crate) source_url: Option<String>,
    pub(crate) page_url: Option<String>,
    pub(crate) license: Option<String>,
    pub(crate) license_url: Option<String>,
    pub(crate) sha256: String,
    pub(crate) bytes: u64,
    pub(crate) frame_count: u64,
    pub(crate) fps: f64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) codec: String,
    pub(crate) container: String,
    pub(crate) mime: String,
}

impl VideoMetadata {
    pub(crate) fn load(path: &Path) -> Result<Vec<Self>, String> {
        let text =
            fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
        let mut rows = Vec::new();
        let mut paths = BTreeSet::new();
        let mut hashes = BTreeSet::new();
        for (idx, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let row: Self = serde_json::from_str(line)
                .map_err(|error| format!("{}:{}: {error}", path.display(), idx + 1))?;
            row.validate(idx + 1)?;
            if !paths.insert(row.path.clone()) {
                return Err(format!(
                    "CALYX_FSV_MEDIA_VIDEO_DUPLICATE_PATH: line {} repeats {}",
                    idx + 1,
                    row.path
                ));
            }
            if !hashes.insert(row.sha256.clone()) {
                return Err(format!(
                    "CALYX_FSV_MEDIA_VIDEO_DUPLICATE_SOURCE_SHA256: line {} repeats {}",
                    idx + 1,
                    row.sha256
                ));
            }
            rows.push(row);
        }
        if rows.is_empty() {
            return Err("CALYX_FSV_MEDIA_VIDEO_EMPTY_METADATA".to_string());
        }
        Ok(rows)
    }

    fn validate(&self, line: usize) -> Result<(), String> {
        if self.path.trim().is_empty() || self.sha256.len() != 64 {
            return Err(format!(
                "CALYX_FSV_MEDIA_VIDEO_INVALID_METADATA: line {line} missing path or sha256"
            ));
        }
        if self.bytes == 0
            || self.frame_count == 0
            || self.width == 0
            || self.height == 0
            || !self.fps.is_finite()
            || self.fps <= 0.0
            || self.codec.trim().is_empty()
            || self.container.trim().is_empty()
            || self.mime.trim().is_empty()
        {
            return Err(format!(
                "CALYX_FSV_MEDIA_VIDEO_INVALID_METADATA: line {line} has incomplete decode metadata"
            ));
        }
        if !self
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(format!(
                "CALYX_FSV_MEDIA_VIDEO_INVALID_METADATA: line {line} sha256 must be lowercase hex"
            ));
        }
        Ok(())
    }
}
