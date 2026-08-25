//! `session/list` and `session/rename` over a real socket: the filters, the
//! search, the paging and the title the two sides have to agree about.
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

use goose_acp_client::{AcpClient, AcpEvent, SessionInfo, SessionKind, SessionQuery};

fn titles(sessions: &[SessionInfo]) -> Vec<String> {
    sessions.iter().map(SessionInfo::display_title).collect()
}

/// Every page of a filter, walked the way the app walks them: each cursor
/// comes from the query that produced the page before it.
async fn walk(client: &AcpClient, query: &SessionQuery) -> Vec<SessionInfo> {
    let mut query = query.clone();
    let mut sessions = Vec::new();
    loop {
        let page = client.session_list(&query).await.unwrap();
        let next = query.next_page(&page);
        sessions.extend(page.sessions);
        let Some(next) = next else {
            return sessions;
        };
        query = next;
        assert!(sessions.len() < 100, "the cursor never ran out");
    }
}

/// The kinds filter, end to end: the mock seeds sessions of all three kinds,
/// so each filter returns its own and the labels come back with them.
#[tokio::test]
async fn kinds_filter_and_label_survive_the_wire() {
    let (_server, client) = common::spawn_mock().await;

    let scheduled = walk(&client, &SessionQuery::new(&[SessionKind::Scheduled], None)).await;
    assert_eq!(
        titles(&scheduled),
        ["Nightly dependency audit", "Weekly changelog digest"]
    );
    assert!(scheduled
        .iter()
        .all(|s| s.kind() == Some(SessionKind::Scheduled)));
    assert_eq!(scheduled[0].kind_label(), Some("Scheduled"));

    // The kind the app hid from itself for its whole life is a `user` filter
    // away from being invisible again.
    let users = walk(&client, &SessionQuery::new(&[SessionKind::User], None)).await;
    assert!(users.iter().all(|s| s.kind_label().is_none()));
    assert!(titles(&users).contains(&"Seeded example chat".to_string()));

    let agents = walk(&client, &SessionQuery::new(&[SessionKind::Acp], None)).await;
    assert_eq!(agents[0].kind_label(), Some("Agent"));

    // Every list row's last line is there because the client asked for it:
    // the mock leaves the snippet off unless `includeLastMessageSnippet`
    // arrives spelled the way goose spells it.
    assert!(users.iter().all(|s| s.last_message_snippet().is_some()));

    client.close();
}

/// The search runs on the server, across every kind, and reads the messages
/// rather than the titles — a session whose title says nothing about
/// advisories is the one that comes back.
#[tokio::test]
async fn the_query_is_the_servers_own_search() {
    let (_server, client) = common::spawn_mock().await;

    let hits = walk(
        &client,
        &SessionQuery::new(&SessionKind::ALL, Some(" advisories ")),
    )
    .await;
    assert_eq!(titles(&hits), ["Nightly dependency audit"]);

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
    let next = query.next_page(&first).unwrap();
    let second = client.session_list(&next).await.unwrap();
    assert_eq!(first.sessions.len(), second.sessions.len());

    let all = walk(&client, &query).await;
    assert!(all.len() > first.sessions.len() * 2, "expected three pages");
    let mut ids: Vec<&str> = all.iter().map(|s| s.session_id.as_str()).collect();
    let listed = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), listed, "a session came back on two pages");

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

/// The whole feature in the order a person performs it: list, search, page,
/// rename, list again. Every method string in it crosses the socket, which is
/// the only place a typo in one is visible.
#[tokio::test]
async fn a_search_a_page_and_a_rename_all_land_on_the_server() {
    let (_server, client) = common::spawn_mock().await;

    let all = SessionQuery::new(&SessionKind::ALL, None);
    let first = client.session_list(&all).await.unwrap();
    assert!(!first.sessions.is_empty());

    // A search is its own first page, never the old cursor with a new
    // keyword — which is the request goose answers `invalid_params` to.
    let audit = SessionQuery::new(&SessionKind::ALL, Some("audit"));
    let hits = client.session_list(&audit).await.unwrap();
    assert_eq!(
        titles(&hits.sessions),
        ["Nightly dependency audit", "Sub-agent: summarise the audit"]
    );

    let second = client
        .session_list(&all.next_page(&first).unwrap())
        .await
        .unwrap();
    assert!(!second.sessions.is_empty());

    let renamed = &first.sessions[1];
    client
        .session_rename(&renamed.session_id, "Renamed from the phone")
        .await
        .unwrap();

    let after = client.session_list(&all).await.unwrap();
    assert_eq!(
        after.sessions[1].display_title(),
        "Renamed from the phone",
        "the rename should be the title the next list shows"
    );
    // Renaming does not reorder: goose sorts on the last message, and nobody
    // said anything.
    assert_eq!(after.sessions[1].session_id, renamed.session_id);

    client.close();
}
