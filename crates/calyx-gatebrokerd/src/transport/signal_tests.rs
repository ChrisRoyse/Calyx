use std::fs;
use std::io;
use std::mem::{size_of, zeroed};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use super::connection::{RECEIVE_INTERRUPTS, SEND_INTERRUPTS};
use super::listener::ACCEPT_INTERRUPTS;
use super::{SeqpacketConnection, SeqpacketListener};

static SIGNAL_TEST: Mutex<()> = Mutex::new(());
static SIGNALS_DELIVERED: AtomicUsize = AtomicUsize::new(0);

extern "C" fn record_signal(_: libc::c_int) {
    SIGNALS_DELIVERED.fetch_add(1, Ordering::Relaxed);
}

struct SignalAction(libc::sigaction);

impl SignalAction {
    fn install() -> Self {
        let mut action = unsafe { zeroed::<libc::sigaction>() };
        action.sa_sigaction = record_signal as *const () as usize;
        action.sa_flags = 0;
        assert_eq!(unsafe { libc::sigemptyset(&mut action.sa_mask) }, 0);
        let mut previous = unsafe { zeroed::<libc::sigaction>() };
        assert_eq!(
            unsafe { libc::sigaction(libc::SIGUSR2, &action, &mut previous) },
            0
        );
        Self(previous)
    }
}

impl Drop for SignalAction {
    fn drop(&mut self) {
        assert_eq!(
            unsafe { libc::sigaction(libc::SIGUSR2, &self.0, std::ptr::null_mut()) },
            0
        );
    }
}

#[derive(Clone, Copy)]
struct ThreadIdentity {
    pthread: libc::pthread_t,
    tid: libc::pid_t,
}

fn current_thread() -> ThreadIdentity {
    ThreadIdentity {
        pthread: unsafe { libc::pthread_self() },
        tid: unsafe { libc::syscall(libc::SYS_gettid) as libc::pid_t },
    }
}

fn wait_for_syscall(thread: ThreadIdentity, expected: libc::c_long) {
    let path = format!("/proc/self/task/{}/syscall", thread.tid);
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut observed = String::new();
    while Instant::now() < deadline {
        observed = fs::read_to_string(&path).unwrap();
        if observed
            .split_ascii_whitespace()
            .next()
            .is_some_and(|value| value == expected.to_string())
        {
            return;
        }
        std::thread::yield_now();
    }
    panic!(
        "thread {} did not enter syscall {expected}; /proc state={observed:?}",
        thread.tid
    );
}

fn wait_for_retry(counter: &AtomicUsize, operation: &str) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if counter.load(Ordering::Acquire) == 1 {
            return;
        }
        std::thread::yield_now();
    }
    panic!("{operation} did not observe EINTR after the signal was sent");
}

#[test]
fn accept_and_receive_retry_real_signal_interruptions() {
    let _serialized = SIGNAL_TEST.lock().unwrap();
    let _action = SignalAction::install();
    SIGNALS_DELIVERED.store(0, Ordering::Relaxed);
    ACCEPT_INTERRUPTS.store(0, Ordering::Relaxed);
    RECEIVE_INTERRUPTS.store(0, Ordering::Relaxed);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("eintr.sock");
    let listener = SeqpacketListener::bind(&path, 2).unwrap();
    let (thread_sender, thread_receiver) = mpsc::channel();
    let acceptor = std::thread::spawn(move || {
        thread_sender.send(current_thread()).unwrap();
        let connection = listener.accept().unwrap();
        assert_eq!(connection.recv().unwrap().bytes, b"after-eintr");
    });
    let thread = thread_receiver.recv().unwrap();
    wait_for_syscall(thread, libc::SYS_accept4);
    assert_eq!(
        unsafe { libc::pthread_kill(thread.pthread, libc::SIGUSR2) },
        0
    );
    wait_for_retry(&ACCEPT_INTERRUPTS, "accept4");
    let client = SeqpacketConnection::connect(&path).unwrap();
    wait_for_syscall(thread, libc::SYS_recvmsg);
    assert_eq!(
        unsafe { libc::pthread_kill(thread.pthread, libc::SIGUSR2) },
        0
    );
    wait_for_retry(&RECEIVE_INTERRUPTS, "recvmsg");
    client.send(b"after-eintr", &[]).unwrap();
    acceptor.join().unwrap();
    assert_eq!(SIGNALS_DELIVERED.load(Ordering::Relaxed), 2);
    assert_eq!(ACCEPT_INTERRUPTS.load(Ordering::Relaxed), 1);
    assert_eq!(RECEIVE_INTERRUPTS.load(Ordering::Relaxed), 1);
    eprintln!("SOURCE_OF_TRUTH accept_eintr_retries=1 receive_eintr_retries=1");
}

#[test]
fn send_retries_a_real_signal_interruption_without_packet_loss() {
    let _serialized = SIGNAL_TEST.lock().unwrap();
    let _action = SignalAction::install();
    SIGNALS_DELIVERED.store(0, Ordering::Relaxed);
    SEND_INTERRUPTS.store(0, Ordering::Relaxed);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("send-eintr.sock");
    let listener = SeqpacketListener::bind(&path, 1).unwrap();
    let client = SeqpacketConnection::connect(&path).unwrap();
    let connection = listener.accept().unwrap();
    connection.set_io_timeout(Duration::from_secs(1)).unwrap();
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
    let filler = vec![b'f'; 2_048];
    let mut queued = 0usize;
    loop {
        let result = unsafe {
            libc::send(
                connection.raw_fd(),
                filler.as_ptr().cast(),
                filler.len(),
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if result == filler.len() as isize {
            queued += 1;
            continue;
        }
        assert_eq!(result, -1);
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::EAGAIN)
        );
        break;
    }
    assert!(queued > 0);

    let (thread_sender, thread_receiver) = mpsc::channel();
    let sender = std::thread::spawn(move || {
        thread_sender.send(current_thread()).unwrap();
        connection.send(&vec![b's'; 2_048], &[]).unwrap();
    });
    let thread = thread_receiver.recv().unwrap();
    wait_for_syscall(thread, libc::SYS_sendmsg);
    assert_eq!(
        unsafe { libc::pthread_kill(thread.pthread, libc::SIGUSR2) },
        0
    );
    wait_for_retry(&SEND_INTERRUPTS, "sendmsg");
    for _ in 0..queued {
        assert_eq!(client.recv().unwrap().bytes, filler);
    }
    sender.join().unwrap();
    assert_eq!(client.recv().unwrap().bytes, vec![b's'; 2_048]);
    assert_eq!(SIGNALS_DELIVERED.load(Ordering::Relaxed), 1);
    assert_eq!(SEND_INTERRUPTS.load(Ordering::Relaxed), 1);
    eprintln!(
        "SOURCE_OF_TRUTH send_eintr_retries=1 filler_packets={queued} delivered_packets={}",
        queued + 1
    );
}
