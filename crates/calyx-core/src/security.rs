//! Canonical transport-security and authentication types (PRD 30 §2).
//!
//! These types live in `calyx-core` so both `calyx-aster` and `calyxd` can
//! reference them without a circular dependency. The central invariant is
//! [`no_anonymous_write`]: every mutation entry point must present an [`AuthN`]
//! identity or be rejected with [`CALYX_AUTHN_REQUIRED`] before any write
//! reaches the vault — fail-closed (A16), never a silent allow.
//!
//! Error codes here are module-local `pub const` strings (the same pattern as
//! [`crate::temporal`]); they are intentionally *not* part of the closed PRD-18
//! catalog (`CALYX_ERROR_CODES`), which `catalog_matches_prd_18_exactly` pins to
//! PRD 18 exactly. See PR/issue notes for the governance follow-up.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::{CalyxError, Result};

/// A mutation was attempted without an authenticated principal (PRD 30 §2,
/// "no anonymous writes"). Fail-closed (A16).
pub const CALYX_AUTHN_REQUIRED: &str = "CALYX_AUTHN_REQUIRED";
/// A TLS configuration references a cert/key/CA path that does not exist or is
/// not readable (PRD 30 §2, "crypto in transit").
pub const CALYX_TLS_CONFIG_INVALID: &str = "CALYX_TLS_CONFIG_INVALID";

/// Server-mode TLS material. When `ca_pem_path` is present the server can verify
/// client certificates, i.e. run mutual TLS (see [`MtlsConfig`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TlsConfig {
    /// Path to the server certificate chain (PEM).
    pub cert_pem_path: PathBuf,
    /// Path to the server private key (PEM).
    pub key_pem_path: PathBuf,
    /// Optional path to the CA bundle used to verify client certs (PEM).
    /// `Some` enables mutual TLS; `None` is server-auth-only TLS.
    pub ca_pem_path: Option<PathBuf>,
}

impl TlsConfig {
    /// Verifies that the configured PEM paths exist and are readable.
    ///
    /// Metadata-only — this does **not** parse the PEM contents (that is the
    /// TLS stack's job at bind time). Returns [`CALYX_TLS_CONFIG_INVALID`] naming
    /// the first offending path so the operator knows exactly what to fix.
    pub fn validate(&self) -> Result<()> {
        Self::check_readable("cert_pem_path", &self.cert_pem_path)?;
        Self::check_readable("key_pem_path", &self.key_pem_path)?;
        if let Some(ca) = &self.ca_pem_path {
            Self::check_readable("ca_pem_path", ca)?;
        }
        Ok(())
    }

    fn check_readable(field: &str, path: &PathBuf) -> Result<()> {
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() => Ok(()),
            Ok(_) => Err(tls_config_invalid(format!(
                "{field} {} is not a regular file",
                path.display()
            ))),
            Err(error) => Err(tls_config_invalid(format!(
                "{field} {} is not readable: {error}",
                path.display()
            ))),
        }
    }
}

/// Mutual-TLS configuration: a [`TlsConfig`] plus the policy switch that decides
/// whether a missing/invalid client certificate is rejected at connect time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MtlsConfig {
    /// Underlying TLS material.
    pub tls: TlsConfig,
    /// When `true`, connections without a valid client certificate are rejected.
    pub require_client_cert: bool,
}

/// The three permitted identity modes for a calling principal (PRD 30 §2).
///
/// Embedded vaults always supply [`AuthN::InProcess`] (the host application owns
/// identity). Server mode (`calyxd`) must supply [`AuthN::MtlsToken`] or
/// [`AuthN::CloudflareAccess`]; anonymous access is never an `AuthN` value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthN {
    /// In-process embedded caller; `host_app_id` identifies the host application.
    InProcess {
        /// Opaque identifier the host application assigns to itself.
        host_app_id: String,
    },
    /// Verified mTLS client identity, keyed by certificate fingerprint.
    MtlsToken {
        /// SHA-256 fingerprint of the verified client certificate.
        fingerprint: [u8; 32],
    },
    /// Cloudflare Access service token identity.
    CloudflareAccess {
        /// The Cloudflare Access service token id presented by the caller.
        service_token_id: String,
    },
}

impl AuthN {
    /// `true` for the network-facing identity modes (mTLS / Cloudflare Access),
    /// `false` for [`AuthN::InProcess`]. `calyxd` uses this to enforce that a
    /// server-mode principal is never in-process.
    pub fn is_server_mode(&self) -> bool {
        matches!(
            self,
            AuthN::MtlsToken { .. } | AuthN::CloudflareAccess { .. }
        )
    }
}

