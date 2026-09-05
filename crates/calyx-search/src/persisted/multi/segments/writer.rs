use super::*;

pub(in crate::persisted) struct StreamingSegmentsWriter {
    vault_dir: PathBuf,
    root: PathBuf,
    slot: SlotId,
    token_dim: u32,
    base_seq: u64,
    current_rows: Vec<EncodedMultiRow>,
    current_bytes: u64,
    row_count: usize,
    token_count: usize,
    segments: Vec<MultiSegmentRef>,
    max_segment_bytes: u64,
    token_ann: Option<super::super::token_ann::TokenAnnBuilder>,
}

#[derive(Debug)]
pub(in crate::persisted) struct SegmentFlush {
    pub(in crate::persisted) ordinal: usize,
    pub(in crate::persisted) row_count: usize,
    pub(in crate::persisted) total_rows: usize,
    pub(in crate::persisted) detail: String,
}

impl StreamingSegmentsWriter {
    pub(in crate::persisted) fn new(
        vault_dir: &Path,
        root: &Path,
        slot: SlotId,
        token_dim: u32,
        base_seq: u64,
        build_backend: calyx_sextant::index::DiskAnnBuildBackend,
    ) -> Self {
        Self::with_max_segment_bytes(
            vault_dir,
            root,
            slot,
            token_dim,
            base_seq,
            build_backend,
            bounds::DEFAULT_MAX_MULTI_BINARY_SEGMENT_BYTES,
        )
    }

    fn with_max_segment_bytes(
        vault_dir: &Path,
        root: &Path,
        slot: SlotId,
        token_dim: u32,
        base_seq: u64,
        build_backend: calyx_sextant::index::DiskAnnBuildBackend,
        max_segment_bytes: u64,
    ) -> Self {
        Self {
            vault_dir: vault_dir.to_path_buf(),
            root: root.to_path_buf(),
            slot,
            token_dim,
            base_seq,
            current_rows: Vec::new(),
            current_bytes: bounds::BINARY_HEADER_BYTES,
            row_count: 0,
            token_count: 0,
            segments: Vec::new(),
            max_segment_bytes,
            token_ann: Some(super::super::token_ann::TokenAnnBuilder::new(
                slot,
                token_dim,
                base_seq,
                build_backend,
            )),
        }
    }

    pub(in crate::persisted) fn push(
        &mut self,
        cx_id: CxId,
        tokens: Vec<Vec<f32>>,
    ) -> CliResult<Option<SegmentFlush>> {
        let token_count = u32::try_from(tokens.len()).map_err(|_| {
            unbounded_multi_sidecar(format!(
                "persistent multi row {cx_id} for slot {} has more than u32::MAX tokens",
                self.slot
            ))
        })?;
        let bytes = encode_slot_vector(&SlotVector::Multi {
            token_dim: self.token_dim,
            tokens,
        })?;
        self.push_encoded(cx_id, token_count, bytes)
    }

    pub(in crate::persisted) fn push_encoded(
        &mut self,
        cx_id: CxId,
        token_count: u32,
        bytes: Vec<u8>,
    ) -> CliResult<Option<SegmentFlush>> {
        let encoded = EncodedMultiSlotVector::new(&bytes).map_err(|error| {
            CalyxError::aster_corrupt_shard(format!(
                "slot {} cx {cx_id} has malformed encoded multi payload: {}",
                self.slot, error.message
            ))
        })?;
        if encoded.token_dim() != self.token_dim || encoded.token_count() != token_count {
            return Err(stale(format!(
                "slot {} cx {cx_id} encoded multi shape mismatch: token_dim={} expected={}, token_count={} expected={token_count}",
                self.slot,
                encoded.token_dim(),
                self.token_dim,
                encoded.token_count()
            )));
        }
        self.token_ann
            .as_mut()
            .expect("token ANN builder is present until finish")
            .push_encoded(cx_id, encoded)?;
        let row_bytes = bounds::row_estimated_bytes(self.token_dim, token_count as usize)?;
        if bounds::BINARY_HEADER_BYTES + row_bytes > self.max_segment_bytes {
            return Err(unbounded_multi_sidecar(format!(
                "persistent multi row {cx_id} for slot {} is estimated {} bytes; exceeds search binary segment limit {} bytes (tokens={}, token_dim={})",
                self.slot,
                bounds::BINARY_HEADER_BYTES + row_bytes,
                self.max_segment_bytes,
                token_count,
                self.token_dim
            )));
        }
        let mut flushed = None;
        if !self.current_rows.is_empty() && self.current_bytes + row_bytes > self.max_segment_bytes
        {
            flushed = self.flush_current()?;
        }
        self.current_bytes += row_bytes;
        self.token_count = self
            .token_count
            .checked_add(token_count as usize)
            .ok_or_else(|| stale("streaming multi token_count overflow"))?;
        self.row_count = self
            .row_count
            .checked_add(1)
            .ok_or_else(|| stale("streaming multi row_count overflow"))?;
        self.current_rows.push(EncodedMultiRow {
            cx_id,
            token_count,
            bytes,
        });
        Ok(flushed)
    }

