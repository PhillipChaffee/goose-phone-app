//! The mock reproducing the failure that was measured on a real goose.
//!
//! MEASURED, not read (`docs/permission-durability.md` section 0): goose
//! 1.46.0 over the tailnet, `mode = approve`, a client that receives a
//! permission ask and never answers. Sessions `20260827_3` (`kill -STOP`, the
//! socket left open, which is what iOS does to a backgrounded app) and
//! `20260827_4` (`kill -9`). Both replayed exactly four things on
//! `session/load`:
//!
//! ```text
//! REPLAY UserMessageChunk(... "Run the shell command `uname -a` ...")
//! REPLAY GooseUpdate usage_update
//! REPLAY UsageUpdate
//! REPLAY AvailableCommandsUpdate
//! ```
//!
//! No `ToolCall`. No assistant message. No decline. The round is gone — and
//! the prompt and the generated title are not, so the session comes back
//! named after work that has no trace of having happened.
//!
//! The default mock does NOT do this: `Turn::ask_permission` parks on a
//! oneshot whose sender outlives the socket, so its turn simply waits forever
//! and a regression test written against it passes on a server that never had
//! the bug. `MOCK_DIE_ON_CLOSE=abort` is the mode that makes the mock able to
//! be wrong in the way the real server is wrong, and this file is the gate
//! over it.

// Test code: a failing unwrap or a wrong-variant panic IS the failing check.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test harness: an unwrap or a wrong-variant panic is the assertion"
)]

mod common;

use std::time::Duration;

use goose_acp_client::{AcpClient, AcpEvent, ContentBlock, SessionUpdate};
use tokio::sync::mpsc::Receiver;

/// Prompt the session and park on the ask, without ever answering it.
///
/// The prompt is deliberately not awaited: `session/prompt` stays pending for
/// the whole agent turn, and the whole point of this scenario is that the turn
/// never ends. Returns the title the ask carried, which is the one string the
/// client knew about the lost work.
async fn ask_and_abandon(client: &AcpClient, events: &mut Receiver<AcpEvent>, sid: &str) -> String {
    let prompt = client.clone();
    let session = sid.to_owned();
    tokio::spawn(async move {
        let _ = prompt
            .prompt(&session, &[ContentBlock::text("Run uname and report back")])
            .await;
    });

    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("no permission ask within 10s")
            .expect("event stream ended before the ask");
        if let AcpEvent::Permission(request) = event {
            return request.tool_call.title.unwrap_or_default();
        }
    }
}

/// Everything `session/load` replayed, drained to empty.
async fn replay(
    client: &AcpClient,
    events: &mut Receiver<AcpEvent>,
    sid: &str,
) -> Vec<SessionUpdate> {
    client.session_load(sid, "/home/demo").await.unwrap();
    let mut updates = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let AcpEvent::Update { update, .. } = event {
            updates.push(update);
        }
    }
    updates
}

fn user_text(updates: &[SessionUpdate]) -> String {
    updates
        .iter()
        .filter_map(|u| match u {
            SessionUpdate::UserMessageChunk(chunk) => Some(chunk.content.text_repr()),
            _ => None,
        })
        .collect()
}

fn tool_calls(updates: &[SessionUpdate]) -> Vec<String> {
    updates
        .iter()
        .filter_map(|u| match u {
            SessionUpdate::ToolCall(call) | SessionUpdate::ToolCallUpdate(call) => Some(
                call.title
                    .clone()
                    .unwrap_or_else(|| call.tool_call_id.clone()),
            ),
            _ => None,
        })
        .collect()
}

/// The measured shape, end to end: ask, die, come back, look.
///
/// Everything asserted here is quoted from the run in section 0 — what is
/// gone, and, just as important, what is not.
#[tokio::test]
async fn a_round_abandoned_on_an_ask_keeps_only_the_prompt_and_the_title() {
    let (mut server, client) = common::spawn_mock_with(&[("MOCK_DIE_ON_CLOSE", "abort")]).await;
    let sid = client.session_new("/home/demo").await.unwrap().session_id;

    let asked = ask_and_abandon(&client, &mut server.events, &sid).await;
    assert_eq!(asked, "shell: uname -a", "the client saw the ask");

    // The client goes away without answering. `close` is the `kill -9` shape:
    // the peer sees the socket end. The `kill -STOP` shape — frozen with the
    // fd still open — is not reproducible in-process, and section 0 measured
    // the two producing an identical replay anyway.
    client.close();
    drop(client);

    // A fresh client, exactly as the app does after `reconnect_loop`.
    let (client, mut events) = server.reconnect().await;
    let replayed = replay(&client, &mut events, &sid).await;

    assert!(
        tool_calls(&replayed).is_empty(),
        "the round survived: {:?}",
        tool_calls(&replayed)
    );
    assert!(
        !replayed
            .iter()
            .any(|u| matches!(u, SessionUpdate::AgentMessageChunk(_))),
        "an assistant message survived the round that produced it"
    );
    assert_eq!(
        user_text(&replayed),
        "Run uname and report back",
        "the prompt is the one thing that survives"
    );

    // And the cruel part: it is still named after the work.
    let listed = client
        .session_list(&goose_acp_client::SessionQuery::new(
            &goose_acp_client::SessionKind::ALL,
            None,
        ))
        .await
        .unwrap();
    let title = listed
        .sessions
        .iter()
        .find(|s| s.session_id == sid)
        .unwrap_or_else(|| panic!("session {sid} is not in the list"))
        .display_title();
    assert_eq!(
        title, "Run uname and report back",
        "the generated title survives the round it was generated from"
    );
}

/// The contrast, stated so nobody has to rediscover it.
///
/// Without the switch the mock parks the ask forever and writes nothing at
/// all — not even the prompt. That is neither the old behaviour being right
/// nor the new one: it is a third thing, and it is why a green `cargo test`
/// against the default mock has never said anything about this failure.
#[tokio::test]
async fn the_default_mock_still_cannot_reproduce_it() {
    let (mut server, client) = common::spawn_mock().await;
    let sid = client.session_new("/home/demo").await.unwrap().session_id;
    ask_and_abandon(&client, &mut server.events, &sid).await;
    client.close();
    drop(client);

    let (client, mut events) = server.reconnect().await;
    let replayed = replay(&client, &mut events, &sid).await;
    assert!(
        replayed.is_empty(),
        "the default mock persists nothing for an abandoned round, \
         so it cannot be used to test one: {replayed:?}"
    );
}
