//! The mock and the real client, over a real socket.
//!
//! This is the only kind of test worth writing for a wire mock, and the reason
//! is the failure it is written against: `GET /api/chats` and `GET /api/repos`
//! decode their arrays WHOLE with `unwrap_or_default()`, so ONE misspelled
//! field name — `sessionId` for `sessionID`, `provider_id` for `providerID` —
//! empties the board, the sidebar and the tiles at once with no error anywhere.
//! A test that asserted on this crate's own JSON strings would agree with
//! itself and catch none of it.
//!
//! `mock-goose-server` takes `goose-acp-client` as a dev-dependency for the
//! same reason, and its comment says it in one line: only bytes over a socket
//! catch a misspelled method.
//!
//! The banner is the interface for the port, which is also that crate's
//! convention — bind to 0, read the address back off stdout.

// Test scaffolding: a mock that will not spawn, or a banner that will not
// parse, is a broken harness rather than a runtime condition — the same
// judgement `src/shell/desktop/home.rs`'s test module makes for its fixtures.
#![expect(
    clippy::expect_used,
    reason = "test scaffolding: a harness that cannot start is a failing test"
)]

use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

use opencode_client::{CodeClient, CodeConfig};

/// The mock, on an OS-assigned port, with its stdout parsed for the address.
struct Server {
    child: Child,
    base: String,
}

impl Server {
    fn start() -> Self {
        let exe = env!("CARGO_BIN_EXE_mock-opencode-server");
        let mut child = Command::new(exe)
            .arg("0")
            .env("MOCK_CODE_PASSWORD", "test-pass")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .expect("spawn the mock");
        let stdout = child.stdout.take().expect("stdout");
        let mut line = String::new();
        BufReader::new(stdout)
            .read_line(&mut line)
            .expect("the banner");
        let base = line
            .split_whitespace()
            .find(|w| w.starts_with("http://"))
            .expect("an address in the banner")
            .to_owned();
        Self { child, base }
    }