    pub(in crate::persisted) fn finish(
        mut self,
    ) -> CliResult<(SearchIndexEntry, Option<SegmentFlush>)> {
        let flushed = match self.flush_current() {
            Ok(flushed) => flushed,
            Err(primary) => {
                return match self.cleanup_segments() {
                    Ok(()) => Err(primary),
                    Err(cleanup) => Err(stale(format!(
                        "multi final segment write failed [{}] {}; cleanup also failed [{}] {}",
                        primary.code(),
                        primary.message(),
                        cleanup.code(),
                        cleanup.message()
                    ))),
                };
            }
        };
        let token_ann = match self
            .token_ann
            .take()
            .expect("token ANN builder is present until finish")
            .finish(&self.vault_dir, &self.root, self.row_count)
        {
            Ok(artifacts) => artifacts,
            Err(primary) => {
                return match self.cleanup_segments() {
                    Ok(()) => Err(primary),
                    Err(cleanup) => Err(stale(format!(
                        "multi token ANN build failed [{}] {}; segment cleanup also failed [{}] {}",
                        primary.code(),
                        primary.message(),
                        cleanup.code(),
                        cleanup.message()
                    ))),
                };
            }
        };
        let result = write_segments_manifest(
            &self.vault_dir,
            &self.root,
            self.slot,
            SegmentManifestBuild {
                token_dim: self.token_dim,
                row_count: self.row_count,
                token_count: self.token_count,
                base_seq: self.base_seq,
                segments: self.segments.clone(),
            },
        );
        match result {
            Ok(entry) => Ok((
                entry.with_token_ann(
                    token_ann.graph_rel,
                    token_ann.id_map_rel,
                    token_ann.id_map_sha256,
                ),
                flushed,
            )),
            Err(primary) => {
                let ann_cleanup = fs::remove_dir_all(&token_ann.directory);
                match self.cleanup_segments() {
                    Ok(()) if ann_cleanup.is_ok() => Err(primary),
                    segment_cleanup => Err(stale(format!(
                        "multi segment manifest publish failed [{}] {}; cleanup status: segments={:?}, token_ann={:?}",
                        primary.code(),
                        primary.message(),
                        segment_cleanup
                            .err()
                            .map(|error| error.message().to_string()),
                        ann_cleanup.err().map(|error| error.to_string())
                    ))),
                }
            }
        }
    }

    pub(in crate::persisted) fn abort(mut self) -> CliResult {
        self.current_rows.clear();
        self.token_ann.take();
        self.cleanup_segments()
    }

    fn flush_current(&mut self) -> CliResult<Option<SegmentFlush>> {
        if self.current_rows.is_empty() {
            return Ok(None);
        }
        let ordinal = self.segments.len();
        let rows = std::mem::take(&mut self.current_rows);
        let segment = write_encoded_binary_segment(
            &self.vault_dir,
            &self.root,
            self.slot,
            self.token_dim,
            &rows,
            self.base_seq,
            ordinal,
        )?;
        let bytes = bounds::segment_estimated_bytes(
            self.token_dim,
            segment.row_count,
            segment.token_count,
        )?;
        let flush = SegmentFlush {
            ordinal,
            row_count: segment.row_count,
            total_rows: self.row_count,
            detail: format!(
                "segment={ordinal} rows={} tokens={} bytes={bytes} sha256={} path={}",
                segment.row_count, segment.token_count, segment.sha256, segment.index_rel
            ),
        };
        self.segments.push(segment);
        self.current_bytes = bounds::BINARY_HEADER_BYTES;
        Ok(Some(flush))
    }

