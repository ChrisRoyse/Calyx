//! Frozen no-truncation TEI measurement for inputs longer than one context.

use std::str;
use std::time::Duration;

use calyx_core::{
    CalyxError, Input, Lens, LensId, Modality, Result, SegmentVector, SegmentationProvenance,
    SlotShape, SlotVector,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::tei_http::{
    DEFAULT_TEI_MAX_BATCH, TeiServiceInfo, parse_embedding_response, post_json,
    read_tei_service_info,
};
use crate::frozen::{
    FrozenLensContract, TEI_SEGMENTED_POOLED_REQUEST_POLICY, TEI_SEGMENTED_REQUEST_POLICY,
    sha256_digest,
};
use crate::lens::ensure_input_modality;

pub const CALYX_LENS_SEGMENTATION_INVALID: &str = "CALYX_LENS_SEGMENTATION_INVALID";

/// TEI enforces a per-input CHARACTER ceiling that is independent of, and can
/// bind before, the token limit: the live law fleet rejects an input of 96,000
/// or more characters with HTTP 422 (`inputs must have less than 96000
/// characters`) even when that input is within the lane's token context. A
/// 32,768-token lane packs a ~28k-token, 125,539-character opinion into a
/// single window that clears the token bound but not this one -- which is
/// exactly how the 109,133-opinion build failed closed at 89,004 rows (#1983).
/// Measured against the live service, not assumed.
const TEI_INPUT_CHAR_LIMIT: usize = 96_000;
/// Character ceiling the segmenter actually enforces on every request it makes,
/// kept safely below [`TEI_INPUT_CHAR_LIMIT`]. The segmenter measures spans in
/// BYTES, and a string's character count never exceeds its byte count, so a
/// window of at most this many bytes is always strictly under the service's
/// character limit regardless of how the server counts (UTF-8 bytes, Unicode
/// scalar values, or NFC folding). Overridable for a dedicated fleet with a
/// larger measured limit via `CALYX_TEI_SEGMENTED_MAX_INPUT_CHARS` (#1983).
const DEFAULT_MAX_TEI_INPUT_CHARS: usize = 90_000;
const MAX_INPUT_CHARS_ENV: &str = "CALYX_TEI_SEGMENTED_MAX_INPUT_CHARS";
/// Character budget for one batched `/tokenize` request, kept well inside the
/// service payload limit.
const MAX_TEI_BATCH_CHARS: usize = 1_000_000;
/// Ceiling on the tokens the segmenter will put in one embed request.
///
/// A lane advertises `max_batch_tokens` on the assumption that it owns the
/// device. This panel runs fifteen TEI containers on one card, so honouring a
/// 65,536-token advertisement on every lane makes their activation peaks
/// collide and several backends answer `CUDA_ERROR_OUT_OF_MEMORY` at once. The
/// effective budget is therefore `min(advertised, this ceiling)`, overridable
/// with `CALYX_TEI_SEGMENTED_MAX_BATCH_TOKENS` for a dedicated device.
const DEFAULT_TEI_MAX_BATCH_TOKENS: usize = 4_096;
const MAX_BATCH_TOKENS_ENV: &str = "CALYX_TEI_SEGMENTED_MAX_BATCH_TOKENS";

/// Resolves the effective per-request token budget, failing closed on a bad
/// override rather than silently falling back.
fn effective_max_batch_tokens(advertised: Option<usize>) -> Result<usize> {
    let ceiling = match std::env::var_os(MAX_BATCH_TOKENS_ENV) {
        Some(raw) => {
            let raw = raw.to_string_lossy().trim().to_string();
            let parsed: usize = raw.parse().map_err(|_| {
                config_error(format!(
                    "{MAX_BATCH_TOKENS_ENV}={raw:?} is not a whole number of tokens"
                ))
            })?;
            if parsed == 0 {
                return Err(config_error(format!("{MAX_BATCH_TOKENS_ENV} must be > 0")));
            }
            parsed
        }
        None => DEFAULT_TEI_MAX_BATCH_TOKENS,
    };
    Ok(advertised
        .filter(|budget| *budget > 0)
        .map_or(ceiling, |budget| budget.min(ceiling)))
}

/// Resolves the effective per-request character ceiling, failing closed on a
/// bad override rather than silently using the default. Any override must stay
/// strictly below the measured service limit; a value at or above it is refused
/// so an operator cannot re-open the exact gap that failed the build (#1983).
fn effective_max_input_chars() -> Result<usize> {
    match std::env::var_os(MAX_INPUT_CHARS_ENV) {
        Some(raw) => {
            let raw = raw.to_string_lossy().trim().to_string();
            let parsed: usize = raw.parse().map_err(|_| {
                config_error(format!(
                    "{MAX_INPUT_CHARS_ENV}={raw:?} is not a whole number of characters"
                ))
            })?;
            if parsed == 0 {
                return Err(config_error(format!("{MAX_INPUT_CHARS_ENV} must be > 0")));
            }
            if parsed >= TEI_INPUT_CHAR_LIMIT {
                return Err(config_error(format!(
                    "{MAX_INPUT_CHARS_ENV}={parsed} must stay below the measured TEI character limit {TEI_INPUT_CHAR_LIMIT}"
                )));
            }
            Ok(parsed)
        }
        None => Ok(DEFAULT_MAX_TEI_INPUT_CHARS),
    }
}

/// How the per-segment vectors of one document are combined into the value the
/// lens actually emits (#1980).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolPolicy {
    /// Preserve every segment as a distinct vector (`SlotShape::Multi`).
    ///
    /// The search index for this shape pins `segments x dim x 4` bytes, so it
    /// is budgeted per lane, never panel-wide (#1491, #1980).
    #[default]
    PreserveSegments,
    /// L2 normalize each segment, take the token-count-weighted mean, then
    /// L2 normalize the result, emitting one `SlotShape::Dense` vector.
    ///
    /// Token weighting keeps a 400-token paragraph from being outvoted by a
    /// 40-token one. Pooling is fail-closed: a document is pooled only if
    /// EVERY one of its segments measured successfully, so a pooled vector is
    /// never a silent partial document.
    MeanTokenWeightedL2,
}

impl PoolPolicy {
    const fn is_default(self) -> bool {
        matches!(self, Self::PreserveSegments)
    }

    const fn fingerprint(self) -> &'static str {
        match self {
            Self::PreserveSegments => "preserve-segments",
            Self::MeanTokenWeightedL2 => "mean-token-weighted-l2",
        }
    }
}

/// Where segment boundaries are allowed to fall.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryPolicy {
    /// Cut purely on token windows, ignoring document structure.
    #[default]
    TokenWindows,
    /// Prefer natural paragraph boundaries, resolved by a cascade of
    /// `{¶ n}` pilcrow markers, then blank lines, then single newlines, and
    /// pack whole paragraphs into each window. A paragraph larger than the
    /// window is split by token windows within that paragraph and counted.
    ///
    /// Measured coverage on the 109,133-opinion corpus: pilcrow 18.4%,
    /// blank-line 65.2%, newline 16.3%, zero documents with no boundary at all.
    ParagraphCascade,
}

impl BoundaryPolicy {
    const fn is_default(self) -> bool {
        matches!(self, Self::TokenWindows)
    }

    const fn fingerprint(self) -> &'static str {
        match self {
            Self::TokenWindows => "token-windows",
            Self::ParagraphCascade => "paragraph-cascade-pilcrow-blankline-newline",
        }
    }
}

/// Versioned, bounded, content-addressed long-input segmentation policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeiSegmentationPolicy {
    pub version: u16,
    pub max_input_tokens: u32,
    pub special_tokens: u16,
    pub overlap_tokens: u16,
    pub max_source_bytes: u64,
    pub max_segments: u32,
    /// How segments are combined. Defaults to segment-preserving so that every
    /// already-persisted policy deserializes and fingerprints unchanged.
    #[serde(default)]
    pub pool: PoolPolicy,
    /// Where cuts may fall. Defaults to token windows for the same reason.
    #[serde(default)]
    pub boundary: BoundaryPolicy,
}

pub const DEFAULT_TEI_SEGMENTATION_POLICY: TeiSegmentationPolicy = TeiSegmentationPolicy {
    version: 1,
    max_input_tokens: 512,
    special_tokens: 2,
    overlap_tokens: 32,
    max_source_bytes: 16 * 1024 * 1024,
    max_segments: 4096,
    pool: PoolPolicy::PreserveSegments,
    boundary: BoundaryPolicy::TokenWindows,
};

/// Pooled, paragraph-aware policy for the full-length law corpus build (#1980).
///
/// `max_input_tokens` is deliberately NOT fixed here: it is packed per lane to
/// that lane's own context limit, so a 384-token lane and an 8192-token lane
/// share paragraph boundaries but not window size. Packing each lane to its own
/// limit measured 15.3 GB panel-wide versus 27.6 GB for a uniform 382-token
/// window across the panel.
///
/// `overlap_tokens` is 0: paragraph boundaries are already semantic, and the
/// published sweeps agree that overlap buys nothing at natural boundaries while
/// inflating index cost (and that large overlap is actively harmful).
pub const fn pooled_paragraph_policy(max_input_tokens: u32) -> TeiSegmentationPolicy {
    TeiSegmentationPolicy {
        version: 1,
        max_input_tokens,
        special_tokens: 2,
        overlap_tokens: 0,
        max_source_bytes: 16 * 1024 * 1024,
        max_segments: 4096,
        pool: PoolPolicy::MeanTokenWeightedL2,
        boundary: BoundaryPolicy::ParagraphCascade,
    }
}

