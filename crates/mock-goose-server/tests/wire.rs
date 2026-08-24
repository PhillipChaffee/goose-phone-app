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
/// not choose, answers `initialize` to a real client, creates a session with
/// the id its own seed data implies, and reports the close.
#[tokio::test]
async fn a_session_round_trips_over_a_real_socket() {
    let (mut server, client) = common::spawn_mock().await;

    let session = client.session_new("/home/demo").await.unwrap();
    // `seed` leaves the counter at 2, so this is the mock's state and not a
    // default that any stub would have produced.
    assert_eq!(session.session_id, "20260821_2");

    client.close();
    let disconnected = tokio::time::timeout(Duration::from_secs(5), server.events.recv())
        .await
        .unwrap();
    assert!(
        matches!(disconnected, Some(AcpEvent::Disconnected { .. })),
        "expected a disconnect, got {disconnected:?}"
    );
}