    fn cleanup_segments(&mut self) -> CliResult {
        let mut failures = Vec::new();
        for segment in self.segments.drain(..) {
            let path = checked_segment_path(&self.vault_dir, &segment.index_rel, self.slot)?;
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => failures.push(format!("{}: {error}", path.display())),
            }
        }
        if !failures.is_empty() {
            return Err(stale(format!(
                "failed to remove partial multi segments for slot {}: {}",
                self.slot,
                failures.join("; ")
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abort_removes_segments_flushed_before_a_later_failure() {
        let vault_dir = std::env::temp_dir().join(format!(
            "calyx-search-stream-writer-abort-{}",
            std::process::id()
        ));
        let root = vault_dir.join("idx/search");
        fs::remove_dir_all(&vault_dir).ok();
        fs::create_dir_all(&root).unwrap();
        let mut writer = StreamingSegmentsWriter::with_max_segment_bytes(
            &vault_dir,
            &root,
            SlotId::new(11),
            2,
            113_411,
            calyx_sextant::index::DiskAnnBuildBackend::CpuVamana,
            90,
        );
        let row = vec![vec![1.0, 0.0], vec![0.0, 1.0]];

        assert!(
            writer
                .push(CxId::from_bytes([1; 16]), row.clone())
                .unwrap()
                .is_none()
        );
        let flush = writer
            .push(CxId::from_bytes([2; 16]), row)
            .unwrap()
            .expect("second row flushes first segment");
        let before_abort = segment_paths(&root);
        assert_eq!(flush.row_count, 1);
        assert_eq!(before_abort.len(), 1);

        writer.abort().expect("abort removes flushed segment");

        let after_abort = segment_paths(&root);
        assert!(after_abort.is_empty());
        println!(
            "STREAM_WRITER_ABORT_FSV before={before_abort:?} after={after_abort:?} slot=11 base_seq=113411"
        );
        fs::remove_dir_all(vault_dir).ok();
    }

    #[test]
    fn prepublish_validation_reopens_and_hashes_segment_bytes() {
        let vault_dir = std::env::temp_dir().join(format!(
            "calyx-search-stream-writer-verify-{}",
            std::process::id()
        ));
        let root = vault_dir.join("idx/search");
        fs::remove_dir_all(&vault_dir).ok();
        fs::create_dir_all(&root).unwrap();
        let slot = SlotId::new(11);
        let mut writer = StreamingSegmentsWriter::new(
            &vault_dir,
            &root,
            slot,
            2,
            113_411,
            calyx_sextant::index::DiskAnnBuildBackend::CpuVamana,
        );
        writer
            .push(
                CxId::from_bytes([3; 16]),
                vec![vec![1.0, 0.0], vec![0.0, 1.0]],
            )
            .unwrap();
        let (entry, _) = writer.finish().expect("finish segment generation");
        let manifest = read_segments_manifest(&vault_dir, &entry, 113_411, slot).unwrap();
        let segment_path =
            checked_segment_path(&vault_dir, &manifest.segments[0].index_rel, slot).unwrap();
        let mut bytes = fs::read(&segment_path).unwrap();
        let before_len = bytes.len();
        let last = bytes.last_mut().unwrap();
        *last ^= 1;
        fs::write(&segment_path, &bytes).unwrap();

        let error = match validate_segment_files(&vault_dir, slot, 2, &manifest) {
            Ok(_) => panic!("same-size corruption must fail prepublish validation"),
            Err(error) => error,
        };

        assert_eq!(
            fs::metadata(&segment_path).unwrap().len() as usize,
            before_len
        );
        assert_eq!(error.code(), "CALYX_STALE_DERIVED");
        assert!(error.message().contains("sha256"));
        println!(
            "SEGMENT_PREPUBLISH_HASH_FSV path={} bytes={} error_code={} detail={}",
            segment_path.display(),
            before_len,
            error.code(),
            error.message()
        );
        fs::remove_dir_all(vault_dir).ok();
    }

    #[test]
    fn encoded_streaming_sidecar_is_byte_exact_with_decoded_writer() {
        let vault_dir = std::env::temp_dir().join(format!(
            "calyx-search-encoded-stream-parity-{}",
            std::process::id()
        ));
        let root = vault_dir.join("idx/search");
        fs::remove_dir_all(&vault_dir).ok();
        fs::create_dir_all(&root).unwrap();
        let slot = SlotId::new(11);
        let cx_id = CxId::from_bytes([7; 16]);
        let tokens = vec![vec![1.25, -2.5], vec![3.75, 4.5]];
        let encoded = encode_slot_vector(&SlotVector::Multi {
            token_dim: 2,
            tokens: tokens.clone(),
        })
        .unwrap();
        let mut writer = StreamingSegmentsWriter::new(
            &vault_dir,
            &root,
            slot,
            2,
            113_411,
            calyx_sextant::index::DiskAnnBuildBackend::CpuVamana,
        );
        writer.push_encoded(cx_id, 2, encoded).unwrap();
        let (entry, _) = writer.finish().unwrap();
        let manifest = read_segments_manifest(&vault_dir, &entry, 113_411, slot).unwrap();
        let encoded_path =
            checked_segment_path(&vault_dir, &manifest.segments[0].index_rel, slot).unwrap();
        let legacy_path = root.join("decoded-reference.multi.bin");
        let legacy_sha =
            binary::write_binary_atomic_hashed(&legacy_path, slot, 2, &[(cx_id, tokens)], 113_411)
                .unwrap();

        let encoded_bytes = fs::read(&encoded_path).unwrap();
        let legacy_bytes = fs::read(&legacy_path).unwrap();
        assert_eq!(encoded_bytes, legacy_bytes);
        assert_eq!(manifest.segments[0].sha256, legacy_sha);
        println!(
            "ENCODED_MULTI_SIDECAR_PARITY_FSV source={} reference={} bytes={} sha256={legacy_sha}",
            encoded_path.display(),
            legacy_path.display(),
            encoded_bytes.len()
        );
        fs::remove_dir_all(vault_dir).ok();
    }

    #[test]
    fn encoded_streaming_failure_removes_every_partial_segment() {
        let vault_dir = std::env::temp_dir().join(format!(
            "calyx-search-encoded-stream-invalid-{}",
            std::process::id()
        ));
        let root = vault_dir.join("idx/search");
        fs::remove_dir_all(&vault_dir).ok();
        fs::create_dir_all(&root).unwrap();
        let slot = SlotId::new(11);
        let valid = encode_slot_vector(&SlotVector::Multi {
            token_dim: 2,
            tokens: vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        })
        .unwrap();
        let mut non_finite = valid.clone();
        non_finite[9..13].copy_from_slice(&f32::NAN.to_bits().to_be_bytes());
        let mut writer = StreamingSegmentsWriter::with_max_segment_bytes(
            &vault_dir,
            &root,
            slot,
            2,
            113_411,
            calyx_sextant::index::DiskAnnBuildBackend::CpuVamana,
            90,
        );
        writer
            .push_encoded(CxId::from_bytes([1; 16]), 2, valid)
            .unwrap();
        writer
            .push_encoded(CxId::from_bytes([2; 16]), 2, non_finite)
            .unwrap();
        let before = segment_paths(&root);
        let error = writer.finish().unwrap_err();
        let after = segment_paths(&root);

        assert_eq!(before.len(), 1);
        assert!(after.is_empty());
        assert_eq!(error.code(), "CALYX_LENS_NUMERICAL_INVARIANT");
        println!(
            "ENCODED_MULTI_INVALID_FSV before={before:?} after={after:?} error_code={} detail={}",
            error.code(),
            error.message()
        );
        fs::remove_dir_all(vault_dir).ok();
    }

    #[test]
    fn encoded_streaming_rejects_mixed_dimensions_before_writing() {
        let vault_dir = std::env::temp_dir().join(format!(
            "calyx-search-encoded-stream-dim-{}",
            std::process::id()
        ));
        let root = vault_dir.join("idx/search");
        fs::remove_dir_all(&vault_dir).ok();
        fs::create_dir_all(&root).unwrap();
        let slot = SlotId::new(11);
        let encoded = encode_slot_vector(&SlotVector::Multi {
            token_dim: 3,
            tokens: vec![vec![1.0, 0.0, 0.0]],
        })
        .unwrap();
        let mut writer = StreamingSegmentsWriter::new(
            &vault_dir,
            &root,
            slot,
            2,
            113_411,
            calyx_sextant::index::DiskAnnBuildBackend::CpuVamana,
        );
        let before = segment_paths(&root);
        let error = writer
            .push_encoded(CxId::from_bytes([3; 16]), 1, encoded)
            .unwrap_err();
        let after = segment_paths(&root);

        assert!(before.is_empty());
        assert!(after.is_empty());
        assert_eq!(error.code(), "CALYX_STALE_DERIVED");
        assert!(error.message().contains("shape mismatch"));
        println!(
            "ENCODED_MULTI_DIM_FSV before={before:?} after={after:?} error_code={} detail={}",
            error.code(),
            error.message()
        );
        fs::remove_dir_all(vault_dir).ok();
    }

    fn segment_paths(root: &Path) -> Vec<PathBuf> {
        let mut paths = fs::read_dir(root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "bin"))
            .collect::<Vec<_>>();
        paths.sort();
        paths
    }
}