/// Paragraph-bounded policy that PRESERVES every segment (#1980).
///
/// This is the controlled counterpart of [`pooled_paragraph_policy`]: identical
/// boundaries and identical windows, differing only in whether the segments are
/// pooled. Measuring a lane under both is what turns "is chunk-level Multi worth
/// its pin?" into a measured question rather than an argued one.
pub const fn segments_paragraph_policy(max_input_tokens: u32) -> TeiSegmentationPolicy {
    TeiSegmentationPolicy {
        version: 1,
        max_input_tokens,
        special_tokens: 2,
        overlap_tokens: 0,
        max_source_bytes: 16 * 1024 * 1024,
        max_segments: 4096,
        pool: PoolPolicy::PreserveSegments,
        boundary: BoundaryPolicy::ParagraphCascade,
    }
}

/// Fixed probe used to count how many special tokens a lane's tokenizer adds.
const SPECIAL_TOKEN_PROBE: &str = "ohio supreme court";

/// Measures how many special tokens a live TEI lane adds to one encoded input.
///
/// The frozen policy asserts this count on every emitted segment, which is what
/// proves a segment was encoded whole rather than silently re-wrapped. It is a
/// property of the lane's own tokenizer -- BERT-family lanes add 2 (`[CLS]`,
/// `[SEP]`) but a last-token-pooled lane such as Qwen3-Embedding adds 1 -- so it
/// is measured per lane instead of assumed (#1980).
pub fn probe_tei_special_tokens(endpoint: &str) -> Result<u16> {
    let body = serde_json::to_vec(&json!({
        "inputs": SPECIAL_TOKEN_PROBE,
        "add_special_tokens": true
    }))
    .map_err(|error| segmentation_error(format!("special-token probe encode failed: {error}")))?;
    let raw = post_json(
        &tokenize_endpoint(endpoint)?,
        &body,
        Duration::from_secs(30),
    )?;
    let batched: Vec<Vec<TeiToken>> = serde_json::from_slice(&raw).map_err(|error| {
        segmentation_error(format!("special-token probe parse failed: {error}"))
    })?;
    let [tokens] = batched.as_slice() else {
        return Err(segmentation_error(format!(
            "special-token probe returned {} rows for one input",
            batched.len()
        )));
    };
    let count = tokens.iter().filter(|token| token.special).count();
    if count == 0 || count > 8 {
        return Err(segmentation_error(format!(
            "TEI lane reports an implausible special-token count {count}; the frozen policy cannot assert it"
        )));
    }
    u16::try_from(count).map_err(|_| segmentation_error("special-token count exceeds u16"))
}

/// Builds the pooled paragraph policy for a live TEI endpoint by reading that
/// lane's own `max_input_length` from `/info` (#1980).
///
/// Packing every lane to its own context limit -- rather than to the panel's
/// smallest -- is what took the measured chunk-level panel cost from 27.6 GB to
/// 15.3 GB, and is why an 8192-token lane pools ~1.0 segments per opinion while
/// a 384-token lane pools ~5.
pub fn pooled_policy_for_endpoint(endpoint: &str) -> Result<TeiSegmentationPolicy> {
    let info = read_tei_service_info(endpoint)?;
    let max_input_tokens = u32::try_from(info.max_input_length).map_err(|_| {
        segmentation_error(format!(
            "TEI service reported max_input_length {} which exceeds u32",
            info.max_input_length
        ))
    })?;
    let mut policy = pooled_paragraph_policy(max_input_tokens);
    policy.special_tokens = probe_tei_special_tokens(endpoint)?;
    policy.validate()?;
    Ok(policy)
}

impl TeiSegmentationPolicy {
    pub fn validate(self) -> Result<()> {
        if self.version == 0
            || self.max_input_tokens == 0
            || u32::from(self.special_tokens) >= self.max_input_tokens
            || u32::from(self.overlap_tokens)
                >= self.max_input_tokens - u32::from(self.special_tokens)
            || self.max_source_bytes == 0
            || self.max_segments == 0
        {
            return Err(segmentation_error(format!(
                "invalid frozen TEI segmentation policy {}",
                self.fingerprint()
            )));
        }
        Ok(())
    }

    pub fn fingerprint(self) -> String {
        // A policy that neither pools nor uses structural boundaries must
        // fingerprint byte-for-byte as it did before #1980, so that every
        // already-registered segmented lens keeps its LensId. Only a policy
        // that actually opts into new behaviour gets the new base string and
        // the extra terms.
        // The base string states whether the lens pools, so it is selected by
        // the pool policy alone: a structurally-bounded but segment-preserving
        // policy must NOT claim the pooled base, which asserts pooling.
        let base = if self.pool.is_default() {
            TEI_SEGMENTED_REQUEST_POLICY
        } else {
            TEI_SEGMENTED_POOLED_REQUEST_POLICY
        };
        let mut fingerprint = format!(
            "{base};version={};max_input_tokens={};special_tokens={};overlap_tokens={};max_source_bytes={};max_segments={}",
            self.version,
            self.max_input_tokens,
            self.special_tokens,
            self.overlap_tokens,
            self.max_source_bytes,
            self.max_segments
        );
        if !(self.pool.is_default() && self.boundary.is_default()) {
            fingerprint.push_str(&format!(
                ";pool={};boundary={}",
                self.pool.fingerprint(),
                self.boundary.fingerprint()
            ));
        }
        fingerprint
    }

    /// True when this policy emits one dense vector per document.
    pub const fn is_pooled(self) -> bool {
        matches!(self.pool, PoolPolicy::MeanTokenWeightedL2)
    }

    pub fn content_hash(self) -> [u8; 32] {
        sha256_digest(&[self.fingerprint().as_bytes()])
    }

    const fn content_capacity(self) -> u32 {
        self.max_input_tokens - self.special_tokens as u32
    }
}

/// TEI runtime that keeps every long-document segment as a distinct vector.
#[derive(Clone, Debug)]
pub struct TeiSegmentedLens {
    id: LensId,
    /// Address the runtime dials. Equal to `frozen_endpoint` unless an audited
    /// relocation has rebound this lens onto a different lane (#2053).
    endpoint: String,
    /// Address hashed into the frozen contract and therefore into the LensId.
    /// Never changes, so relocating a lens cannot change its identity.
    frozen_endpoint: String,
    modality: Modality,
    dim: u32,
    policy: TeiSegmentationPolicy,
    timeout: Duration,
    max_batch: usize,
}

impl TeiSegmentedLens {
    pub fn new(
        name: impl Into<String>,
        endpoint: impl Into<String>,
        modality: Modality,
        dim: u32,
        policy: TeiSegmentationPolicy,
    ) -> Result<Self> {
        policy.validate()?;
        if dim == 0 {
            return Err(CalyxError::lens_dim_mismatch(
                "segmented TEI output dimension must be greater than zero",
            ));
        }
        let name = name.into();
        let endpoint = endpoint.into();
        tokenize_endpoint(&endpoint)?;
        let id = contract_for(&name, &endpoint, modality, dim, policy).lens_id();
        Ok(Self {
            id,
            frozen_endpoint: endpoint.clone(),
            endpoint,
            modality,
            dim,
            policy,
            timeout: Duration::from_secs(30),
            max_batch: DEFAULT_TEI_MAX_BATCH,
        })
    }

    /// Rebinds this lens onto an audited relocated lane.
    ///
    /// Only the dialled address moves. `frozen_endpoint`, and therefore the
    /// contract and the `LensId`, are untouched -- that is the whole point: the
    /// corpus measured through this lens stays valid because the lens is still
    /// the same instrument, proven by [`crate::relocation`] and by the
    /// persisted runtime golden replayed on the new lane.
    pub fn relocated_to(mut self, live_endpoint: impl Into<String>) -> Result<Self> {
        let live_endpoint = live_endpoint.into();
        // Fail here, not on the first measurement: a relocation that names an
        // address this runtime cannot even form a /tokenize URL for is a
        // configuration error, and the resident must refuse to warm the lens.
        tokenize_endpoint(&live_endpoint)?;
        self.endpoint = live_endpoint;
        Ok(self)
    }

