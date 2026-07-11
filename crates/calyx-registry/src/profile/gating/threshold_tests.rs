use std::sync::Mutex;

use super::*;

static CAPABILITY_ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn environment_overrides_cannot_weaken_contract_thresholds() {
    let _lock = CAPABILITY_ENV_LOCK.lock().unwrap();
    let old_min = env::var_os(CAPABILITY_MIN_SIGNAL_BITS_ENV);
    let old_max = env::var_os(CAPABILITY_MAX_PAIRWISE_CORR_ENV);

    unsafe {
        env::set_var(CAPABILITY_MIN_SIGNAL_BITS_ENV, "0");
        env::set_var(
            CAPABILITY_MAX_PAIRWISE_CORR_ENV,
            MAX_PAIRWISE_CORR.to_string(),
        );
    }
    let weak_min = CapabilityGateThresholds::from_env().unwrap_err();

    unsafe {
        env::set_var(CAPABILITY_MIN_SIGNAL_BITS_ENV, MIN_SIGNAL_BITS.to_string());
        env::set_var(CAPABILITY_MAX_PAIRWISE_CORR_ENV, "1.0");
    }
    let weak_max = CapabilityGateThresholds::from_env().unwrap_err();

    unsafe {
        match old_min {
            Some(value) => env::set_var(CAPABILITY_MIN_SIGNAL_BITS_ENV, value),
            None => env::remove_var(CAPABILITY_MIN_SIGNAL_BITS_ENV),
        }
        match old_max {
            Some(value) => env::set_var(CAPABILITY_MAX_PAIRWISE_CORR_ENV, value),
            None => env::remove_var(CAPABILITY_MAX_PAIRWISE_CORR_ENV),
        }
    }

    assert_eq!(weak_min.code, "CALYX_ASSAY_LOW_SIGNAL");
    assert_eq!(weak_max.code, "CALYX_ASSAY_REDUNDANT");
}
