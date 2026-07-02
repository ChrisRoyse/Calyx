//! Embedded build identity for deployed Calyx binaries (issue #1108).
//!
//! The 2026-06-27 runner-binary incident (#1108) happened because deployed
//! binaries carried no self-identity: the only staleness signal was file
//! mtime, so a runner silently served pre-#1058 behavior for four days.
//! This crate closes that hole in two halves:
//!
//! * **Build side** — each deployed binary crate calls [`emit`] from its
//!   `build.rs`. It reads the git commit SHA, a tracked-files dirty flag, and
//!   the commit timestamp, then re-exports them as `CALYX_BUILD_*` rustc env
//!   vars. There is no fallback: a build outside a git checkout fails loudly
//!   with `CALYX_BUILD_INFO_GIT_UNAVAILABLE` instead of embedding "unknown".
//! * **Runtime side** — the binary constructs a [`BuildInfo`] via
//!   [`build_info!`] and surfaces it (`calyx build-info`, `calyxd
//!   --build-info`, `calyx-mcp --build-info`, healthcheck JSON) so deploy
//!   tooling can compare the deployed identity against `origin/main`.
//!
//! The commit timestamp (not the build wall clock) is embedded so identical
//! sources produce identical identity and incremental builds do not churn.
//! The dirty flag is recomputed only when the build script reruns (HEAD or
//! index changes); the deploy gate independently refuses dirty worktrees.

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

/// Identity of the running binary, embedded at compile time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BuildInfo {
    /// Cargo package that produced the binary (e.g. `calyx-cli`).
    pub package: &'static str,
    /// Cargo package version (workspace version).
    pub package_version: &'static str,
    /// Full 40-hex git commit SHA the binary was built from.
    pub git_sha: &'static str,
    /// True when tracked files differed from `git_sha` at build-script time.
    pub git_dirty: bool,
    /// Committer timestamp of `git_sha` (unix seconds).
    pub git_commit_unix_secs: u64,
    /// Cargo features enabled when the binary crate was built (sorted,
    /// hyphenated lowercase), read back from `CARGO_FEATURE_*` in `emit`.
    /// A non-cuda `calyxd` took the service down on 2026-07-02 because no
    /// gate could see the feature set (#1116); deploy gates compare this
    /// list against a per-binary required-feature policy.
    ///
    /// Cargo mangles feature names to `CARGO_FEATURE_<UPPER_SNAKE>`, so `_`
    /// and `-` are indistinguishable here; names are reported in hyphen form
    /// (every Calyx feature uses hyphens).
    pub features: Vec<&'static str>,
}

/// Constructs the [`BuildInfo`] embedded by this crate's [`emit`] build step.
///
/// Panics (with the underlying validation message) if the embedded values are
/// malformed — that can only happen if a binary bypassed [`emit`].
#[macro_export]
macro_rules! build_info {
    () => {
        $crate::BuildInfo::from_embedded(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            env!("CALYX_BUILD_GIT_SHA"),
            env!("CALYX_BUILD_GIT_DIRTY"),
            env!("CALYX_BUILD_GIT_COMMIT_UNIX_SECS"),
            env!("CALYX_BUILD_FEATURES"),
        )
        .expect("CALYX_BUILD_INFO_INVALID: embedded build identity is malformed")
    };
}

impl BuildInfo {
    /// Validates and assembles the embedded identity values.
    pub fn from_embedded(
        package: &'static str,
        package_version: &'static str,
        git_sha: &'static str,
        git_dirty: &'static str,
        git_commit_unix_secs: &'static str,
        features: &'static str,
    ) -> Result<Self, String> {
        validate_sha(git_sha)?;
        let git_dirty = match git_dirty {
            "0" => false,
            "1" => true,
            other => {
                return Err(format!(
                    "CALYX_BUILD_INFO_INVALID: dirty flag must be 0 or 1, got {other:?}"
                ));
            }
        };
        let git_commit_unix_secs = git_commit_unix_secs.parse::<u64>().map_err(|error| {
            format!("CALYX_BUILD_INFO_INVALID: commit timestamp {git_commit_unix_secs:?}: {error}")
        })?;
        Ok(Self {
            package,
            package_version,
            git_sha,
            git_dirty,
            git_commit_unix_secs,
            features: parse_embedded_features(features)?,
        })
    }
}

/// Parses the comma-joined feature list embedded by [`emit`]. Empty input
/// means no features (a valid state, e.g. `calyx-mcp` declares none); any
/// malformed token means the binary bypassed [`emit`] and fails loudly.
fn parse_embedded_features(raw: &'static str) -> Result<Vec<&'static str>, String> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let features: Vec<&'static str> = raw.split(',').collect();
    for feature in &features {
        let valid = !feature.is_empty()
            && feature
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid {
            return Err(format!(
                "CALYX_BUILD_INFO_INVALID: feature name must be lowercase hyphenated ascii, got {feature:?} in {raw:?}"
            ));
        }
    }
    Ok(features)
}

fn validate_sha(sha: &str) -> Result<(), String> {
    if sha.len() == 40
        && sha
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(format!(
            "CALYX_BUILD_INFO_INVALID: git sha must be 40 lowercase hex chars, got {sha:?}"
        ))
    }
}

