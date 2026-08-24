//! What the mock and the client agree on when the bytes actually travel.
//!
//! One test today, and it is about the harness rather than about a feature:
//! the branches that add methods to the mock each add their own file here,
//! and they all rely on [`common::spawn_mock`] working.

// Test code: a failing unwrap IS the failing check. `expect` rather than
// `allow`: if a use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    reason = "test harness: an unwrap is the assertion"
)]

mod common;

use std::time::Duration;

use goose_acp_client::AcpEvent;

/// The whole round trip in one test: the binary starts, prints a port we did
/// not choose, answers `initialize` to a real client, creates sessions with
/// ids its own seed data implies, and reports the close.
#[tokio::test]
async fn a_session_round_trips_over_a_real_socket() {
    let (mut server, client) = common::spawn_mock().await;

    // goose numbers session ids within their day, counting from what the
    // store already holds — so consecutive creations step by one, and neither
    // lands on a session seeded earlier today. That is the mock's state
    // answering, not a default any stub would have produced.
    let first = client.session_new("/home/demo").await.unwrap().session_id;
    let second = client.session_new("/home/demo").await.unwrap().session_id;
    let number = |id: &str| id.split_once('_').unwrap().1.parse::<u32>().unwrap();
    assert_eq!(
        first.split_once('_').map(|(day, _)| day.to_string()),
        second.split_once('_').map(|(day, _)| day.to_string())
    );
    assert_eq!(number(&second), number(&first) + 1);

    client.close();
    let disconnected = tokio::time::timeout(Duration::from_secs(5), server.events.recv())
        .await
        .unwrap();
    assert!(
        matches!(disconnected, Some(AcpEvent::Disconnected { .. })),
        "expected a disconnect, got {disconnected:?}"
    );
}
