use std::ops::Range;

use calyx_core::{CalyxError, Result};

use crate::append::LedgerRow;
use crate::codec::decode_unchecked;
use crate::entry::{HASH_BYTES, LedgerEntry, compute_entry_hash};
use crate::head_anchor::LedgerHeadAnchor;
use crate::verify::VerifyResult;

#[derive(Debug)]
pub enum StreamingStart {
    Ready(StreamingChainVerifier),
    Complete(VerifyResult),
}

#[derive(Debug)]
pub struct StreamingChainVerifier {
    range: Range<u64>,
    next_seq: u64,
    expected_prev: [u8; HASH_BYTES],
    count: u64,
    anchor: Option<LedgerHeadAnchor>,
}

impl StreamingChainVerifier {
    pub fn start(
        range: Range<u64>,
        anchor: Option<LedgerHeadAnchor>,
        previous: Option<&LedgerRow>,
    ) -> Result<StreamingStart> {
        if range.start > range.end {
            return Err(CalyxError::ledger_corrupt(format!(
                "invalid ledger range {}..{}",
                range.start, range.end
            )));
        }
        if range.start == range.end {
            return Ok(StreamingStart::Complete(VerifyResult::Intact { count: 0 }));
        }
        if range.start == 0
            && let Some(anchor) = &anchor
            && range.end != anchor.height
        {
            return Ok(StreamingStart::Complete(corrupt_result(
                range.end.min(anchor.height),
                format!(
                    "ledger head anchor mismatch: requested head {}, anchored head {}",
                    range.end, anchor.height
                ),
            )));
        }
        let expected_prev = expected_prev_hash(range.start, previous)?;
        Ok(StreamingStart::Ready(Self {
            next_seq: range.start,
            range,
            expected_prev,
            count: 0,
            anchor,
        }))
    }

    pub const fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub const fn end(&self) -> u64 {
        self.range.end
    }

    pub const fn count(&self) -> u64 {
        self.count
    }

    pub fn verify_next(&mut self, row: Option<LedgerRow>) -> Result<Option<VerifyResult>> {
        let seq = self.next_seq;
        let Some(row) = row else {
            return Ok(Some(corrupt_result(
                seq,
                format!("missing ledger row for seq {seq}"),
            )));
        };
        let entry = match decode_unchecked(&row.bytes) {
            Ok(entry) => entry,
            Err(error) => {
                return Ok(Some(corrupt_result(
                    seq,
                    format!("decode ledger row seq {seq}: {error}"),
                )));
            }
        };
        if row.seq != seq || entry.seq != seq {
            return Ok(Some(corrupt_result(
                seq,
                format!("ledger key seq {seq} != encoded seq {}", entry.seq),
            )));
        }
        if entry.prev_hash != self.expected_prev {
            return Ok(Some(VerifyResult::Broken {
                at_seq: seq,
                expected: self.expected_prev,
                found: entry.prev_hash,
            }));
        }
        let expected_entry_hash = recompute_hash(&entry);
        if entry.entry_hash != expected_entry_hash {
            return Ok(Some(VerifyResult::Broken {
                at_seq: seq,
                expected: expected_entry_hash,
                found: entry.entry_hash,
            }));
        }
        self.expected_prev = entry.entry_hash;
        self.count += 1;
        self.next_seq += 1;
        Ok((self.next_seq == self.range.end).then(|| self.finish()))
    }

    fn finish(&self) -> VerifyResult {
        if self.range.start == 0
            && let Some(anchor) = &self.anchor
            && self.range.end == anchor.height
            && self.expected_prev != anchor.tip_hash
        {
            return VerifyResult::Broken {
                at_seq: self.range.end.saturating_sub(1),
                expected: anchor.tip_hash,
                found: self.expected_prev,
            };
        }
        VerifyResult::Intact { count: self.count }
    }
}

fn expected_prev_hash(start: u64, previous: Option<&LedgerRow>) -> Result<[u8; HASH_BYTES]> {
    if start == 0 {
        return Ok([0; HASH_BYTES]);
    }
    let previous_seq = start - 1;
    let Some(row) = previous else {
        return Err(CalyxError::ledger_corrupt(format!(
            "missing ledger row for previous seq {previous_seq}"
        )));
    };
    let entry = decode_unchecked(&row.bytes).map_err(|error| {
        CalyxError::ledger_corrupt(format!(
            "cannot verify range start {start}: previous seq {previous_seq}: {error}"
        ))
    })?;
    if row.seq != previous_seq || entry.seq != previous_seq {
        return Err(CalyxError::ledger_corrupt(format!(
            "previous key seq {previous_seq} != encoded seq {}",
            entry.seq
        )));
    }
    if !entry.verify() {
        return Err(CalyxError::ledger_corrupt(format!(
            "cannot verify range start {start}: previous seq {previous_seq} is broken"
        )));
    }
    Ok(entry.entry_hash)
}

fn recompute_hash(entry: &LedgerEntry) -> [u8; HASH_BYTES] {
    compute_entry_hash(
        entry.seq,
        &entry.prev_hash,
        entry.kind,
        &entry.subject,
        &entry.payload,
        &entry.actor,
        entry.ts,
    )
}

fn corrupt_result(at_seq: u64, reason: impl Into<String>) -> VerifyResult {
    VerifyResult::Corrupt {
        at_seq,
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ActorId, EntryKind, LedgerEntry, SubjectId, encode};

    #[test]
    fn streaming_verifier_reports_intact_rows() {
        let rows = rows(0, 3, [0; HASH_BYTES]);
        let mut verifier = match StreamingChainVerifier::start(0..3, None, None).unwrap() {
            StreamingStart::Ready(verifier) => verifier,
            StreamingStart::Complete(_) => panic!("unexpected complete start"),
        };

        let mut result = None;
        for row in rows {
            result = verifier.verify_next(Some(row)).unwrap();
        }

        assert_eq!(result, Some(VerifyResult::Intact { count: 3 }));
    }

    #[test]
    fn streaming_verifier_uses_previous_row_for_offset_range() {
        let rows = rows(0, 4, [0; HASH_BYTES]);
        let previous = rows[1].clone();
        let mut verifier = match StreamingChainVerifier::start(2..4, None, Some(&previous)).unwrap()
        {
            StreamingStart::Ready(verifier) => verifier,
            StreamingStart::Complete(_) => panic!("unexpected complete start"),
        };

        assert_eq!(verifier.verify_next(Some(rows[2].clone())).unwrap(), None);
        assert_eq!(
            verifier.verify_next(Some(rows[3].clone())).unwrap(),
            Some(VerifyResult::Intact { count: 2 })
        );
    }

    #[test]
    fn streaming_verifier_fails_missing_row_closed() {
        let mut verifier = match StreamingChainVerifier::start(0..1, None, None).unwrap() {
            StreamingStart::Ready(verifier) => verifier,
            StreamingStart::Complete(_) => panic!("unexpected complete start"),
        };

        assert!(matches!(
            verifier.verify_next(None).unwrap(),
            Some(VerifyResult::Corrupt { at_seq: 0, .. })
        ));
    }

    fn rows(start: u64, count: u64, mut previous: [u8; HASH_BYTES]) -> Vec<LedgerRow> {
        let mut out = Vec::new();
        for seq in start..start + count {
            let entry = LedgerEntry::new(
                seq,
                previous,
                EntryKind::Ingest,
                SubjectId::Query(vec![seq as u8]),
                Vec::new(),
                ActorId::System,
                seq,
            );
            previous = entry.entry_hash;
            out.push(LedgerRow {
                seq,
                bytes: encode(&entry),
            });
        }
        out
    }
}
