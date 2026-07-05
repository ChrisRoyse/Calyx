use calyx_core::{CalyxError, Result};

const PIPELINE_ENV: &str = "CALYX_ONNX_CUSTOM_PIPELINE";
const BATCH_WINDOW_ENV: &str = "CALYX_ONNX_CUSTOM_PIPELINE_BATCH_WINDOW";
const OUTPUT_WINDOW_ENV: &str = "CALYX_ONNX_CUSTOM_PIPELINE_OUTPUT_WINDOW";
const DEFAULT_BATCH_WINDOW: usize = 16;
const DEFAULT_OUTPUT_WINDOW: usize = 2;

pub(super) fn custom_pipeline_enabled() -> bool {
    match std::env::var(PIPELINE_ENV) {
        Ok(value) => {
            let value = value.trim();
            !(value.eq_ignore_ascii_case("0")
                || value.eq_ignore_ascii_case("false")
                || value.eq_ignore_ascii_case("off")
                || value.eq_ignore_ascii_case("no"))
        }
        Err(_) => true,
    }
}

pub(super) fn should_pipeline(input_len: usize, max_batch: Option<usize>) -> bool {
    custom_pipeline_enabled() && max_batch.is_some_and(|limit| input_len > limit)
}

pub(super) fn pipeline_batch_window() -> Result<usize> {
    positive_env_usize(BATCH_WINDOW_ENV, DEFAULT_BATCH_WINDOW)
}

pub(super) fn pipeline_output_window() -> Result<usize> {
    positive_env_usize(OUTPUT_WINDOW_ENV, DEFAULT_OUTPUT_WINDOW)
}

pub(super) fn log_pipeline_start(
    inputs: usize,
    batch_window: usize,
    output_window: usize,
    max_batch: Option<usize>,
) {
    eprintln!(
        "CALYX_ONNX_RUNTIME phase=custom_pipeline_start inputs={inputs} batch_window={batch_window} output_window={output_window} internal_max_batch={}",
        max_batch
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    );
}

fn positive_env_usize(name: &str, default: usize) -> Result<usize> {
    let Ok(raw) = std::env::var(name) else {
        return Ok(default);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(default);
    }
    raw.parse::<usize>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| CalyxError {
            code: "CALYX_ONNX_CUSTOM_PIPELINE_WINDOW_INVALID",
            message: format!("{name}={raw} is not a positive integer"),
            remediation: "set custom ONNX pipeline window env vars to positive integers",
        })
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::{
        BATCH_WINDOW_ENV, OUTPUT_WINDOW_ENV, PIPELINE_ENV, custom_pipeline_enabled,
        pipeline_batch_window, pipeline_output_window, should_pipeline,
    };

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn custom_pipeline_env_defaults_on_and_accepts_off_values() {
        let _guard = env_lock().lock().unwrap();
        let old = std::env::var_os(PIPELINE_ENV);

        unsafe { std::env::remove_var(PIPELINE_ENV) };
        assert!(custom_pipeline_enabled());
        assert!(should_pipeline(9, Some(4)));
        assert!(!should_pipeline(4, Some(4)));
        assert!(!should_pipeline(9, None));

        for value in ["0", "false", "off", "no"] {
            unsafe { std::env::set_var(PIPELINE_ENV, value) };
            assert!(!custom_pipeline_enabled());
        }

        unsafe { std::env::set_var(PIPELINE_ENV, "on") };
        assert!(custom_pipeline_enabled());

        unsafe {
            match old {
                Some(value) => std::env::set_var(PIPELINE_ENV, value),
                None => std::env::remove_var(PIPELINE_ENV),
            }
        }
    }

    #[test]
    fn custom_pipeline_windows_default_and_fail_closed_on_invalid_values() {
        let _guard = env_lock().lock().unwrap();
        let old_batch = std::env::var_os(BATCH_WINDOW_ENV);
        let old_output = std::env::var_os(OUTPUT_WINDOW_ENV);

        unsafe {
            std::env::remove_var(BATCH_WINDOW_ENV);
            std::env::remove_var(OUTPUT_WINDOW_ENV);
        }
        assert_eq!(pipeline_batch_window().unwrap(), 16);
        assert_eq!(pipeline_output_window().unwrap(), 2);

        unsafe {
            std::env::set_var(BATCH_WINDOW_ENV, "8");
            std::env::set_var(OUTPUT_WINDOW_ENV, "3");
        }
        assert_eq!(pipeline_batch_window().unwrap(), 8);
        assert_eq!(pipeline_output_window().unwrap(), 3);

        unsafe { std::env::set_var(BATCH_WINDOW_ENV, "0") };
        assert_eq!(
            pipeline_batch_window().unwrap_err().code,
            "CALYX_ONNX_CUSTOM_PIPELINE_WINDOW_INVALID"
        );
        unsafe { std::env::set_var(OUTPUT_WINDOW_ENV, "nope") };
        assert_eq!(
            pipeline_output_window().unwrap_err().code,
            "CALYX_ONNX_CUSTOM_PIPELINE_WINDOW_INVALID"
        );

        unsafe {
            match old_batch {
                Some(value) => std::env::set_var(BATCH_WINDOW_ENV, value),
                None => std::env::remove_var(BATCH_WINDOW_ENV),
            }
            match old_output {
                Some(value) => std::env::set_var(OUTPUT_WINDOW_ENV, value),
                None => std::env::remove_var(OUTPUT_WINDOW_ENV),
            }
        }
    }
}