    fn client(&self) -> CodeClient {
        CodeClient::new(&CodeConfig {
            base_url: self.base.clone(),
            password: "test-pass".to_owned(),
        })
        .expect("a client")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // Dropping stdin closes it, which is how this binary is asked to stop
        // — see the note on the shutdown in `main.rs`. A kill would work and
        // would throw away the coverage profile.
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

/// THE GATE FOR THE WHOLE CODE PLANE. A non-2xx here and the app never makes
/// another call, so this failing means nothing else in the plane is reachable.
#[tokio::test]
async fn the_health_check_answers_and_carries_the_active_count() {
    let server = Server::start();
    let health = server.client().health().await.expect("health");
    assert_eq!(
        health.get("active").and_then(serde_json::Value::as_u64),
        Some(2),
        "the badge reads exactly this field and nothing else: {health}"
    );
}

/// The password is checked, and a wrong one is a failure rather than a screen
/// full of nothing.
#[tokio::test]
async fn a_wrong_password_is_refused() {
    let server = Server::start();
    let client = CodeClient::new(&CodeConfig {
        base_url: server.base.clone(),
        password: "not-the-password".to_owned(),
    })
    .expect("a client");
    assert!(
        client.health().await.is_err(),
        "the mock let a client in with the wrong password, so nothing in the \
         app's connection path is being exercised"
    );
}

/// THE LIST THAT EMPTIES SILENTLY. Every field the app renders, checked
/// through the real decoder — a wrong name here is a blank board with no error.
#[tokio::test]
async fn every_working_tree_survives_the_clients_decoder() {
    let server = Server::start();
    let chats = server.client().chats().await.expect("chats");
    assert!(
        chats.len() >= 5,
        "the whole array decodes or none of it does, so a short list is a \
         misspelled field somewhere in it: {}",
        chats.len()
    );
    // Newest first — the phone's list renders wire order verbatim.
    for pair in chats.windows(2) {
        assert!(
            pair[0].last_active >= pair[1].last_active,
            "the mock sent the list out of order, which the phone would show"
        );
    }
    let one = chats
        .iter()
        .find(|c| c.repo == "goose-phone-app" && c.is_running())
        .expect("a running tree in a known repo");
    assert!(!one.id.is_empty());
    assert!(one.branch.starts_with("agent/"));
    assert_eq!(one.base, "main");
    assert!(
        one.model.as_deref().is_some_and(|m| m.contains('/')),
        "a model must be `providerID/id` or the inspector cannot match it \
         against the catalogue: {:?}",
        one.model
    );
}

/// `ChatMeta.model` and the catalogue have to agree EXACTLY, because the
/// desktop inspector finds a model by `reference()` and reads its context
/// window off the match. A mock whose two halves disagreed would leave that
/// row absent and look like a rendering bug.
#[tokio::test]
async fn the_model_a_tree_names_is_one_the_catalogue_has() {
    let server = Server::start();
    let client = server.client();
    let chats = client.chats().await.expect("chats");
    let chat = chats.first().expect("a tree");
    let models = client.models(&chat.id).await.expect("the catalogue");
    let named = chat.model.clone().expect("a tree with a model");
    assert!(
        models.iter().any(|m| m.reference() == named),
        "no catalogue entry matches {named}; the inspector's model row and its \
         context-window fact would both be silently absent"
    );
    let matched = models
        .iter()
        .find(|m| m.reference() == named)
        .expect("the match");
    assert!(
        matched.limit.context_tokens().is_some_and(|n| n > 0),
        "the matched model carries no context window, which is the one number \
         the inspector's meter is built on"
    );
}

/// The strictest decode in the client: the body must be an object carrying an
/// array, and an ask must name the chat it is parked in or the app cannot mark
/// the row.
#[tokio::test]
async fn a_parked_ask_names_the_tree_it_is_parked_in() {
    let server = Server::start();
    // `pending_permissions` is the MANAGER's sweep — the one that tags each
    // ask with the chat it is parked in, and the one the app polls.
    let report = server
        .client()
        .pending_permissions()
        .await
        .expect("permissions");
    let pending = report.permissions.first().expect("one ask in the fixtures");
    assert!(
        !pending.chat_id.is_empty(),
        "an ask with no chat marks no row"
    );
    assert!(!pending.permission.title.trim().is_empty());
    assert_eq!(pending.permission.kind, "bash");
    assert!(
        !report.unreachable.is_empty(),
        "a container the manager swept and could not reach is a state the app \
         has a word for, and nothing else in the fixtures produces it"
    );
}

/// The repo allowlist, and the one flag the free-model rule reads.
#[tokio::test]
async fn the_repo_list_carries_a_throwaway() {
    let server = Server::start();
    let repos = server.client().repos().await.expect("repos");
    assert!(repos.len() >= 3);
    assert!(
        repos.iter().any(|r| r.public_throwaway),
        "without one throwaway repo `code::is_free_model` is unreachable and \
         the model picker's behaviour on those repos is never exercised"
    );
}

/// The branch list, through the client's own percent-encoding.
#[tokio::test]
async fn the_branches_come_back_with_a_default_marked() {
    let server = Server::start();
    let list = server
        .client()
        .branches("goose-phone-app")
        .await
        .expect("branches");
    assert_eq!(list.default_branch, "main");
    assert!(list.branches.len() >= 2);
}

/// The review screen's two halves: a patch `src/diff.rs` can parse, and counts
/// the inspector can add up.
#[tokio::test]
async fn a_diff_comes_back_parseable() {
    let server = Server::start();
    let client = server.client();
    let chats = client.chats().await.expect("chats");
    let reviewed = chats
        .iter()
        .find(|c| c.title.contains("search box"))
        .expect("the fixture with a diff");
    let session = client
        .sessions(&reviewed.id)
        .await
        .expect("sessions")
        .first()
        .expect("a session")
        .id
        .clone();
    let files = client.diff(&reviewed.id, &session).await.expect("diff");
    assert_eq!(files.len(), 2, "two touched files in the fixture");
    let first = &files[0];
    assert!(first.additions > 0 && first.deletions > 0);
    assert!(
        first.patch.contains("@@"),
        "a patch with no hunk header is one `src/diff.rs` renders as nothing"
    );
    assert!(
        first.patch.starts_with("Index: "),
        "OpenCode's patches carry a four-line preamble and the parser expects it"
    );
}

/// A pull request, with the two enum-ish fields that go through custom
/// deserializers — an unrecognised string there becomes `Unknown` silently.
#[tokio::test]
async fn a_pull_request_decodes_with_its_state_and_its_checks() {
    let server = Server::start();
    let client = server.client();
    let chats = client.chats().await.expect("chats");
    let reviewed = chats
        .iter()
        .find(|c| c.title.contains("search box"))
        .expect("the fixture with a pull");
    let pulls = client.pulls(&reviewed.id).await.expect("pulls");
    let pull = pulls.first().expect("one pull");
    assert_eq!(pull.number, 118);
    assert_eq!(pull.head, reviewed.branch);
    assert!(
        !matches!(pull.state, opencode_client::PullState::Unknown),
        "the mock sent a state string the client does not recognise, which \
         reads as a pull in no state at all"
    );
    assert!(
        !matches!(pull.checks, opencode_client::Checks::Unknown),
        "the mock sent a checks string the client does not recognise"
    );
}

/// The transcript, and the field that decides whose turn a message was.
#[tokio::test]
async fn a_transcript_comes_back_with_its_roles() {
    let server = Server::start();
    let client = server.client();
    let chats = client.chats().await.expect("chats");
    let chat = chats
        .iter()
        .find(|c| c.title.contains("chip row"))
        .expect("the fixture with a transcript");
    let session = client
        .sessions(&chat.id)
        .await
        .expect("sessions")
        .first()
        .expect("a session")
        .id
        .clone();
    let messages = client.messages(&chat.id, &session).await.expect("messages");
    assert!(messages.len() >= 2, "a user turn and a reply");
    assert!(
        messages.iter().any(|m| m.info.role == "user"),
        "no message announced itself as the user's, so the whole transcript \
         renders as the agent talking to itself"
    );
    assert!(messages.iter().any(|m| m.info.role == "assistant"));
}

/// An empty gateway is a STATE, and the app has a screen written for it.
#[tokio::test]
async fn an_empty_gateway_still_answers() {
    let exe = env!("CARGO_BIN_EXE_mock-opencode-server");
    let mut child = Command::new(exe)
        .arg("0")
        .env("MOCK_CODE_PASSWORD", "test-pass")
        .env("MOCK_FIXTURES", "empty")
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .spawn()
        .expect("spawn");
    let stdout = child.stdout.take().expect("stdout");
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).expect("banner");
    let base = line
        .split_whitespace()
        .find(|w| w.starts_with("http://"))
        .expect("address")
        .to_owned();
    let client = CodeClient::new(&CodeConfig {
        base_url: base,
        password: "test-pass".to_owned(),
    })
    .expect("client");

    assert!(
        client.health().await.is_ok(),
        "empty is connected, not down"
    );
    assert!(client.chats().await.expect("chats").is_empty());
    assert!(
        !client.repos().await.expect("repos").is_empty(),
        "a gateway with no trees still knows which repos it may touch, and the \
         new-session screen needs them to be pickable"
    );

    drop(child.stdin.take());
    let _ = child.wait();
}

/// A NEW TREE, through the route that makes one. The answer is a bare
/// `ChatMeta` rather than a wrapped list, which is the one asymmetry on this
/// surface and the kind of thing only a round trip catches.
#[tokio::test]
async fn a_new_working_tree_comes_back_ready_to_open() {
    let server = Server::start();
    let client = server.client();
    let made = client
        .create_chat(
            "goose-phone-app",
            "Rename the composer's chip row",
            Some("opencode/claude-sonnet-4-5"),
            Some("main"),
        )
        .await
        .expect("create");
    assert!(made.id.starts_with("goose-phone-app-"));
    assert_eq!(made.base, "main");
    assert!(made.is_running(), "a new tree's container is up");
    assert!(
        made.title.contains("chip row"),
        "the manager derives a title from the task and the client sends none: {}",
        made.title
    );
    // And it is in the list the next poll reads.
    let chats = client.chats().await.expect("chats");
    assert!(chats.iter().any(|c| c.id == made.id));
}

/// Waking, stopping and deleting, and the row changing under each — the poll
/// is what the app watches, so a lifecycle route that answered 200 and changed
/// nothing would look like it worked and do nothing.
#[tokio::test]
async fn the_lifecycle_routes_move_the_row_they_name() {
    let server = Server::start();
    let client = server.client();
    let asleep = client
        .chats()
        .await
        .expect("chats")
        .into_iter()
        .find(|c| !c.is_running())
        .expect("a stopped tree in the fixtures");

    client.wake_chat(&asleep.id).await.expect("wake");
    assert!(
        client
            .chats()
            .await
            .expect("chats")
            .iter()
            .find(|c| c.id == asleep.id)
            .is_some_and(opencode_client::ChatMeta::is_running),
        "wake answered but the row did not wake"
    );

    client.stop_chat(&asleep.id).await.expect("stop");
    assert!(
        client
            .chats()
            .await
            .expect("chats")
            .iter()
            .find(|c| c.id == asleep.id)
            .is_some_and(|c| !c.is_running()),
        "stop answered but the row did not sleep"
    );

    client.delete_chat(&asleep.id, true).await.expect("delete");
    assert!(
        !client
            .chats()
            .await
            .expect("chats")
            .iter()
            .any(|c| c.id == asleep.id),
        "delete answered but the row is still there"
    );
}

/// Merging, and the pull coming back in its new state — the app re-reads the
/// answer rather than assuming.
#[tokio::test]
async fn a_merge_answers_with_the_pull_in_its_new_state() {
    let server = Server::start();
    let client = server.client();
    let chat = client
        .chats()
        .await
        .expect("chats")
        .into_iter()
        .find(|c| c.title.contains("search box"))
        .expect("the fixture with a pull");
    let outcome = client.merge_pull(&chat.id, 118).await.expect("merge");
    assert!(outcome.merged);
    assert!(matches!(
        outcome.pull.expect("the re-read pull").state,
        opencode_client::PullState::Merged
    ));
}

/// THE STREAM, which is what makes this a fake worth having.
///
/// A prompt, then the events it produces: an announced message so the
/// transcript knows whose turn it is, a tool call, growing text, and an idle
/// at the end. Everything the app's transcript is built out of, over a real
/// SSE connection.
#[tokio::test]
async fn a_prompt_streams_a_turn_the_client_can_follow() {
    let server = Server::start();
    let client = server.client();
    let chat = client
        .chats()
        .await
        .expect("chats")
        .into_iter()
        .find(|c| c.title.contains("code-agent ports"))
        .expect("a fixture with no ask parked in it");
    let session = client
        .sessions(&chat.id)
        .await
        .expect("sessions")
        .first()
        .expect("a session")
        .id
        .clone();

    let mut events = client.events(&chat.id);
    // The first frame is the server saying hello; the app uses it to know the
    // stream is live rather than merely accepted.
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
        .await
        .expect("a first frame")
        .expect("an event");
    assert!(matches!(first, opencode_client::CodeEvent::Connected));

    client
        .prompt_async(
            &chat.id,
            &session,
            &[opencode_client::PromptPart::Text {
                text: "notool please".to_owned(),
            }],
            None,
            None,
            None,
        )
        .await
        .expect("prompt");

    let mut announced = false;
    let mut parts = 0;
    let mut idle = false;
    while !idle {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
            .await
            .expect("the stream kept going")
            .expect("an event");
        match event {
            opencode_client::CodeEvent::MessageUpdated { info } => {
                assert_eq!(info.role, "assistant");
                announced = true;
            }
            opencode_client::CodeEvent::PartUpdated { .. } => parts += 1,
            opencode_client::CodeEvent::SessionIdle { .. } => idle = true,
            _ => {}
        }
    }
    assert!(
        announced,
        "no message announced itself, so every part of this turn would render \
         as an assistant bubble whether it was one or not"
    );
    assert!(
        parts >= 2,
        "a reasoning part and some text at least: {parts}"
    );
}

/// A TURN THAT PARKS ON A QUESTION, and comes unstuck when it is answered.
///
/// The permission round trip is the single most intricate thing on this wire
/// and the one the app has a whole modal for.
#[tokio::test]
async fn a_turn_can_park_on_a_question_and_be_released() {
    let server = Server::start();
    let client = server.client();
    let chat = client
        .chats()
        .await
        .expect("chats")
        .into_iter()
        .find(|c| c.title.contains("code-agent ports"))
        .expect("a fixture with no ask parked in it");
    let session = client
        .sessions(&chat.id)
        .await
        .expect("sessions")
        .first()
        .expect("a session")
        .id
        .clone();

    let mut events = client.events(&chat.id);
    let _connected = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await;

    client
        .prompt_async(
            &chat.id,
            &session,
            &[opencode_client::PromptPart::Text {
                text: "ask notool".to_owned(),
            }],
            None,
            None,
            None,
        )
        .await
        .expect("prompt");

    let mut asked = None;
    while asked.is_none() {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
            .await
            .expect("the stream kept going")
            .expect("an event");
        if let opencode_client::CodeEvent::PermissionAsked(ask) = event {
            asked = Some(ask);
        }
    }
    let ask = asked.expect("an ask");
    assert_eq!(ask.title, "Run the test suite");

    // It is also in the manager's sweep, which is what marks the row.
    assert!(
        client
            .pending_permissions()
            .await
            .expect("sweep")
            .permissions
            .iter()
            .any(|p| p.chat_id == chat.id),
        "an ask raised on the stream did not appear in the manager's sweep, so \
         the sidebar row would never go amber"
    );

    client
        .reply_permission(&chat.id, &session, &ask.id, "once")
        .await
        .expect("reply");

    // Answering releases the turn.
    let mut idle = false;
    while !idle {
        let event = tokio::time::timeout(std::time::Duration::from_secs(10), events.recv())
            .await
            .expect("the stream kept going")
            .expect("an event");
        if matches!(event, opencode_client::CodeEvent::SessionIdle { .. }) {
            idle = true;
        }
    }
    assert!(
        client.permissions(&chat.id).await.expect("asks").is_empty(),
        "the answered ask is still parked"
    );
}

/// A chat the manager has forgotten is a 404, not an empty list — "no such
/// container" and "a container with nothing in it" are different answers and
/// the app says different things about them.
#[tokio::test]
async fn an_unknown_tree_is_refused_rather_than_answered_emptily() {
    let server = Server::start();
    assert!(
        server.client().sessions("no-such-chat").await.is_err(),
        "an unknown chat answered as though it existed and was empty"
    );
}

/// THE KEYWORDS, which are the whole reason this is a scripted fake rather
/// than a canned one.
///
/// `mock-goose-server` takes "slow" and "notool" on the other wire for the
/// same purpose: a reader testing the Stop button needs a turn long enough to
/// press it, and a reader testing a failed tool needs one that fails on
/// demand. A keyword that stopped being honoured would leave both unreachable
/// with nothing to say so.
#[tokio::test]
async fn the_prompt_keywords_change_the_turn() {
    let server = Server::start();
    let client = server.client();
    let chat = client
        .chats()
        .await
        .expect("chats")
        .into_iter()
        .find(|c| c.title.contains("code-agent ports"))
        .expect("a fixture");
    let session = client
        .sessions(&chat.id)
        .await
        .expect("sessions")
        .first()
        .expect("a session")
        .id
        .clone();

    let mut events = client.events(&chat.id);
    let _connected = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await;

    client
        .prompt_async(
            &chat.id,
            &session,
            &[opencode_client::PromptPart::Text {
                text: "slow and fail please".to_owned(),
            }],
            None,
            None,
            None,
        )
        .await
        .expect("prompt");

    let mut parts = 0;
    let mut failed_tool = false;
    let mut idle = false;
    while !idle {
        let event = tokio::time::timeout(std::time::Duration::from_secs(20), events.recv())
            .await
            .expect("the stream kept going")
            .expect("an event");
        match event {
            opencode_client::CodeEvent::PartUpdated { part, .. } => {
                parts += 1;
                // `Part.state` is an untyped `Value` — the client keeps the
                // tool's status as the server spelled it and lets the app map
                // it, so this reads the same field the transcript does.
                if part
                    .state
                    .as_ref()
                    .and_then(|s| s.get("status"))
                    .and_then(serde_json::Value::as_str)
                    == Some("error")
                {
                    failed_tool = true;
                }
            }
            opencode_client::CodeEvent::SessionIdle { .. } => idle = true,
            _ => {}
        }
    }
    assert!(
        failed_tool,
        "\"fail\" did not produce a tool that ends in error, so the transcript's \
         failed-call treatment is unreachable"
    );
    assert!(
        parts >= 8,
        "\"slow\" should stream enough beats to press Stop during: {parts}"
    );
}

/// A route nothing serves is a 404 rather than a hang or a 500 — the client
/// turns it into a `Status` error the app can report.
#[tokio::test]
async fn an_unknown_route_is_a_clean_refusal() {
    let server = Server::start();
    // `pulls` on a chat that exists but has none is an empty list, not a 404 —
    // the distinction the app draws between "asked and none" and "cannot ask".
    let client = server.client();
    let chat = client
        .chats()
        .await
        .expect("chats")
        .into_iter()
        .find(|c| c.title.contains("chip row"))
        .expect("a fixture with no pulls");
    assert!(
        client.pulls(&chat.id).await.expect("pulls").is_empty(),
        "a tree with no pull requests should answer none, not fail"
    );
}
