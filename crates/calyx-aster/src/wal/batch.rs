use super::{AppendAck, Wal};
use calyx_core::{CalyxError, Clock, Result};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

type BatchReply = Sender<Result<Option<AppendAck>>>;

struct BatchRequest {
    payload: Option<Vec<u8>>,
    respond: BatchReply,
}

/// Fsync-backed group commit wrapper around `Wal`.
#[derive(Debug)]
pub struct GroupCommitBatcher {
    sender: Sender<BatchRequest>,
    _thread: JoinHandle<()>,
}

impl GroupCommitBatcher {
    pub fn new(wal: Wal, group_commit_window: Duration, clock: Arc<dyn Clock>) -> Result<Self> {
        validate_window(group_commit_window)?;
        let (sender, receiver) = mpsc::channel();
        let wal = Arc::new(Mutex::new(wal));
        let thread = thread::spawn(move || run_batcher(wal, receiver, group_commit_window, clock));
        Ok(Self {
            sender,
            _thread: thread,
        })
    }

    pub fn submit(&self, payload: Vec<u8>) -> Result<AppendAck> {
        let (respond, receive) = mpsc::channel();
        self.sender
            .send(BatchRequest {
                payload: Some(payload),
                respond,
            })
            .map_err(|_| CalyxError::disk_pressure("group commit batcher is closed"))?;
        receive
            .recv()
            .map_err(|_| CalyxError::disk_pressure("group commit response channel closed"))?
            .and_then(|ack| ack.ok_or_else(|| CalyxError::disk_pressure("missing WAL ack")))
    }

    pub fn flush_sync(&self) -> Result<()> {
        let (respond, receive) = mpsc::channel();
        self.sender
            .send(BatchRequest {
                payload: None,
                respond,
            })
            .map_err(|_| CalyxError::disk_pressure("group commit batcher is closed"))?;
        receive
            .recv()
            .map_err(|_| CalyxError::disk_pressure("group commit flush channel closed"))?
            .map(|_| ())
    }
}

pub(super) fn validate_window(window: Duration) -> Result<()> {
    if window > super::DEFAULT_GROUP_COMMIT_WINDOW {
        return Err(CalyxError::disk_pressure(
            "group_commit_window exceeds 2 ms limit",
        ));
    }
    Ok(())
}

fn run_batcher(
    wal: Arc<Mutex<Wal>>,
    receiver: Receiver<BatchRequest>,
    group_commit_window: Duration,
    clock: Arc<dyn Clock>,
) {
    while let Ok(first) = receiver.recv() {
        let start = clock.now();
        let mut requests = vec![first];
        while elapsed_ms(start, clock.now()) <= group_commit_window.as_millis() as u64 {
            match receiver.try_recv() {
                Ok(request) => requests.push(request),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        flush_requests(&wal, requests);
    }
}

fn flush_requests(wal: &Mutex<Wal>, requests: Vec<BatchRequest>) {
    let payloads: Vec<_> = requests
        .iter()
        .filter_map(|request| request.payload.as_deref())
        .collect();
    let result = if payloads.is_empty() {
        Ok(Vec::new())
    } else {
        wal.lock()
            .expect("group commit WAL lock poisoned")
            .append_batch(&payloads)
    };
    match result {
        Ok(acks) => {
            let mut acks = acks.into_iter();
            for request in requests {
                let response = if request.payload.is_some() {
                    acks.next()
                        .map(Some)
                        .ok_or_else(|| CalyxError::disk_pressure("missing WAL ack"))
                } else {
                    Ok(None)
                };
                let _ = request.respond.send(response);
            }
        }
        Err(error) => {
            for request in requests {
                let _ = request.respond.send(Err(error.clone()));
            }
        }
    }
}

fn elapsed_ms(start: u64, now: u64) -> u64 {
    now.saturating_sub(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wal::{WalOptions, replay_dir};
    use calyx_core::FixedClock;
    use proptest::prelude::*;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn concurrent_submitters_replay_byte_exact_payloads() {
        let dir = test_dir("batcher-concurrent");
        let wal = Wal::open(&dir, WalOptions::default()).expect("open wal");
        let batcher = Arc::new(
            GroupCommitBatcher::new(
                wal,
                super::super::DEFAULT_GROUP_COMMIT_WINDOW,
                Arc::new(FixedClock::new(1)),
            )
            .expect("batcher"),
        );
        let handles: Vec<_> = (0..5)
            .map(|index| {
                let batcher = batcher.clone();
                thread::spawn(move || batcher.submit(vec![index]).expect("submit"))
            })
            .collect();
        let mut acks = Vec::new();
        for handle in handles {
            acks.push(handle.join().expect("join"));
        }
        batcher.flush_sync().expect("flush");

        let replay = replay_dir(&dir).expect("replay");
        assert_eq!(replay.records.len(), 5);
        assert_eq!(
            replay
                .records
                .iter()
                .map(|record| record.seq)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        assert_eq!(acks.len(), 5);
        cleanup(dir);
    }

    #[test]
    fn oversized_window_fails_closed() {
        let dir = test_dir("batcher-window");
        let options = WalOptions {
            group_commit_window: Duration::from_millis(3),
            ..WalOptions::default()
        };
        let error = Wal::open(&dir, options).expect_err("window rejected");

        assert_eq!(error.code, "CALYX_DISK_PRESSURE");
        assert!(
            error
                .message
                .contains("group_commit_window exceeds 2 ms limit")
        );
        cleanup(dir);
    }

    proptest! {
        #[test]
        fn submitted_payloads_are_replayed(payloads in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..32), 1..20)) {
            let dir = test_dir("batcher-proptest");
            let wal = Wal::open(&dir, WalOptions::default()).expect("open wal");
            let batcher = GroupCommitBatcher::new(wal, super::super::DEFAULT_GROUP_COMMIT_WINDOW, Arc::new(FixedClock::new(1))).expect("batcher");
            for payload in &payloads {
                batcher.submit(payload.clone()).expect("submit payload");
            }
            batcher.flush_sync().expect("flush");
            let replay = replay_dir(&dir).expect("replay");
            prop_assert_eq!(replay.records.iter().map(|record| record.payload.clone()).collect::<Vec<_>>(), payloads);
            cleanup(dir);
        }
    }

    fn test_dir(name: &str) -> PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }

    fn cleanup(dir: PathBuf) {
        fs::remove_dir_all(dir).expect("cleanup test dir");
    }
}
