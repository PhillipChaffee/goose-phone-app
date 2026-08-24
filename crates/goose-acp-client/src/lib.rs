//! Client library for the goose AI agent server's ACP interface.
//!
//! goose ≥ 1.42 exposes a single API: the Agent Client Protocol (JSON-RPC
//! 2.0) at `/acp`, served by `goose serve`. This crate speaks it over a
//! WebSocket — the same transport the official goose Desktop app uses — and
//! is UI-framework agnostic: pure tokio + tungstenite + rustls, so it builds
//! unchanged for iOS, Android, and desktop targets.
//!
//! ```no_run
//! # async fn demo() -> Result<(), goose_acp_client::AcpError> {
//! use goose_acp_client::{AcpClient, ConnectConfig};
//!
//! let cfg = ConnectConfig {
//!     base_url: "https://goose-box.tailnet.ts.net".into(),
//!     secret: "my-secret".into(),
//!     fingerprint: None,
//! };
//! let (client, mut events, info) = AcpClient::connect(&cfg).await?;
//! println!("connected to {} {}", info.agent_name, info.agent_version);
//! let session = client.session_new("/home/me").await?;
//! let stop = client.prompt(&session.session_id, "Hello goose!").await?;
//! # let _ = (stop, events.recv().await);
//! # Ok(())
//! # }
//! ```
//!
//! The layout: `client` owns the transport and the base ACP methods, `types`
//! holds the base ACP wire types, `goose` holds everything in goose's own
//! `_goose/unstable/*` namespace, and `error` holds the single error type.
//! All four are private modules re-exported flat, so the public paths are
//! `goose_acp_client::Thing` regardless of which file the thing lives in.

mod client;
mod error;
mod goose;
mod probe;
mod tls;
mod types;

use std::any::type_name;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

pub use client::{normalize_base_url, ws_url, AcpClient, ConnectConfig, CLIENT_NAME};
pub use error::{AcpError, Feature};
pub use goose::*;
pub use probe::{probe, ProbeOutcome};
pub use tls::{ensure_crypto_provider, format_fingerprint, parse_fingerprint};
pub use types::*;

/// Deserialize `value` into `T`, serialize it straight back, and fail if the
/// two do not match. Returns the parsed value so a test can go on asserting
/// about it.
///
/// This exists because goose sets `deny_unknown_fields` on nothing. A field
/// this crate spells wrong — `filePath` where the wire says `file_path` —
/// deserializes to `None`, the call succeeds, the screen shows a blank, and
/// *nothing anywhere says so*. Round-tripping is what turns that silence into
/// a failing test that names the field: the correctly-spelled key comes back
/// out of the DTO's `extra` catch-all, and the mis-spelled one this crate
/// declared shows up beside it as an invented `null`.
///
/// It replaces the check it is easy to reach for instead —
/// `assert!(serde_json::from_value::<T>(raw).is_ok())` — which proves nothing
/// at all for a struct whose fields are every one of them `Option` or
/// `#[serde(default)]`, which is every DTO in this crate. That assertion
/// passes on `{}`.
///
/// Two things follow for the fixtures it is given. They must be complete
/// server responses, not minimal ones, because a field absent from the input
/// cannot be shown to have been read. And `Option` fields on the DTO must not
/// be `skip_serializing_if`-ed away, because the `null` they serialize is the
/// evidence.
///
/// # Panics
///
/// If `value` does not deserialize into `T`, if `T` does not serialize, or if
/// the round trip changed anything.
#[expect(
    clippy::panic,
    reason = "test helper: the panic IS the assertion, and it names the field"
)]
#[must_use]
pub fn assert_round_trip<T: DeserializeOwned + Serialize>(value: &Value) -> T {
    let parsed: T = match serde_json::from_value(value.clone()) {
        Ok(parsed) => parsed,
        Err(e) => panic!("{} does not parse from {value}: {e}", type_name::<T>()),
    };
    let back = match serde_json::to_value(&parsed) {
        Ok(back) => back,
        Err(e) => panic!("{} does not serialize: {e}", type_name::<T>()),
    };
    assert!(
        back == *value,
        "{} did not round-trip: {}\n  sent: {value}\n  back: {back}",
        type_name::<T>(),
        key_diff(value, &back)
    );
    parsed
}

/// Name the keys that changed across a round trip, so the failure message
/// points at a field instead of asking a reader to diff two long lines.
fn key_diff(sent: &Value, back: &Value) -> String {
    let (Some(sent), Some(back)) = (sent.as_object(), back.as_object()) else {
        return "not both objects".to_string();
    };
    let mut notes = Vec::new();
    for (key, value) in sent {
        match back.get(key) {
            None => notes.push(format!("dropped `{key}`")),
            Some(other) if other != value => {
                notes.push(format!("changed `{key}`: {value} -> {other}"));
            }
            Some(_) => {}
        }
    }
    for key in back.keys() {
        if !sent.contains_key(key) {
            notes.push(format!("invented `{key}`"));
        }
    }
    notes.join(", ")
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a failing unwrap is the failing check"
)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::{json, Map};

    /// A DTO in the shape this crate writes them, with one field spelled the
    /// way it is easy to spell it wrong: goose sends `file_path`.
    #[derive(Debug, Serialize, Deserialize)]
    struct Misspelled {
        id: String,
        file_pathe: Option<String>,
        #[serde(flatten)]
        extra: Map<String, Value>,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct Correct {
        id: String,
        file_path: Option<String>,
        #[serde(flatten)]
        extra: Map<String, Value>,
    }

    fn entry() -> Value {
        json!({"id": "review", "file_path": "/home/me/.config/goose/recipes/review.yaml"})
    }

    #[test]
    fn round_trip_catches_a_misspelled_field() {
        let panicked = std::panic::catch_unwind(|| assert_round_trip::<Misspelled>(&entry()));
        let err = panicked.unwrap_err();
        let message = err
            .downcast_ref::<String>()
            .map_or("", String::as_str)
            .to_string();
        assert!(
            message.contains("invented `file_pathe`"),
            "the failure should name the field, got: {message}"
        );
    }

    #[test]
    fn round_trip_accepts_the_right_spelling() {
        let parsed: Correct = assert_round_trip(&entry());
        assert_eq!(
            parsed.file_path.as_deref(),
            Some("/home/me/.config/goose/recipes/review.yaml")
        );
        assert!(parsed.extra.is_empty());
    }
}
