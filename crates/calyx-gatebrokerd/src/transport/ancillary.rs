use std::mem::size_of;
use std::os::fd::{FromRawFd, OwnedFd, RawFd};

use super::{MAX_RIGHTS, TransportError};

pub(super) unsafe fn received_rights(
    message: &libc::msghdr,
) -> Result<Vec<OwnedFd>, TransportError> {
    let mut rights = Vec::new();
    let mut first_error = None;
    let base = unsafe { libc::CMSG_LEN(0) } as usize;
    let total = message.msg_controllen;
    if total == 0 {
        return Ok(rights);
    }
    if message.msg_control.is_null() || base < size_of::<libc::cmsghdr>() {
        return Err(TransportError::MalformedAncillary(
            "nonempty control data has no valid storage".into(),
        ));
    }

    let mut offset = 0usize;
    while total.saturating_sub(offset) >= base {
        let header_pointer = unsafe {
            message
                .msg_control
                .cast::<u8>()
                .add(offset)
                .cast::<libc::cmsghdr>()
        };
        let header = unsafe { std::ptr::read_unaligned(header_pointer) };
        let length = header.cmsg_len;
        let remaining = total - offset;
        if length < base || length > remaining {
            first_error.get_or_insert_with(|| {
                TransportError::MalformedAncillary(format!(
                    "invalid cmsg_len={length} at offset={offset} remaining={remaining}"
                ))
            });
            break;
        }

        let data_bytes = length - base;
        if header.cmsg_level == libc::SOL_SOCKET && header.cmsg_type == libc::SCM_RIGHTS {
            if !data_bytes.is_multiple_of(size_of::<RawFd>()) {
                first_error.get_or_insert_with(|| {
                    TransportError::MalformedAncillary(format!(
                        "SCM_RIGHTS payload has {data_bytes} bytes"
                    ))
                });
            }
            let count = data_bytes / size_of::<RawFd>();
            let data = unsafe {
                message
                    .msg_control
                    .cast::<u8>()
                    .add(offset + base)
                    .cast::<RawFd>()
            };
            for index in 0..count {
                let fd = unsafe { std::ptr::read_unaligned(data.add(index)) };
                if fd < 0 {
                    first_error.get_or_insert_with(|| {
                        TransportError::MalformedAncillary(format!(
                            "SCM_RIGHTS delivered invalid fd={fd}"
                        ))
                    });
                } else {
                    // Ownership is taken immediately. Vec drop closes every
                    // delivered descriptor if a later validation fails.
                    rights.push(unsafe { OwnedFd::from_raw_fd(fd) });
                }
            }
        } else {
            first_error.get_or_insert(TransportError::UnexpectedAncillary {
                level: header.cmsg_level,
                kind: header.cmsg_type,
            });
        }

        let aligned = cmsg_align(length).ok_or_else(|| {
            TransportError::MalformedAncillary("cmsg length alignment overflow".into())
        })?;
        if aligned > remaining {
            break;
        }
        offset += aligned;
    }

    if rights.len() > MAX_RIGHTS {
        first_error.get_or_insert(TransportError::TooManyRights(rights.len()));
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(rights),
    }
}

fn cmsg_align(length: usize) -> Option<usize> {
    let alignment = size_of::<usize>();
    length
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}
