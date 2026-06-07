use calyx_core::{CalyxError, Result};

const MAGIC: &[u8; 4] = b"CXA1";
const VERSION: u32 = 1;
const HEADER_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq)]
pub struct ArrowChunkView<'a> {
    raw: &'a [u8],
    rows: Vec<f32>,
    n_rows: usize,
    dim: usize,
}

impl<'a> ArrowChunkView<'a> {
    pub fn row(&self, index: usize) -> Result<&[f32]> {
        if index >= self.n_rows {
            return Err(CalyxError::aster_corrupt_shard(
                "arrow row index out of bounds",
            ));
        }
        let start = index * self.dim;
        Ok(&self.rows[start..start + self.dim])
    }

    pub const fn n_rows(&self) -> usize {
        self.n_rows
    }

    pub const fn dim(&self) -> usize {
        self.dim
    }

    pub const fn raw_bytes(&self) -> &'a [u8] {
        self.raw
    }
}

pub fn encode_column_chunk(rows: &[&[f32]]) -> Result<Vec<u8>> {
    let dim = rows
        .first()
        .ok_or_else(|| CalyxError::aster_corrupt_shard("arrow chunk has no rows"))?
        .len();
    if dim == 0 {
        return Err(CalyxError::aster_corrupt_shard(
            "arrow chunk dim must be > 0",
        ));
    }
    if rows.iter().any(|row| row.len() != dim) {
        return Err(CalyxError::aster_corrupt_shard(
            "arrow chunk row dims differ",
        ));
    }
    let mut out = Vec::with_capacity(HEADER_LEN + rows.len() * dim * 4);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    out.extend_from_slice(&(dim as u32).to_le_bytes());
    for row in rows {
        for value in *row {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    Ok(out)
}

pub fn decode_column_chunk(bytes: &[u8]) -> Result<ArrowChunkView<'_>> {
    if bytes.len() < HEADER_LEN {
        return Err(CalyxError::aster_corrupt_shard(
            "arrow chunk header missing",
        ));
    }
    if &bytes[0..4] != MAGIC {
        return Err(CalyxError::aster_corrupt_shard(
            "arrow chunk magic mismatch",
        ));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().expect("version"));
    if version != VERSION {
        return Err(CalyxError::aster_corrupt_shard(
            "unsupported arrow chunk version",
        ));
    }
    let n_rows = u32::from_le_bytes(bytes[8..12].try_into().expect("rows")) as usize;
    let dim = u32::from_le_bytes(bytes[12..16].try_into().expect("dim")) as usize;
    if n_rows == 0 || dim == 0 {
        return Err(CalyxError::aster_corrupt_shard(
            "arrow chunk shape must be non-zero",
        ));
    }
    let expected = HEADER_LEN + n_rows * dim * 4;
    if bytes.len() != expected {
        return Err(CalyxError::aster_corrupt_shard(
            "arrow chunk byte length mismatch",
        ));
    }
    let mut rows = Vec::with_capacity(n_rows * dim);
    for chunk in bytes[HEADER_LEN..].chunks_exact(4) {
        rows.push(f32::from_le_bytes(chunk.try_into().expect("f32")));
    }
    Ok(ArrowChunkView {
        raw: bytes,
        rows,
        n_rows,
        dim,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn known_chunk_roundtrips_and_exposes_magic() {
        let rows = [vec![1.0, 2.0, 3.5, 4.25], vec![5.0, 6.0, 7.0, 8.0]];
        let refs: Vec<_> = rows.iter().map(Vec::as_slice).collect();
        let bytes = encode_column_chunk(&refs).expect("encode");
        let decoded = decode_column_chunk(&bytes).expect("decode");

        assert_eq!(&bytes[0..4], b"CXA1");
        assert_eq!(&bytes[4..8], &1_u32.to_le_bytes());
        assert_eq!(decoded.n_rows(), 2);
        assert_eq!(decoded.dim(), 4);
        assert_eq!(decoded.row(0).unwrap(), rows[0].as_slice());
        assert_eq!(decoded.raw_bytes(), bytes.as_slice());
    }

    #[test]
    fn fail_closed_edges() {
        assert!(encode_column_chunk(&[]).is_err());
        assert!(encode_column_chunk(&[&[]]).is_err());
        assert!(encode_column_chunk(&[&[1.0][..], &[1.0, 2.0][..]]).is_err());
        assert!(decode_column_chunk(b"").is_err());
        let mut bad = encode_column_chunk(&[&[1.0][..]]).unwrap();
        bad[0] = 0;
        assert!(decode_column_chunk(&bad).is_err());
        let truncated = &bad[..bad.len() - 1];
        assert!(decode_column_chunk(truncated).is_err());
    }

    proptest! {
        #[test]
        fn chunks_roundtrip_bit_exact(n in 1usize..16, dim in 1usize..32, values in proptest::collection::vec(any::<u32>(), 1..512)) {
            let mut rows = Vec::new();
            let mut cursor = 0;
            for _ in 0..n {
                let mut row = Vec::new();
                for _ in 0..dim {
                    row.push(f32::from_bits(values[cursor % values.len()]));
                    cursor += 1;
                }
                rows.push(row);
            }
            let refs: Vec<_> = rows.iter().map(Vec::as_slice).collect();
            let bytes = encode_column_chunk(&refs).expect("encode");
            let decoded = decode_column_chunk(&bytes).expect("decode");
        for (index, row) in rows.iter().enumerate() {
            let got = decoded.row(index).unwrap();
            let got_bits: Vec<_> = got.iter().map(|value| value.to_bits()).collect();
            let want_bits: Vec<_> = row.iter().map(|value| value.to_bits()).collect();
            prop_assert_eq!(got_bits, want_bits);
        }
        }
    }
}
