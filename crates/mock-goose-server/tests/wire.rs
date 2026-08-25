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

use goose_acp_client::{AcpEvent, ContentBlock, SessionUpdate};
use tokio::sync::mpsc::Receiver;

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