    /// Address this lens actually dials, which differs from the frozen contract
    /// endpoint exactly when an audited relocation is in force.
    pub fn live_endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Address hashed into the frozen contract and the `LensId`.
    pub fn frozen_endpoint(&self) -> &str {
        &self.frozen_endpoint
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_declared_max_batch(mut self, max_batch: Option<usize>) -> Result<Self> {
        match max_batch {
            Some(0) => Err(config_error("segmented TEI max_batch must be > 0")),
            Some(max_batch) => {
                self.max_batch = max_batch;
                Ok(self)
            }
            None => Ok(self),
        }
    }

    pub fn contract(&self, name: impl Into<String>) -> FrozenLensContract {
        // Deliberately the frozen endpoint, never the dialled one: the contract
        // is the lens identity, and a relocation must not be able to mint a new
        // one (#2053).
        contract_for(
            name,
            &self.frozen_endpoint,
            self.modality,
            self.dim,
            self.policy,
        )
    }

    fn validate_service_contract(&self) -> Result<TeiServiceInfo> {
        let info = read_tei_service_info(&self.endpoint)?;
        if info.max_input_length != self.policy.max_input_tokens as usize {
            return Err(CalyxError::lens_frozen_violation(format!(
                "segmented TEI service max_input_length {} != frozen {}",
                info.max_input_length, self.policy.max_input_tokens
            )));
        }
        if self.max_batch > info.max_client_batch_size {
            return Err(config_error(format!(
                "segmented TEI max_batch {} exceeds live max_client_batch_size {}",
                self.max_batch, info.max_client_batch_size
            )));
        }
        Ok(info)
    }

    fn prepare(&self, input: &Input) -> Result<PreparedInput> {
        ensure_input_modality(self, input)?;
        let text = str::from_utf8(&input.bytes).map_err(|error| {
            segmentation_error(format!("segmented TEI input is not UTF-8: {error}"))
        })?;
        let source_len = u64::try_from(input.bytes.len())
            .map_err(|_| segmentation_error("source byte length exceeds u64"))?;
        if source_len == 0 || source_len > self.policy.max_source_bytes {
            return Err(segmentation_error(format!(
                "source byte length {source_len} is outside 1..={} for policy {}",
                self.policy.max_source_bytes,
                self.policy.fingerprint()
            )));
        }
        // The paragraph path never tokenizes the whole document: TEI refuses an
        // input over MAX_TEI_INPUT_CHARS, and this corpus contains opinions of
        // 284k characters. Cutting on paragraphs first means every tokenize
        // request is a paragraph, which is bounded by construction.
        let (segments, source_token_count) = match self.policy.boundary {
            BoundaryPolicy::TokenWindows => {
                let tokens = self.tokenize(text, false)?;
                validate_source_tokens(text, &tokens)?;
                let count = u32::try_from(tokens.len())
                    .map_err(|_| segmentation_error("source token count exceeds u32"))?;
                (self.token_window_segments(text, &tokens)?, count)
            }
            BoundaryPolicy::ParagraphCascade => {
                let segments = self.paragraph_segments(text)?;
                // Counted, not assumed: the sum of the content tokens actually
                // measured in each emitted segment. Because the segments tile
                // the source, this is the whole document's measured token count.
                let count = segments
                    .iter()
                    .map(|segment| {
                        segment
                            .measured_tokens
                            .saturating_sub(u32::from(self.policy.special_tokens))
                    })
                    .sum();
                (segments, count)
            }
        };
        if source_token_count == 0 {
            return Err(segmentation_error(
                "segmented TEI source contains no measurable content tokens",
            ));
        }
        Ok(PreparedInput {
            source_hash: *blake3::hash(&input.bytes).as_bytes(),
            source_byte_len: source_len,
            source_token_count,
            segments,
        })
    }

    fn guard_segment_count(&self, produced: usize) -> Result<()> {
        if produced >= self.policy.max_segments as usize {
            return Err(segmentation_error(format!(
                "source requires more than frozen max_segments {}",
                self.policy.max_segments
            )));
        }
        Ok(())
    }

    /// Emits one segment beginning at token `start`, never extending past token
    /// `limit`, shrinking the window until the live tokenizer confirms it fits
    /// the frozen context. The returned `token_end` is the real end, which may
    /// be smaller than `limit`.
    fn emit_window(
        &self,
        source: &str,
        base: usize,
        token_base: u32,
        tokens: &[TeiToken],
        start: usize,
        limit: usize,
    ) -> Result<PreparedSegment> {
        let mut end = limit;
        loop {
            let byte_start = if start == 0 {
                0
            } else {
                content_offset(tokens, start)?
            };
            let byte_end = if end == tokens.len() {
                source.len()
            } else {
                content_offset(tokens, end)?
            };
            if byte_start >= byte_end
                || !source.is_char_boundary(byte_start)
                || !source.is_char_boundary(byte_end)
            {
                return Err(segmentation_error(format!(
                    "token range {start}..{end} produced invalid UTF-8 byte bounds {byte_start}..{byte_end}"
                )));
            }
            let text = source[byte_start..byte_end].to_string();
            let measured = self.tokenize(&text, true)?;
            let special_count = measured.iter().filter(|token| token.special).count();
            if special_count != self.policy.special_tokens as usize {
                return Err(segmentation_error(format!(
                    "segment special-token count {special_count} != frozen {}",
                    self.policy.special_tokens
                )));
            }
            if measured.len() <= self.policy.max_input_tokens as usize {
                return Ok(PreparedSegment {
                    byte_start: (base + byte_start) as u64,
                    byte_end: (base + byte_end) as u64,
                    token_start: token_base + start as u32,
                    token_end: token_base + end as u32,
                    measured_tokens: measured.len() as u32,
                    text,
                });
            }
            if end <= start + 1 {
                return Err(segmentation_error(format!(
                    "one source token cannot fit the frozen {}-token TEI context",
                    self.policy.max_input_tokens
                )));
            }
            end -= 1;
        }
    }

    fn token_window_segments(
        &self,
        source: &str,
        tokens: &[TeiToken],
    ) -> Result<Vec<PreparedSegment>> {
        let capacity = self.policy.content_capacity() as usize;
        let overlap = self.policy.overlap_tokens as usize;
        let mut start = 0usize;
        let mut segments = Vec::new();
        while start < tokens.len() {
            self.guard_segment_count(segments.len())?;
            let limit = (start + capacity).min(tokens.len());
            let prepared = self.emit_window(source, 0, 0, tokens, start, limit)?;
            let end = prepared.token_end as usize;
            segments.push(prepared);
            if end == tokens.len() {
                break;
            }
            let next = end.checked_sub(overlap).ok_or_else(|| {
                segmentation_error("overlap prevents deterministic segment progress")
            })?;
            if next <= start {
                return Err(segmentation_error(
                    "segmentation did not advance beyond the previous token start",
                ));
            }
            start = next;
        }
        Ok(segments)
    }

    /// Cuts on the paragraph cascade and packs whole paragraphs into each
    /// lane-sized window (#1980).
    ///
    /// Works entirely in byte space over paragraph spans, so the whole document
    /// is never sent to `/tokenize`: TEI refuses an input over
    /// `MAX_TEI_INPUT_CHARS`, and this corpus reaches 284k characters in one
    /// opinion. Every tokenize request is one paragraph or one emitted segment.
    ///
    /// A packed group is verified against the live tokenizer, not trusted: if
    /// merges across a paragraph joint push it over the window, the group is
    /// split by token windows inside itself. Segments tile the source exactly,
    /// so no byte is dropped and none is measured twice.
    fn paragraph_segments(&self, source: &str) -> Result<Vec<PreparedSegment>> {
        let capacity = self.policy.content_capacity();
        // TEI bounds a request by CHARACTERS as well as by tokens, so every
        // window must respect both. Resolve the character ceiling once and
        // thread it through both the single-paragraph split and the packer.
        let max_chars = effective_max_input_chars()?;
        let (spans, _boundary_source) = paragraph_spans(source);
        if spans.is_empty() {
            return Err(segmentation_error(
                "paragraph cascade produced no spans for a non-empty source",
            ));
        }
        // A single paragraph can itself exceed the character ceiling, so split
        // those on char boundaries before anything is sent anywhere.
        let spans = split_spans_to_request_limit(source, &spans, max_chars);
        let texts = spans
            .iter()
            .map(|(start, end)| &source[*start..*end])
            .collect::<Vec<_>>();
        let counts = self.count_tokens_batch(&texts)?;
        let (groups, _oversized) = pack_paragraphs(&spans, &counts, capacity, max_chars)?;

        let mut segments: Vec<PreparedSegment> = Vec::new();
        let mut token_cursor = 0u32;
        for group in &groups {
            let (Some(first), Some(last)) = (group.first(), group.last()) else {
                return Err(segmentation_error("paragraph packing produced empty group"));
            };
            let start = spans[*first].0;
            let end = spans[*last].1;
            if start >= end {
                continue;
            }
            self.emit_group(source, start, end, &mut segments, &mut token_cursor)?;
        }
        if segments.is_empty() {
            return Err(segmentation_error(
                "paragraph segmentation produced no measurable segments",
            ));
        }
        Ok(segments)
    }

    /// Emits the segments covering `source[start..end]`, which packing believes
    /// fits one window. The belief is checked against the live tokenizer; if it
    /// does not fit, the region is split by token windows within itself.
    fn emit_group(
        &self,
        source: &str,
        start: usize,
        end: usize,
        segments: &mut Vec<PreparedSegment>,
        token_cursor: &mut u32,
    ) -> Result<()> {
        let slice = &source[start..end];
        let measured = self.tokenize(slice, true)?;
        let special_count = measured.iter().filter(|token| token.special).count();
        if special_count != self.policy.special_tokens as usize {
            return Err(segmentation_error(format!(
                "segment special-token count {special_count} != frozen {}",
                self.policy.special_tokens
            )));
        }
        if measured.len() <= self.policy.max_input_tokens as usize {
            self.guard_segment_count(segments.len())?;
            let content =
                (measured.len() as u32).saturating_sub(u32::from(self.policy.special_tokens));
            segments.push(PreparedSegment {
                byte_start: start as u64,
                byte_end: end as u64,
                token_start: *token_cursor,
                token_end: *token_cursor + content,
                measured_tokens: measured.len() as u32,
                text: slice.to_string(),
            });
            *token_cursor += content;
            return Ok(());
        }

        // Oversized: split inside this region only, never across its neighbours.
        let tokens = self.tokenize(slice, false)?;
        validate_source_tokens(slice, &tokens)?;
        let capacity = self.policy.content_capacity() as usize;
        let mut cursor = 0usize;
        while cursor < tokens.len() {
            self.guard_segment_count(segments.len())?;
            let limit = (cursor + capacity).min(tokens.len());
            let mut prepared = self.emit_window(slice, start, 0, &tokens, cursor, limit)?;
            let next = prepared.token_end as usize;
            if next <= cursor {
                return Err(segmentation_error(
                    "paragraph segmentation did not advance beyond the previous token start",
                ));
            }
            cursor = next;
            // Segment token indices are the running count of content tokens
            // actually measured, so consecutive segments join exactly and the
            // final one lands on the document's measured token count.
            let content = prepared
                .measured_tokens
                .saturating_sub(u32::from(self.policy.special_tokens));
            prepared.token_start = *token_cursor;
            prepared.token_end = *token_cursor + content;
            *token_cursor += content;
            segments.push(prepared);
        }
        Ok(())
    }

    /// Counts content tokens for each text with one batched `/tokenize` request
    /// per bounded chunk, instead of one request per paragraph.
    fn count_tokens_batch(&self, texts: &[&str]) -> Result<Vec<u32>> {
        let mut counts = Vec::with_capacity(texts.len());
        let mut index = 0usize;
        while index < texts.len() {
            let mut chunk_end = index;
            let mut chunk_chars = 0usize;
            while chunk_end < texts.len() {
                let len = texts[chunk_end].len();
                if chunk_end > index
                    && (chunk_end - index >= self.max_batch
                        || chunk_chars + len > MAX_TEI_BATCH_CHARS)
                {
                    break;
                }
                chunk_chars += len;
                chunk_end += 1;
            }
            let body = serde_json::to_vec(&json!({
                "inputs": &texts[index..chunk_end],
                "add_special_tokens": false
            }))
            .map_err(|error| {
                segmentation_error(format!("tokenize batch encode failed: {error}"))
            })?;
            let raw = post_json(&tokenize_endpoint(&self.endpoint)?, &body, self.timeout)?;
            let batched: Vec<Vec<TeiToken>> = serde_json::from_slice(&raw).map_err(|error| {
                segmentation_error(format!("tokenize batch parse failed: {error}"))
            })?;
            if batched.len() != chunk_end - index {
                return Err(segmentation_error(format!(
                    "tokenize batch returned {} rows for {} inputs",
                    batched.len(),
                    chunk_end - index
                )));
            }
            for row in batched {
                counts.push(
                    u32::try_from(row.len())
                        .map_err(|_| segmentation_error("paragraph token count exceeds u32"))?,
                );
            }
            index = chunk_end;
        }
        Ok(counts)
    }

    fn tokenize(&self, text: &str, add_special_tokens: bool) -> Result<Vec<TeiToken>> {
        let body = serde_json::to_vec(&json!({
            "inputs": text,
            "add_special_tokens": add_special_tokens
        }))
        .map_err(|error| segmentation_error(format!("tokenize request encode failed: {error}")))?;
        let raw = post_json(&tokenize_endpoint(&self.endpoint)?, &body, self.timeout)?;
        // TEI `/tokenize` returns one token array per input, i.e. `[[..]]` even
        // for a single string. Parse the batched shape and take the sole row.
        let mut batched: Vec<Vec<TeiToken>> = serde_json::from_slice(&raw).map_err(|error| {
            segmentation_error(format!("tokenize response parse failed: {error}"))
        })?;
        if batched.len() != 1 {
            return Err(segmentation_error(format!(
                "tokenize returned {} rows for one input",
                batched.len()
            )));
        }
        Ok(batched.remove(0))
    }

    fn embed(
        &self,
        prepared: &[PreparedInput],
        max_batch_tokens: usize,
    ) -> Result<Vec<Vec<Vec<f32>>>> {
        let segments = prepared
            .iter()
            .flat_map(|input| input.segments.iter())
            .collect::<Vec<_>>();
        let texts = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>();
        let mut vectors = Vec::with_capacity(texts.len());
        // Batch by TOKENS as well as by count. A 32k-context lane accepts a
        // client batch of 128 by count, but 128 full-window segments is four
        // million tokens and the backend answers CUDA_ERROR_OUT_OF_MEMORY. The
        // budget is the lane's own advertised `max_batch_tokens`.
        for chunk in token_bounded_chunks(&segments, self.max_batch, max_batch_tokens) {
            let chunk = &texts[chunk.0..chunk.1];
            let body = serde_json::to_vec(&json!({ "inputs": chunk, "truncate": false })).map_err(
                |error| {
                    CalyxError::lens_unreachable(format!(
                        "segmented TEI embed request encode failed: {error}"
                    ))
                },
            )?;
            let raw = post_json(&self.endpoint, &body, self.timeout)?;
            vectors.extend(parse_embedding_response(&raw)?);
        }
        if vectors.len() != texts.len() {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "segmented TEI returned {} vectors for {} segments",
                vectors.len(),
                texts.len()
            )));
        }
        let mut cursor = vectors.into_iter();
        prepared
            .iter()
            .map(|input| {
                input
                    .segments
                    .iter()
                    .map(|_| {
                        let vector = cursor.next().expect("validated vector count");
                        if vector.len() != self.dim as usize {
                            return Err(CalyxError::lens_dim_mismatch(format!(
                                "segmented TEI dim {} != expected {}",
                                vector.len(),
                                self.dim
                            )));
                        }
                        if vector.iter().any(|value| !value.is_finite()) {
                            return Err(CalyxError::lens_numerical_invariant(
                                "segmented TEI vector contains NaN or Inf",
                            ));
                        }
                        Ok(vector)
                    })
                    .collect()
            })
            .collect()
    }
}

