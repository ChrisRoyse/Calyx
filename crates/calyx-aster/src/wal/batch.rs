use super::{AppendAck, Wal};
use calyx_core::{CalyxError, Clock, Result};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

type BatchReply = Sender<Result<BatchResponse>>;

enum BatchOp {
    Append(Vec<u8>),
    Flush,
    TipSeq,
}

enum BatchResponse {
    Ack(AppendAck),
    Flush,
    TipSeq(u64),
}

struct BatchRequest {
    op: BatchOp,
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
                op: BatchOp::Append(payload),
                respond,
            })
            .map_err(|_| CalyxError::disk_pressure("group commit batcher is closed"))?;
        match receive
            .recv()
            .map_err(|_| CalyxError::disk_pressure("group commit response channel closed"))?
        {
            Ok(BatchResponse::Ack(ack)) => Ok(ack),
            Ok(_) => Err(CalyxError::disk_pressure("missing WAL ack")),
            Err(error) => Err(error),
        }
    }

    pub fn flush_sync(&self) -> Result<()> {
        let (respond, receive) = mpsc::channel();
        self.sender
            .send(BatchRequest {
                op: BatchOp::Flush,
                respond,
            })
            .map_err(|_| CalyxError::disk_pressure("group commit batcher is closed"))?;
        match receive
            .recv()
            .map_err(|_| CalyxError::disk_pressure("group commit flush channel closed"))?
        {
            Ok(BatchResponse::Flush) => Ok(()),
            Ok(_) => Err(CalyxError::disk_pressure("missing WAL flush ack")),
            Err(error) => Err(error),
        }
    }

    pub fn tip_seq(&self) -> Result<u64> {
        let (respond, receive) = mpsc::channel();
        self.sender
            .send(BatchRequest {
                op: BatchOp::TipSeq,
                respond,
            })
            .map_err(|_| CalyxError::disk_pressure("group commit batcher is closed"))?;
        match receive
            .recv()
            .map_err(|_| CalyxError::disk_pressure("group commit tip channel closed"))?
        {
            Ok(BatchResponse::TipSeq(seq)) => Ok(seq),
            Ok(_) => Err(CalyxError::disk_pressure("missing WAL tip ack")),
            Err(error) => Err(error),
        }
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
    _clock: Arc<dyn Clock>,
) {
    while let Ok(first) = receiver.recv() {
        if !matches!(first.op, BatchOp::Append(_)) {
            flush_requests(&wal, vec![first]);
            continue;
        }
        let mut requests = vec![first];
        match receiver.try_recv() {
            Ok(request) => requests.push(request),
            Err(TryRecvError::Empty) => {
                flush_requests(&wal, requests);
                continue;
            }
            Err(TryRecvError::Disconnected) => break,
        }
        let deadline = std::time::Instant::now() + group_commit_window;
        loop {
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            match receiver.recv_timeout(deadline.saturating_duration_since(now)) {
                Ok(request) => requests.push(request),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        flush_requests(&wal, requests);
    }
}

fn flush_requests(wal: &Mutex<Wal>, requests: Vec<BatchRequest>) {
    let payloads: Vec<_> = requests
        .iter()
        .filter_map(|request| match &request.op {
            BatchOp::Append(payload) => Some(payload.as_slice()),
            BatchOp::Flush | BatchOp::TipSeq => None,
        })
        .collect();
    let wants_tip = requests
        .iter()
        .any(|request| matches!(request.op, BatchOp::TipSeq));
    let (acks, tip_seq) = {
        let mut wal = wal.lock().expect("group commit WAL lock poisoned");
        let acks = if payloads.is_empty() {
            Vec::new()
        } else {
            match wal.append_batch(&payloads) {
                Ok(acks) => acks,
                Err(error) => return send_error(&requests, error),
            }
        };
        let tip_seq = if wants_tip {
            match wal.durable_tip_seq() {
                Ok(seq) => Some(seq),
                Err(error) => return send_error(&requests, error),
            }
        } else {
            None
        };
        (acks, tip_seq)
    };
    let mut acks = acks.into_iter();
    for request in requests {
        let response = match request.op {
            BatchOp::Append(_) => acks
                .next()
                .map(BatchResponse::Ack)
                .ok_or_else(|| CalyxError::disk_pressure("missing WAL ack")),
            BatchOp::Flush => Ok(BatchResponse::Flush),
            BatchOp::TipSeq => tip_seq
                .map(BatchResponse::TipSeq)
                .ok_or_else(|| CalyxError::disk_pressure("missing WAL tip")),
        };
        let _ = request.respond.send(response);
    }
}

fn send_error(requests: &[BatchRequest], error: CalyxError) {
    for request in requests {
        let _ = request.respond.send(Err(error.clone()));
    }
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
