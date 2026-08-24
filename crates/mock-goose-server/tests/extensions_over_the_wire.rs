//! The extensions flow end to end, over a real WebSocket, against the real
//! mock binary.
//!
//! The unit tests next to the handlers check the config store's behaviour;
//! this one checks that the methods are actually *reachable* — that
//! `_goose/unstable/...` names are routed, that the client's frames parse on
//! the server and the server's replies parse on the client. A typo in a method
//! string is invisible to both sides' unit tests and fatal in the app.

// Test code: a failing unwrap IS the failing assertion.
#![expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test harness: an unwrap or a wrong-variant panic is the assertion"
)]

mod common;

use std::time::Duration;

use goose_acp_client::{AcpError, AcpEvent, GooseExtension, McpServer, StdioMcpServer};
use serde_json::Value;

#[tokio::test]
async fn the_whole_extensions_flow_works_over_the_wire() {
    let (mut server, client) = common::spawn_mock().await;

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

    // The handshake: no credential, no start. And it runs with no session
    // open — `verify_extension_starts` makes a throwaway one — because a
    // fresh install has never opened a chat, which is exactly when a mistyped
    // credential most needs catching.
    let err = client
        .verify_extension_starts(None, "/home/demo", &extension)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("MCP_EMAIL_SERVER_PASSWORD"),
        "got: {err}"
    );
    // The reason reached us through `data`; `message` is the canned text and
    // carries nothing a user could act on. Asserting both keeps the mock
    // honest about the shape goose actually sends.
    match &err {
        AcpError::Rpc { message, data, .. } => {
            assert_eq!(message, "Internal error");
            assert!(
                data.as_ref()
                    .and_then(Value::as_str)
                    .is_some_and(|d| d.contains("MCP_EMAIL_SERVER_PASSWORD")),
                "the reason must ride in data: {data:?}"
            );
        }
        other => panic!("expected an Rpc error, got {other:?}"),
    }

    client
        .store_secret("MCP_EMAIL_SERVER_PASSWORD", "an-app-password")
        .await
        .unwrap();
    client
        .verify_extension_starts(None, "/home/demo", &extension)
        .await
        .unwrap();

    // And with a session in hand, that one is used.
    let session = client.session_new("/home/demo").await.unwrap();
    client
        .verify_extension_starts(Some(&session.session_id), "/home/demo", &extension)
        .await
        .unwrap();

    // And the toggle the list screen drives. Looked up by key rather than by
    // position: the default fixture set arrives with a config already in it,
    // so index 0 would be asserting about the seed instead of about the write.
    client
        .config_extension_set_enabled("mail-imap", false)
        .await
        .unwrap();
    let listed = client.config_extensions_list().await.unwrap();
    let mail = listed
        .extensions
        .iter()
        .find(|e| e.config_key.as_deref() == Some("mail-imap"))
        .unwrap();
    assert!(!mail.enabled);

    client.close();
    let disconnected = tokio::time::timeout(Duration::from_secs(5), server.events.recv())
        .await
        .unwrap();
    assert!(
        matches!(disconnected, Some(AcpEvent::Disconnected { .. })),
        "expected a disconnect, got {disconnected:?}"
    );
}
