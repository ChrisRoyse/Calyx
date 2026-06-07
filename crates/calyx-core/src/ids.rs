//! Stable Calyx identifiers and content-addressing helpers.

use core::fmt;
use core::str::FromStr;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ulid::Ulid;

const ID_BYTES: usize = 16;
const HEX_CHARS: usize = ID_BYTES * 2;

/// Error returned when parsing an ID from its stable display form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseIdError {
    /// The input had the wrong byte length.
    InvalidLength { expected: usize, actual: usize },
    /// The input contained a non-hex byte at the given byte index.
    InvalidHex { index: usize },
    /// The input was not a valid ULID string.
    InvalidUlid,
    /// The input was not a valid `u16` slot id.
    InvalidSlotId,
}

impl fmt::Display for ParseIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(f, "invalid id length: expected {expected}, got {actual}")
            }
            Self::InvalidHex { index } => write!(f, "invalid hex byte at index {index}"),
            Self::InvalidUlid => write!(f, "invalid ULID"),
            Self::InvalidSlotId => write!(f, "invalid slot id"),
        }
    }
}

impl std::error::Error for ParseIdError {}

/// A vault identifier, stable for the life of a vault.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VaultId(pub Ulid);

impl VaultId {
    /// Builds a vault id from an existing ULID.
    pub const fn from_ulid(id: Ulid) -> Self {
        Self(id)
    }

    /// Returns the wrapped ULID.
    pub const fn as_ulid(self) -> Ulid {
        self.0
    }
}

impl fmt::Debug for VaultId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("VaultId").field(&self.0.to_string()).finish()
    }
}

impl fmt::Display for VaultId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for VaultId {
    type Err = ParseIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value
            .parse::<Ulid>()
            .map(Self)
            .map_err(|_| ParseIdError::InvalidUlid)
    }
}

impl Serialize for VaultId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for VaultId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(StringIdVisitor::<VaultId>::new("a vault ULID"))
    }
}

macro_rules! hex_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub [u8; ID_BYTES]);

        impl $name {
            /// Builds an id from raw bytes.
            pub const fn from_bytes(bytes: [u8; ID_BYTES]) -> Self {
                Self(bytes)
            }

            /// Returns the raw id bytes by value.
            pub const fn to_bytes(self) -> [u8; ID_BYTES] {
                self.0
            }

            /// Returns the raw id bytes by reference.
            pub const fn as_bytes(&self) -> &[u8; ID_BYTES] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name))
                    .field(&hex_lower(&self.0))
                    .finish()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&hex_lower(&self.0))
            }
        }

        impl FromStr for $name {
            type Err = ParseIdError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                parse_hex_16(value).map(Self)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                deserializer.deserialize_str(StringIdVisitor::<$name>::new(stringify!($name)))
            }
        }
    };
}

hex_id! {
    /// A frozen lens content identifier.
    ///
    /// It is derived from lens name, weights hash, corpus hash, and output
    /// shape, so identical lens specs produce the same id across vaults.
    LensId
}

impl LensId {
    /// Builds a lens id from the PRD lens spec fields.
    pub fn from_parts(
        name: &str,
        weights_sha256: &[u8],
        corpus_hash: &[u8],
        output_shape: &[u8],
    ) -> Self {
        Self(content_address([
            name.as_bytes(),
            weights_sha256,
            corpus_hash,
            output_shape,
        ]))
    }
}

hex_id! {
    /// A content-addressed constellation identifier.
    CxId
}

impl CxId {
    /// Builds a constellation id from input bytes, panel version, and vault salt.
    pub fn from_input(input_bytes: &[u8], panel_version: u32, vault_salt: &[u8]) -> Self {
        let panel_version = panel_version.to_be_bytes();
        Self(content_address([
            input_bytes,
            panel_version.as_slice(),
            vault_salt,
        ]))
    }
}

/// A small stable slot index into a panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SlotId(pub u16);

impl SlotId {
    /// Builds a slot id from a stable panel index.
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the raw slot index.
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Pairs this id with a stable human-readable slot key.
    pub fn with_key(self, key: impl Into<String>) -> SlotKey {
        SlotKey::new(self, key)
    }
}

impl fmt::Display for SlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for SlotId {
    type Err = ParseIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value
            .parse::<u16>()
            .map(Self)
            .map_err(|_| ParseIdError::InvalidSlotId)
    }
}

/// A stable slot key paired with its compact panel index.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SlotKey {
    id: SlotId,
    key: String,
}

impl SlotKey {
    /// Creates a slot key pair.
    pub fn new(id: SlotId, key: impl Into<String>) -> Self {
        Self {
            id,
            key: key.into(),
        }
    }

    /// Returns the compact slot id.
    pub const fn id(&self) -> SlotId {
        self.id
    }

    /// Returns the stable human-readable key.
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// Computes a stable 16-byte BLAKE3 content address from ordered byte parts.
///
/// Each part is length-delimited before hashing, which preserves the PRD's
/// ordered-concatenation semantics while preventing boundary ambiguity.
pub fn content_address<I, P>(parts: I) -> [u8; ID_BYTES]
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

