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
