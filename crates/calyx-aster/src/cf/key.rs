//! Big-endian key codecs for Aster column families.

use calyx_core::{AnchorKind, CalyxError, CxId, Result, SlotId};

const CX_ID_BYTES: usize = 16;
const FULL_HASH_BYTES: usize = 32;

/// Materialized cross-term kind in the `xterm` CF.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum XTermKind {
    Concat,
    Interaction,
    Agreement,
    Delta,
}

impl XTermKind {
    const fn code(self) -> u8 {
        match self {
            Self::Concat => 0,
            Self::Interaction => 1,
            Self::Agreement => 2,
            Self::Delta => 3,
        }
    }
}

/// Scalar column id for the `(ScalarId, CxId)` `scalars` CF key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ScalarId(u32);

impl ScalarId {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Typed keyspace inside the `online` CF.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OnlineKeyKind {
    MistakeLog,
    ReplayBuffer,
    HeadState,
    DeltaJQueue,
}

impl OnlineKeyKind {
    const fn code(self) -> u8 {
        match self {
            Self::MistakeLog => 0,
            Self::ReplayBuffer => 1,
            Self::HeadState => 2,
            Self::DeltaJQueue => 3,
        }
    }
}

/// Lexicographic key range. `end == None` means unbounded upper range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRange {
    pub start: Vec<u8>,
    pub end: Option<Vec<u8>>,
}

impl KeyRange {
    pub fn contains(&self, key: &[u8]) -> bool {
        key >= self.start.as_slice() && self.end.as_ref().is_none_or(|end| key < end.as_slice())
    }
}

/// `base` CF key: `CxId`.
pub fn base_key(cx_id: CxId) -> Vec<u8> {
    cx_id.as_bytes().to_vec()
}

/// `slot_*` and `slot_*.raw` CF key: `CxId`.
pub fn slot_key(cx_id: CxId) -> Vec<u8> {
    base_key(cx_id)
}

/// `xterm` CF key: `(CxId, SlotId_a, SlotId_b, XTermKind)`.
pub fn xterm_key(cx_id: CxId, a: SlotId, b: SlotId, kind: XTermKind) -> Vec<u8> {
    let mut key = Vec::with_capacity(CX_ID_BYTES + 5);
    key.extend_from_slice(cx_id.as_bytes());
    key.extend_from_slice(&a.get().to_be_bytes());
    key.extend_from_slice(&b.get().to_be_bytes());
    key.push(kind.code());
    key
}

/// `temporal_xterm` CF key: `(CxId_a, CxId_b)`.
pub fn temporal_xterm_key(cx_a: CxId, cx_b: CxId) -> Vec<u8> {
    let mut key = Vec::with_capacity(CX_ID_BYTES * 2);
    key.extend_from_slice(cx_a.as_bytes());
    key.extend_from_slice(cx_b.as_bytes());
    key
}

/// `scalars` CF key: `(ScalarId, CxId)`.
pub fn scalar_key(scalar: ScalarId, cx_id: CxId) -> Vec<u8> {
    let mut key = Vec::with_capacity(4 + CX_ID_BYTES);
    key.extend_from_slice(&scalar.get().to_be_bytes());
    key.extend_from_slice(cx_id.as_bytes());
    key
}

/// `anchors` CF key: `(CxId, AnchorKind)`.
pub fn anchor_key(cx_id: CxId, kind: &AnchorKind) -> Vec<u8> {
    let mut key = Vec::with_capacity(CX_ID_BYTES + 16);
    key.extend_from_slice(cx_id.as_bytes());
    encode_anchor_kind(kind, &mut key);
    key
}

/// `ledger` CF key: `seq`.
pub fn ledger_key(seq: u64) -> Vec<u8> {
    seq.to_be_bytes().to_vec()
}

/// `recurrence` CF key: `(CxId, OccurrenceId)`.
pub fn recurrence_key(cx_id: CxId, occurrence_id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(CX_ID_BYTES + 8);
    key.extend_from_slice(cx_id.as_bytes());
    key.extend_from_slice(&occurrence_id.to_be_bytes());
    key
}

/// `online` CF key: `(OnlineKeyKind, seq_or_id)`.
pub fn online_key(kind: OnlineKeyKind, seq_or_id: u64) -> Vec<u8> {
    let mut key = Vec::with_capacity(9);
    key.push(kind.code());
    key.extend_from_slice(&seq_or_id.to_be_bytes());
    key
}