impl Lens for TeiSegmentedLens {
    fn id(&self) -> LensId {
        self.id
    }

    fn shape(&self) -> SlotShape {
        if self.policy.is_pooled() {
            // Pooled lenses emit one vector per document, so they index and pin
            // exactly like any other dense lane -- no `multi_maxsim_segments`
            // structure and no per-segment pin (#1980).
            SlotShape::Dense(self.dim)
        } else {
            SlotShape::Multi {
                token_dim: self.dim,
            }
        }
    }

    fn modality(&self) -> Modality {
        self.modality
    }

    fn measure(&self, input: &Input) -> Result<SlotVector> {
        self.measure_batch(std::slice::from_ref(input))?
            .pop()
            .ok_or_else(|| CalyxError::lens_dim_mismatch("segmented TEI returned no slot"))
    }

    fn measure_batch(&self, inputs: &[Input]) -> Result<Vec<SlotVector>> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let info = self.validate_service_contract()?;
        let max_batch_tokens = effective_max_batch_tokens(info.max_batch_tokens)?;
        let prepared = inputs
            .iter()
            .map(|input| self.prepare(input))
            .collect::<Result<Vec<_>>>()?;
        let vectors = self.embed(&prepared, max_batch_tokens)?;
        prepared
            .into_iter()
            .zip(vectors)
            .map(|(input, vectors)| {
                if self.policy.is_pooled() {
                    // Every segment measured successfully (any failure already
                    // returned above), so pooling here can never hide a partial
                    // document (#1980).
                    let weighted = input
                        .segments
                        .iter()
                        .zip(vectors.iter())
                        .map(|(segment, data)| (segment.measured_tokens, data.clone()))
                        .collect::<Vec<_>>();
                    let vector = SlotVector::Dense {
                        dim: self.dim,
                        data: pool_mean_token_weighted_l2(&weighted, self.dim)?,
                    };
                    vector.validate_schema()?;
                    return Ok(vector);
                }
                let segments = input
                    .segments
                    .into_iter()
                    .zip(vectors)
                    .map(|(segment, data)| SegmentVector {
                        byte_start: segment.byte_start,
                        byte_end: segment.byte_end,
                        token_start: segment.token_start,
                        token_end: segment.token_end,
                        measured_tokens: segment.measured_tokens,
                        data,
                    })
                    .collect();
                let vector = SlotVector::SegmentedMulti {
                    token_dim: self.dim,
                    provenance: SegmentationProvenance {
                        policy_version: self.policy.version,
                        policy_hash: self.policy.content_hash(),
                        source_hash: input.source_hash,
                        source_byte_len: input.source_byte_len,
                        source_token_count: input.source_token_count,
                        max_input_tokens: self.policy.max_input_tokens,
                        special_tokens: self.policy.special_tokens,
                        overlap_tokens: self.policy.overlap_tokens,
                        max_source_bytes: self.policy.max_source_bytes,
                        max_segments: self.policy.max_segments,
                    },
                    segments,
                };
                vector.validate_schema()?;
                Ok(vector)
            })
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize)]
struct TeiToken {
    #[serde(default)]
    special: bool,
    // TEI reports `null` byte offsets for special tokens (e.g. `<s>`/`</s>`),
    // so these must be optional; content tokens always carry real offsets.
    #[serde(default)]
    start: Option<usize>,
    #[serde(default)]
    stop: Option<usize>,
}

struct PreparedInput {
    source_hash: [u8; 32],
    source_byte_len: u64,
    source_token_count: u32,
    segments: Vec<PreparedSegment>,
}

struct PreparedSegment {
    byte_start: u64,
    byte_end: u64,
    token_start: u32,
    token_end: u32,
    measured_tokens: u32,
    text: String,
}

/// Returns the validated content-token start byte offset, failing closed if a
/// content token unexpectedly lacks an offset.
fn content_offset(tokens: &[TeiToken], index: usize) -> Result<usize> {
    tokens[index].start.ok_or_else(|| {
        segmentation_error(format!("content token {index} is missing a byte offset"))
    })
}

fn validate_source_tokens(source: &str, tokens: &[TeiToken]) -> Result<()> {
    // Subword tokenizers (SentencePiece/XLM-R) may emit several content tokens
    // over the same or overlapping source bytes for one glyph (e.g. byte-fallback
    // pieces, or one emoji splitting into two ids). Token starts must therefore
    // be non-decreasing and char-aligned, but need not be byte-disjoint.
    //
    // A content token may also be ZERO-WIDTH (`start == stop`): a normalizer that
    // collapses a run of whitespace still emits a token for the collapsed piece.
    // Measured on this corpus, jina-embeddings-v2-base-code emits token id 225
    // with empty text at the double space in `"docket  | Published"`. That is a
    // real tokenizer behaviour on real text, not corruption, and rejecting it
    // would refuse to measure the document at all. The invariant that actually
    // matters -- offsets are ordered, in bounds, and land on char boundaries so
    // segment byte ranges tile -- is unaffected.
    let mut previous_start = 0usize;
    for (index, token) in tokens.iter().enumerate() {
        let (Some(start), Some(stop)) = (token.start, token.stop) else {
            return Err(segmentation_error(format!(
                "source token {index} is missing byte offsets (special={})",
                token.special
            )));
        };
        if token.special
            || start > stop
            || start < previous_start
            || stop > source.len()
            || !source.is_char_boundary(start)
            || !source.is_char_boundary(stop)
        {
            return Err(segmentation_error(format!(
                "source token {index} has invalid UTF-8 offsets {start}..{stop} special={} previous_start={previous_start}",
                token.special
            )));
        }
        previous_start = start;
    }
    Ok(())
}

