//! Streaming ingest pipeline: channel-backed ingester, on-the-fly TurboQuant
//! quantization, and token-bucket backpressure (PH72 · T01).
//!
//! A [`StreamIngester`] accepts events at a real-time rate over an `mpsc`
//! channel. A background flush task drains the channel in microbatches of at most
//! [`MICROBATCH_MAX`] events, quantizes each dense slot on the fly (seed
//! content-addressed per `LensId + CxId`, never random — A25), persists each
//! event through [`ingest_at`](crate::dedup::ingest_at), and writes exactly one
//! `STREAM_BATCH` Ledger entry per microbatch (A15). A [`BackpressureGuard`]
//! enforces A26: when the token budget is exhausted, [`StreamIngester::send`]
//! returns `CALYX_STREAM_BACKPRESSURE` rather than letting the channel grow
//! unbounded.
//!
//! Fail-closed boundaries (DOCTRINE §0):
//! - A non-finite slot coefficient is rejected at `send` with
//!   `CALYX_FORGE_INPUT_NAN` *before* the event is queued, quantized, or written.
//! - Backpressure rejection returns `CALYX_STREAM_BACKPRESSURE` and the event is
//!   never queued.
//! - A storage fault in the flush task is captured and surfaced by
//!   [`StreamIngester::drain_and_close`] as an `Err` — never silently dropped.

mod backpressure;
mod quantize_online;

pub use backpressure::{BackpressureGuard, CALYX_STREAM_BACKPRESSURE};
pub use quantize_online::{
    CALYX_FORGE_INPUT_NAN, QuantizeOnlineConfig, quantize_slot_online, rotation_seed_entropy,
};

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use calyx_core::{CalyxError, Clock, CxId, LedgerRef, Result, SlotVector, VaultStore};
use calyx_ledger::{ActorId, EntryKind, SubjectId};

use crate::cf::{ColumnFamily, ledger_key};
use crate::dedup::{DedupResult, EpochSecs, IngestInput, ingest_at};
use crate::vault::AsterVault;
use quantize_online::input_nan_error;

/// Maximum number of events drained into a single microbatch.
pub const MICROBATCH_MAX: usize = 256;

/// Marker embedded in each per-microbatch Ledger payload and subject.
pub const STREAM_BATCH_MARKER: &str = "STREAM_BATCH";

const STREAM_CLOSED_CODE: &str = "CALYX_STREAM_CLOSED";
const STREAM_CLOSED_REMEDIATION: &str =
    "the stream ingester was already drained/closed; build a new StreamIngester to resume";

/// Hook invoked after each successful `ingest_at` and before the stream-batch
/// ledger row. The `LedgerRef` is read from the Ledger CF after the ingest write.
pub type PostIngestHook<C> =
    Arc<dyn Fn(&AsterVault<C>, CxId, LedgerRef) -> Result<()> + Send + Sync>;

fn stream_closed_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: STREAM_CLOSED_CODE,
        message: message.into(),
        remediation: STREAM_CLOSED_REMEDIATION,
    }
}

/// A single streamed event: the raw ingest input plus its explicit event time.
pub struct StreamEvent {
    /// The constellation input to persist.
    pub input: IngestInput,
    /// Explicit event time; honored verbatim by `ingest_at` (no silent re-stamp).
    pub at: EpochSecs,
}

/// Outcome counters for a stream session, read after a clean shutdown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamStats {
    /// Events persisted to the vault.
    pub ingested: usize,
    /// Dense slots quantized on the fly.
    pub quantized: usize,
    /// `send` calls rejected by backpressure.
    pub backpressured: usize,
    /// Microbatches flushed (one Ledger entry each).
    pub batches: usize,
}

#[derive(Default)]
struct FlushState {
    stats: StreamStats,
    error: Option<CalyxError>,
}

/// Channel-backed streaming ingester with backpressure and on-the-fly quantize.
pub struct StreamIngester<C>
where
    C: Clock + Send + Sync + 'static,
{
    sender: Option<Sender<StreamEvent>>,
    guard: Arc<BackpressureGuard>,
    backpressured: Arc<AtomicUsize>,
    flush: Arc<Mutex<FlushState>>,
    handle: Option<JoinHandle<()>>,
    _vault: Arc<AsterVault<C>>,
}

