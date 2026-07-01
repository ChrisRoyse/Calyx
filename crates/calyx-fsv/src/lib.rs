//! Canonical resolution of FSV evidence-root environment variables.
//!
//! Cargo sets the working directory of every unit/integration test to the
//! root of the crate under test, not the workspace root, so a relative
//! `CALYX_FSV_ROOT` scatters evidence under `crates/<name>/...` while
//! operators read back `target/fsv/...` at the workspace root (issue #1014).
//! An evidence root is only deterministic when it is absolute, so a set
//! value that is empty or relative is a fatal configuration error: every
//! consumer in the workspace must resolve the variable through this crate
//! and fail closed instead of writing to a cwd-dependent location.

use std::ffi::OsString;
use std::path::PathBuf;

/// The workspace-wide FSV evidence root variable.
pub const FSV_ROOT_ENV: &str = "CALYX_FSV_ROOT";

/// A set-but-unusable FSV root variable. Fatal by design: there is no
/// correct directory to fall back to once the operator asked for a
/// specific evidence root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsvRootError {
    /// Stable machine-readable code: `CALYX_FSV_ROOT_EMPTY` or
    /// `CALYX_FSV_ROOT_NOT_ABSOLUTE`.
    pub code: &'static str,
    /// The environment variable that held the rejected value.
    pub var: String,
    /// The rejected value, byte-for-byte.
    pub value: OsString,
    /// The process working directory the relative value would have
    /// silently resolved against.
    pub cwd: PathBuf,
}

impl std::fmt::Display for FsvRootError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{code} var={var} value={value:?} cwd={cwd} remediation=\"set {var} to an \
             absolute path; relative values resolve against the per-crate test cwd and \
             scatter FSV artifacts (issue #1014)\"",
            code = self.code,
            var = self.var,
            value = self.value,
            cwd = self.cwd.display(),
        )
    }
}

impl std::error::Error for FsvRootError {}

/// Reads `var` as an FSV evidence root.
///
/// Unset means the caller owns its root (`Ok(None)`); a set value must be
/// an absolute path or the caller gets a structured [`FsvRootError`].
pub fn env_fsv_root(var: &str) -> Result<Option<PathBuf>, FsvRootError> {
    let Some(raw) = std::env::var_os(var) else {
        return Ok(None);
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("<unknown-cwd>"));
    if raw.is_empty() {
        return Err(FsvRootError {
            code: "CALYX_FSV_ROOT_EMPTY",
            var: var.to_string(),
            value: raw,
            cwd,
        });
    }
    let path = PathBuf::from(&raw);
    if !path.is_absolute() {
        return Err(FsvRootError {
            code: "CALYX_FSV_ROOT_NOT_ABSOLUTE",
            var: var.to_string(),
            value: raw,
            cwd,
        });
    }
    Ok(Some(path))
}

/// [`env_fsv_root`] for tests: panics with the structured message on a
/// set-but-invalid value.
pub fn fsv_root(var: &str) -> Option<PathBuf> {
    match env_fsv_root(var) {
        Ok(root) => root,
        Err(error) => panic!("{error}"),
    }
}

/// [`fsv_root`] with a caller-owned fallback for when `var` is unset.
pub fn fsv_root_or_else(var: &str, fallback: impl FnOnce() -> PathBuf) -> PathBuf {
    fsv_root(var).unwrap_or_else(fallback)
}

