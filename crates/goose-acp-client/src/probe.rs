//! Pre-flight reachability / auth check, mirroring what goose Desktop does:
//! `GET /status` (unauthenticated) proves the server is up, and `GET /acp`
//! with the secret returning HTTP 406 (Not Acceptable) proves the secret is
//! valid — 406 is only reachable after auth passes.

use std::time::Duration;

use crate::client::normalize_base_url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    /// Server reachable and (if a secret was given) the secret was accepted.
    Ok,
    /// Server reachable but the secret was rejected (401/403).
    AuthFailed,
    /// Could not reach the server at all.
    Unreachable(String),
}

/// Check the server.
///
/// `pinned` should be true when connecting to a `goose serve --tls`
/// self-signed certificate: the probe then skips certificate validation and
/// leaves the real trust decision to the fingerprint-pinned WebSocket
/// connection.
pub async fn probe(base_url: &str, secret: &str, pinned: bool) -> ProbeOutcome {
    let base = match normalize_base_url(base_url) {
        Ok(b) => b,
        Err(e) => return ProbeOutcome::Unreachable(e.to_string()),
    };

    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(15));
    if pinned {
        builder = builder.danger_accept_invalid_certs(true);
    }
    let client = match builder.build() {
        Ok(c) => c,
        Err(e) => return ProbeOutcome::Unreachable(e.to_string()),
    };

    match client.get(format!("{base}/status")).send().await {
        Ok(resp) if resp.status().is_success() => {}
        Ok(resp) => {
            return ProbeOutcome::Unreachable(format!(
                "GET /status returned HTTP {}",
                resp.status().as_u16()
            ))
        }
        Err(e) => return ProbeOutcome::Unreachable(e.to_string()),
    }

    let mut req = client.get(format!("{base}/acp"));
    if !secret.is_empty() {
        req = req.header("X-Secret-Key", secret);
    }
    match req.send().await {
        Ok(resp) => match resp.status().as_u16() {
            // 406 is the documented auth-success signal: a plain GET /acp
            // only reaches content negotiation after the secret is accepted.
            406 => ProbeOutcome::Ok,
            401 | 403 => ProbeOutcome::AuthFailed,
            status => ProbeOutcome::Unreachable(format!(
                "GET /acp returned HTTP {status}, expected 406 — is this a goose server?"
            )),
        },
        Err(e) => ProbeOutcome::Unreachable(e.to_string()),
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test assertions: a failing unwrap or a wrong-variant panic is the failing check"
)]
mod tests {
    use std::io::{Read, Write};
    use std::sync::Arc;

    use rustls_pki_types::PrivatePkcs8KeyDer;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{probe, ProbeOutcome};

    /// What a stub should answer for a given request path: a status line, or
    /// `None` to hang up mid-request the way a half-open tailnet route does.
    type Reply = fn(&str) -> Option<&'static str>;

    fn head_path(head: &[u8]) -> String {
        String::from_utf8_lossy(head)
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("/")
            .to_string()
    }

    fn response(status: &str) -> String {
        format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    }

