//! The Connect flow end to end, over a real WebSocket, against the real mock
//! binary.
//!
//! The unit tests next to the handlers check the config store's behaviour;
//! this one checks that the methods are actually *reachable* — that
//! `_goose/unstable/...` names are routed, that the client's frames parse on
//! the server and the server's replies parse on the client. A typo in a method
//! string is invisible to both sides' unit tests and fatal in the app.

// Test code: a failing unwrap IS the failing assertion, and the port choice is
// printed so a collision is diagnosable.
#![expect(
    clippy::unwrap_used,
    reason = "test harness: an unwrap here is the assertion"
)]

use std::process::Command;
use std::time::Duration;

use goose_acp_client::{AcpClient, ConnectConfig, GooseExtension, McpServer, StdioMcpServer};

/// A port of its own, so this can run beside a hand-started mock on 3285.
const PORT: u16 = 3391;

/// Kills the server when the test ends, including on a panic.
struct Server(std::process::Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

#[tokio::test]
async fn the_whole_connect_flow_works_over_the_wire() {
    let _server = Server(
        Command::new(env!("CARGO_BIN_EXE_mock-goose-server"))
            .arg(PORT.to_string())
            .spawn()
            .unwrap(),
    );
    tokio::time::sleep(Duration::from_millis(600)).await;

    let (client, _events, _info) = AcpClient::connect(&ConnectConfig {
        base_url: format!("http://127.0.0.1:{PORT}"),
        secret: "mock-secret".to_string(),
        fingerprint: None,
    })
    .await
    .unwrap();

    // goose's own catalogue: present, and unrestricted, which is the honest
    // answer rather than a flattering one.
    let available = client.extensions_available().await.unwrap();
    assert!(available.iter().any(|e| e.name() == "developer"));
    assert!(available.iter().all(|e| e.available_tools().is_empty()));

    let extension = GooseExtension::mcp(
        McpServer::Stdio(StdioMcpServer::new(
            "mail-imap",
            "uvx",
            vec!["mcp-email-server@1.4.2".to_string(), "stdio".to_string()],
        )),
        vec!["MCP_EMAIL_SERVER_PASSWORD".to_string()],
        "IMAP mail",
        vec![
            "list_mailboxes".to_string(),
            "get_emails_content".to_string(),
        ],
    );

    // Add, with the allowlist proved rather than assumed.
    let entry = client
        .add_extension_verified(&extension, true)
        .await
        .unwrap();
    assert_eq!(entry.config_key.as_deref(), Some("mail-imap"));
    assert_eq!(
        entry.extension.available_tools(),
        ["list_mailboxes", "get_emails_content"]
    );

    // The handshake: no credential, no start.
    let session = client.session_new("/home/demo").await.unwrap();
    let err = client
        .session_extension_add(&session.session_id, &extension)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("MCP_EMAIL_SERVER_PASSWORD"),
        "got: {err}"
    );

    client
        .store_secret("MCP_EMAIL_SERVER_PASSWORD", "an-app-password")
        .await
        .unwrap();
    client
        .session_extension_add(&session.session_id, &extension)
        .await
        .unwrap();

    // And the toggle the list screen drives.
    client
        .config_extension_set_enabled("mail-imap", false)
        .await
        .unwrap();
    let listed = client.config_extensions_list().await.unwrap();
    assert!(!listed.extensions[0].enabled);

    client.close();
}
