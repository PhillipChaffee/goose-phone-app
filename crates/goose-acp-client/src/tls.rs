//! TLS setup for the WebSocket connection.
//!
//! Two modes:
//! - Normal `WebPKI` validation (Mozilla roots) — works out of the box with
//!   `tailscale serve` (Let's Encrypt certs on the tailnet) or any real cert.
//! - SHA-256 certificate fingerprint pinning — for `goose serve --tls`
//!   self-signed certificates. goose prints `GOOSED_CERT_FINGERPRINT=AA:BB:…`
//!   on startup; pinning that exact certificate replaces chain validation,
//!   which is the same scheme the goose Desktop app uses.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::{DigitallySignedStruct, SignatureScheme};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use tokio_tungstenite::Connector;

/// Parse a SHA-256 fingerprint like `AA:BB:...` (or bare hex, any case).
/// Empty/whitespace input means "no pin".
///
/// # Errors
///
/// A message naming the expected format if the input, once colons and
/// whitespace are stripped, is not exactly 64 hex digits.
pub fn parse_fingerprint(input: &str) -> Result<Option<[u8; 32]>, String> {
    let cleaned: String = input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ':')
        .collect();
    if cleaned.is_empty() {
        return Ok(None);
    }
    if cleaned.len() != 64 || !cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("fingerprint must be 32 hex bytes (SHA-256), e.g. AA:BB:…".to_string());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(Some(out))
}

#[must_use]
pub fn format_fingerprint(fp: &[u8; 32]) -> String {
    fp.iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Certificate verifier that accepts exactly one certificate: the one whose
/// DER SHA-256 matches the pinned fingerprint. Hostname and chain checks are
/// intentionally skipped — the pin is the trust decision.
#[derive(Debug)]
struct FingerprintVerifier {
    fingerprint: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let actual: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if actual == self.fingerprint {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "server certificate fingerprint mismatch (got {})",
                format_fingerprint(&actual)
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Make sure a rustls crypto provider is installed process-wide. Safe to call
/// repeatedly; the first call wins.
pub fn ensure_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// Build the tungstenite TLS connector: pinned when a fingerprint is given,
/// `WebPKI` (Mozilla roots) otherwise. Ignored for plain `ws://` URLs.
pub(crate) fn build_connector(fingerprint: Option<[u8; 32]>) -> Connector {
    ensure_crypto_provider();
    let provider = CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::ring::default_provider()));

    let config = if let Some(fp) = fingerprint {
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(FingerprintVerifier {
                fingerprint: fp,
                provider,
            }))
            .with_no_client_auth()
    } else {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth()
    };
    Connector::Rustls(Arc::new(config))
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;

    #[test]
    fn parses_colon_hex_fingerprint() {
        let s = "AB:CD:".repeat(16);
        let s = s.trim_end_matches(':');
        let fp = parse_fingerprint(s).unwrap().unwrap();
        assert_eq!(fp[0], 0xAB);
        assert_eq!(fp[1], 0xCD);
        assert_eq!(format_fingerprint(&fp), s);
    }

    #[test]
    fn parses_bare_hex_and_rejects_garbage() {
        let bare = "ab".repeat(32);
        assert!(parse_fingerprint(&bare).unwrap().is_some());
        assert!(parse_fingerprint("").unwrap().is_none());
        assert!(parse_fingerprint("   ").unwrap().is_none());
        assert!(parse_fingerprint("zz".repeat(32).as_str()).is_err());
        assert!(parse_fingerprint("abcd").is_err());
    }
}
