pub(super) use calyx_gatebrokerd::fs_tx::{MAX_OPAQUE_HANDLE_BYTES, ObjectIdentity, OpaqueHandle};
pub(super) use calyx_gatebrokerd::journal::{
    BeginOperation, IntentRecord, Journal, OperationIntent, OperationState, RecordedCgroupIdentity,
    RunIntent, RunState, StageIntent, StageRunningEvidence, StageState, TransactionState,
    TransitionUpdate,
};
pub(super) use calyx_gatebrokerd::protocol::{
    AbortRunRequest, AbsolutePath, ErrorResponse, ErrorText, FinishRunRequest, InvocationId,
    LeafName, ObjectId, PROTOCOL_VERSION, ProfileName, ReasonText, Request, RequestEnvelope,
    RequestId, Response, ResponseEnvelope, ResponseOutcome, RoleName, RootAlias, RunId, RunStatus,
    RunToken, StableCode, StageId, StageLabel, UnitName, encode_response,
};
pub(super) use rusqlite::Connection;
pub(super) use tempfile::TempDir;

pub(super) fn request(value: u128) -> RequestId {
    let raw = format!("{value:032x}");
    RequestId::new(format!(
        "{}-{}-{}-{}-{}",
        &raw[0..8],
        &raw[8..12],
        &raw[12..16],
        &raw[16..20],
        &raw[20..32]
    ))
    .expect("request UUID")
}

pub(super) fn operation_intent(
    request: Request,
    verb: &str,
    run_id: Option<RunId>,
) -> OperationIntent {
    let request_id = request.request_id().clone();
    let request_json = serde_json::to_vec(&RequestEnvelope {
        version: PROTOCOL_VERSION,
        request,
    })
    .unwrap();
    OperationIntent {
        request_id,
        request_hash: *blake3::hash(&request_json).as_bytes(),
        request_json,
        verb: verb.into(),
        run_id,
    }
}

pub(super) fn identity(byte: u8) -> ObjectIdentity {
    ObjectIdentity {
        device: 42,
        inode: 9001,
        owner_uid: 1001,
        owner_gid: 1002,
        mode: 0o700,
        opaque: OpaqueHandle::new(7, 1, vec![byte; MAX_OPAQUE_HANDLE_BYTES])
            .expect("bounded handle"),
    }
}

#[cfg(unix)]
pub(super) fn assert_live_journal_files(path: &std::path::Path) {
    use std::os::unix::fs::MetadataExt;

    let database = path.metadata().expect("physical SQLite database");
    let wal = path.with_file_name(format!(
        "{}-wal",
        path.file_name().unwrap().to_string_lossy()
    ));
    let wal_metadata = wal.metadata().expect("physical SQLite WAL");
    let effective_uid = unsafe { libc::geteuid() };
    println!(
        "SOURCE_OF_TRUTH database={} uid={} mode={:04o} nlink={} bytes={} wal={} wal_uid={} wal_mode={:04o} wal_nlink={} wal_bytes={}",
        path.display(),
        database.uid(),
        database.mode() & 0o7777,
        database.nlink(),
        database.len(),
        wal.display(),
        wal_metadata.uid(),
        wal_metadata.mode() & 0o7777,
        wal_metadata.nlink(),
        wal_metadata.len(),
    );
    assert_eq!(
        (database.uid(), database.mode() & 0o777, database.nlink()),
        (effective_uid, 0o600, 1)
    );
    assert_eq!(
        (
            wal_metadata.uid(),
            wal_metadata.mode() & 0o777,
            wal_metadata.nlink()
        ),
        (effective_uid, 0o600, 1)
    );
    assert!(database.len() > 0 && wal_metadata.len() > 0);
}

pub(super) fn seed(journal: &mut Journal) -> (RunId, ObjectId) {
    let run_id = RunId::new("11111111111111111111111111111111").unwrap();
    journal
        .begin_run(&RunIntent {
            run_id: run_id.clone(),
            request_id: request(1),
            run_token: RunToken::new("ab".repeat(32)).unwrap(),
            profile: ProfileName::new("gate").unwrap(),
            owner_uid: 1000,
            owner_pid: 4321,
            owner_starttime: 987_654,
        })
        .expect("persist run");
    let object_id = ObjectId::new("22222222222222222222222222222222").unwrap();
    journal
        .begin_intent(&IntentRecord {
            object_id: object_id.clone(),
            request_id: request(2),
            run_id: run_id.clone(),
            role: RoleName::new("target").unwrap(),
            root_alias: RootAlias::new("target").unwrap(),
            leaf: LeafName::new("proof-leaf").unwrap(),
        })
        .expect("persist intent");
    (run_id, object_id)
}
