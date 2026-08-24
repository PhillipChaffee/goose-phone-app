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
//! use goose_acp_client::{AcpClient, ConnectConfig, ContentBlock};
//!
//! let cfg = ConnectConfig {
//!     base_url: "https://goose-box.tailnet.ts.net".into(),
//!     secret: "my-secret".into(),
//!     fingerprint: None,
//! };
//! let (client, mut events, info) = AcpClient::connect(&cfg).await?;
//! println!("connected to {} {}", info.agent_name, info.agent_version);
//! let session = client.session_new("/home/me").await?;
//! let stop = client
//!     .prompt(&session.session_id, &[ContentBlock::text("Hello goose!")])
//!     .await?;
//! # let _ = (stop, events.recv().await);
//! # Ok(())
//! # }
//! ```

mod client;
mod probe;
mod tls;
mod types;

pub use client::{
    config_options_from, normalize_base_url, ws_url, AcpClient, AcpError, ConnectConfig,
    CLIENT_NAME,
};
pub use probe::{probe, ProbeOutcome};
pub use tls::{ensure_crypto_provider, format_fingerprint, parse_fingerprint};
pub use types::*;
