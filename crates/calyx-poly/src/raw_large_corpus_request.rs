use std::env;
use std::fs;
use std::path::PathBuf;

use crate::raw_large_corpus::LargeCorpusRequest;
use crate::raw_large_corpus_support::validate_large_corpus_request;
use crate::raw_source_support::display_safe_path;
use crate::{PolyError, Result};

impl LargeCorpusRequest {
    pub fn target_default() -> Self {
        Self {
            output_root: PathBuf::from("target/fsv/issue191_large_corpus_core"),
            timeout_secs: 45,
            max_body_bytes: 50 * 1024 * 1024,
            page_size: 100,
            max_pages_per_dataset: 5,
            require_exhaustive: false,
        }
    }

    pub fn normalized(mut self) -> Result<Self> {
        if self.output_root.is_relative() {
            let current_dir = env::current_dir().map_err(|err| {
                PolyError::raw_source(
                    "POLY_LARGE_CORPUS_CURRENT_DIR_FAILED",
                    format!("read current directory: {err}"),
                )
            })?;
            self.output_root = current_dir.join(&self.output_root);
        }
        fs::create_dir_all(&self.output_root).map_err(|err| {
            PolyError::raw_source(
                "POLY_LARGE_CORPUS_OUTPUT_ROOT_CREATE_FAILED",
                format!("create output root {}: {err}", self.output_root.display()),
            )
        })?;
        self.output_root =
            display_safe_path(fs::canonicalize(&self.output_root).map_err(|err| {
                PolyError::raw_source(
                    "POLY_LARGE_CORPUS_OUTPUT_ROOT_CANONICALIZE_FAILED",
                    format!(
                        "canonicalize output root {}: {err}",
                        self.output_root.display()
                    ),
                )
            })?);
        validate_large_corpus_request(&self)?;
        Ok(self)
    }
}