fn tokenize_endpoint(endpoint: &str) -> Result<String> {
    endpoint
        .strip_suffix("/embed")
        .map(|base| format!("{base}/tokenize"))
        .ok_or_else(|| {
            config_error(format!(
                "segmented TEI endpoint {endpoint:?} must end in /embed"
            ))
        })
}

/// Which structural signal produced a document's paragraph spans.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundarySource {
    /// `{¶ n}` markers retained in the opinion text (18.4% of the law corpus).
    Pilcrow,
    /// Blank-line separated paragraphs (65.2%).
    BlankLine,
    /// Single-newline separated lines (16.3%).
    Newline,
    /// A document with no interior boundary at all: one span.
    WholeDocument,
}

/// Resolves paragraph spans by the #1980 cascade: pilcrow markers, then blank
/// lines, then single newlines. Returns byte ranges into `text` that tile the
/// document with no gaps and no overlap, so packing them can never drop or
/// duplicate content.
///
/// Measured on the 109,133-opinion corpus, this cascade yields a structural
/// boundary for 100% of documents; none fall through to blind token windows.
pub fn paragraph_spans(text: &str) -> (Vec<(usize, usize)>, BoundarySource) {
    if text.is_empty() {
        return (Vec::new(), BoundarySource::WholeDocument);
    }

    // 1. Pilcrow markers. A marker opens a paragraph, so cut at each marker.
    let pilcrows: Vec<usize> = pilcrow_marker_starts(text);
    if pilcrows.len() >= 2 {
        return (spans_from_cuts(text, &pilcrows), BoundarySource::Pilcrow);
    }

    // 2. Blank lines. Cut after each run of >=2 newlines.
    let blank_cuts = cuts_after_newline_runs(text, 2);
    if blank_cuts.len() >= 2 {
        return (
            spans_from_cuts(text, &blank_cuts),
            BoundarySource::BlankLine,
        );
    }

    // 3. Single newlines.
    let line_cuts = cuts_after_newline_runs(text, 1);
    if line_cuts.len() >= 2 {
        return (spans_from_cuts(text, &line_cuts), BoundarySource::Newline);
    }

    (vec![(0, text.len())], BoundarySource::WholeDocument)
}

/// Byte offsets where a `{¶ n}` marker begins.
fn pilcrow_marker_starts(text: &str) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut starts = Vec::new();
    for (index, _) in text.match_indices('¶') {
        // Walk back over whitespace to an opening brace.
        let mut cursor = index;
        while cursor > 0 && (bytes[cursor - 1] as char).is_ascii_whitespace() {
            cursor -= 1;
        }
        if cursor > 0 && bytes[cursor - 1] == b'{' {
            starts.push(cursor - 1);
        }
    }
    starts
}

/// Byte offsets immediately after each run of at least `min_run` newlines.
fn cuts_after_newline_runs(text: &str, min_run: usize) -> Vec<usize> {
    let bytes = text.as_bytes();
    let mut cuts = vec![0usize];
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'\n' {
            let start = index;
            while index < bytes.len() && (bytes[index] == b'\n' || bytes[index] == b'\r') {
                index += 1;
            }
            let newlines = bytes[start..index].iter().filter(|b| **b == b'\n').count();
            if newlines >= min_run && index < bytes.len() {
                cuts.push(index);
            }
        } else {
            index += 1;
        }
    }
    cuts
}

/// Turns ascending cut offsets into contiguous `(start, end)` spans covering
/// the whole document.
fn spans_from_cuts(text: &str, cuts: &[usize]) -> Vec<(usize, usize)> {
    let mut spans = Vec::with_capacity(cuts.len());
    let mut ordered: Vec<usize> = cuts.to_vec();
    ordered.sort_unstable();
    ordered.dedup();
    if ordered.first() != Some(&0) {
        ordered.insert(0, 0);
    }
    for window in 0..ordered.len() {
        let start = ordered[window];
        let end = ordered.get(window + 1).copied().unwrap_or(text.len());
        if end > start {
            spans.push((start, end));
        }
    }
    spans
}

/// Groups consecutive segments into `[start, end)` request ranges bounded by
/// both a count and a token budget. Always yields at least one segment per
/// range, so a single over-budget segment is still sent (and refused by the
/// service) rather than silently dropped.
fn token_bounded_chunks(
    segments: &[&PreparedSegment],
    max_batch: usize,
    max_batch_tokens: usize,
) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    let mut index = 0usize;
    while index < segments.len() {
        let mut end = index + 1;
        let mut tokens = segments[index].measured_tokens as usize;
        while end < segments.len() && end - index < max_batch {
            let next = segments[end].measured_tokens as usize;
            if tokens + next > max_batch_tokens {
                break;
            }
            tokens += next;
            end += 1;
        }
        ranges.push((index, end));
        index = end;
    }
    ranges
}

/// Splits any span whose byte length exceeds `max_chars` into consecutive
/// char-aligned pieces, so every request the segmenter makes is within the
/// service's character ceiling. Byte length is a conservative proxy for the
/// character count the server enforces (chars <= bytes). The pieces still tile
/// the span, so this changes where cuts may fall, never what is measured (#1983).
fn split_spans_to_request_limit(
    text: &str,
    spans: &[(usize, usize)],
    max_chars: usize,
) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(spans.len());
    for (start, end) in spans.iter().copied() {
        let mut cursor = start;
        while end - cursor > max_chars {
            let mut cut = cursor + max_chars;
            while cut > cursor && !text.is_char_boundary(cut) {
                cut -= 1;
            }
            if cut <= cursor {
                break;
            }
            out.push((cursor, cut));
            cursor = cut;
        }
        if cursor < end {
            out.push((cursor, end));
        }
    }
    out
}

/// Packs whole paragraphs into windows bounded by BOTH `capacity` tokens and
/// `max_chars` characters; whichever bound the next paragraph would breach
/// closes the current window (#1983). Character length is measured in bytes, a
/// conservative proxy for the server's character count (chars <= bytes). The
/// token bound alone is not enough: a 32,768-token lane can pack a whole
/// 125,539-character opinion under its token budget yet blow past TEI's
/// character limit, which is how the 109k build failed closed.
///
/// A paragraph that alone exceeds either bound is emitted as its own oversized
/// unit; the caller splits it within itself (by token windows, which -- being
/// no larger than the paragraph -- also stay under the character ceiling) and
/// counts it. On the law corpus 7.16% of paragraphs (44,442 of 620,393) take
/// that path. Returns `(packed_groups, oversized_count)`.
pub fn pack_paragraphs(
    spans: &[(usize, usize)],
    token_counts: &[u32],
    capacity: u32,
    max_chars: usize,
) -> Result<(Vec<Vec<usize>>, usize)> {
    if spans.len() != token_counts.len() {
        return Err(segmentation_error(format!(
            "paragraph span count {} != token count {}",
            spans.len(),
            token_counts.len()
        )));
    }
    if capacity == 0 {
        return Err(segmentation_error("paragraph packing capacity must be > 0"));
    }
    if max_chars == 0 {
        return Err(segmentation_error(
            "paragraph packing character ceiling must be > 0",
        ));
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    let mut current_tokens = 0u32;
    let mut current_chars = 0usize;
    let mut oversized = 0usize;

    for (index, tokens) in token_counts.iter().copied().enumerate() {
        let chars = spans[index].1.saturating_sub(spans[index].0);
        // A paragraph that alone breaches either bound cannot share a window
        // with anything: flush the open window and emit it alone for the caller
        // to split within itself.
        if tokens > capacity || chars > max_chars {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
                current_tokens = 0;
                current_chars = 0;
            }
            groups.push(vec![index]);
            oversized += 1;
            continue;
        }
        // Close the window when adding this paragraph would breach EITHER the
        // token budget or the character ceiling.
        if !current.is_empty()
            && (current_tokens + tokens > capacity || current_chars + chars > max_chars)
        {
            groups.push(std::mem::take(&mut current));
            current_tokens = 0;
            current_chars = 0;
        }
        current.push(index);
        current_tokens += tokens;
        current_chars += chars;
    }
    if !current.is_empty() {
        groups.push(current);
    }
    Ok((groups, oversized))
}