impl<C> StreamIngester<C>
where
    C: Clock + Send + Sync + 'static,
{
    /// Builds an ingester over `vault`, spawning the background flush task.
    pub fn new(
        vault: Arc<AsterVault<C>>,
        config: QuantizeOnlineConfig,
        guard: BackpressureGuard,
    ) -> Self {
        Self::new_with_post_ingest_hook(vault, config, guard, None)
    }

    /// Builds an ingester with a post-ingest hook for reactive evaluation.
    pub fn new_with_post_ingest_hook(
        vault: Arc<AsterVault<C>>,
        config: QuantizeOnlineConfig,
        guard: BackpressureGuard,
        post_ingest: Option<PostIngestHook<C>>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel::<StreamEvent>();
        let guard = Arc::new(guard);
        let backpressured = Arc::new(AtomicUsize::new(0));
        let flush = Arc::new(Mutex::new(FlushState::default()));
        let handle = {
            let vault = Arc::clone(&vault);
            let flush = Arc::clone(&flush);
            thread::spawn(move || {
                flush_loop(&vault, &receiver, &config, &flush, post_ingest.as_ref())
            })
        };
        Self {
            sender: Some(sender),
            guard,
            backpressured,
            flush,
            handle: Some(handle),
            _vault: vault,
        }
    }

    /// Submits one event for streamed ingestion.
    ///
    /// Validates finiteness at the boundary (fail-closed `CALYX_FORGE_INPUT_NAN`),
    /// acquires one backpressure token (fail-closed `CALYX_STREAM_BACKPRESSURE`),
    /// then hands the event to the flush task. Never blocks indefinitely.
    pub fn send(&self, input: IngestInput, at: EpochSecs) -> Result<()> {
        for (slot_id, vector) in &input.slots {
            if let Some(idx) = nonfinite_index(vector) {
                return Err(input_nan_error(format!(
                    "slot {slot_id} has a non-finite coefficient at index {idx}"
                )));
            }
        }
        if let Err(err) = self.guard.acquire(1) {
            self.backpressured.fetch_add(1, Ordering::AcqRel);
            return Err(err);
        }
        let Some(sender) = self.sender.as_ref() else {
            return Err(stream_closed_error("send on a closed stream ingester"));
        };
        sender
            .send(StreamEvent { input, at })
            .map_err(|_| stream_closed_error("flush task is gone; channel closed"))
    }

    /// Backpressure guard, for driving refill or observing available tokens.
    pub fn guard(&self) -> &BackpressureGuard {
        &self.guard
    }

    /// Flushes remaining events, joins the flush task, and returns final stats.
    ///
    /// Returns `Err` if the flush task captured a storage fault, so a failure is
    /// never masked by a green return value.
    pub fn drain_and_close(mut self) -> Result<StreamStats> {
        // Dropping the only Sender signals the flush loop to finish draining.
        self.sender = None;
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .map_err(|_| stream_closed_error("flush task panicked"))?;
        }
        let mut state = self
            .flush
            .lock()
            .map_err(|_| stream_closed_error("flush state lock poisoned"))?;
        if let Some(err) = state.error.take() {
            return Err(err);
        }
        let mut stats = state.stats.clone();
        stats.backpressured = self.backpressured.load(Ordering::Acquire);
        Ok(stats)
    }
}

/// Returns the index of the first non-finite coefficient, if any.
fn nonfinite_index(vector: &SlotVector) -> Option<usize> {
    match vector {
        SlotVector::Dense { data, .. } => data.iter().position(|v| !v.is_finite()),
        SlotVector::Sparse { entries, .. } => {
            entries.iter().position(|entry| !entry.val.is_finite())
        }
        SlotVector::Multi { tokens, .. } => {
            let mut offset = 0;
            for token in tokens {
                if let Some(local) = token.iter().position(|v| !v.is_finite()) {
                    return Some(offset + local);
                }
                offset += token.len();
            }
            None
        }
        SlotVector::Absent { .. } => None,
    }
}

#[derive(Default)]
struct BatchOutcome {
    ingested: usize,
    quantized: usize,
}

