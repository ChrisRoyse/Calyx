//! Periodic Merkle checkpoint rows for the append-only ledger.

use std::collections::BTreeMap;
use std::ops::Range;

use calyx_core::{CalyxError, Clock, Result};
use serde::{Deserialize, Serialize};

use crate::append::{LedgerAppender, LedgerCfStore, LedgerRow, PreparedLedgerEntry};
use crate::entry::{ActorId, HASH_BYTES, SubjectId};
use crate::kind::EntryKind;
use crate::merkle::{MerkleExportBundle, merkle_root};

pub const CHECKPOINT_TAG: &str = "checkpoint_v1";
pub const DEFAULT_CHECKPOINT_INTERVAL: u64 = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointConfig {
    pub interval_entries: u64,
    pub sign_key: Option<[u8; HASH_BYTES]>,
}

impl CheckpointConfig {
    pub const fn new(interval_entries: u64) -> Self {
        Self {
            interval_entries,
            sign_key: None,
        }
    }

    pub const fn with_sign_key(mut self, sign_key: [u8; HASH_BYTES]) -> Self {
        self.sign_key = Some(sign_key);
        self
    }

    fn validate(&self) -> Result<()> {
        if self.interval_entries == 0 {
            return Err(CalyxError::ledger_corrupt(
                "checkpoint interval_entries must be greater than zero",
            ));
        }
        Ok(())
    }
}

impl Default for CheckpointConfig {
    fn default() -> Self {
        Self::new(DEFAULT_CHECKPOINT_INTERVAL)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointScheduler {
    config: CheckpointConfig,
    range_start: u64,
    next_checkpoint_at: u64,
}

impl CheckpointScheduler {
    pub fn new(config: CheckpointConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            next_checkpoint_at: config.interval_entries,
            config,
            range_start: 0,
        })
    }

    pub fn recover(config: CheckpointConfig, store: &dyn LedgerCfStore) -> Result<Self> {
        let mut scheduler = Self::new(config)?;
        for row in store.scan()? {
            let Ok(entry) = crate::codec::decode(&row.bytes) else {
                continue;
            };
            if entry.kind != EntryKind::Admin {
                continue;
            }
            if let Some(payload) = CheckpointPayload::decode_optional(&entry.payload)? {
                scheduler.advance_from_payload(&payload)?;
            }
        }
        Ok(scheduler)
    }

    pub const fn config(&self) -> &CheckpointConfig {
        &self.config
    }

    pub const fn range_start(&self) -> u64 {
        self.range_start
    }

    pub const fn next_checkpoint_at(&self) -> u64 {
        self.next_checkpoint_at
    }

    pub fn should_checkpoint(&self, current_seq: u64) -> bool {
        current_seq >= self.next_checkpoint_at && self.range_start < current_seq
    }

    pub fn prepare_checkpoint_after<S, C>(
        &self,
        appender: &LedgerAppender<S, C>,
        store: &dyn LedgerCfStore,
        predecessor: &PreparedLedgerEntry,
        range_end_seq: u64,
    ) -> Result<PreparedLedgerEntry>
    where
        S: LedgerCfStore,
        C: Clock,
    {
        let range = self.range_start..range_end_seq;
        let root = merkle_root(store, range.clone())?;
        let payload = CheckpointPayload::from_root(range, root, self.config.sign_key.as_ref());
        appender.prepare_after(
            predecessor,
            EntryKind::Admin,
            checkpoint_subject(),
            payload.encode(),
            ActorId::System,
        )
    }

    pub fn advance_after_checkpoint(&mut self, range_end_seq: u64) -> Result<()> {
        self.range_start = range_end_seq.checked_add(1).ok_or_else(|| {
            CalyxError::ledger_chain_broken("checkpoint range end exhausted sequence space")
        })?;
        self.next_checkpoint_at = self
            .range_start
            .saturating_add(self.config.interval_entries);
        Ok(())
    }