    /// A plain-HTTP stub that answers whatever `reply` says. Returns its base
    /// URL.
    async fn spawn_stub(reply: Reply) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                        match sock.read(&mut byte).await {
                            Ok(0) | Err(_) => return,
                            Ok(_) => head.push(byte[0]),
                        }
                    }
                    if let Some(status) = reply(&head_path(&head)) {
                        let _ = sock.write_all(response(status).as_bytes()).await;
                    }
                    // `None`: drop the socket without answering.
                });
            }
        });
        format!("http://{addr}")
    }

    /// The same stub over TLS, presenting the self-signed certificate from
    /// `tls`'s fixtures — i.e. what `goose serve --tls` looks like to a
    /// client that has not been told to trust it.
    fn spawn_tls_stub(reply: Reply) -> String {
        crate::tls::ensure_crypto_provider();
        let cert = rustls_pki_types::CertificateDer::from(
            include_bytes!("../tests/fixtures/pinned-cert.der").to_vec(),
        );
        let key =
            PrivatePkcs8KeyDer::from(include_bytes!("../tests/fixtures/pinned-key.der").to_vec());
        let config = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key.into())
                .unwrap(),
        );

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            while let Ok((sock, _)) = listener.accept() {
                let conn = rustls::ServerConnection::new(Arc::clone(&config)).unwrap();
                let mut tls = rustls::StreamOwned::new(conn, sock);
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                    match tls.read(&mut byte) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => head.push(byte[0]),
                    }
                }
                if let Some(status) = reply(&head_path(&head)) {
                    let _ = tls.write_all(response(status).as_bytes());
                    let _ = tls.flush();
                }
                tls.conn.send_close_notify();
                let _ = tls.flush();
            }
        });
        format!("https://{addr}")
    }

    /// A goose server behaving itself: `/status` up, `/acp` answering 406 to
    /// the secret.
    const GOOSE: Reply = |path| match path {
        "/status" => Some("200 OK"),
        "/acp" => Some("406 Not Acceptable"),
        _ => Some("404 Not Found"),
    };

    /// A typo in Settings must come back as an answer, not as a fifteen-second
    /// wait on a request that was never worth making.
    #[tokio::test]
    async fn an_unusable_url_is_reported_without_a_round_trip() {
        assert_eq!(
            probe("", "secret", false).await,
            ProbeOutcome::Unreachable("invalid configuration: server URL is empty".to_string()),
            "an empty host must be named as the problem, not timed out"
        );
        match probe("ftp://box:21", "secret", false).await {
            ProbeOutcome::Unreachable(msg) => assert!(
                msg.contains("ftp"),
                "the rejected scheme belongs in the message: {msg}"
            ),
            other => panic!("an unsupported scheme must be unreachable, got {other:?}"),
        }
    }

    /// Something is listening, but it is not a healthy goose: the HTTP status
    /// is the only clue the user gets, so it has to reach them.
    #[tokio::test]
    async fn a_sick_server_reports_the_status_it_sent() {
        const SICK: Reply = |_| Some("503 Service Unavailable");
        assert_eq!(
            probe(&spawn_stub(SICK).await, "secret", false).await,
            ProbeOutcome::Unreachable("GET /status returned HTTP 503".to_string()),
            "a failing /status must quote the code rather than blame the network"
        );
    }

    /// `/status` answers, `/acp` does not 406 — the shape of pointing the app
    /// at some other web server on the tailnet. Saying "expected 406" alone
    /// would tell a user nothing, so the message asks the real question.
    #[tokio::test]
    async fn a_server_that_is_not_goose_is_named_as_such() {
        // Every path answers 200: something is up, but it is not goose.
        const NOT_GOOSE: Reply = |_| Some("200 OK");
        assert_eq!(
            probe(&spawn_stub(NOT_GOOSE).await, "secret", false).await,
            ProbeOutcome::Unreachable(
                "GET /acp returned HTTP 200, expected 406 — is this a goose server?".to_string()
            ),
            "an unexpected /acp status must say so, and say what it was"
        );
    }

    /// A connection that dies between the two requests is unreachable, not a
    /// rejected secret: telling a user their secret is wrong when the tailnet
    /// dropped sends them to change the one thing that was right.
    #[tokio::test]
    async fn a_dropped_connection_is_not_mistaken_for_a_bad_secret() {
        const DIES_ON_ACP: Reply = |path| match path {
            "/status" => Some("200 OK"),
            _ => None,
        };
        match probe(&spawn_stub(DIES_ON_ACP).await, "secret", false).await {
            ProbeOutcome::Unreachable(msg) => {
                assert!(!msg.is_empty(), "the transport error must carry a reason");
            }
            other => panic!("a hung-up /acp must be unreachable, got {other:?}"),
        }
    }

    /// The `pinned` flag is the whole reason a self-signed `goose serve --tls`
    /// is usable at all: without it the pre-flight rejects the certificate and
    /// the user never gets as far as the WebSocket that would have accepted
    /// the pin.
    #[tokio::test]
    async fn pinning_lets_the_probe_past_a_self_signed_certificate() {
        let base = spawn_tls_stub(GOOSE);
        assert_eq!(
            probe(&base, "secret", true).await,
            ProbeOutcome::Ok,
            "a pinned probe must not validate the chain it was told to skip"
        );
        match probe(&base, "secret", false).await {
            ProbeOutcome::Unreachable(msg) => assert!(
                !msg.is_empty(),
                "an unpinned probe must refuse the same certificate, with a reason"
            ),
            other => panic!("an unpinned probe must reject a self-signed server, got {other:?}"),
        }
    }
}