/// Drains the channel into microbatches and processes each, capturing the first
/// storage fault into shared state and stopping (fail-closed).
fn flush_loop<C>(
    vault: &AsterVault<C>,
    receiver: &Receiver<StreamEvent>,
    config: &QuantizeOnlineConfig,
    flush: &Arc<Mutex<FlushState>>,
    post_ingest: Option<&PostIngestHook<C>>,
) where
    C: Clock + Send + Sync + 'static,
{
    loop {
        let mut batch = Vec::new();
        match receiver.recv() {
            Ok(event) => batch.push(event),
            Err(_) => break, // all senders dropped — clean shutdown.
        }
        while batch.len() < MICROBATCH_MAX {
            match receiver.try_recv() {
                Ok(event) => batch.push(event),
                Err(_) => break,
            }
        }
        match process_batch(vault, config, &batch, post_ingest) {
            Ok(outcome) => {
                if let Ok(mut state) = flush.lock() {
                    state.stats.ingested += outcome.ingested;
                    state.stats.quantized += outcome.quantized;
                    state.stats.batches += 1;
                }
            }
            Err(err) => {
                if let Ok(mut state) = flush.lock() {
                    state.error.get_or_insert(err);
                }
                return; // stop on first fault; subsequent sends fail closed.
            }
        }
    }
}

/// Quantizes, persists, and ledger-marks one microbatch.
fn process_batch<C>(
    vault: &AsterVault<C>,
    config: &QuantizeOnlineConfig,
    batch: &[StreamEvent],
    post_ingest: Option<&PostIngestHook<C>>,
) -> Result<BatchOutcome>
where
    C: Clock + Send + Sync + 'static,
{
    let mut outcome = BatchOutcome::default();
    if batch.is_empty() {
        return Ok(outcome);
    }
    for event in batch {
        let cx_id = vault.cx_id_for_input(&event.input.raw_bytes, event.input.panel_version);
        let mut input = event.input.clone();
        let mut quantized_any = false;
        for (slot_id, vector) in &event.input.slots {
            if let SlotVector::Dense { data, .. } = vector {
                let quantized = quantize_slot_online(data, config, cx_id)?;
                input.metadata.insert(
                    format!("quant_slot_{}", slot_id.0),
                    to_hex(&quantized.bytes),
                );
                outcome.quantized += 1;
                quantized_any = true;
            }
        }
        if quantized_any {
            input
                .metadata
                .insert("quantized".to_string(), "true".to_string());
        }
        let result = ingest_at(vault, &input, event.at, None)?;
        let ledger_ref = latest_ledger_ref(vault)?;
        if let Some(hook) = post_ingest {
            hook(vault, result_cx_id(&result), ledger_ref)?;
        }
        outcome.ingested += 1;
    }
    let payload = format!(
        "{{\"marker\":\"{STREAM_BATCH_MARKER}\",\"count\":{},\"quantized\":{}}}",
        outcome.ingested, outcome.quantized
    )
    .into_bytes();
    vault.append_ledger_entry(
        EntryKind::Ingest,
        SubjectId::Query(STREAM_BATCH_MARKER.as_bytes().to_vec()),
        payload,
        ActorId::Service("calyx-stream".to_string()),
    )?;
    Ok(outcome)
}

fn result_cx_id(result: &DedupResult) -> CxId {
    match result {
        DedupResult::New(cx_id) | DedupResult::ExactDuplicate(cx_id) => *cx_id,
        DedupResult::DedupMerge { into, .. } => *into,
    }
}

fn latest_ledger_ref<C>(vault: &AsterVault<C>) -> Result<LedgerRef>
where
    C: Clock + Send + Sync + 'static,
{
    let (key, value) = vault
        .scan_cf_at(vault.snapshot(), ColumnFamily::Ledger)?
        .into_iter()
        .max_by(|left, right| left.0.cmp(&right.0))
        .ok_or_else(|| stream_closed_error("ingest wrote no Ledger row"))?;
    let key_seq = ledger_seq(&key)?;
    let entry = calyx_ledger::decode(&value)?;
    if entry.seq != key_seq {
        return Err(CalyxError::ledger_corrupt(format!(
            "Ledger CF key seq {key_seq} != decoded entry seq {}",
            entry.seq
        )));
    }
    Ok(LedgerRef {
        seq: entry.seq,
        hash: entry.entry_hash,
    })
}

fn ledger_seq(key: &[u8]) -> Result<u64> {
    if key.len() != 8 {
        return Err(CalyxError::ledger_corrupt(format!(
            "ledger key length {} != 8",
            key.len()
        )));
    }
    let seq = u64::from_be_bytes(key.try_into().expect("length checked"));
    if ledger_key(seq) != key {
        return Err(CalyxError::ledger_corrupt(
            "ledger key is not canonical big-endian seq",
        ));
    }
    Ok(seq)
}

/// Lowercase hex encoding (no external dependency).
fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;
