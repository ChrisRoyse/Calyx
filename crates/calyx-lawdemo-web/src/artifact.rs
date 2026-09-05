//! Artifact mode — the GPU-free, SHA-verified corpus-census spine.
//!
//! `GET /api/demo/4` reads the configured historical corpus-census JSON from the
//! evidence dir and verifies the bytes against the pinned SHA-256 BEFORE parsing or
//! serving. The live console does not consume this artifact as substitute state. A
//! mismatch (or an absent file, or non-JSON bytes) is a structured refusal — the
//! service NEVER serves unverified bytes. On success the response is
//! `{demo, artifact, sha256, verified: true, provenance}`.

use std::path::Path;

use axum::{
    Json,
    extract::{Path as AxumPath, State},
    response::{IntoResponse, Response},
};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::config::{ArtifactSpec, Config};
use crate::error::{ApiError, ErrorCode};

/// Lowercase-hex encode a byte slice (avoids pulling in a `hex` dependency).
fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    out
}

/// A verified artifact ready to serve: the parsed JSON plus its confirmed hash.
#[derive(Debug)]
pub struct VerifiedArtifact {
    pub value: serde_json::Value,
    pub sha256: String,
}

/// Read `spec.filename` from `evidence_dir`, confirm its SHA-256 equals the pin,
/// then parse it as JSON. Every failure mode is a distinct structured error.
pub fn load_and_verify(
    evidence_dir: &Path,
    spec: &ArtifactSpec,
) -> Result<VerifiedArtifact, ApiError> {
    let path = evidence_dir.join(&spec.filename);
    let bytes = std::fs::read(&path).map_err(|_| {
        ApiError::new(
            ErrorCode::ArtifactMissing,
            format!(
                "demo {} artifact {} is not readable",
                spec.demo, spec.filename
            ),
        )
    })?;

    let digest = to_hex(&Sha256::digest(&bytes));
    if digest != spec.sha256_hex {
        // Never serve unverified bytes: report the mismatch, not the content.
        return Err(ApiError::new(
            ErrorCode::ArtifactUnverified,
            format!(
                "demo {} artifact {} SHA-256 mismatch (expected {}, computed {})",
                spec.demo, spec.filename, spec.sha256_hex, digest
            ),
        ));
    }

    let value: serde_json::Value = serde_json::from_slice(&bytes).map_err(|_| {
        ApiError::new(
            ErrorCode::ArtifactUnverified,
            format!(
                "demo {} artifact {} verified its hash but is not valid JSON",
                spec.demo, spec.filename
            ),
        )
    })?;

    Ok(VerifiedArtifact {
        value,
        sha256: digest,
    })
}

/// Build the `provenance` block describing where a served artifact came from.
fn provenance(cfg: &Config, spec: &ArtifactSpec) -> serde_json::Value {
    json!({
        "filename": spec.filename,
        "evidence_label": cfg.evidence_label,
        "sha256_pin": spec.sha256_hex,
        "mode": "artifact",
        "note": "bytes verified against the configured SHA-256 pin before serving",
    })
}

/// `GET /api/demo/{demo}` — artifact mode handler. The console consumes only the
/// demo-4 corpus census; any other index is an unconfigured artifact.
pub async fn demo_handler(State(state): State<AppState>, AxumPath(demo): AxumPath<u8>) -> Response {
    let cfg = &state.config;
    let Some(spec) = cfg.artifact(demo) else {
        return ApiError::new(
            ErrorCode::BadRequest,
            format!("demo index {demo} is not a configured artifact (the console uses demo 4)"),
        )
        .into_response();
    };

    match load_and_verify(&cfg.evidence_dir, spec) {
        Ok(verified) => Json(json!({
            "demo": demo,
            "verified": true,
            "sha256": verified.sha256,
            "provenance": provenance(cfg, spec),
            "artifact": verified.value,
        }))
        .into_response(),
        Err(err) => err.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(name: &str, contents: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "calyx-lawdemo-artifact-{}-{}",
            std::process::id(),
            name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), contents).unwrap();
        dir
    }

    #[test]
    fn hex_encoding_is_lowercase_and_padded() {
        assert_eq!(to_hex(&[0x00, 0x0f, 0xff, 0xa0]), "000fffa0");
    }

    #[test]
    fn verified_bytes_are_served() {
        let body = br#"{"physical_opinions_n":12596}"#;
        let sha = to_hex(&Sha256::digest(body));
        let dir = write_tmp("ok.json", body);
        let spec = ArtifactSpec {
            demo: 4,
            filename: "ok.json".into(),
            sha256_hex: sha.clone(),
        };
        let out = load_and_verify(&dir, &spec).expect("verified");
        assert_eq!(out.sha256, sha);
        assert_eq!(out.value["physical_opinions_n"], 12596);
    }

    #[test]
    fn sha_mismatch_is_a_structured_refusal() {
        let dir = write_tmp("tamper.json", b"{\"n\":1}");
        let spec = ArtifactSpec {
            demo: 1,
            filename: "tamper.json".into(),
            sha256_hex: "0".repeat(64),
        };
        let err = load_and_verify(&dir, &spec).unwrap_err();
        assert_eq!(err.code(), ErrorCode::ArtifactUnverified);
        assert!(err.body()["message"].as_str().unwrap().contains("mismatch"));
    }

    #[test]
    fn absent_file_is_artifact_missing() {
        let dir = std::env::temp_dir().join("calyx-lawdemo-nope-dir");
        let spec = ArtifactSpec {
            demo: 2,
            filename: "absent.json".into(),
            sha256_hex: "0".repeat(64),
        };
        let err = load_and_verify(&dir, &spec).unwrap_err();
        assert_eq!(err.code(), ErrorCode::ArtifactMissing);
    }

    #[test]
    fn verified_but_non_json_is_refused() {
        let body = b"this is not json";
        let sha = to_hex(&Sha256::digest(body));
        let dir = write_tmp("bad.json", body);
        let spec = ArtifactSpec {
            demo: 1,
            filename: "bad.json".into(),
            sha256_hex: sha,
        };
        let err = load_and_verify(&dir, &spec).unwrap_err();
        assert_eq!(err.code(), ErrorCode::ArtifactUnverified);
    }
}