/// Values computed from the git checkout for one build-script run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EmittedBuildInfo {
    pub git_sha: String,
    pub git_dirty: bool,
    pub git_commit_unix_secs: u64,
    /// Files whose changes must rerun the build script (HEAD, index, refs).
    pub rerun_paths: Vec<PathBuf>,
}

/// Build-script entry point: computes the git identity for
/// `$CARGO_MANIFEST_DIR` and prints the cargo directives that embed it.
///
/// Panics with `CALYX_BUILD_INFO_GIT_UNAVAILABLE` when the crate is built
/// outside a usable git checkout — deployed Calyx binaries must never exist
/// without a verifiable identity.
pub fn emit() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CALYX_BUILD_INFO_GIT_UNAVAILABLE: CARGO_MANIFEST_DIR is not set");
    let info = compute_for_dir(&manifest_dir).unwrap_or_else(|error| panic!("{error}"));
    for path in &info.rerun_paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rustc-env=CALYX_BUILD_GIT_SHA={}", info.git_sha);
    println!(
        "cargo:rustc-env=CALYX_BUILD_GIT_DIRTY={}",
        if info.git_dirty { "1" } else { "0" }
    );
    println!(
        "cargo:rustc-env=CALYX_BUILD_GIT_COMMIT_UNIX_SECS={}",
        info.git_commit_unix_secs
    );
    // #1116: embed the enabled cargo feature set so deploy gates can verify
    // the artifact configuration (e.g. calyxd on gpuhost REQUIRES cuda).
    // Cargo re-runs build scripts when the feature set changes, so this
    // stays consistent with the compiled binary.
    println!(
        "cargo:rustc-env=CALYX_BUILD_FEATURES={}",
        features_from_env_keys(std::env::vars().map(|(key, _)| key)).join(",")
    );
}

/// Maps the build script's `CARGO_FEATURE_*` environment (one variable per
/// enabled feature, name uppercased with `-` mangled to `_`) back to a
/// sorted, deduplicated, hyphenated-lowercase feature list.
pub fn features_from_env_keys<I: IntoIterator<Item = String>>(keys: I) -> Vec<String> {
    let mut features: Vec<String> = keys
        .into_iter()
        .filter_map(|key| {
            key.strip_prefix("CARGO_FEATURE_")
                .map(|name| name.to_ascii_lowercase().replace('_', "-"))
        })
        .collect();
    features.sort_unstable();
    features.dedup();
    features
}

/// Computes the build identity for a directory inside a git checkout.
pub fn compute_for_dir(dir: &str) -> Result<EmittedBuildInfo, String> {
    let git_sha = git_output(dir, &["rev-parse", "HEAD"])?;
    validate_sha(&git_sha)?;
    let git_commit_unix_secs = git_output(dir, &["show", "-s", "--format=%ct", "HEAD"])?
        .parse::<u64>()
        .map_err(|error| {
            format!("CALYX_BUILD_INFO_GIT_UNAVAILABLE: parse HEAD commit timestamp: {error}")
        })?;
    let status = git_output(dir, &["status", "--porcelain", "--untracked-files=no"])?;
    Ok(EmittedBuildInfo {
        git_sha,
        git_dirty: !status.is_empty(),
        git_commit_unix_secs,
        rerun_paths: rerun_paths(dir)?,
    })
}

/// HEAD and index for this worktree, plus the loose ref file behind a
/// symbolic HEAD. A packed (absent) loose ref makes cargo rerun the build
/// script each build, which keeps the identity fresh at a small cost.
fn rerun_paths(dir: &str) -> Result<Vec<PathBuf>, String> {
    let git_dir = PathBuf::from(git_output(dir, &["rev-parse", "--absolute-git-dir"])?);
    let mut paths = vec![git_dir.join("HEAD"), git_dir.join("index")];
    if let Ok(symbolic_ref) = git_output(dir, &["symbolic-ref", "-q", "HEAD"]) {
        let common_dir = git_output(dir, &["rev-parse", "--git-common-dir"])?;
        let mut common_dir = PathBuf::from(common_dir);
        if common_dir.is_relative() {
            common_dir = PathBuf::from(dir).join(common_dir);
        }
        paths.push(common_dir.join(symbolic_ref));
    }
    Ok(paths)
}

fn git_output(dir: &str, args: &[&str]) -> Result<String, String> {
    // --no-optional-locks: concurrent build scripts (calyx-cli, calyxd,
    // calyx-mcp) must not contend on index.lock during `git status`.
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| {
            format!("CALYX_BUILD_INFO_GIT_UNAVAILABLE: spawn git {args:?} in {dir}: {error}")
        })?;
    if !output.status.success() {
        return Err(format!(
            "CALYX_BUILD_INFO_GIT_UNAVAILABLE: git {args:?} in {dir} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|stdout| stdout.trim().to_string())
        .map_err(|error| {
            format!("CALYX_BUILD_INFO_GIT_UNAVAILABLE: git {args:?} output not utf-8: {error}")
        })
}

#[cfg(test)]
mod tests;