/// Combines one document's segment vectors into a single dense vector:
/// L2 normalize each segment, take the token-count-weighted mean, L2 normalize
/// the result (#1980).
///
/// Fail-closed by construction: an empty segment set, a dimension mismatch, a
/// non-finite component, a zero-norm segment, or a zero total weight is an
/// error rather than a silently degraded vector. A pooled document is therefore
/// never a partial document.
pub fn pool_mean_token_weighted_l2(segments: &[(u32, Vec<f32>)], dim: u32) -> Result<Vec<f32>> {
    let width = dim as usize;
    if segments.is_empty() {
        return Err(segmentation_error(
            "cannot pool a document with zero measured segments",
        ));
    }
    let mut accumulator = vec![0f64; width];
    let mut total_weight = 0f64;

    for (tokens, data) in segments {
        if data.len() != width {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "pooled segment dim {} != lens dim {width}",
                data.len()
            )));
        }
        let mut norm = 0f64;
        for value in data {
            if !value.is_finite() {
                return Err(segmentation_error(
                    "pooled segment contains a non-finite component",
                ));
            }
            norm += f64::from(*value) * f64::from(*value);
        }
        let norm = norm.sqrt();
        if norm <= 0.0 || !norm.is_finite() {
            return Err(segmentation_error(
                "pooled segment has zero or non-finite norm",
            ));
        }
        // Token count is the weight, so a long paragraph is not outvoted by a
        // short one. A zero-token segment cannot exist, but guard anyway.
        let weight = f64::from((*tokens).max(1));
        total_weight += weight;
        for (slot, value) in accumulator.iter_mut().zip(data.iter()) {
            *slot += weight * (f64::from(*value) / norm);
        }
    }

    if total_weight <= 0.0 {
        return Err(segmentation_error("pooled document has zero total weight"));
    }
    for slot in accumulator.iter_mut() {
        *slot /= total_weight;
    }
    let mut norm = 0f64;
    for value in accumulator.iter() {
        norm += value * value;
    }
    let norm = norm.sqrt();
    if norm <= 0.0 || !norm.is_finite() {
        return Err(segmentation_error(
            "pooled document vector collapsed to zero norm",
        ));
    }
    Ok(accumulator
        .into_iter()
        .map(|value| (value / norm) as f32)
        .collect())
}

/// Picks the frozen contract matching the policy's emitted shape: a pooled
/// policy declares `Dense`, a segment-preserving one declares `Multi` (#1980).
fn contract_for(
    name: impl Into<String>,
    endpoint: &str,
    modality: Modality,
    dim: u32,
    policy: TeiSegmentationPolicy,
) -> FrozenLensContract {
    if policy.is_pooled() {
        FrozenLensContract::tei_http_segmented_pooled(
            name,
            endpoint,
            modality,
            dim,
            policy.fingerprint(),
        )
    } else {
        FrozenLensContract::tei_http_segmented(name, endpoint, modality, dim, policy.fingerprint())
    }
}

fn segmentation_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_LENS_SEGMENTATION_INVALID,
        message: message.into(),
        remediation: "fix the frozen segmentation policy or source bytes; never truncate, pool, or substitute the input",
    }
}

fn config_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix the frozen segmented TEI LensSpec and register a new LensId",
    }
}