    fn advance_from_payload(&mut self, payload: &CheckpointPayload) -> Result<()> {
        payload.root_bytes()?;
        self.advance_after_checkpoint(payload.range_end)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointPayload {
    pub tag: String,
    pub range_start: u64,
    pub range_end: u64,
    pub root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signer_pubkey: Option<String>,
}

impl CheckpointPayload {
    pub fn from_root(
        range: Range<u64>,
        root: [u8; HASH_BYTES],
        sign_key: Option<&[u8; HASH_BYTES]>,
    ) -> Self {
        let bundle = match sign_key {
            Some(key) => MerkleExportBundle::signed(range.clone(), root, key),
            None => MerkleExportBundle::unsigned(range.clone(), root),
        };
        Self {
            tag: CHECKPOINT_TAG.to_string(),
            range_start: range.start,
            range_end: range.end,
            root: hex(&bundle.root),
            signature: bundle.signature.map(|bytes| hex(&bytes)),
            signer_pubkey: bundle.signer_pubkey.map(|bytes| hex(&bytes)),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("checkpoint payload serializes")
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let payload: Self = serde_json::from_slice(bytes).map_err(|error| {
            CalyxError::ledger_corrupt(format!("decode checkpoint payload: {error}"))
        })?;
        if payload.tag != CHECKPOINT_TAG {
            return Err(CalyxError::ledger_corrupt(format!(
                "unknown checkpoint payload tag {}",
                payload.tag
            )));
        }
        if payload.range_start > payload.range_end {
            return Err(CalyxError::ledger_corrupt(format!(
                "invalid checkpoint range {}..{}",
                payload.range_start, payload.range_end
            )));
        }
        payload.root_bytes()?;
        validate_optional_hex(&payload.signature, 64, "signature")?;
        validate_optional_hex(&payload.signer_pubkey, HASH_BYTES, "signer_pubkey")?;
        Ok(payload)
    }

    pub fn decode_optional(bytes: &[u8]) -> Result<Option<Self>> {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(bytes) else {
            return Ok(None);
        };
        if value.get("tag").and_then(|tag| tag.as_str()) != Some(CHECKPOINT_TAG) {
            return Ok(None);
        }
        let bytes = serde_json::to_vec(&value).expect("serde_json value serializes");
        Self::decode(&bytes).map(Some)
    }

    pub fn root_bytes(&self) -> Result<[u8; HASH_BYTES]> {
        parse_hex_array::<HASH_BYTES>(&self.root, "root")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayLedgerStore {
    rows: BTreeMap<u64, Vec<u8>>,
}

impl OverlayLedgerStore {
    pub fn new(
        base: &dyn LedgerCfStore,
        pending: impl IntoIterator<Item = PreparedLedgerEntry>,
    ) -> Result<Self> {
        let mut rows = BTreeMap::new();
        for row in base.scan()? {
            insert_unique(&mut rows, row.seq, row.bytes)?;
        }
        for prepared in pending {
            insert_unique(&mut rows, prepared.seq(), prepared.bytes().to_vec())?;
        }
        Ok(Self { rows })
    }
}

impl LedgerCfStore for OverlayLedgerStore {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        Ok(self
            .rows
            .iter()
            .map(|(seq, bytes)| LedgerRow {
                seq: *seq,
                bytes: bytes.clone(),
            })
            .collect())
    }

    fn put_new(&mut self, seq: u64, _bytes: &[u8]) -> Result<()> {
        Err(CalyxError::ledger_append_only_violation(format!(
            "checkpoint overlay store is read-only for seq {seq}"
        )))
    }
}

fn checkpoint_subject() -> SubjectId {
    SubjectId::Query(CHECKPOINT_TAG.as_bytes().to_vec())
}

fn insert_unique(rows: &mut BTreeMap<u64, Vec<u8>>, seq: u64, bytes: Vec<u8>) -> Result<()> {
    if let Some(existing) = rows.get(&seq) {
        if existing == &bytes {
            return Ok(());
        }
        return Err(CalyxError::ledger_corrupt(format!(
            "divergent ledger bytes for seq {seq}"
        )));
    }
    rows.insert(seq, bytes);
    Ok(())
}

fn validate_optional_hex(value: &Option<String>, bytes: usize, label: &str) -> Result<()> {
    if let Some(value) = value {
        parse_hex_array_dyn(value, bytes, label)?;
    }
    Ok(())
}

fn parse_hex_array<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    let bytes = parse_hex_array_dyn(value, N, label)?;
    Ok(bytes.try_into().expect("length checked"))
}

fn parse_hex_array_dyn(value: &str, bytes: usize, label: &str) -> Result<Vec<u8>> {
    if value.len() != bytes * 2 {
        return Err(CalyxError::ledger_corrupt(format!(
            "{label} hex has {} chars, expected {}",
            value.len(),
            bytes * 2
        )));
    }
    let mut out = Vec::with_capacity(bytes);
    for chunk in value.as_bytes().chunks_exact(2) {
        out.push(parse_hex_byte(chunk, label)?);
    }
    Ok(out)
}

fn parse_hex_byte(bytes: &[u8], label: &str) -> Result<u8> {
    let hi = hex_value(bytes[0])
        .ok_or_else(|| CalyxError::ledger_corrupt(format!("{label} hex contains non-hex digit")))?;
    let lo = hex_value(bytes[1])
        .ok_or_else(|| CalyxError::ledger_corrupt(format!("{label} hex contains non-hex digit")))?;
    Ok((hi << 4) | lo)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn hex(bytes: &[u8]) -> String {
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
