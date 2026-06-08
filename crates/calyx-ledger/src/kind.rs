//! Stable ledger entry kinds and wire codes.

use core::fmt;

use serde::{Deserialize, Serialize};

/// Ledger event kind recorded in the append-only provenance chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EntryKind {
    Ingest,
    Measure,
    Assay,
    Kernel,
    Guard,
    Answer,
    Anneal,
    Migrate,
    Admin,
    Erase,
}

impl EntryKind {
    /// All valid kinds in stable wire-code order.
    pub const ALL: [Self; 10] = [
        Self::Ingest,
        Self::Measure,
        Self::Assay,
        Self::Kernel,
        Self::Guard,
        Self::Answer,
        Self::Anneal,
        Self::Migrate,
        Self::Admin,
        Self::Erase,
    ];

    /// Returns the stable one-byte discriminant used in ledger hashes/codecs.
    pub const fn wire_code(self) -> u8 {
        match self {
            Self::Ingest => 0,
            Self::Measure => 1,
            Self::Assay => 2,
            Self::Kernel => 3,
            Self::Guard => 4,
            Self::Answer => 5,
            Self::Anneal => 6,
            Self::Migrate => 7,
            Self::Admin => 8,
            Self::Erase => 9,
        }
    }

    /// Parses a stable wire code.
    pub const fn from_wire_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Ingest),
            1 => Some(Self::Measure),
            2 => Some(Self::Assay),
            3 => Some(Self::Kernel),
            4 => Some(Self::Guard),
            5 => Some(Self::Answer),
            6 => Some(Self::Anneal),
            7 => Some(Self::Migrate),
            8 => Some(Self::Admin),
            9 => Some(Self::Erase),
            _ => None,
        }
    }

    /// Stable lowercase label for logs/readbacks.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ingest => "ingest",
            Self::Measure => "measure",
            Self::Assay => "assay",
            Self::Kernel => "kernel",
            Self::Guard => "guard",
            Self::Answer => "answer",
            Self::Anneal => "anneal",
            Self::Migrate => "migrate",
            Self::Admin => "admin",
            Self::Erase => "erase",
        }
    }
}

impl fmt::Display for EntryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_codes_are_stable_and_roundtrip() {
        for (expected, kind) in EntryKind::ALL.into_iter().enumerate() {
            assert_eq!(kind.wire_code(), expected as u8);
            assert_eq!(EntryKind::from_wire_code(expected as u8), Some(kind));
        }
        assert_eq!(EntryKind::from_wire_code(10), None);
    }
}
