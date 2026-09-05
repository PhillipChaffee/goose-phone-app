//! What the mock and the client agree on when the bytes actually travel.
//!
//! The branches that add methods to the mock each add their own file here,
//! and they all rely on [`common::spawn_mock`] working.

// Test code: a failing unwrap IS the failing check. `expect` rather than
// `allow`: if a use goes away, so should its exception.
#![expect(
    clippy::unwrap_used,
    reason = "test harness: an unwrap is the assertion"
)]

mod common;

use std::time::Duration;

use goose_acp_client::{AcpEvent, ContentBlock, SessionUpdate, ToolCallContent};
use tokio::sync::mpsc::Receiver;

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

/// A prompt is an array, and the whole array has to survive the trip: the
/// mock reads the text out of the *text* block rather than out of block zero,
/// says the attachment count back, and replays every block on `session/load`.
///
/// This is the only gate over that behaviour. The client's own attachment
/// tests run against a stub inside their crate, so nothing else would notice
/// if the mock quietly went back to reading `prompt/0/text`.
#[tokio::test]
async fn an_attachment_reaches_the_mock_and_comes_back_on_load() {
    let (mut server, client) = common::spawn_mock().await;
    let session = client.session_new("/home/demo").await.unwrap();
    let sid = session.session_id.clone();

    // The image goes FIRST, ahead of the text: that ordering is what the old
    // `prompt/0/text` read got wrong, and "notool" has to still be found.
    client
        .prompt(
            &sid,
            &[
                ContentBlock::image("QUJD", "image/png"),
                ContentBlock::text("notool what is this photo"),
            ],
        )
        .await
        .unwrap();

    let answer: String = drain_updates(&mut server.events)
        .iter()
        .filter_map(|update| match update {
            SessionUpdate::AgentMessageChunk(chunk) => Some(chunk.content.text_repr()),
            _ => None,
        })
        .collect();
    assert!(
        answer.starts_with("Got 1 attachment."),
        "the answer should say the attachment arrived, got: {answer:?}"
    );
    assert!(
        !answer.contains("permission"),
        "\"notool\" must still be read out of the text block: {answer:?}"
    );

    // `session/load` replays the recorded conversation as notifications, so
    // the blocks come back on the event stream rather than in the reply.
    client.session_load(&sid, "/home/demo").await.unwrap();
    let replayed: Vec<ContentBlock> = drain_updates(&mut server.events)
        .into_iter()
        .filter_map(|update| match update {
            SessionUpdate::UserMessageChunk(chunk) => Some(chunk.content),
            _ => None,
        })
        .collect();
    assert_eq!(
        replayed,
        vec![
            ContentBlock::image("QUJD", "image/png"),
            ContentBlock::text("notool what is this photo"),
        ],
        "session/load must replay every prompt block, in order"
    );
}

/// A DIFF WITH BOTH HALVES TRAVELS, which is the half of #191 no unit test
/// could claim.
///
/// `goose-acp-client` decodes `{"type":"diff"}` and keeps `oldText`, and until
/// this test the only thing that had ever handed it one was a `json!` literal
/// inside its own crate. Nothing on either mock wire emitted a diff — the
/// scripted turn answers with `{"type":"content"}` and `OpenCode`'s does the
/// same — so the arm was reachable only from the tests that were written
/// alongside it, which is the arrangement that let the crate discard `oldText`
/// for its whole life without anything noticing.
///
/// So this asserts the round trip and not the parse: the mock puts the shape
/// on the socket, the real client reads it off, and BOTH texts come back
/// different from each other. `oldText` present but equal to `newText` would
/// satisfy a weaker assertion and would still be a fixture no diff card could
/// be built against.
#[tokio::test]
async fn an_edit_arrives_with_the_text_it_replaced_still_on_it() {
    let (mut server, client) = common::spawn_mock().await;
    let sid = client
        .session_new("/home/demo")
        .await
        .unwrap()
        .session_id
        .clone();
    client
        .prompt(&sid, &[ContentBlock::text("notool show me a diff")])
        .await
        .unwrap();

    let updates = drain_updates(&mut server.events);
    let started: Vec<&str> = updates
        .iter()
        .filter_map(|update| match update {
            SessionUpdate::ToolCall(call) => Some(call.tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    // The two keywords compose rather than one swallowing the other: "notool"
    // took the shell call and its permission ask out, and the edit is still
    // here.
    assert_eq!(started, ["tc_2"], "got {updates:?}");

    let diffs: Vec<_> = updates
        .iter()
        .filter_map(|update| match update {
            SessionUpdate::ToolCallUpdate(call) => Some(call.contents()),
            _ => None,
        })
        .flatten()
        .filter_map(|item| match item {
            ToolCallContent::Diff(diff) => Some(diff),
            _ => None,
        })
        .collect();

    assert_eq!(
        diffs.len(),
        1,
        "the \"diff\" keyword must put exactly one file edit on the wire, got \
         {diffs:?}"
    );
    let diff = &diffs[0];
    assert_eq!(diff.display_path(), "src/scheduler.rs");
    let old = diff.old_text.as_deref().unwrap_or_default();
    let new = diff.new_text.as_deref().unwrap_or_default();
    assert!(
        !old.is_empty() && !new.is_empty() && old != new,
        "a diff card needs two different texts to compute a deletion against \
         an addition; this fixture carries old {old:?} and new {new:?}"
    );
    assert!(
        diff.new_text
            .as_deref()
            .is_some_and(|t| t.lines().count() > old.lines().count()),
        "the fixture is meant to have an added line as well as a changed one, \
         so a renderer built on it has a `+`, a `-` and a context line to show"
    );

    // AND THE PHONE'S CARD DOES NOT MOVE. `src/views/chat.rs` renders
    // `content_text` into a `<pre>` and is shared by both shells, so the
    // structured decode is only safe to land while the flat string it is now
    // written over stays exactly what it was.
    let shown: Vec<String> = updates
        .iter()
        .filter_map(|update| match update {
            SessionUpdate::ToolCallUpdate(call) if !call.content_text().is_empty() => {
                Some(call.content_text())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        shown,
        ["[diff: src/scheduler.rs]\nfn tick() {\n    sleep(2);\n    log(\"tick\");\n}\n"],
        "the flat rendering of a diff is what the phone's tool card shows, and \
         carrying the structure alongside it must not change one byte of it"
    );
}

/// Every `session/update` queued so far, in arrival order. Draining to empty
/// is enough because the call that produced them has already resolved and the
/// mock writes them down the same socket ahead of its reply — and it has to
/// take the whole queue, not stop at the first frame that is not an update,
/// because usage reports arrive interleaved on their own goose channel.
fn drain_updates(events: &mut Receiver<AcpEvent>) -> Vec<SessionUpdate> {
    let mut updates = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let AcpEvent::Update { update, .. } = event {
            updates.push(update);
        }
    }
    updates
}
