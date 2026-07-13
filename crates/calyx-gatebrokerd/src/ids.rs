//! IDs sourced directly from the Linux kernel CSPRNG.

use std::io;

use thiserror::Error;

use crate::protocol::{ObjectId, ProtocolError, RequestId, RunId, RunToken, StageId};

#[derive(Debug, Error)]
pub enum IdError {
    #[error("kernel random source failed: {0}")]
    Random(#[source] io::Error),
    #[error("generated identifier violated the protocol invariant: {0}")]
    Protocol(#[from] ProtocolError),
}

pub fn request_id() -> Result<RequestId, IdError> {
    let mut bytes = random_bytes::<16>()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let compact = hex(&bytes);
    RequestId::new(format!(
        "{}-{}-{}-{}-{}",
        &compact[0..8],
        &compact[8..12],
        &compact[12..16],
        &compact[16..20],
        &compact[20..32]
    ))
    .map_err(Into::into)
}

pub fn run_id() -> Result<RunId, IdError> {
    RunId::new(hex(&random_bytes::<16>()?)).map_err(Into::into)
}

pub fn object_id() -> Result<ObjectId, IdError> {
    ObjectId::new(hex(&random_bytes::<16>()?)).map_err(Into::into)
}

pub fn stage_id() -> Result<StageId, IdError> {
    StageId::new(hex(&random_bytes::<16>()?)).map_err(Into::into)
}

pub fn run_token() -> Result<RunToken, IdError> {
    RunToken::new(hex(&random_bytes::<32>()?)).map_err(Into::into)
}

fn random_bytes<const N: usize>() -> Result<[u8; N], IdError> {
    let mut value = [0_u8; N];
    let mut offset = 0;
    while offset < value.len() {
        let result = unsafe {
            libc::getrandom(value[offset..].as_mut_ptr().cast(), value.len() - offset, 0)
        };
        if result > 0 {
            offset += result as usize;
            continue;
        }
        if result == 0 {
            return Err(IdError::Random(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "getrandom returned zero bytes",
            )));
        }
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return Err(IdError::Random(error));
    }
    Ok(value)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_ids_match_strict_protocol_types() {
        let first = request_id().unwrap();
        let second = request_id().unwrap();
        assert_ne!(first, second);
        assert_eq!(first.as_str().as_bytes()[14], b'4');
        assert!(matches!(
            first.as_str().as_bytes()[19],
            b'8' | b'9' | b'a' | b'b'
        ));
        assert_eq!(run_id().unwrap().as_str().len(), 32);
        assert_eq!(object_id().unwrap().as_str().len(), 32);
        assert_eq!(stage_id().unwrap().as_str().len(), 32);
        assert_eq!(run_token().unwrap().as_str().len(), 64);
    }
}