/// The no-anonymous-write predicate every mutation entry point must satisfy.
///
/// Returns `Ok(())` when an identity is present and [`CALYX_AUTHN_REQUIRED`] when
/// it is absent. This is deliberately the *only* gate on identity *presence*;
/// validating the identity's *contents* (fingerprint trust, token validity) is
/// the caller's responsibility. It must never return `Ok(())` for `None`.
pub fn no_anonymous_write(authn: Option<&AuthN>) -> Result<()> {
    match authn {
        Some(_) => Ok(()),
        None => Err(authn_required(
            "mutation rejected: no authenticated principal (mTLS, Cloudflare Access, \
             or in-process host identity required)",
        )),
    }
}

fn authn_required(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_AUTHN_REQUIRED,
        message: message.into(),
        remediation: "present a valid mTLS, Cloudflare Access, or in-process identity before writing",
    }
}

fn tls_config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_TLS_CONFIG_INVALID,
        message: message.into(),
        remediation: "point cert_pem_path/key_pem_path/ca_pem_path at existing, readable PEM files",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_pem(name: &str) -> PathBuf {
        // A real on-disk file (no mock): validate() reads its metadata.
        let mut path = std::env::temp_dir();
        path.push(format!("calyx_sec_{}_{name}", std::process::id()));
        let mut file = std::fs::File::create(&path).expect("create temp pem");
        file.write_all(b"-----BEGIN CERTIFICATE-----\n")
            .expect("write temp pem");
        path
    }

    #[test]
    fn no_anonymous_write_rejects_none_with_exact_code() {
        let err = no_anonymous_write(None).unwrap_err();
        assert_eq!(err.code, CALYX_AUTHN_REQUIRED);
    }

    #[test]
    fn no_anonymous_write_accepts_in_process() {
        let authn = AuthN::InProcess {
            host_app_id: "leapable".into(),
        };
        assert!(no_anonymous_write(Some(&authn)).is_ok());
    }

    #[test]
    fn no_anonymous_write_accepts_mtls_token() {
        let authn = AuthN::MtlsToken {
            fingerprint: [7u8; 32],
        };
        assert!(no_anonymous_write(Some(&authn)).is_ok());
    }

    #[test]
    fn empty_host_app_id_is_still_an_identity() {
        // Presence, not content, is what this gate checks.
        let authn = AuthN::InProcess {
            host_app_id: String::new(),
        };
        assert!(no_anonymous_write(Some(&authn)).is_ok());
    }

    #[test]
    fn is_server_mode_per_variant() {
        assert!(
            !AuthN::InProcess {
                host_app_id: "h".into()
            }
            .is_server_mode()
        );
        assert!(
            AuthN::MtlsToken {
                fingerprint: [0u8; 32]
            }
            .is_server_mode()
        );
        assert!(
            AuthN::CloudflareAccess {
                service_token_id: "svc".into()
            }
            .is_server_mode()
        );
    }

    #[test]
    fn authn_round_trips_for_all_three_variants() {
        for authn in [
            AuthN::InProcess {
                host_app_id: "host-1".into(),
            },
            AuthN::MtlsToken {
                fingerprint: [3u8; 32],
            },
            AuthN::CloudflareAccess {
                service_token_id: "tok-9".into(),
            },
        ] {
            let bytes = serde_json::to_vec(&authn).expect("serialize");
            let back: AuthN = serde_json::from_slice(&bytes).expect("deserialize");
            assert_eq!(authn, back);
        }
    }

    #[test]
    fn tls_validate_ok_for_existing_files_without_ca() {
        let cfg = TlsConfig {
            cert_pem_path: temp_pem("cert"),
            key_pem_path: temp_pem("key"),
            ca_pem_path: None,
        };
        assert!(cfg.validate().is_ok());
        let _ = std::fs::remove_file(&cfg.cert_pem_path);
        let _ = std::fs::remove_file(&cfg.key_pem_path);
    }

    #[test]
    fn tls_validate_fails_for_missing_path() {
        let cfg = TlsConfig {
            cert_pem_path: PathBuf::from("/nonexistent/calyx/cert.pem"),
            key_pem_path: PathBuf::from("/nonexistent/calyx/key.pem"),
            ca_pem_path: None,
        };
        let err = cfg.validate().unwrap_err();
        assert_eq!(err.code, CALYX_TLS_CONFIG_INVALID);
        assert!(err.message.contains("cert_pem_path"));
    }

    #[test]
    fn mtls_config_carries_require_client_cert() {
        let cfg = MtlsConfig {
            tls: TlsConfig {
                cert_pem_path: temp_pem("mcert"),
                key_pem_path: temp_pem("mkey"),
                ca_pem_path: Some(temp_pem("mca")),
            },
            require_client_cert: true,
        };
        assert!(cfg.require_client_cert);
        assert!(cfg.tls.validate().is_ok());
        let _ = std::fs::remove_file(&cfg.tls.cert_pem_path);
        let _ = std::fs::remove_file(&cfg.tls.key_pem_path);
        if let Some(ca) = &cfg.tls.ca_pem_path {
            let _ = std::fs::remove_file(ca);
        }
    }
}
