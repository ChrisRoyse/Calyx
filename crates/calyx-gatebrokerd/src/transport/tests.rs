use std::io;
use std::mem::{size_of, size_of_val, zeroed};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

use super::address::verify_listener_contract;
use super::*;
use crate::protocol::MAX_FRAME_BYTES;

fn pipe() -> (OwnedFd, OwnedFd) {
    let mut descriptors = [-1; 2];
    assert_eq!(
        unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) },
        0,
        "pipe2 failed: {}",
        io::Error::last_os_error()
    );
    unsafe {
        (
            OwnedFd::from_raw_fd(descriptors[0]),
            OwnedFd::from_raw_fd(descriptors[1]),
        )
    }
}

fn socket_pair(socket_type: i32) -> (OwnedFd, OwnedFd) {
    let mut descriptors = [-1; 2];
    assert_eq!(
        unsafe {
            libc::socketpair(
                libc::AF_UNIX,
                socket_type | libc::SOCK_CLOEXEC,
                0,
                descriptors.as_mut_ptr(),
            )
        },
        0,
        "socketpair failed: {}",
        io::Error::last_os_error()
    );
    unsafe {
        (
            OwnedFd::from_raw_fd(descriptors[0]),
            OwnedFd::from_raw_fd(descriptors[1]),
        )
    }
}

fn raw_send(connection: &SeqpacketConnection, bytes: &[u8], rights: &[RawFd]) {
    let mut iov = libc::iovec {
        iov_base: bytes.as_ptr() as *mut libc::c_void,
        iov_len: bytes.len(),
    };
    let required = unsafe { libc::CMSG_SPACE(size_of_val(rights) as u32) } as usize;
    let mut control = vec![0usize; required.div_ceil(size_of::<usize>())];
    let mut message = unsafe { zeroed::<libc::msghdr>() };
    message.msg_iov = &mut iov;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    message.msg_controllen = required;
    unsafe {
        let header = libc::CMSG_FIRSTHDR(&message);
        assert!(!header.is_null());
        (*header).cmsg_level = libc::SOL_SOCKET;
        (*header).cmsg_type = libc::SCM_RIGHTS;
        (*header).cmsg_len = libc::CMSG_LEN(size_of_val(rights) as u32) as usize;
        std::ptr::copy_nonoverlapping(
            rights.as_ptr().cast::<u8>(),
            libc::CMSG_DATA(header),
            size_of_val(rights),
        );
        assert_eq!(
            libc::sendmsg(connection.raw_fd(), &message, libc::MSG_NOSIGNAL),
            bytes.len() as isize
        );
    }
}

fn assert_pipe_has_no_leaked_writer(read_end: &OwnedFd) {
    let mut descriptor = libc::pollfd {
        fd: read_end.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    assert_eq!(unsafe { libc::poll(&mut descriptor, 1, 1_000) }, 1);
    let mut byte = 0u8;
    assert_eq!(
        unsafe { libc::read(read_end.as_raw_fd(), (&mut byte as *mut u8).cast(), 1) },
        0,
        "pipe did not reach EOF after failed receive"
    );
}

fn socket_inode(fd: RawFd) -> u64 {
    let mut metadata = unsafe { zeroed::<libc::stat>() };
    assert_eq!(unsafe { libc::fstat(fd, &mut metadata) }, 0);
    metadata.st_ino
}

#[test]
fn explicit_seqpacket_round_trip_credentials_and_deadlines() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("control.sock");
    let listener = SeqpacketListener::bind(&path, 4).unwrap();
    let thread = std::thread::spawn({
        let path = path.clone();
        move || {
            let connection = SeqpacketConnection::connect(&path).unwrap();
            connection.send(b"request", &[]).unwrap();
            assert_eq!(connection.recv().unwrap().bytes, b"response");
        }
    });
    let connection = listener.accept().unwrap();
    let credentials = connection.peer_credentials().unwrap();
    assert_eq!(credentials.uid, unsafe { libc::getuid() });
    assert_eq!(connection.recv().unwrap().bytes, b"request");
    connection.send(b"response", &[]).unwrap();
    thread.join().unwrap();

    for option in [libc::SO_RCVTIMEO, libc::SO_SNDTIMEO] {
        let mut timeout = unsafe { zeroed::<libc::timeval>() };
        let mut length = size_of::<libc::timeval>() as libc::socklen_t;
        assert_eq!(
            unsafe {
                libc::getsockopt(
                    connection.raw_fd(),
                    libc::SOL_SOCKET,
                    option,
                    (&mut timeout as *mut libc::timeval).cast(),
                    &mut length,
                )
            },
            0
        );
        assert!(timeout.tv_sec > 0 || timeout.tv_usec > 0);
    }
    eprintln!(
        "SOURCE_OF_TRUTH socket={} inode={} peer_pid={} recv_send_deadlines=nonzero",
        path.display(),
        socket_inode(connection.raw_fd()),
        credentials.pid
    );
}

#[test]
fn receive_deadline_is_a_real_kernel_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("timeout.sock");
    let listener = SeqpacketListener::bind(&path, 1).unwrap();
    let _client = SeqpacketConnection::connect(&path).unwrap();
    let connection = listener.accept().unwrap();
    connection
        .set_io_timeout(Duration::from_millis(40))
        .unwrap();
    let before = Instant::now();
    assert!(matches!(
        connection.recv(),
        Err(TransportError::TimedOut {
            operation: "receive seqpacket frame"
        })
    ));
    let elapsed = before.elapsed();
    assert!(elapsed >= Duration::from_millis(20));
    assert!(elapsed < Duration::from_secs(1));
    eprintln!(
        "EDGE receive_timeout before=blocked after=TimedOut elapsed_ms={}",
        elapsed.as_millis()
    );
}

