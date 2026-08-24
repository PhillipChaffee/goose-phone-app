//! `session/list` over a real socket: the filters, the search and the paging
//! the two sides have to agree about.
//!
//! Both crates spell the three session kinds out for themselves, and the
//! client mints its own cursor pairing — neither unit-test suite can see the
//! other being wrong about either. This is where they meet.

// Test code: a failing unwrap IS the failing check. `expect` rather than
// `allow`: if a use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    reason = "test harness: an unwrap is the assertion"
)]

mod common;

use std::time::Duration;

use goose_acp_client::{AcpEvent, SessionKind, SessionQuery};

/// The kinds filter, end to end: the mock seeds one session of each kind, so
/// each filter returns its own and the labels come back with them.
#[tokio::test]
async fn kinds_filter_and_label_survive_the_wire() {
    let (_server, client) = common::spawn_mock().await;

    let scheduled = client
        .session_list(&SessionQuery::new(&[SessionKind::Scheduled], None))
        .await
        .unwrap();
    let ids: Vec<&str> = scheduled
        .sessions
        .iter()
        .map(|s| s.session_id.as_str())
        .collect();
    assert_eq!(ids, ["20260819_1"]);
    assert_eq!(scheduled.sessions[0].kind(), Some(SessionKind::Scheduled));
    assert_eq!(scheduled.sessions[0].kind_label(), Some("Scheduled"));

    // The kind the app hid from itself for its whole life is a `user` filter
    // away from being invisible again.
    let users = client
        .session_list(&SessionQuery::new(&[SessionKind::User], None))
        .await
        .unwrap();
    assert!(users.sessions.iter().all(|s| s.kind_label().is_none()));

    let agent = client
        .session_list(&SessionQuery::new(&[SessionKind::Acp], None))
        .await
        .unwrap();
    assert_eq!(agent.sessions[0].kind_label(), Some("Agent"));

    client.close();
}

/// The search runs on the server, across every kind, and finds a session no
/// page of the unfiltered list would have reached first.
#[tokio::test]
async fn the_query_is_the_servers_own_search() {
    let (_server, client) = common::spawn_mock().await;

    let hits = client
        .session_list(&SessionQuery::new(&SessionKind::ALL, Some(" advisories ")))
        .await
        .unwrap();
    let ids: Vec<&str> = hits
        .sessions
        .iter()
        .map(|s| s.session_id.as_str())
        .collect();
    assert_eq!(ids, ["20260819_1"]);

    client.close();
}

/// Paging with the cursor the server minted, which is the only pairing
/// [`SessionQuery`] can produce — and the server re-hashes the filters to
/// check it. The pages tile the list and the last one ends the chain.
#[tokio::test]
async fn pages_walk_the_list_with_the_cursor_the_filters_minted() {
    let (mut server, client) = common::spawn_mock().await;

    let query = SessionQuery::new(&SessionKind::ALL, None);
    let first = client.session_list(&query).await.unwrap();
    assert_eq!(first.sessions.len(), 2);

    let next = query.next_page(&first).unwrap();
    let second = client.session_list(&next).await.unwrap();
    assert_eq!(second.sessions.len(), 1);
    assert_eq!(second.sessions[0].session_id, "20260818_1");
    assert!(
        next.next_page(&second).is_none(),
        "the last page should end the chain"
    );

    // Listing is a plain request/response: nothing was pushed at us, and the
    // only event on the stream is the close we asked for.
    client.close();
    let disconnected = tokio::time::timeout(Duration::from_secs(5), server.events.recv())
        .await
        .unwrap();
    assert!(
        matches!(disconnected, Some(AcpEvent::Disconnected { .. })),
        "expected a disconnect, got {disconnected:?}"
    );
}