#[cfg(test)]
mod pooled_tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f64 {
        let mut dot = 0f64;
        let mut na = 0f64;
        let mut nb = 0f64;
        for (x, y) in a.iter().zip(b.iter()) {
            dot += f64::from(*x) * f64::from(*y);
            na += f64::from(*x) * f64::from(*x);
            nb += f64::from(*y) * f64::from(*y);
        }
        dot / (na.sqrt() * nb.sqrt())
    }

    /// The load-bearing backward-compatibility guarantee: a policy that does
    /// not opt into #1980 behaviour must fingerprint exactly as it did before,
    /// so no already-registered segmented lens is re-identified.
    #[test]
    fn default_policy_fingerprint_is_byte_identical_to_pre_1980() {
        let expected = format!(
            "{TEI_SEGMENTED_REQUEST_POLICY};version=1;max_input_tokens=512;special_tokens=2;overlap_tokens=32;max_source_bytes={};max_segments=4096",
            16 * 1024 * 1024
        );
        assert_eq!(DEFAULT_TEI_SEGMENTATION_POLICY.fingerprint(), expected);
        assert!(!DEFAULT_TEI_SEGMENTATION_POLICY.is_pooled());
    }

    #[test]
    fn pooled_policy_gets_distinct_identity() {
        let pooled = pooled_paragraph_policy(384);
        let fingerprint = pooled.fingerprint();
        assert!(fingerprint.starts_with(TEI_SEGMENTED_POOLED_REQUEST_POLICY));
        assert!(fingerprint.contains("pool=mean-token-weighted-l2"));
        assert!(fingerprint.contains("boundary=paragraph-cascade"));
        assert!(fingerprint.contains("truncate=false"));
        assert!(pooled.is_pooled());
        // Distinct identity from the segment-preserving policy.
        assert_ne!(
            pooled.content_hash(),
            DEFAULT_TEI_SEGMENTATION_POLICY.content_hash()
        );
    }

    #[test]
    fn pooled_lane_declares_dense_not_multi() {
        // A pooled lane must index as an ordinary dense lane so it carries no
        // multi_maxsim_segments pin (#1491/#1980).
        assert!(pooled_paragraph_policy(512).is_pooled());
        assert!(!DEFAULT_TEI_SEGMENTATION_POLICY.is_pooled());
    }

    #[test]
    fn cascade_prefers_pilcrow_then_blankline_then_newline() {
        let (spans, source) =
            paragraph_spans("{\u{b6} 1} first.\n{\u{b6} 2} second.\n{\u{b6} 3} third.");
        assert_eq!(source, BoundarySource::Pilcrow);
        assert_eq!(spans.len(), 3);

        let (spans, source) = paragraph_spans("alpha text\n\nbeta text\n\ngamma text");
        assert_eq!(source, BoundarySource::BlankLine);
        assert_eq!(spans.len(), 3);

        let (spans, source) = paragraph_spans("alpha\nbeta\ngamma");
        assert_eq!(source, BoundarySource::Newline);
        assert_eq!(spans.len(), 3);

        let (spans, source) = paragraph_spans("one single run of prose");
        assert_eq!(source, BoundarySource::WholeDocument);
        assert_eq!(spans.len(), 1);
    }

    /// Spans must tile the document exactly: no dropped bytes, no duplicates.
    /// This is what makes "we never truncate" checkable rather than asserted.
    #[test]
    fn spans_tile_the_document_exactly() {
        for text in [
            "{\u{b6} 1} a\n{\u{b6} 2} b\n{\u{b6} 3} c",
            "alpha\n\nbeta\n\n\ngamma",
            "alpha\nbeta",
            "solo",
        ] {
            let (spans, _) = paragraph_spans(text);
            assert_eq!(spans.first().map(|s| s.0), Some(0));
            assert_eq!(spans.last().map(|s| s.1), Some(text.len()));
            for pair in spans.windows(2) {
                assert_eq!(pair[0].1, pair[1].0, "gap or overlap in {text:?}");
            }
            let covered: usize = spans.iter().map(|(s, e)| e - s).sum();
            assert_eq!(covered, text.len(), "coverage mismatch for {text:?}");
        }
    }

    #[test]
    fn packing_respects_capacity_and_counts_oversized() {
        let spans = vec![(0, 1), (1, 2), (2, 3), (3, 4)];
        let tokens = vec![100u32, 200, 900, 50];
        // Character lengths are one byte each, so only the token bound binds.
        let (groups, oversized) =
            pack_paragraphs(&spans, &tokens, 382, DEFAULT_MAX_TEI_INPUT_CHARS).expect("pack");
        assert_eq!(oversized, 1, "the 900-token paragraph is oversized");
        // No packed group of non-oversized paragraphs may exceed capacity.
        for group in &groups {
            if group.len() > 1 {
                let total: u32 = group.iter().map(|i| tokens[*i]).sum();
                assert!(total <= 382, "group {group:?} exceeded capacity");
            }
        }
        // Every paragraph appears exactly once: packing never drops content.
        let mut seen: Vec<usize> = groups.iter().flatten().copied().collect();
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 2, 3]);
    }

    /// A zero-width content token is a real tokenizer behaviour on this corpus
    /// (jina-embeddings-v2-base-code emits one at a collapsed double space), so
    /// the source-token validator must accept it while still rejecting genuinely
    /// broken offsets.
    #[test]
    fn zero_width_content_token_is_accepted_but_disorder_is_not() {
        let source = "docket  | Published";
        let ok = vec![
            TeiToken {
                special: false,
                start: Some(0),
                stop: Some(6),
            },
            TeiToken {
                special: false,
                start: Some(6),
                stop: Some(6),
            },
            TeiToken {
                special: false,
                start: Some(8),
                stop: Some(9),
            },
        ];
        validate_source_tokens(source, &ok).expect("zero-width content token must be accepted");

        let backwards = vec![
            TeiToken {
                special: false,
                start: Some(8),
                stop: Some(9),
            },
            TeiToken {
                special: false,
                start: Some(0),
                stop: Some(6),
            },
        ];
        assert!(
            validate_source_tokens(source, &backwards).is_err(),
            "out-of-order offsets must still fail closed"
        );
        let inverted = vec![TeiToken {
            special: false,
            start: Some(9),
            stop: Some(8),
        }];
        assert!(
            validate_source_tokens(source, &inverted).is_err(),
            "stop before start must still fail closed"
        );
        let out_of_bounds = vec![TeiToken {
            special: false,
            start: Some(0),
            stop: Some(source.len() + 1),
        }];
        assert!(
            validate_source_tokens(source, &out_of_bounds).is_err(),
            "an offset past the source must still fail closed"
        );
    }

    /// A single paragraph can exceed the per-request character limit (this
    /// corpus reaches 284k characters in one opinion), so those spans must be
    /// pre-split -- still tiling the source exactly.
    #[test]
    fn oversized_spans_are_split_to_the_request_limit_without_loss() {
        let ceiling = DEFAULT_MAX_TEI_INPUT_CHARS;
        let text = "a".repeat(ceiling * 2 + 37);
        let split = split_spans_to_request_limit(&text, &[(0, text.len())], ceiling);
        assert_eq!(split.len(), 3, "two full pieces plus the remainder");
        assert_eq!(split[0].0, 0);
        assert_eq!(split.last().expect("piece").1, text.len());
        for piece in &split {
            assert!(
                piece.1 - piece.0 <= ceiling,
                "piece {piece:?} exceeds the request limit"
            );
        }
        for pair in split.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "gap or overlap after splitting");
        }
        let covered: usize = split.iter().map(|(s, e)| e - s).sum();
        assert_eq!(covered, text.len());

        // A span already inside the limit is returned untouched.
        assert_eq!(
            split_spans_to_request_limit("short", &[(0, 5)], ceiling),
            vec![(0, 5)]
        );
    }

    /// A long-context lane accepts a large client batch by count and still
    /// refuses it by tokens, so request batching must respect both.
    #[test]
    fn embed_batches_are_bounded_by_tokens_as_well_as_count() {
        fn segment(tokens: u32) -> PreparedSegment {
            PreparedSegment {
                byte_start: 0,
                byte_end: 1,
                token_start: 0,
                token_end: tokens,
                measured_tokens: tokens,
                text: String::new(),
            }
        }
        let owned = [segment(8_000), segment(8_000), segment(8_000), segment(100)];
        let segments = owned.iter().collect::<Vec<_>>();

        let ranges = token_bounded_chunks(&segments, 128, 16_384);
        assert_eq!(
            ranges,
            vec![(0, 2), (2, 4)],
            "count alone would have sent all four in one 24k-token request"
        );

        // The count bound still applies when tokens are small.
        let small = [segment(10), segment(10), segment(10)];
        let small_refs = small.iter().collect::<Vec<_>>();
        assert_eq!(
            token_bounded_chunks(&small_refs, 2, 16_384),
            vec![(0, 2), (2, 3)]
        );

        // A single over-budget segment is still emitted, never dropped.
        let huge = [segment(40_000)];
        let huge_refs = huge.iter().collect::<Vec<_>>();
        assert_eq!(token_bounded_chunks(&huge_refs, 128, 16_384), vec![(0, 1)]);
    }

    #[test]
    fn pooling_is_unit_norm_and_deterministic() {
        let segments = vec![
            (100u32, vec![1.0f32, 0.0, 0.0, 0.0]),
            (50u32, vec![0.0f32, 1.0, 0.0, 0.0]),
        ];
        let a = pool_mean_token_weighted_l2(&segments, 4).expect("pool");
        let b = pool_mean_token_weighted_l2(&segments, 4).expect("pool");
        assert_eq!(a, b, "pooling must be deterministic");
        assert_eq!(a.len(), 4, "dimension preserved");
        let norm: f64 = a
            .iter()
            .map(|v| f64::from(*v) * f64::from(*v))
            .sum::<f64>()
            .sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "pooled vector must be unit norm, got {norm}"
        );
        // Token weighting: the 100-token segment must dominate the 50-token one.
        assert!(a[0] > a[1], "token-count weighting not applied");
    }

    #[test]
    fn pooling_fails_closed_on_bad_input() {
        assert!(
            pool_mean_token_weighted_l2(&[], 4).is_err(),
            "empty must fail"
        );
        assert!(
            pool_mean_token_weighted_l2(&[(1, vec![0.0, 0.0, 0.0, 0.0])], 4).is_err(),
            "zero-norm segment must fail closed, never pool to garbage"
        );
        assert!(
            pool_mean_token_weighted_l2(&[(1, vec![f32::NAN, 0.0, 0.0, 0.0])], 4).is_err(),
            "non-finite must fail closed"
        );
        assert!(
            pool_mean_token_weighted_l2(&[(1, vec![1.0, 0.0])], 4).is_err(),
            "dim mismatch must fail closed"
        );
    }

    /// A verbatim excerpt of a real Ohio Supreme Court opinion from the corpus
    /// being built: `State ex rel. Long v. Hamilton Cty. Coroner`, Slip Opinion
    /// No. 2021-Ohio-3315 (ohio-sc-whole canonical-v5). Public-domain judicial
    /// text. Kept verbatim, including the OCR form-feed and the `{¶ n}` markers,
    /// because the cascade is being tested against what the corpus actually
    /// contains rather than against synthetic prose.
    const REAL_OPINION: &str = "State ex rel. Long v. Hamilton Cty. Coroner (Slip Opinion)\nohio | filed 2021-09-23 | docket 2021-0354 | Published\nJudges: Per Curiam\n\n[Until this opinion appears in the Ohio Official Reports advance sheets, it may be cited as State\nex rel. Long v. Hamilton Cty. Coroner, Slip Opinion No. 2021-Ohio-3315.]\n\n                                           NOTICE\n      This slip opinion is subject to formal revision before it is published in an\n      advance sheet of the Ohio Official Reports. Readers are requested to\n      promptly notify the Reporter of Decisions, Supreme Court of Ohio, 65\n      South Front Street, Columbus, Ohio 43215, of any typographical or other\n      formal errors in the opinion, in order that corrections may be made before\n      the opinion is published.\n\n                          SLIP OPINION NO. 2021-OHIO-3315\n  THE STATE EX REL. LONG, APPELLANT, v. HAMILTON COUNTY CORONER,\n                                         APPELLEE.\nMandamus\u{2014}Writ of mandamus sought to compel a county coroner to produce a\n          DNA record\u{2014}A public office has no duty to produce a record that it does\n          not possess\u{2014}Court of appeals' dismissal of mandamus action affirmed.\n  (No. 2021-0354\u{2014}Submitted August 3, 2021\u{2014}Decided September 23, 2021.)\n     APPEAL from the Court of Appeals for Hamilton County, No. C-210120.\n          Per Curiam.\n          {\u{b6} 1} In this direct appeal, appellant, John W. Long, seeks a writ of\nmandamus to compel appellee, the Hamilton County Coroner, to produce a DNA\nrecord.     The First District Court of Appeals dismissed Long's complaint,\ndetermining that the coroner does not possess the requested record. We affirm.\n\u{c}                                SUPREME COURT OF OHIO\n\n                                      Background\n           {\u{b6} 2} Long attached to his complaint several pieces of correspondence\nbetween himself and the coroner's office. The earliest is a letter from the coroner's\noffice to Long dated February 26, 2019, which states, \"Attached are the complete\nrecords that you have requested.\"\n           {\u{b6} 3} A public office has no duty under R.C. 149.43 to produce a record\nthat it does not possess. Accordingly, we affirm the judgment of the court of\nappeals dismissing the mandamus action.\n";

    /// The cascade must find the pilcrow structure in real opinion text (18.4%
    /// of the corpus carries it) and cut there rather than mid-sentence.
    #[test]
    fn real_opinion_cascade_cuts_on_pilcrow_paragraphs() {
        let (spans, source) = paragraph_spans(REAL_OPINION);
        assert_eq!(
            source,
            BoundarySource::Pilcrow,
            "a real slip opinion with {{\u{b6} n}} markers must resolve on the pilcrow rung"
        );
        assert_eq!(
            spans.len(),
            4,
            "one preamble span plus one span per pilcrow marker"
        );
        // Every pilcrow-opened span must actually begin at its marker.
        for span in &spans[1..] {
            assert!(
                REAL_OPINION[span.0..].starts_with('{'),
                "pilcrow span must begin at the marker, got {:?}",
                &REAL_OPINION[span.0..span.0 + 12]
            );
        }
        // The holding sentence lives in the last span, not the first.
        assert!(
            REAL_OPINION[spans[spans.len() - 1].0..spans[spans.len() - 1].1]
                .contains("no duty under R.C. 149.43"),
            "the holding must land in the final paragraph span"
        );
    }

    /// Tiling on real text is the checkable form of "nothing was truncated".
    #[test]
    fn real_opinion_spans_tile_and_pack_without_loss() {
        let (spans, _) = paragraph_spans(REAL_OPINION);
        assert_eq!(spans.first().map(|s| s.0), Some(0));
        assert_eq!(spans.last().map(|s| s.1), Some(REAL_OPINION.len()));
        for pair in spans.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "gap or overlap in a real opinion");
        }
        let covered: usize = spans.iter().map(|(s, e)| e - s).sum();
        assert_eq!(
            covered,
            REAL_OPINION.len(),
            "real opinion not fully covered"
        );

        // Approximate the tokenizer at the measured corpus ratio of 4.4
        // chars/token to pack against the tightest lane in the roster (STAR-QA,
        // 384 ctx => 382 content tokens).
        let counts: Vec<u32> = spans
            .iter()
            .map(|(start, end)| ((end - start) as f64 / 4.4).ceil() as u32)
            .collect();
        let (groups, oversized) =
            pack_paragraphs(&spans, &counts, 382, DEFAULT_MAX_TEI_INPUT_CHARS).expect("pack");
        let mut seen: Vec<usize> = groups.iter().flatten().copied().collect();
        seen.sort_unstable();
        assert_eq!(
            seen,
            (0..spans.len()).collect::<Vec<_>>(),
            "every real paragraph must be packed exactly once"
        );
        assert_eq!(
            oversized, 0,
            "no paragraph of this excerpt exceeds the tightest lane window"
        );
        // Packing is doing real work: 4 paragraphs do not each become their own
        // request, and no window exceeds the lane's own capacity.
        assert!(
            groups.len() < spans.len(),
            "whole-paragraph packing must combine paragraphs, got {} groups for {} paragraphs",
            groups.len(),
            spans.len()
        );
        for group in &groups {
            let total: u32 = group.iter().map(|index| counts[*index]).sum();
            assert!(total <= 382, "group {group:?} exceeded the lane window");
        }

        // Against a tighter window the caption/NOTICE preamble no longer fits a
        // single window, so it takes the counted in-paragraph split path rather
        // than being silently dropped.
        let (tight_groups, tight_oversized) =
            pack_paragraphs(&spans, &counts, 128, DEFAULT_MAX_TEI_INPUT_CHARS).expect("pack tight");
        assert_eq!(
            tight_oversized, 1,
            "the caption/NOTICE preamble must be counted as oversized at a 128-token window"
        );
        let mut tight_seen: Vec<usize> = tight_groups.iter().flatten().copied().collect();
        tight_seen.sort_unstable();
        assert_eq!(
            tight_seen,
            (0..spans.len()).collect::<Vec<_>>(),
            "the oversized path must still place every paragraph exactly once"
        );
    }

    /// The point of the whole representation change: for a long opinion whose
    /// holding sits in the LAST paragraph, the pooled vector must retain that
    /// late content, while first-window truncation discards it entirely.
    #[test]
    fn pooled_vector_retains_late_content_that_truncation_discards() {
        // Four orthogonal "content directions": three preamble paragraphs and
        // one late holding paragraph.
        let preamble_a = vec![1.0f32, 0.0, 0.0, 0.0];
        let preamble_b = vec![0.0f32, 1.0, 0.0, 0.0];
        let preamble_c = vec![0.0f32, 0.0, 1.0, 0.0];
        let holding = vec![0.0f32, 0.0, 0.0, 1.0];

        let segments = vec![
            (120u32, preamble_a.clone()),
            (120u32, preamble_b),
            (120u32, preamble_c),
            (360u32, holding.clone()), // the substantive, longest paragraph
        ];

        let pooled = pool_mean_token_weighted_l2(&segments, 4).expect("pool");
        // Truncation keeps only the first window.
        let truncated = preamble_a;

        let pooled_to_holding = cosine(&pooled, &holding);
        let truncated_to_holding = cosine(&truncated, &holding);

        assert!(
            truncated_to_holding.abs() < 1e-9,
            "truncation should retain none of the holding, got {truncated_to_holding}"
        );
        assert!(
            pooled_to_holding > 0.5,
            "pooled vector must retain the late holding, got {pooled_to_holding}"
        );
        println!(
            "late-content retention: pooled cos={pooled_to_holding:.4} vs truncated cos={truncated_to_holding:.4}"
        );
    }

    /// The character ceiling must close a window even when the token budget is
    /// nowhere near spent -- the exact shape that sank the 109k build on the
    /// 32,768-token lane, where a whole opinion fit the token context but blew
    /// past TEI's character limit (#1983).
    #[test]
    fn packing_closes_windows_on_the_character_ceiling_not_only_tokens() {
        // Four paragraphs of 60 bytes each: 4 tokens total against a 10,000
        // token budget, so tokens never bind; only the 100-char ceiling does.
        let spans = vec![(0usize, 60usize), (60, 120), (120, 180), (180, 240)];
        let tokens = vec![1u32, 1, 1, 1];
        let (groups, oversized) = pack_paragraphs(&spans, &tokens, 10_000, 100).expect("pack");
        assert_eq!(
            oversized, 0,
            "no single paragraph exceeds the 100-char ceiling"
        );
        for group in &groups {
            let bytes: usize = group.iter().map(|i| spans[*i].1 - spans[*i].0).sum();
            assert!(
                bytes <= 100,
                "group {group:?} = {bytes} bytes exceeds the 100-char ceiling"
            );
        }
        // Token-only packing would have put all four in one window.
        assert!(
            groups.len() > 1,
            "the character ceiling failed to close any window"
        );
        // Tiling: every paragraph placed exactly once, no gaps, no duplicates.
        let mut seen: Vec<usize> = groups.iter().flatten().copied().collect();
        seen.sort_unstable();
        assert_eq!(seen, vec![0, 1, 2, 3]);
    }

    /// The resolved character ceiling -- default or a valid override -- must
    /// always land strictly under the measured service limit, so the segmenter
    /// can never re-open the gap that failed the build (#1983).
    #[test]
    fn resolved_char_ceiling_stays_under_the_measured_limit() {
        let resolved = effective_max_input_chars().expect("default ceiling resolves");
        assert!(
            resolved < TEI_INPUT_CHAR_LIMIT,
            "resolved ceiling {resolved} must stay under the measured limit {TEI_INPUT_CHAR_LIMIT}"
        );
    }

    /// The verbatim largest opinion in the 109,133-document corpus being built
    /// (`corpus_109133.jsonl`, sha256 42a5bb6f71…, 324,288 bytes / 320,957
    /// characters of public-domain judicial text). This is the class of input
    /// that failed the build closed at 89,004 rows: within a 32,768-token
    /// lane's context yet far over TEI's 96,000-character request ceiling
    /// (#1983). Kept verbatim so the packer is tested against what the corpus
    /// actually contains rather than synthetic prose.
    const OVERSIZED_OPINION: &str = include_str!("testdata/oversized_opinion.txt");

    /// Approximate content tokens at the corpus-measured 4.4 chars/token ratio
    /// (matching the pilcrow test above). Token counts only drive the token
    /// bound; the character bound this fixture exists to prove is exact
    /// regardless of the ratio, because it is computed from byte spans.
    fn approx_content_tokens(spans: &[(usize, usize)]) -> Vec<u32> {
        spans
            .iter()
            .map(|(s, e)| ((((e - s) as f64) / 4.4).ceil() as u32).max(1))
            .collect()
    }

    #[test]
    fn real_oversized_opinion_chunks_within_both_bounds_on_every_lane() {
        assert!(
            OVERSIZED_OPINION.len() > TEI_INPUT_CHAR_LIMIT,
            "fixture must actually exceed the TEI character limit, got {} bytes",
            OVERSIZED_OPINION.len()
        );
        let (spans, _) = paragraph_spans(OVERSIZED_OPINION);
        assert_eq!(spans.first().map(|s| s.0), Some(0));
        assert_eq!(spans.last().map(|s| s.1), Some(OVERSIZED_OPINION.len()));

        let max_chars = DEFAULT_MAX_TEI_INPUT_CHARS;
        let split = split_spans_to_request_limit(OVERSIZED_OPINION, &spans, max_chars);
        // Every request-sized span is within the ceiling and they still tile.
        assert_eq!(split.first().map(|s| s.0), Some(0));
        assert_eq!(split.last().map(|s| s.1), Some(OVERSIZED_OPINION.len()));
        for pair in split.windows(2) {
            assert_eq!(
                pair[0].1, pair[1].0,
                "gap or overlap after request-limit split"
            );
        }
        for piece in &split {
            assert!(
                piece.1 - piece.0 <= max_chars,
                "piece {piece:?} exceeds the character ceiling"
            );
        }
        let counts = approx_content_tokens(&split);

        // qwen3 (Qwen3-Embedding, 32,768 ctx, 1 special token) and medembed
        // (512 ctx, 2 special tokens): the widest and tightest lanes in the
        // roster. Content capacity = ctx - special.
        for (lane, capacity) in [("qwen3", 32_768u32 - 1), ("medembed", 512u32 - 2)] {
            let (groups, _oversized) = pack_paragraphs(&split, &counts, capacity, max_chars)
                .unwrap_or_else(|error| panic!("pack {lane}: {error:?}"));
            // Deterministic: identical input yields identical windows.
            let (again, _) =
                pack_paragraphs(&split, &counts, capacity, max_chars).expect("pack again");
            assert_eq!(groups, again, "{lane}: packing must be deterministic");

            // Tiling: every request span placed exactly once.
            let mut seen: Vec<usize> = groups.iter().flatten().copied().collect();
            seen.sort_unstable();
            assert_eq!(
                seen,
                (0..split.len()).collect::<Vec<_>>(),
                "{lane}: every span must be placed exactly once"
            );

            for group in &groups {
                let start = split[*group.first().expect("non-empty group")].0;
                let end = split[*group.last().expect("non-empty group")].1;
                // CHARACTER bound: every window is under the ceiling. This is
                // the invariant whose absence failed the build.
                assert!(
                    end - start <= max_chars,
                    "{lane}: window {start}..{end} = {} bytes exceeds the {max_chars}-char ceiling",
                    end - start
                );
                // TOKEN bound: a multi-paragraph window never exceeds the lane
                // context; a single-paragraph window may (the caller token-
                // splits it within itself, staying under the char ceiling too).
                if group.len() > 1 {
                    let total: u32 = group.iter().map(|i| counts[*i]).sum();
                    assert!(
                        total <= capacity,
                        "{lane}: window {group:?} = {total} tokens exceeds {capacity}"
                    );
                }
            }
        }
    }

    #[test]
    fn real_oversized_opinion_late_paragraphs_are_represented_not_truncated() {
        // On the 32k lane the opinion no longer collapses into a single window
        // (which is what pushed it over 96k chars); it tiles into several, and
        // the final window reaches the end of the document -- so a pooled
        // vector sees the closing paragraphs, not just the caption (#1980/#1983).
        let (spans, _) = paragraph_spans(OVERSIZED_OPINION);
        let max_chars = DEFAULT_MAX_TEI_INPUT_CHARS;
        let split = split_spans_to_request_limit(OVERSIZED_OPINION, &spans, max_chars);
        let counts = approx_content_tokens(&split);
        let (groups, _) =
            pack_paragraphs(&split, &counts, 32_768 - 1, max_chars).expect("pack 32k lane");
        let min_windows = OVERSIZED_OPINION.len() / max_chars;
        assert!(
            groups.len() >= min_windows,
            "a {}-byte opinion must tile into at least {} windows on the 32k lane, got {}",
            OVERSIZED_OPINION.len(),
            min_windows,
            groups.len()
        );
        // The last window ends at the last byte of the opinion: closing
        // paragraphs are covered, never dropped.
        let last = groups.last().expect("at least one window");
        assert_eq!(
            split[*last.last().expect("non-empty group")].1,
            OVERSIZED_OPINION.len(),
            "the final window must reach the end of the document"
        );
    }
}