/// Prefix range for all rows under one `CxId` in CFs whose keys start with it.
pub fn cx_prefix_range(cx_id: CxId) -> KeyRange {
    prefix_range(cx_id.as_bytes())
}

/// Prefix range for all xterms under one `CxId`.
pub fn xterm_prefix_range(cx_id: CxId) -> KeyRange {
    cx_prefix_range(cx_id)
}

/// Prefix range for all temporal cross-terms where `cx_id` is the left side.
pub fn temporal_xterm_prefix_range(cx_id: CxId) -> KeyRange {
    cx_prefix_range(cx_id)
}

/// Prefix range for all anchors under one `CxId`.
pub fn anchor_prefix_range(cx_id: CxId) -> KeyRange {
    cx_prefix_range(cx_id)
}

/// Prefix range for all scalar rows under one scalar id.
pub fn scalar_prefix_range(scalar: ScalarId) -> KeyRange {
    prefix_range(&scalar.get().to_be_bytes())
}

/// Big-endian ledger range `[start_seq, end_seq)`.
pub fn ledger_range(start_seq: u64, end_seq: u64) -> KeyRange {
    KeyRange {
        start: ledger_key(start_seq),
        end: Some(ledger_key(end_seq)),
    }
}

/// Prefix range for all recurrence rows under one `CxId`.
pub fn recurrence_prefix_range(cx_id: CxId) -> KeyRange {
    cx_prefix_range(cx_id)
}

/// Builds a lexicographic range that contains all keys starting with `prefix`.
pub fn prefix_range(prefix: &[u8]) -> KeyRange {
    KeyRange {
        start: prefix.to_vec(),
        end: prefix_upper_bound(prefix),
    }
}

/// Full 32-byte BLAKE3 content hash using Calyx length-delimited parts.
pub fn full_content_hash<I, P>(parts: I) -> [u8; FULL_HASH_BYTES]
where
    I: IntoIterator<Item = P>,
    P: AsRef<[u8]>,
{
    let mut hasher = blake3::Hasher::new();
    for part in parts {
        let part = part.as_ref();
        hasher.update(&(part.len() as u64).to_be_bytes());
        hasher.update(part);
    }
    *hasher.finalize().as_bytes()
}

/// Returns the PRD 16-byte `CxId` prefix for a full BLAKE3 content hash.
pub fn cx_id_from_full_hash(full_hash: &[u8; FULL_HASH_BYTES]) -> CxId {
    let mut prefix = [0_u8; CX_ID_BYTES];
    prefix.copy_from_slice(&full_hash[..CX_ID_BYTES]);
    CxId::from_bytes(prefix)
}

/// Verifies a stored `CxId` still matches the full content hash path.
pub fn verify_cx_hash_prefix(cx_id: CxId, full_hash: &[u8; FULL_HASH_BYTES]) -> Result<()> {
    if cx_id.as_bytes()[..] == full_hash[..CX_ID_BYTES] {
        return Ok(());
    }
    Err(CalyxError::aster_corrupt_shard(
        "CxId prefix does not match full content hash",
    ))
}

fn prefix_upper_bound(prefix: &[u8]) -> Option<Vec<u8>> {
    let mut end = prefix.to_vec();
    for index in (0..end.len()).rev() {
        if end[index] != 0xff {
            end[index] += 1;
            end.truncate(index + 1);
            return Some(end);
        }
    }
    None
}

fn encode_anchor_kind(kind: &AnchorKind, out: &mut Vec<u8>) {
    match kind {
        AnchorKind::TestPass => out.extend_from_slice(&0_u16.to_be_bytes()),
        AnchorKind::TieFormed => out.extend_from_slice(&1_u16.to_be_bytes()),
        AnchorKind::Thumbs => out.extend_from_slice(&2_u16.to_be_bytes()),
        AnchorKind::Reward => out.extend_from_slice(&3_u16.to_be_bytes()),
        AnchorKind::SpeakerMatch => out.extend_from_slice(&4_u16.to_be_bytes()),
        AnchorKind::StyleHold => out.extend_from_slice(&5_u16.to_be_bytes()),
        AnchorKind::Recurrence => out.extend_from_slice(&6_u16.to_be_bytes()),
        AnchorKind::Label(value) => {
            out.extend_from_slice(&7_u16.to_be_bytes());
            let bytes = value.as_bytes();
            out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
            out.extend_from_slice(bytes);
        }
    }
}