#[test]
fn send_deadline_is_a_real_kernel_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("send-timeout.sock");
    let listener = SeqpacketListener::bind(&path, 1).unwrap();
    let _client = SeqpacketConnection::connect(&path).unwrap();
    let connection = listener.accept().unwrap();
    connection
        .set_io_timeout(Duration::from_millis(40))
        .unwrap();
    let requested_buffer = 4_096i32;
    assert_eq!(
        unsafe {
            libc::setsockopt(
                connection.raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                (&requested_buffer as *const i32).cast(),
                size_of::<i32>() as libc::socklen_t,
            )
        },
        0
    );

    let packet = vec![b'x'; 2_048];
    let before = Instant::now();
    let mut delivered = 0usize;
    loop {
        match connection.send(&packet, &[]) {
            Ok(()) => delivered += 1,
            Err(TransportError::TimedOut {
                operation: "send seqpacket frame",
            }) => break,
            Err(error) => panic!("unexpected send failure: {error}"),
        }
    }
    let elapsed = before.elapsed();
    assert!(delivered > 0);
    assert!(elapsed >= Duration::from_millis(20));
    assert!(elapsed < Duration::from_secs(1));
    eprintln!(
        "EDGE send_timeout before=receiver-not-reading after=TimedOut queued_packets={delivered} elapsed_ms={}",
        elapsed.as_millis()
    );
}

#[test]
fn scm_rights_arrive_close_on_exec() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rights.sock");
    let listener = SeqpacketListener::bind(&path, 4).unwrap();
    let thread = std::thread::spawn({
        let path = path.clone();
        move || {
            SeqpacketConnection::connect(&path)
                .unwrap()
                .send(b"with-right", &[std::io::stdout().as_raw_fd()])
                .unwrap();
        }
    });
    let frame = listener.accept().unwrap().recv().unwrap();
    assert_eq!(frame.rights.len(), 1);
    let flags = unsafe { libc::fcntl(frame.rights[0].as_raw_fd(), libc::F_GETFD) };
    assert_ne!(flags & libc::FD_CLOEXEC, 0);
    thread.join().unwrap();
}

#[test]
fn listener_contract_is_kernel_verified() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("contract.sock");
    let listener = SeqpacketListener::bind(&path, 4).unwrap();
    verify_listener_contract(listener.0.as_raw_fd()).unwrap();
    listener.verify_bound_path(&path).unwrap();
    assert!(matches!(
        listener.verify_bound_path(&temp.path().join("different.sock")),
        Err(TransportError::Activation(message))
            if message.contains("activated socket pathname mismatch")
    ));

    let enabled = 1i32;
    assert_eq!(
        unsafe {
            libc::setsockopt(
                listener.0.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PASSCRED,
                (&enabled as *const i32).cast(),
                size_of::<i32>() as libc::socklen_t,
            )
        },
        0
    );
    assert!(matches!(
        verify_listener_contract(listener.0.as_raw_fd()),
        Err(TransportError::Activation(message)) if message.contains("SO_PASSCRED=0")
    ));
}

#[test]
fn listener_contract_rejects_wrong_socket_objects() {
    let inet_fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    assert!(inet_fd >= 0);
    let inet = unsafe { OwnedFd::from_raw_fd(inet_fd) };
    assert!(matches!(
        verify_listener_contract(inet.as_raw_fd()),
        Err(TransportError::Activation(message)) if message.contains("SO_DOMAIN=AF_UNIX")
    ));
    let (stream, _) = socket_pair(libc::SOCK_STREAM);
    assert!(matches!(
        verify_listener_contract(stream.as_raw_fd()),
        Err(TransportError::Activation(message)) if message.contains("SO_TYPE=SOCK_SEQPACKET")
    ));
    let (seqpacket, _) = socket_pair(libc::SOCK_SEQPACKET);
    assert!(matches!(
        verify_listener_contract(seqpacket.as_raw_fd()),
        Err(TransportError::Activation(message)) if message.contains("SO_ACCEPTCONN=1")
    ));
}

#[test]
fn descriptor_errors_close_every_kernel_delivered_fd() {
    for case in ["too-many", "oversized", "truncated"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(format!("{case}.sock"));
        let listener = SeqpacketListener::bind(&path, 4).unwrap();
        let (read_end, write_end) = pipe();
        let thread = std::thread::spawn({
            let path = path.clone();
            let case = case.to_owned();
            move || {
                let connection = SeqpacketConnection::connect(&path).unwrap();
                match case.as_str() {
                    "too-many" => raw_send(
                        &connection,
                        b"rights",
                        &[write_end.as_raw_fd(); MAX_RIGHTS + 1],
                    ),
                    "oversized" => raw_send(
                        &connection,
                        &vec![b'x'; MAX_FRAME_BYTES + 4_096],
                        &[write_end.as_raw_fd()],
                    ),
                    _ => raw_send(&connection, b"truncated", &vec![write_end.as_raw_fd(); 80]),
                }
            }
        });
        let result = listener.accept().unwrap().recv();
        assert!(match (case, result) {
            ("too-many", Err(TransportError::TooManyRights(count))) => count == MAX_RIGHTS + 1,
            ("oversized", Err(TransportError::OversizedFrame { actual, maximum })) => {
                actual > maximum && maximum == MAX_FRAME_BYTES
            }
            ("truncated", Err(TransportError::TruncatedAncillary { capacity })) => {
                capacity == CONTROL_BYTES
            }
            _ => false,
        });
        thread.join().unwrap();
        assert_pipe_has_no_leaked_writer(&read_end);
        eprintln!("EDGE ancillary_{case} before=writer-delivered after=pipe-eof");
    }
}
