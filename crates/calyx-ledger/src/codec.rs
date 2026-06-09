//! Deterministic binary codec for ledger entries.

use calyx_core::{CalyxError, CxId, LensId, Result};

use crate::entry::{
    ActorId, HASH_BYTES, LedgerEntry, SubjectId, TAG_AGENT, TAG_CX, TAG_GUARD, TAG_KERNEL,
    TAG_LENS, TAG_QUERY, TAG_SERVICE, TAG_SYSTEM,
};
use crate::kind::EntryKind;

const SEQ_LEN: usize = 8;
const KIND_OFFSET: usize = SEQ_LEN + HASH_BYTES;
const HEADER_LEN: usize = KIND_OFFSET;

/// Encodes a ledger entry with a stable, padding-free binary layout.
pub fn encode(entry: &LedgerEntry) -> Vec<u8> {
    let subject = entry.subject.wire_bytes();
    let actor = entry.actor.wire_bytes();
    assert!(subject.len() <= u16::MAX as usize, "subject id too long");
    assert!(actor.len() <= u16::MAX as usize, "actor id too long");
    assert!(entry.payload.len() <= u32::MAX as usize, "payload too long");

    let mut out = Vec::with_capacity(encoded_len(entry, subject.len(), actor.len()));
    out.extend_from_slice(&entry.seq.to_be_bytes());
    out.extend_from_slice(&entry.prev_hash);
    out.push(entry.kind.wire_code());
    out.push(entry.subject.wire_tag());
    out.extend_from_slice(&(subject.len() as u16).to_be_bytes());
    out.extend_from_slice(&subject);
    out.extend_from_slice(&(entry.payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&entry.payload);
    out.push(entry.actor.wire_tag());
    out.extend_from_slice(&(actor.len() as u16).to_be_bytes());
    out.extend_from_slice(actor);
    out.extend_from_slice(&entry.ts.to_be_bytes());
    out.extend_from_slice(&entry.entry_hash);
    out
}

/// Decodes a ledger entry and verifies its embedded hash.
pub fn decode(bytes: &[u8]) -> Result<LedgerEntry> {
    let entry = decode_unchecked(bytes)?;
    if entry.verify() {
        Ok(entry)
    } else {
        Err(corrupt(format!(
            "ledger entry seq {} hash mismatch",
            entry.seq
        )))
    }
}

pub(crate) fn decode_unchecked(bytes: &[u8]) -> Result<LedgerEntry> {
    let mut cursor = Cursor::new(bytes);
    let seq = cursor.u64("seq")?;
    let prev_hash = cursor.hash("prev_hash")?;
    let kind_code = cursor.u8("kind")?;
    let kind = EntryKind::from_wire_code(kind_code)
        .ok_or_else(|| corrupt(format!("invalid ledger kind code {kind_code}")))?;
    let subject_tag = cursor.u8("subject_tag")?;
    let subject_len = cursor.u16("subject_len")? as usize;
    let subject_bytes = cursor.bytes(subject_len, "subject_bytes")?;
    let subject = decode_subject(subject_tag, subject_bytes)?;
    let payload_len = cursor.u32("payload_len")? as usize;
    let payload = cursor.bytes(payload_len, "payload")?.to_vec();
    let actor_tag = cursor.u8("actor_tag")?;
    let actor_len = cursor.u16("actor_len")? as usize;
    let actor_bytes = cursor.bytes(actor_len, "actor_bytes")?;
    let actor = decode_actor(actor_tag, actor_bytes)?;
    let ts = cursor.u64("ts")?;
    let entry_hash = cursor.hash("entry_hash")?;
    cursor.finish()?;

    let entry = LedgerEntry {
        seq,
        prev_hash,
        kind,
        subject,
        payload,
        actor,
        ts,
        entry_hash,
    };
    Ok(entry)
}

/// Decodes only `seq` and `prev_hash` for fast chain-link checks.
pub fn decode_header(bytes: &[u8]) -> Result<(u64, [u8; HASH_BYTES])> {
    if bytes.len() < HEADER_LEN {
        return Err(corrupt(format!(
            "ledger header requires {HEADER_LEN} bytes, got {}",
            bytes.len()
        )));
    }
    let seq = u64::from_be_bytes(bytes[0..SEQ_LEN].try_into().expect("seq length checked"));
    let prev_hash = bytes[SEQ_LEN..HEADER_LEN]
        .try_into()
        .expect("hash length checked");
    Ok((seq, prev_hash))
}

fn encoded_len(entry: &LedgerEntry, subject_len: usize, actor_len: usize) -> usize {
    SEQ_LEN
        + HASH_BYTES
        + 1
        + 1
        + 2
        + subject_len
        + 4
        + entry.payload.len()
        + 1
        + 2
        + actor_len
        + 8
        + HASH_BYTES
}

fn decode_subject(tag: u8, bytes: &[u8]) -> Result<SubjectId> {
    match tag {
        TAG_CX => Ok(SubjectId::Cx(CxId::from_bytes(copy_16(bytes, "cx")?))),
        TAG_LENS => Ok(SubjectId::Lens(LensId::from_bytes(copy_16(bytes, "lens")?))),
        TAG_KERNEL => Ok(SubjectId::Kernel(bytes.to_vec())),
        TAG_GUARD => Ok(SubjectId::Guard(bytes.to_vec())),
        TAG_QUERY => Ok(SubjectId::Query(bytes.to_vec())),
        _ => Err(corrupt(format!("invalid subject tag {tag}"))),
    }
}

fn decode_actor(tag: u8, bytes: &[u8]) -> Result<ActorId> {
    let value = String::from_utf8(bytes.to_vec())
        .map_err(|_| corrupt(format!("actor tag {tag} is not utf8")))?;
    match tag {
        TAG_AGENT => Ok(ActorId::Agent(value)),
        TAG_SERVICE => Ok(ActorId::Service(value)),
        TAG_SYSTEM if bytes.is_empty() => Ok(ActorId::System),
        TAG_SYSTEM => Err(corrupt("system actor must have zero actor bytes")),
        _ => Err(corrupt(format!("invalid actor tag {tag}"))),
    }
}

fn copy_16(bytes: &[u8], label: &str) -> Result<[u8; 16]> {
    bytes.try_into().map_err(|_| {
        corrupt(format!(
            "{label} subject requires 16 bytes, got {}",
            bytes.len()
        ))
    })
}

fn corrupt(message: impl Into<String>) -> CalyxError {
    CalyxError::ledger_corrupt(message)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn u8(&mut self, label: &str) -> Result<u8> {
        let bytes = self.bytes(1, label)?;
        Ok(bytes[0])
    }

    fn u16(&mut self, label: &str) -> Result<u16> {
        let bytes = self.bytes(2, label)?;
        Ok(u16::from_be_bytes(
            bytes.try_into().expect("u16 length checked"),
        ))
    }

    fn u32(&mut self, label: &str) -> Result<u32> {
        let bytes = self.bytes(4, label)?;
        Ok(u32::from_be_bytes(
            bytes.try_into().expect("u32 length checked"),
        ))
    }

    fn u64(&mut self, label: &str) -> Result<u64> {
        let bytes = self.bytes(8, label)?;
        Ok(u64::from_be_bytes(
            bytes.try_into().expect("u64 length checked"),
        ))
    }

    fn hash(&mut self, label: &str) -> Result<[u8; HASH_BYTES]> {
        let bytes = self.bytes(HASH_BYTES, label)?;
        Ok(bytes.try_into().expect("hash length checked"))
    }

    fn bytes(&mut self, len: usize, label: &str) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| corrupt(format!("{label} length overflows")))?;
        if end > self.bytes.len() {
            return Err(corrupt(format!(
                "{label} extends past buffer at {}..{} of {}",
                self.offset,
                end,
                self.bytes.len()
            )));
        }
        let out = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(out)
    }

    fn finish(&self) -> Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(corrupt(format!(
                "ledger entry has {} trailing bytes",
                self.bytes.len() - self.offset
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const CODEC_GOLDEN_HEX: &str = "000000000000002a111111111111111111111111111111111111111111111111111111111111111101000010222222222222222222222222222222220000000973796e74686574696301000373766300000000000000636f079f27c0fc4e990e831f3ea196c09342ed5a220fc705d9a0db0317a6dd636e";

    fn codec_entry() -> LedgerEntry {
        LedgerEntry::new(
            42,
            [0x11; HASH_BYTES],
            EntryKind::Measure,
            SubjectId::Cx(CxId::from_bytes([0x22; 16])),
            b"synthetic".to_vec(),
            ActorId::Service("svc".to_string()),
            99,
        )
    }

    fn kind_strategy() -> impl Strategy<Value = EntryKind> {
        prop_oneof![
            Just(EntryKind::Ingest),
            Just(EntryKind::Measure),
            Just(EntryKind::Assay),
            Just(EntryKind::Kernel),
            Just(EntryKind::Guard),
            Just(EntryKind::Answer),
            Just(EntryKind::Anneal),
            Just(EntryKind::Migrate),
            Just(EntryKind::Admin),
            Just(EntryKind::Erase),
        ]
    }

    fn subject_strategy() -> impl Strategy<Value = SubjectId> {
        prop_oneof![
            any::<[u8; 16]>().prop_map(|bytes| SubjectId::Cx(CxId::from_bytes(bytes))),
            any::<[u8; 16]>().prop_map(|bytes| SubjectId::Lens(LensId::from_bytes(bytes))),
            prop::collection::vec(any::<u8>(), 0..255).prop_map(SubjectId::Kernel),
            prop::collection::vec(any::<u8>(), 0..255).prop_map(SubjectId::Guard),
            prop::collection::vec(any::<u8>(), 0..255).prop_map(SubjectId::Query),
        ]
    }

    fn actor_strategy() -> impl Strategy<Value = ActorId> {
        prop_oneof![
            Just(ActorId::System),
            "[a-z0-9_-]{0,32}".prop_map(ActorId::Agent),
            "[a-z0-9_-]{0,32}".prop_map(ActorId::Service),
        ]
    }

    #[test]
    fn codec_golden() {
        let entry = codec_entry();
        let bytes = encode(&entry);
        println!("CODEC_GOLDEN_HEX {}", hex(&bytes));
        assert_eq!(&bytes[0..8], &42_u64.to_be_bytes());
        assert_eq!(&bytes[8..40], &[0x11; HASH_BYTES]);
        assert_eq!(bytes[40], EntryKind::Measure.wire_code());
        assert_eq!(decode_header(&bytes).unwrap(), (42, [0x11; HASH_BYTES]));
        assert_eq!(decode(&bytes).unwrap(), entry);
        assert_eq!(hex(&bytes), CODEC_GOLDEN_HEX);
    }

    #[test]
    fn codec_edges_roundtrip() {
        let entries = [
            LedgerEntry::new(
                0,
                [0; HASH_BYTES],
                EntryKind::Ingest,
                SubjectId::Query(vec![0x55; 255]),
                Vec::new(),
                ActorId::Agent("a".to_string()),
                0,
            ),
            LedgerEntry::new(
                7,
                [1; HASH_BYTES],
                EntryKind::Admin,
                SubjectId::Kernel(Vec::new()),
                vec![0],
                ActorId::Service(String::new()),
                u64::MAX,
            ),
            LedgerEntry::new(
                8,
                [2; HASH_BYTES],
                EntryKind::Admin,
                SubjectId::Guard(Vec::new()),
                vec![1],
                ActorId::System,
                1,
            ),
        ];
        for entry in entries {
            assert_eq!(decode(&encode(&entry)).unwrap(), entry);
        }
    }

    #[test]
    fn codec_fail_closed_on_bad_bytes() {
        let entry = codec_entry();
        let bytes = encode(&entry);
        let truncated = &bytes[..bytes.len() - 1];
        assert_eq!(decode(truncated).unwrap_err().code, "CALYX_LEDGER_CORRUPT");
        assert_eq!(decode(&[]).unwrap_err().code, "CALYX_LEDGER_CORRUPT");

        let mut corrupted = bytes;
        let last = corrupted.len() - 1;
        corrupted[last] ^= 0x01;
        assert_eq!(decode(&corrupted).unwrap_err().code, "CALYX_LEDGER_CORRUPT");
    }

    proptest! {
        #[test]
        fn codec_roundtrips(
            seq in any::<u64>(),
            prev_hash in any::<[u8; HASH_BYTES]>(),
            kind in kind_strategy(),
            subject in subject_strategy(),
            payload in prop::collection::vec(any::<u8>(), 0..255),
            actor in actor_strategy(),
            ts in any::<u64>(),
        ) {
            let entry = LedgerEntry::new(seq, prev_hash, kind, subject, payload, actor, ts);
            prop_assert_eq!(decode(&encode(&entry)).unwrap(), entry);
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
