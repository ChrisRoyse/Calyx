use super::*;
use proptest::prelude::*;

#[test]
fn freshness_policy_fails_closed_when_derived_is_too_old() {
    Freshness::FreshDerived
        .ensure(10, 10)
        .expect("same seq is fresh");
    Freshness::StaleOk { max_lag: 2 }
        .ensure(10, 8)
        .expect("bounded lag accepted");

    let fresh_error = Freshness::FreshDerived
        .ensure(10, 9)
        .expect_err("fresh required");
    let stale_error = Freshness::StaleOk { max_lag: 2 }
        .ensure(10, 7)
        .expect_err("lag too large");

    assert_eq!(fresh_error.code, "CALYX_STALE_DERIVED");
    assert_eq!(stale_error.code, "CALYX_STALE_DERIVED");
}

#[test]
fn reader_lease_expiration_fails_closed() {
    let lease = ReaderLease::new(1, 7, 100, 5);
    let live = FixedClock::new(104);
    let expired = FixedClock::new(105);

    lease.ensure_live(&live).expect("lease still live");
    let error = lease.ensure_live(&expired).expect_err("lease expired");

    assert_eq!(error.code, "CALYX_READER_LEASE_EXPIRED");

    let zero = ReaderLease::new(2, 7, 100, 0);
    assert!(zero.is_expired(&FixedClock::new(100)));
    let max = ReaderLease::new(3, 7, 100, u64::MAX);
    assert!(!max.is_expired(&FixedClock::new(u64::MAX - 1)));
}

#[test]
fn expired_snapshot_read_releases_pin_and_returns_calyx_code() {
    let store = VersionedCfStore::default();
    store
        .commit_batch([(ColumnFamily::Base, b"k".to_vec(), b"v".to_vec())])
        .unwrap();
    let issued = FixedClock::new(100);
    let expired = FixedClock::new(105);
    let snapshot = store.pin_snapshot(Freshness::FreshDerived, &issued, 5);
    assert_eq!(store.lease_view(104).active_leases, 1);

    let error = store
        .read_at(snapshot, ColumnFamily::Base, b"k", &expired)
        .expect_err("read must fail closed after lease expiry");

    assert_eq!(error.code, "CALYX_READER_LEASE_EXPIRED");
    let view = store.lease_view(105);
    assert_eq!(view.active_leases, 0);
    assert_eq!(view.reader_lease_expired_total, 1);
}

#[test]
fn freshness_boundaries_and_snapshot_policy_are_enforced() {
    Freshness::FreshDerived.ensure(10, 10).unwrap();
    assert_eq!(
        Freshness::FreshDerived.ensure(10, 9).unwrap_err().code,
        "CALYX_STALE_DERIVED"
    );
    Freshness::StaleOk { max_lag: 5 }.ensure(10, 5).unwrap();
    assert_eq!(
        Freshness::StaleOk { max_lag: 5 }
            .ensure(10, 4)
            .unwrap_err()
            .code,
        "CALYX_STALE_DERIVED"
    );
    Freshness::StaleOk { max_lag: 0 }.ensure(10, 10).unwrap();
    assert_eq!(
        Freshness::StaleOk { max_lag: 0 }
            .ensure(10, 9)
            .unwrap_err()
            .code,
        "CALYX_STALE_DERIVED"
    );
    Freshness::FreshDerived.ensure(10, 11).unwrap();
    Freshness::StaleOk { max_lag: u64::MAX }
        .ensure(u64::MAX, 0)
        .unwrap();
    Freshness::FreshDerived.ensure(0, 0).unwrap();

    let store = VersionedCfStore::default();
    let snapshot = store.pin_snapshot(Freshness::StaleOk { max_lag: 3 }, &FixedClock::new(1), 10);
    assert_eq!(snapshot.freshness(), Freshness::StaleOk { max_lag: 3 });
    assert!(
        snapshot
            .freshness()
            .ensure(snapshot.seq(), snapshot.seq())
            .is_ok()
    );
    println!("MVCC_FRESHNESS_STALE_ERROR CALYX_STALE_DERIVED");
}

proptest! {
    #[test]
    fn stale_ok_matches_lag_predicate(pinned in any::<u64>(), derived in any::<u64>(), max_lag in any::<u64>()) {
        let result = Freshness::StaleOk { max_lag }.ensure(pinned, derived);
        let expected = derived >= pinned || pinned.saturating_sub(derived) <= max_lag;
        prop_assert_eq!(result.is_ok(), expected);
    }
}
