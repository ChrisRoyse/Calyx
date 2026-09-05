use std::sync::{Arc, mpsc};

use crate::protocol::StableCode;
use crate::transport::{SeqpacketConnection, SeqpacketListener};

use super::{ConnectionCounter, connection_overload_error};

#[test]
fn real_connections_are_bounded_and_panic_releases_slot() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("capacity.sock");
    let listener = SeqpacketListener::bind(&path, 4).unwrap();
    let counter = Arc::new(ConnectionCounter::new(1));

    let first_client = SeqpacketConnection::connect(&path).unwrap();
    let first_server = listener.accept().unwrap();
    let first_slot = counter.try_acquire().unwrap();
    assert_eq!(counter.active(), 1);

    let second_client = SeqpacketConnection::connect(&path).unwrap();
    let second_server = listener.accept().unwrap();
    let capacity = counter.try_acquire().unwrap_err();
    assert_eq!(capacity.active, 1);
    assert_eq!(capacity.limit, 1);
    let overload = connection_overload_error(&second_server, capacity);
    assert_eq!(overload.code, StableCode::Busy);
    assert_eq!(overload.context["active_connections"], "1");
    overload.log("connection_overload_test");
    eprintln!(
        "EDGE overload before=active:1 after=Busy peer_uid={}",
        overload.context["peer_uid"]
    );

    let (ready_sender, ready_receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let _connection = first_server;
        let _slot = first_slot;
        ready_sender.send(()).unwrap();
        panic!("synthetic connection worker panic");
    });
    ready_receiver.recv().unwrap();
    let panic = worker.join().unwrap_err();
    assert_eq!(
        panic.downcast_ref::<&str>(),
        Some(&"synthetic connection worker panic")
    );
    assert_eq!(counter.active(), 0);
    eprintln!("SOURCE_OF_TRUTH connection_slots before_panic=1 after_panic=0");

    let recovered_slot = counter.try_acquire().unwrap();
    assert_eq!(counter.active(), 1);
    drop(recovered_slot);
    assert_eq!(counter.active(), 0);
    drop((first_client, second_client, second_server));
}
