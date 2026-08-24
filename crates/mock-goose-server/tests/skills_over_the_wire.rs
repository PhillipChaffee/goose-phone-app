//! `sources/list`, sent as a string by the client and matched as a string by
//! the mock.
//!
//! This is the one bug neither crate's unit tests can see: the client asks
//! for `_goose/unstable/sources/list` and the mock answers
//! `_goose/unstable/source/list`, both sides pass everything they test, and
//! the app shows an empty screen against a server that has the data.

// Test code: a failing unwrap IS the failing check. `expect` rather than
// `allow`: if a use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    reason = "test harness: an unwrap is the assertion"
)]

mod common;

use std::time::Duration;

use goose_acp_client::{AcpEvent, SourceType};

const PILOT: &str = "/Users/me/work/pilot";
/// The stress fixture, which is also the one that sorts between the built-ins.
const LONG: &str = "incident-postmortem-timeline-reconstruction-and-follow-up-assignment";

/// The two calls the Skills screen makes, merged, in the order it shows them.
///
/// The mock answers each half in discovery order, and the two orders
/// interleave — `goose-doc-guide` belongs between `deploy` and the long one —
/// so this list is proof the client sorted the merge rather than concatenating
/// two lists that were already in order.
#[tokio::test]
async fn skills_list_reaches_the_mock_and_comes_back_merged() {
    let (_server, client) = common::spawn_mock().await;

    let (skills, partial) = client.skills_list(Some(PILOT), false).await.unwrap();
    assert!(partial.is_none(), "both halves should have answered");

    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "code-review",
            "deploy",
            "goose-doc-guide",
            LONG,
            "pdf-form-filling",
            "subagents"
        ]
    );

    // goose omits `supportingFiles` and `properties` when they are empty, so
    // the common entry arrives with neither key — `None`, not `Some(vec![])`.
    let review = &skills[0];
    assert_eq!(review.supporting_files, None);
    assert_eq!(review.properties, None);
    assert!(review.is_editable());

    let deploy = &skills[1];
    assert_eq!(deploy.scope_label(), "This project");
    assert_eq!(deploy.supporting_file_count(), 2);
    assert!(
        deploy.extra.is_empty(),
        "the mock sent keys the client does not model: {:?}",
        deploy.extra.keys().collect::<Vec<_>>()
    );

    // The one goose really gets wrong: a built-in arrives claiming to be
    // writable, and must still not be editable.
    let builtin = &skills[2];
    assert_eq!(builtin.writable, Some(true));
    assert!(!builtin.is_editable());
    assert_eq!(builtin.scope_label(), "Built in");

    // On disk, and still not editable — the third state, and the one the
    // detail screen draws "Read-only" for.
    let bundled = &skills[4];
    assert_eq!(bundled.writable, Some(false));
    assert!(!bundled.is_editable());
    assert_eq!(bundled.scope_label(), "Global");
    assert_eq!(bundled.supporting_file_count(), 1);
}

/// `includeProjectSources`, which is a request field and therefore a spelling
/// only a round trip can check: the client sends camelCase, and what comes
/// back is a skill from a project the phone never named.
#[tokio::test]
async fn asking_for_project_sources_brings_back_the_other_projects_skills() {
    let (_server, client) = common::spawn_mock().await;

    let (without, _) = client.skills_list(Some(PILOT), false).await.unwrap();
    assert!(!without.iter().any(|s| s.name == "release-notes"));

    let (with, _) = client.skills_list(Some(PILOT), true).await.unwrap();
    // Missing here means the mock ignored the flag, or read it under some
    // other spelling than the one the client sends.
    let tagged = with.iter().find(|s| s.name == "release-notes").unwrap();
    // The tag goose adds as it walks the registry, which is how a client tells
    // this apart from a skill in the project it is pointed at.
    let properties = tagged.properties.as_ref().unwrap();
    assert_eq!(properties["projectDir"], "/Users/me/work/legacy");
    assert_eq!(tagged.scope_label(), "This project");
}

/// The degraded case the screen has a hint for: no working directory, so no
/// `projectDir`, so the project's own skills are the only thing missing.
#[tokio::test]
async fn without_a_project_dir_the_global_skills_still_arrive() {
    let (_server, client) = common::spawn_mock().await;

    let (skills, partial) = client.skills_list(None, false).await.unwrap();
    assert!(partial.is_none());
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "code-review",
            "goose-doc-guide",
            LONG,
            "pdf-form-filling",
            "subagents"
        ]
    );
}

/// A read is a read: `sources/list` must not push anything down the event
/// stream, so the first thing to arrive on it is the disconnect at the end.
///
/// The screen depends on that — its event pump folds `session/update` into
/// the open chat's transcript, and a list call that pushed one would write
/// into a conversation the user is not having.
#[tokio::test]
async fn listing_skills_notifies_nobody() {
    let (mut server, client) = common::spawn_mock().await;

    client
        .skills_list(Some("/Users/me/work/pilot"), false)
        .await
        .unwrap();
    client.close();

    let first = tokio::time::timeout(Duration::from_secs(5), server.events.recv())
        .await
        .unwrap();
    assert!(
        matches!(first, Some(AcpEvent::Disconnected { .. })),
        "a list should push no events, got {first:?}"
    );
}

/// A type the app never asks for still has to round trip, because the request
/// builder spells all six and only two of them are exercised above.
#[tokio::test]
async fn sources_list_takes_every_type_the_client_can_name() {
    let (_server, client) = common::spawn_mock().await;

    for kind in [
        SourceType::Recipe,
        SourceType::Subrecipe,
        SourceType::Agent,
        SourceType::Project,
    ] {
        let sources = client.sources_list(kind, None, false).await.unwrap();
        assert!(sources.is_empty(), "{kind:?} should be empty in the mock");
    }
}