    let mut out = [0_u8; ID_BYTES];
    out.copy_from_slice(&hasher.finalize().as_bytes()[..ID_BYTES]);
    out
}

struct StringIdVisitor<T> {
    expected: &'static str,
    marker: core::marker::PhantomData<T>,
}

impl<T> StringIdVisitor<T> {
    const fn new(expected: &'static str) -> Self {
        Self {
            expected,
            marker: core::marker::PhantomData,
        }
    }
}

impl<'de, T> Visitor<'de> for StringIdVisitor<T>
where
    T: FromStr<Err = ParseIdError>,
{
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.expected)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        value.parse::<T>().map_err(E::custom)
    }
}

fn hex_lower(bytes: &[u8; ID_BYTES]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(HEX_CHARS);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

fn parse_hex_16(value: &str) -> Result<[u8; ID_BYTES], ParseIdError> {
    if value.len() != HEX_CHARS {
        return Err(ParseIdError::InvalidLength {
            expected: HEX_CHARS,
            actual: value.len(),
        });
    }

    let mut out = [0_u8; ID_BYTES];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let hi = hex_value(chunk[0]).ok_or(ParseIdError::InvalidHex { index: index * 2 })?;
        let lo = hex_value(chunk[1]).ok_or(ParseIdError::InvalidHex {
            index: index * 2 + 1,
        })?;
        out[index] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn roundtrip_json<T>(value: T)
    where
        T: core::fmt::Debug + Eq + Serialize + for<'de> Deserialize<'de>,
    {
        let encoded = serde_json::to_string(&value).expect("serialize id");
        let decoded = serde_json::from_str(&encoded).expect("deserialize id");
        assert_eq!(value, decoded);
    }

    proptest! {
        #[test]
        fn lens_id_display_and_serde_roundtrip(bytes in any::<[u8; ID_BYTES]>()) {
            let id = LensId::from_bytes(bytes);
            prop_assert_eq!(id.to_string().parse::<LensId>().unwrap(), id);
            roundtrip_json(id);
        }

        #[test]
        fn cx_id_display_and_serde_roundtrip(bytes in any::<[u8; ID_BYTES]>()) {
            let id = CxId::from_bytes(bytes);
            prop_assert_eq!(id.to_string().parse::<CxId>().unwrap(), id);
            roundtrip_json(id);
        }

        #[test]
        fn vault_id_display_and_serde_roundtrip(bytes in any::<[u8; ID_BYTES]>()) {
            let id = VaultId::from_ulid(Ulid::from_bytes(bytes));
            prop_assert_eq!(id.to_string().parse::<VaultId>().unwrap(), id);
            roundtrip_json(id);
        }

        #[test]
        fn slot_id_display_and_serde_roundtrip(value in any::<u16>()) {
            let id = SlotId::new(value);
            prop_assert_eq!(id.to_string().parse::<SlotId>().unwrap(), id);
            roundtrip_json(id);
        }
    }

    #[test]
    fn content_address_is_length_delimited() {
        let first = content_address([b"ab".as_slice(), b"c".as_slice()]);
        let second = content_address([b"a".as_slice(), b"bc".as_slice()]);

        assert_ne!(first, second);
    }

    #[test]
    fn cx_id_is_deterministic_on_raw_bytes() {
        let a = CxId::from_input(b"synthetic-input", 7, b"vault-salt");
        let b = CxId::from_input(b"synthetic-input", 7, b"vault-salt");
        let changed_panel = CxId::from_input(b"synthetic-input", 8, b"vault-salt");

        println!("cxid bytes: {:?}", a.as_bytes());
        assert_eq!(a.as_bytes(), b.as_bytes());
        assert_ne!(a.as_bytes(), changed_panel.as_bytes());
    }

    #[test]
    fn lens_id_is_deterministic_on_raw_bytes() {
        let a = LensId::from_parts("sem-self", b"weights", b"corpus", b"768xf32");
        let b = LensId::from_parts("sem-self", b"weights", b"corpus", b"768xf32");
        let changed_shape = LensId::from_parts("sem-self", b"weights", b"corpus", b"1024xf32");

        println!("lensid bytes: {:?}", a.as_bytes());
        assert_eq!(a.as_bytes(), b.as_bytes());
        assert_ne!(a.as_bytes(), changed_shape.as_bytes());
    }

    #[test]
    fn slot_key_keeps_id_and_key_together() {
        let slot = SlotId::new(3).with_key("sem-self");

        assert_eq!(slot.id(), SlotId::new(3));
        assert_eq!(slot.key(), "sem-self");
    }

    #[test]
    fn invalid_hex_is_rejected() {
        assert!(matches!(
            "not-a-valid-id".parse::<LensId>(),
            Err(ParseIdError::InvalidLength { .. })
        ));
        assert!(matches!(
            "0000000000000000000000000000000z".parse::<CxId>(),
            Err(ParseIdError::InvalidHex { index: 31 })
        ));
    }
}