/// [`fsv_root`] for manual-FSV tests that cannot run without an operator
/// supplied evidence root: panics when `var` is unset or invalid.
pub fn required_fsv_root(var: &str) -> PathBuf {
    fsv_root(var).unwrap_or_else(|| {
        panic!(
            "CALYX_FSV_ROOT_UNSET var={var} remediation=\"set {var} to an absolute \
             evidence directory before running this manual FSV\""
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_var(var: &str, value: &str) {
        // SAFETY: each test uses a variable name unique to itself, so no
        // other thread in this test binary reads or writes it concurrently.
        unsafe { std::env::set_var(var, value) };
    }

    #[test]
    fn unset_var_resolves_to_none() {
        assert_eq!(env_fsv_root("CALYX_FSV_TEST_UNSET"), Ok(None));
        assert_eq!(fsv_root("CALYX_FSV_TEST_UNSET"), None);
    }

    #[test]
    fn absolute_var_resolves_to_path() {
        let root = std::env::temp_dir().join("calyx-fsv-abs");
        assert!(root.is_absolute());
        set_var("CALYX_FSV_TEST_ABS", root.to_str().unwrap());
        assert_eq!(env_fsv_root("CALYX_FSV_TEST_ABS"), Ok(Some(root.clone())));
        assert_eq!(fsv_root("CALYX_FSV_TEST_ABS"), Some(root.clone()));
        assert_eq!(required_fsv_root("CALYX_FSV_TEST_ABS"), root);
    }

    #[test]
    fn relative_var_fails_closed_with_structured_error() {
        set_var("CALYX_FSV_TEST_REL", "target/fsv/issue1014");
        let error = env_fsv_root("CALYX_FSV_TEST_REL").unwrap_err();
        assert_eq!(error.code, "CALYX_FSV_ROOT_NOT_ABSOLUTE");
        assert_eq!(error.var, "CALYX_FSV_TEST_REL");
        assert_eq!(error.value, OsString::from("target/fsv/issue1014"));
        assert_eq!(error.cwd, std::env::current_dir().unwrap());
        let message = error.to_string();
        assert!(message.contains("CALYX_FSV_ROOT_NOT_ABSOLUTE"));
        assert!(message.contains("target/fsv/issue1014"));
        assert!(message.contains("issue #1014"));
    }

    #[test]
    fn empty_var_fails_closed_with_structured_error() {
        set_var("CALYX_FSV_TEST_EMPTY", "");
        let error = env_fsv_root("CALYX_FSV_TEST_EMPTY").unwrap_err();
        assert_eq!(error.code, "CALYX_FSV_ROOT_EMPTY");
        assert!(error.to_string().contains("CALYX_FSV_ROOT_EMPTY"));
    }

    #[test]
    #[should_panic(expected = "CALYX_FSV_ROOT_NOT_ABSOLUTE")]
    fn fsv_root_panics_on_relative_value() {
        set_var("CALYX_FSV_TEST_REL_PANIC", "target/fsv/relative");
        let _ = fsv_root("CALYX_FSV_TEST_REL_PANIC");
    }

    #[test]
    #[should_panic(expected = "CALYX_FSV_ROOT_UNSET")]
    fn required_fsv_root_panics_when_unset() {
        let _ = required_fsv_root("CALYX_FSV_TEST_REQUIRED_UNSET");
    }

    #[test]
    fn fallback_is_used_only_when_unset() {
        let fallback = std::env::temp_dir().join("calyx-fsv-fallback");
        let resolved = fsv_root_or_else("CALYX_FSV_TEST_FALLBACK_UNSET", || fallback.clone());
        assert_eq!(resolved, fallback);

        let root = std::env::temp_dir().join("calyx-fsv-set");
        set_var("CALYX_FSV_TEST_FALLBACK_SET", root.to_str().unwrap());
        let resolved = fsv_root_or_else("CALYX_FSV_TEST_FALLBACK_SET", || {
            panic!("fallback must not run when the variable is set")
        });
        assert_eq!(resolved, root);
    }

    #[cfg(windows)]
    #[test]
    fn drive_relative_value_fails_closed() {
        set_var("CALYX_FSV_TEST_DRIVE_REL", r"C:target\fsv");
        let error = env_fsv_root("CALYX_FSV_TEST_DRIVE_REL").unwrap_err();
        assert_eq!(error.code, "CALYX_FSV_ROOT_NOT_ABSOLUTE");
    }
}
