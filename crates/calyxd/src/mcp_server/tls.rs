use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;

use calyx_core::{AuthN, MtlsConfig};
use calyx_mcp::McpServer;
use rustls::server::WebPkiClientVerifier;
use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use sha2::{Digest, Sha256};

use crate::error::DaemonError;

pub(super) fn build_server_config(mtls: &MtlsConfig) -> Result<Arc<ServerConfig>, DaemonError> {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    if !mtls.require_client_cert {
        return Err(DaemonError::tls_config_invalid(
            "mcp_mtls.require_client_cert must be true for calyxd MCP",
        ));
    }
    let Some(ca_path) = mtls.tls.ca_pem_path.as_deref() else {
        return Err(DaemonError::tls_config_invalid(
            "mcp_mtls.tls.ca_pem_path is required for calyxd MCP mTLS",
        ));
    };
    mtls.tls.validate().map_err(|error| {
        DaemonError::tls_config_invalid(format!("{}: {}", error.code, error.message))
    })?;

    let certs = load_certs(&mtls.tls.cert_pem_path)?;
    let key = load_key(&mtls.tls.key_pem_path)?;
    let ca_certs = load_certs(ca_path)?;
    let mut roots = RootCertStore::empty();
    let (added, ignored) = roots.add_parsable_certificates(ca_certs);
    if added == 0 {
        return Err(DaemonError::tls_config_invalid(format!(
            "mcp_mtls.tls.ca_pem_path {} did not contain a parsable CA certificate ({ignored} ignored)",
            ca_path.display()
        )));
    }
    let verifier = WebPkiClientVerifier::builder(Arc::new(roots))
        .build()
        .map_err(|error| {
            DaemonError::tls_config_invalid(format!("build client verifier: {error}"))
        })?;
    let config = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(|error| {
            DaemonError::tls_config_invalid(format!("build server TLS config: {error}"))
        })?;
    Ok(Arc::new(config))
}

pub(super) fn serve_connection(
    stream: TcpStream,
    dispatcher: &McpServer,
    config: Arc<ServerConfig>,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(super::IO_TIMEOUT))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(super::IO_TIMEOUT))
        .map_err(|error| format!("set write timeout: {error}"))?;

    let conn =
        ServerConnection::new(config).map_err(|error| format!("create TLS connection: {error}"))?;
    let mut tls = StreamOwned::new(conn, stream);
    while tls.conn.is_handshaking() {
        tls.conn
            .complete_io(&mut tls.sock)
            .map_err(|error| format!("complete TLS handshake: {error}"))?;
    }
    let authn = AuthN::MtlsToken {
        fingerprint: client_fingerprint(&tls.conn)?,
    };
    super::serve_stream(&mut tls, dispatcher, Some(&authn))
}

fn client_fingerprint(conn: &ServerConnection) -> Result<[u8; 32], String> {
    let cert = conn
        .peer_certificates()
        .and_then(|certs| certs.first())
        .ok_or_else(|| "mTLS handshake completed without a client certificate".to_string())?;
    let mut hasher = Sha256::new();
    hasher.update(cert.as_ref());
    let digest = hasher.finalize();
    let mut fingerprint = [0_u8; 32];
    fingerprint.copy_from_slice(&digest);
    Ok(fingerprint)
}

fn load_certs(path: &Path) -> Result<Vec<CertificateDer<'static>>, DaemonError> {
    let bytes = std::fs::read(path).map_err(|error| {
        DaemonError::tls_config_invalid(format!("read certificate PEM {}: {error}", path.display()))
    })?;
    let certs: Result<Vec<_>, _> = CertificateDer::pem_slice_iter(&bytes).collect();
    let certs = certs.map_err(|error| {
        DaemonError::tls_config_invalid(format!(
            "parse certificate PEM {}: {error}",
            path.display()
        ))
    })?;
    if certs.is_empty() {
        return Err(DaemonError::tls_config_invalid(format!(
            "certificate PEM {} contains no certificates",
            path.display()
        )));
    }
    Ok(certs)
}

fn load_key(path: &Path) -> Result<PrivateKeyDer<'static>, DaemonError> {
    let bytes = std::fs::read(path).map_err(|error| {
        DaemonError::tls_config_invalid(format!("read private key PEM {}: {error}", path.display()))
    })?;
    PrivateKeyDer::from_pem_slice(&bytes).map_err(|error| {
        DaemonError::tls_config_invalid(format!(
            "parse private key PEM {}: {error}",
            path.display()
        ))
    })
}
