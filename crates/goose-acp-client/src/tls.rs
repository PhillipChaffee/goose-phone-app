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
    clippy::panic,
    reason = "test assertions: a failing unwrap or a wrong-variant panic is the failing check"
)]
mod tests {
    use super::*;

    use rustls_pki_types::PrivatePkcs8KeyDer;

    /// A self-signed P-256 certificate and its key, standing in for the one
    /// `goose serve --tls` mints on the box. Regenerate with:
    ///
    /// ```text
    /// openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \
    ///   -keyout k.pem -out c.pem -days 36500 -nodes -subj /CN=goose.test \
    ///   -addext subjectAltName=DNS:goose.test \
    ///   -addext basicConstraints=critical,CA:FALSE -set_serial 1
    /// openssl x509 -in c.pem -outform DER -out pinned-cert.der
    /// openssl pkey -in k.pem -outform DER -out pinned-key.der
    /// ```
    ///
    /// It expires in 2126 so that the unpinned run below fails for the one
    /// reason under test (nobody signed it) rather than for having gone stale.
    const CERT: &[u8] = include_bytes!("../tests/fixtures/pinned-cert.der");
    const KEY: &[u8] = include_bytes!("../tests/fixtures/pinned-key.der");

    fn cert() -> CertificateDer<'static> {
        CertificateDer::from(CERT.to_vec())
    }

    /// What goose would print as `GOOSED_CERT_FINGERPRINT` for [`CERT`].
    fn pin() -> [u8; 32] {
        Sha256::digest(CERT).into()
    }

    fn server_config(version: &'static rustls::SupportedProtocolVersion) -> rustls::ServerConfig {
        ensure_crypto_provider();
        rustls::ServerConfig::builder_with_protocol_versions(&[version])
            .with_no_client_auth()
            .with_single_cert(vec![cert()], PrivatePkcs8KeyDer::from(KEY.to_vec()).into())
            .unwrap()
    }

    /// Drain everything one side wants to say and feed it to the other,
    /// surfacing the receiver's verdict on it.
    fn pipe(
        from: &mut rustls::Connection,
        to: &mut rustls::Connection,
    ) -> Result<(), rustls::Error> {
        let mut wire = Vec::new();
        while from.wants_write() {
            from.write_tls(&mut wire).unwrap();
        }
        let mut cursor = std::io::Cursor::new(&wire[..]);
        while cursor.position() < wire.len() as u64 {
            to.read_tls(&mut cursor).unwrap();
            to.process_new_packets()?;
        }
        Ok(())
    }

    /// Run a real TLS handshake in memory between the connector under test and
    /// a server presenting [`CERT`]. No socket: `write_tls`/`read_tls` is the
    /// same code path a connected client takes, minus the plumbing.
    ///
    /// Returns the negotiated protocol version, which is how a test proves
    /// which of the two signature-verification hooks ran.
    fn handshake(
        connector: &Connector,
        version: &'static rustls::SupportedProtocolVersion,
    ) -> Result<rustls::ProtocolVersion, rustls::Error> {
        let Connector::Rustls(config) = connector else {
            panic!("build_connector must hand back a rustls connector")
        };
        let client = rustls::ClientConnection::new(
            Arc::clone(config),
            ServerName::try_from("goose.test").unwrap(),
        )
        .unwrap();
        let server = rustls::ServerConnection::new(Arc::new(server_config(version))).unwrap();
        let mut client = rustls::Connection::Client(client);
        let mut server = rustls::Connection::Server(server);

        for _ in 0..10 {
            pipe(&mut client, &mut server)?;
            pipe(&mut server, &mut client)?;
            if !client.is_handshaking() && !server.is_handshaking() {
                return Ok(client.protocol_version().unwrap());
            }
        }
        panic!("handshake never settled")
    }

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

    /// The pin is over the certificate's exact DER bytes, and the rejection
    /// message carries the fingerprint that *did* turn up — that string is the
    /// only way a user staring at "connection failed" can compare what the
    /// server presented against the `GOOSED_CERT_FINGERPRINT` goose printed.
    #[test]
    fn the_verifier_pins_the_exact_der_and_names_what_it_saw() {
        ensure_crypto_provider();
        let verifier = FingerprintVerifier {
            fingerprint: pin(),
            provider: CryptoProvider::get_default().cloned().unwrap(),
        };
        let name = ServerName::try_from("goose.test").unwrap();
        let now = UnixTime::now();

        assert!(
            verifier
                .verify_server_cert(&cert(), &[], &name, &[], now)
                .is_ok(),
            "the pinned certificate itself must be accepted, or no self-signed goose is reachable"
        );

        let impostor = CertificateDer::from(vec![0x30, 0x82, 0x01, 0x02]);
        let err = verifier
            .verify_server_cert(&impostor, &[], &name, &[], now)
            .unwrap_err();
        let seen: [u8; 32] = Sha256::digest([0x30, 0x82, 0x01, 0x02]).into();
        assert_eq!(
            err,
            rustls::Error::General(format!(
                "server certificate fingerprint mismatch (got {})",
                format_fingerprint(&seen)
            )),
            "a different certificate must be refused, and the message must quote its fingerprint"
        );
    }

    /// A handshake signature as rustls hands one over, kept so a test can
    /// re-verify it against a transcript the server never signed.
    /// `DigitallySignedStruct` has no public constructor, so borrowing one
    /// from a real handshake is the only way to get one at all.
    #[derive(Debug)]
    struct Signed {
        message: Vec<u8>,
        cert: CertificateDer<'static>,
        dss: DigitallySignedStruct,
    }

    #[derive(Debug)]
    struct Spy {
        inner: FingerprintVerifier,
        seen: std::sync::Mutex<Vec<Signed>>,
    }

    impl Spy {
        fn record(&self, message: &[u8], cert: &CertificateDer<'_>, dss: &DigitallySignedStruct) {
            self.seen.lock().unwrap().push(Signed {
                message: message.to_vec(),
                cert: cert.clone().into_owned(),
                dss: dss.clone(),
            });
        }
    }

    impl ServerCertVerifier for Spy {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            intermediates: &[CertificateDer<'_>],
            server_name: &ServerName<'_>,
            ocsp_response: &[u8],
            now: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            self.inner.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            )
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            self.record(message, cert, dss);
            self.inner.verify_tls12_signature(message, cert, dss)
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &CertificateDer<'_>,
            dss: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            self.record(message, cert, dss);
            self.inner.verify_tls13_signature(message, cert, dss)
        }

        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.inner.supported_verify_schemes()
        }
    }

    fn verifier() -> FingerprintVerifier {
        ensure_crypto_provider();
        FingerprintVerifier {
            fingerprint: pin(),
            provider: CryptoProvider::get_default().cloned().unwrap(),
        }
    }

    /// Complete a pinned handshake at `version` and hand back the signature
    /// the server actually produced over that transcript.
    fn signature_from_a_real_handshake(
        version: &'static rustls::SupportedProtocolVersion,
    ) -> Signed {
        let spy = Arc::new(Spy {
            inner: verifier(),
            seen: std::sync::Mutex::new(Vec::new()),
        });
        let as_verifier: Arc<dyn ServerCertVerifier> = spy.clone();
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(as_verifier)
            .with_no_client_auth();
        handshake(&Connector::Rustls(Arc::new(config)), version).unwrap();

        let mut seen = spy.seen.lock().unwrap();
        assert_eq!(
            seen.len(),
            1,
            "one handshake signature per handshake, or the hook under test never ran"
        );
        seen.pop().unwrap()
    }

    /// The pin replaces chain validation, not signature checking. Both hooks
    /// must pass the signature the server really made and refuse the same
    /// signature over any other transcript — returning
    /// `HandshakeSignatureValid::assertion()` from either one would let anyone
    /// holding a copy of the pinned certificate, which is a public document,
    /// impersonate the server without ever having its key.
    #[test]
    fn a_pin_does_not_excuse_a_bad_handshake_signature() {
        assert!(
            verifier()
                .supported_verify_schemes()
                .contains(&SignatureScheme::ECDSA_NISTP256_SHA256),
            "the certificate under test is P-256; without that scheme offered no server can answer"
        );

        for version in [&rustls::version::TLS13, &rustls::version::TLS12] {
            let tls13 = version.version == rustls::ProtocolVersion::TLSv1_3;
            let signed = signature_from_a_real_handshake(version);
            let check = |message: &[u8]| {
                if tls13 {
                    verifier().verify_tls13_signature(message, &signed.cert, &signed.dss)
                } else {
                    verifier().verify_tls12_signature(message, &signed.cert, &signed.dss)
                }
            };
            assert!(
                check(&signed.message).is_ok(),
                "the server's own signature must verify ({version:?})"
            );

            let mut tampered = signed.message.clone();
            tampered[0] ^= 0x01;
            assert!(
                check(&tampered).is_err(),
                "one flipped bit of transcript must invalidate the signature ({version:?})"
            );
        }
    }

    /// The whole point of the pinned connector, end to end: a real handshake
    /// against a server holding the self-signed key succeeds under TLS 1.3 and
    /// TLS 1.2 — so the signature hooks pass real signatures as well as
    /// refusing forged ones — and the same certificate under the wrong pin is
    /// refused by name.
    #[test]
    fn a_pinned_connector_completes_a_real_handshake_and_a_wrong_pin_does_not() {
        let pinned = build_connector(Some(pin()));
        assert_eq!(
            handshake(&pinned, &rustls::version::TLS13).unwrap(),
            rustls::ProtocolVersion::TLSv1_3,
            "a pinned client must reach a goose serve --tls certificate over TLS 1.3"
        );
        assert_eq!(
            handshake(&pinned, &rustls::version::TLS12).unwrap(),
            rustls::ProtocolVersion::TLSv1_2,
            "a pinned client must reach one over TLS 1.2 too"
        );

        let mut wrong = pin();
        wrong[0] ^= 0xFF;
        let err = handshake(&build_connector(Some(wrong)), &rustls::version::TLS13).unwrap_err();
        assert!(
            err.to_string().contains("fingerprint mismatch"),
            "a mistyped pin must fail as a mismatch, not as some unrelated TLS error: {err}"
        );
    }

    /// With no fingerprint the connector is ordinary `WebPKI`, which is what
    /// makes `tailscale serve` work out of the box — and what must keep a
    /// self-signed certificate out. If this ever passes, pinning has silently
    /// become optional.
    #[test]
    fn an_unpinned_connector_still_demands_a_real_chain() {
        let err = handshake(&build_connector(None), &rustls::version::TLS13).unwrap_err();
        assert_eq!(
            err,
            rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer),
            "an unpinned client must reject a self-signed certificate as unrooted"
        );
    }
}
