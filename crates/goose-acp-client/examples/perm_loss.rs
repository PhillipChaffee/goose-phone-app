//! What happens to a pending permission ask when the client stops answering?
//!
//! ANSWERED — this harness has been run, and the answer is account 2. Kept so
//! the answer can be re-checked against a new server build rather than assumed
//! to travel. Full write-up: `docs/permission-durability.md` §0.
//!
//! Three competing accounts had been written into this repository, all from
//! reading source, TWO OF THEM AS FACT:
//!
//!   1. goose answers its own question with `Permission::Cancel`, so the tool
//!      is DECLINED and the transcript says the user declined it.
//!      -> FALSIFIED. No declined tool, no `Failed` status, no tool response.
//!   2. That arm never runs on a WebSocket, because teardown is abortive and
//!      kills the actor that would poll it — so the whole ROUND is discarded.
//!      -> CONFIRMED as to outcome. The round is gone.
//!   3. Nothing happens at all, because the server never notices: there is no
//!      server-side keepalive, so a client that goes quiet is indistinguishable
//!      from one that is thinking.
//!      -> FALSIFIED. Something happens, within 75 seconds.
//!
//! Account 2's MECHANISM is still inference, though: an abort and a
//! cancel-that-was-never-persisted are indistinguishable from this side of the
//! wire. Only the server's own log tells them apart.
//!
//! MEASURED: goose 1.46.0 over a Tailscale tailnet, `mode = approve`, sessions
//! `20260827_3` and `20260827_4`. After the reconnect, `session/load` replayed
//! exactly four things — the user message, `usage_update`, `UsageUpdate`,
//! `AvailableCommandsUpdate` — and nothing else. What survives is the prompt
//! and the generated title: a session called "Run uname command" holding only
//! the request to run it.
//!
//! Each account leaves a different fingerprint in the transcript, so one run
//! tells them apart. Run it against a REAL goose (`goose serve`), never the
//! mock — the mock parks a pending ask forever on a `oneshot` and would report
//! health.
//!
//!   ask     — connect, prompt until a permission ask arrives, then park.
//!             Prints its own pid so the harness can decide how it dies.
//!   inspect — reconnect, `session/load` the session id, dump the transcript.
//!
//! HOW IT DIES is the whole experiment, and the interesting one is not a close.
//! iOS SUSPENDS a backgrounded app: the process freezes, the socket stays open,
//! and nothing reads it. `kill -STOP` is that, exactly. `kill -9` is the
//! different case where the fd is closed and the peer sees a FIN. Both were
//! run; both lost the round; the results were identical.
//!
//! One thing the run did NOT settle: WHEN the round dies. The script closes the
//! fd during cleanup before it inspects, so the `STOP` case does not prove the
//! round died while the socket was still open. `inspect` takes a session id and
//! can be pointed at a session from a *third* client while the frozen one is
//! still frozen, which is the experiment that would date it
//! (`docs/permission-durability.md` §7.4).
//!
//! # The second question: do finished side effects outlive the round?
//!
//! The run above cannot speak to it — the tool never executed, because it was
//! blocked on the permission for the whole run. `docs/permission-durability.md`
//! §7.6 is the open question and `sideeffect` + `readback` are its harness,
//! driven by `scripts/verify/side-effect-experiment.sh`.
//!
//!   sideeffect — one turn that WRITES A FILE and then asks to run a second
//!                tool. The first ask is answered `allow_once`, so that tool
//!                runs to completion and lands a mark on the server's disk;
//!                the second ask is never answered and the process parks.
//!   readback   — a SEPARATE, throwaway session on the same server that
//!                `cat`s the probe file. This is how the side effect is
//!                observed WITHOUT the transcript under test: a fresh session
//!                has its own history, so reading through it does not disturb
//!                the one being measured.
//!
//! The readback is nonce-guarded, because it is a model reporting on a file
//! and a model can invent a `ls` that succeeds. Two nonces are generated: one
//! goes in the PATH, which the readback prompt has to name, and one goes in the
//! CONTENTS, which it never mentions. Only a real read of a real file can put
//! the content nonce in the answer.
//!
//! Passing `-` in the secret position reads `GOOSE_SERVER__SECRET_KEY` from the
//! environment instead of argv, so the key is not visible to `ps` for the 75
//! seconds `sideeffect` sits parked.

// Example code: `expect` on a fixture is a failing check, and stdout is how an
// example reports what it verified. Both are denied for shipped code.
#![expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "example harness: assertions and progress output are the point"
)]

use goose_acp_client::{AcpClient, AcpEvent, ConnectConfig, ContentBlock, SessionUpdate};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let base_url = args
        .next()
        .expect("usage: perm_loss <url> <secret|-> ask|inspect|sideeffect|readback [args]");
    // `-` means "take it from the environment": argv is world-readable through
    // `ps`, and `sideeffect` sits parked for over a minute.
    let secret = match args.next().as_deref() {
        Some("-") | None => std::env::var("GOOSE_SERVER__SECRET_KEY").unwrap_or_default(),
        Some(literal) => literal.to_owned(),
    };
    let mode = args.next().unwrap_or_else(|| "ask".to_owned());
    let rest: Vec<String> = args.collect();
    let want_sid = rest.first().cloned();

    let cfg = ConnectConfig {
        base_url,
        secret,
        fingerprint: None,
    };
    let (client, mut events, info) = AcpClient::connect(&cfg).await.expect("connect");
    println!("connected: {} {}", info.agent_name, info.agent_version);

    if mode == "raw" {
        // Straight to the wire: what does THIS goose actually answer with?
        match client
            .request(
                "session/new",
                serde_json::json!({"cwd": "/tmp", "mcpServers": []}),
            )
            .await
        {
            Ok(v) => println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default()),
            Err(e) => println!("session/new failed: {e}"),
        }
        return;
    }

    if mode == "inspect" {
        let sid = want_sid.expect("inspect needs a session id");
        inspect(&client, &mut events, &sid).await;
        return;
    }

    if mode == "readback" {
        let path = rest.first().expect("readback needs a probe path");
        readback(&client, &mut events, path).await;
        return;
    }

    if mode == "sideeffect" {
        let path_nonce = rest.first().expect("sideeffect needs a path nonce");
        let content_nonce = rest.get(1).expect("sideeffect needs a content nonce");
        let shape = rest.get(2).map_or("sequential", String::as_str);
        side_effect(&client, &mut events, path_nonce, content_nonce, shape).await;
        return;
    }

    ask_and_park(&client, &mut events).await;
}

/// `session/load` a session id and dump whatever the server kept.
///
/// A reconnect is a fresh agent built from the persisted session, so this is
/// exactly what the phone sees when it comes back.
async fn inspect(
    client: &AcpClient,
    events: &mut tokio::sync::mpsc::Receiver<AcpEvent>,
    sid: &str,
) {
    match client.session_load(sid, "/tmp").await {
        Ok(v) => {
            println!("--- session/load raw ---");
            println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
        }
        Err(e) => println!("session_load failed: {e}"),
    }
    // And the update stream that load replays, which is where tool calls and
    // their statuses actually appear.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_secs(2), events.recv()).await {
            // The `Debug` rendering leads with the variant name, which is what
            // makes `grep '^REPLAY ToolCall'` a real discriminator: a script
            // cannot look for the probe nonce, because the user's prompt names
            // the probe and the user's prompt is the one thing that survives.
            Ok(Some(AcpEvent::Update { update, .. })) => {
                println!("REPLAY {}", short(&update));
            }
            Ok(Some(other)) => println!("REPLAY event: {}", summarise(&other)),
            Ok(None) | Err(_) => break,
        }
    }
}

/// The original experiment: prompt until a permission ask arrives, then park
/// on it forever and let the script decide how this process dies.
async fn ask_and_park(client: &AcpClient, events: &mut tokio::sync::mpsc::Receiver<AcpEvent>) {
    let session = client.session_new("/tmp").await.expect("session/new");
    println!("SESSION {}", session.session_id);

    // The server's default mode is `auto`, which approves every tool without
    // asking — so there would be no ask to lose. `approve` is what a phone user
    // running anything consequential would be on.
    match client
        .set_config_option(&session.session_id, "mode", "approve")
        .await
    {
        Ok(_) => println!("mode -> approve"),
        Err(e) => println!("could not set mode: {e}"),
    }

    let sid = session.session_id.clone();
    let prompt_client = client.clone();
    tokio::spawn(async move {
        // A tool the developer extension will want to run, phrased so the model
        // has nothing else to do. What matters is that it ASKS.
        let r = prompt_client
            .prompt(
                &sid,
                &[ContentBlock::text(
                    "Run the shell command `uname -a` using your developer tool. \
                     Do not explain, just run it.",
                )],
            )
            .await;
        println!("PROMPT RESOLVED: {r:?}");
    });

    println!("pid {}", std::process::id());
    while let Some(ev) = events.recv().await {
        match ev {
            AcpEvent::Permission(req) => {
                println!("ASK RECEIVED: {:?}", req.tool_call.title);
                println!(
                    "PARKED pid={} session={}",
                    std::process::id(),
                    session.session_id
                );
                // Never answer. The harness decides how this process dies.
                std::future::pending::<()>().await;
            }
            AcpEvent::Update { update, .. } => {
                println!("update: {}", short(&update));
            }
            AcpEvent::Disconnected { reason } => {
                println!("disconnected: {reason}");
                return;
            }
            other => println!("event: {other:?}"),
        }
    }
}

/// Read the probe file back through a SEPARATE, throwaway session.
///
/// This is how the side effect is observed without the transcript under test:
/// a fresh session has its own history, so nothing here disturbs the one being
/// measured. `docs/permission-durability.md` §7.6's trap is asking the SAME
/// session; this is not that.
///
/// It is still a model reporting on a file, so the caller nonce-guards it: the
/// prompt below names the PATH nonce and never the CONTENT nonce, and a model
/// that invents a successful read cannot invent the content nonce.
async fn readback(
    client: &AcpClient,
    events: &mut tokio::sync::mpsc::Receiver<AcpEvent>,
    path: &str,
) {
    // `auto` because there is nobody here to answer an ask, and because a
    // `cat` is the least consequential thing this repo asks of a server.
    let session = client.session_new("/tmp").await.expect("session/new");
    println!("READBACK SESSION {}", session.session_id);
    if let Err(e) = client
        .set_config_option(&session.session_id, "mode", "auto")
        .await
    {
        println!("could not set mode: {e}");
    }

    let sid = session.session_id.clone();
    let prompt_client = client.clone();
    let ask = format!(
        "Run the shell command `cat {path}` and reply with its exact output and \
         nothing else. If the command fails because the file does not exist, \
         reply with exactly: NO SUCH FILE."
    );
    tokio::spawn(async move {
        let r = prompt_client
            .prompt(&sid, &[ContentBlock::text(&ask)])
            .await;
        println!("READBACK PROMPT RESOLVED: {r:?}");
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while std::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await {
            Ok(Some(AcpEvent::Update { update, .. })) => match update {
                SessionUpdate::AgentMessageChunk(chunk) => {
                    println!("READBACK SAYS: {}", chunk.content.text_repr());
                }
                SessionUpdate::ToolCall(tc) | SessionUpdate::ToolCallUpdate(tc) => {
                    println!(
                        "READBACK TOOL {:?} status={:?} out={}",
                        tc.title,
                        tc.status,
                        tc.content_text()
                    );
                }
                other => println!("readback update: {}", short(&other)),
            },
            Ok(Some(AcpEvent::Disconnected { reason })) => {
                println!("readback disconnected: {reason}");
                break;
            }
            Ok(Some(_) | None) | Err(_) => {}
        }
    }

    // Leave nothing behind: this session is an instrument, not evidence.
    if let Err(e) = client.session_delete(&session.session_id).await {
        println!("could not delete readback session: {e}");
    }
}

/// One turn that finishes a tool and then asks about another, so that the
/// client can die with a completed side effect and an unanswered ask in the
/// same turn. `docs/permission-durability.md` §7.6.
///
/// The two shapes are two different experiments, because of how goose orders
/// approval against execution:
///
/// SEQUENTIAL is the shape that can actually produce a completed side effect,
/// because a round's messages are persisted at the BOTTOM of each loop
/// iteration (goose `crates/goose/src/agents/agent.rs:3339`, inside the loop
/// opened at `:2331`). A tool that finished in round 1 was already written;
/// the ask that blocks round 2 was not.
///
/// BATCHED is the control, and it is the shape this repo's severity claim
/// assumed. Source says it cannot happen: approved tools are dispatched into
/// `tool_futures` (`agent.rs:865-910`) but those futures are not POLLED until
/// the approval stream finishes (`agent.rs:2654-2683`), and
/// `handle_approval_tool_requests` awaits each confirmation in turn
/// (`crates/goose/src/agents/tool_execution.rs:183`) — so a tool approved in
/// the same message as one that is still asking never runs at all. That is a
/// prediction from reading source, which is exactly the kind of thing §0
/// falsified twice, so it is measured rather than assumed.
async fn side_effect(
    client: &AcpClient,
    events: &mut tokio::sync::mpsc::Receiver<AcpEvent>,
    path_nonce: &str,
    content_nonce: &str,
    shape: &str,
) {
    let probe = format!("/tmp/perm-loss-probe-{path_nonce}.txt");
    let session = client.session_new("/tmp").await.expect("session/new");
    println!("SESSION {}", session.session_id);
    println!("PROBE PATH {probe}");
    println!("PROBE CONTENT {content_nonce}");
    match client
        .set_config_option(&session.session_id, "mode", "approve")
        .await
    {
        Ok(_) => println!("mode -> approve"),
        Err(e) => println!("could not set mode: {e}"),
    }

    let ask = side_effect_prompt(shape, &probe, content_nonce);

    let sid = session.session_id.clone();
    let prompt_client = client.clone();
    tokio::spawn(async move {
        let r = prompt_client
            .prompt(&sid, &[ContentBlock::text(&ask)])
            .await;
        println!("PROMPT RESOLVED: {r:?}");
    });

    // What has to be true for the run to mean anything: the write was asked
    // about, answered, and REPORTED COMPLETE on the live stream, all before the
    // second ask arrived. Anything less and the transcript comparison afterwards
    // is comparing against nothing, so the harness says so in its own output
    // rather than letting the script draw a conclusion from a bad setup.
    let mut write_call_id: Option<String> = None;
    let mut write_answered = false;
    let mut write_completed = false;

    println!("pid {}", std::process::id());
    while let Some(ev) = events.recv().await {
        match ev {
            AcpEvent::Permission(req) => {
                let tool = req.tool_call.tool_name().unwrap_or("?").to_owned();
                println!("ASK: tool={tool} title={:?}", req.tool_call.title);
                // Identify the write by the probe path in its arguments rather
                // than by tool name: which tool an agent reaches for to create
                // a file is its business, and `raw_input` carries the path
                // either way.
                let is_write = format!("{:?}", req.tool_call.raw_input).contains(path_nonce);
                if is_write && !write_answered {
                    let option = req
                        .options
                        .iter()
                        .find(|o| o.option_id == "allow_once")
                        .or_else(|| req.options.first())
                        .map(|o| o.option_id.clone());
                    println!("ANSWERING the write ask with {option:?}");
                    write_call_id = Some(req.tool_call.tool_call_id.clone());
                    write_answered = true;
                    client.respond_permission(req.request_id, option);
                    continue;
                }
                println!("SECOND ASK (never answered): tool={tool}");
                report_setup(write_answered, write_completed);
                println!(
                    "PARKED pid={} session={}",
                    std::process::id(),
                    session.session_id
                );
                // Never answer this one. The script decides how this dies.
                std::future::pending::<()>().await;
            }
            AcpEvent::Update { update, .. } => {
                if let SessionUpdate::ToolCall(ref tc) | SessionUpdate::ToolCallUpdate(ref tc) =
                    update
                {
                    if write_call_id.as_deref() == Some(tc.tool_call_id.as_str())
                        && tc.status.as_deref() == Some("completed")
                    {
                        write_completed = true;
                        println!("WRITE COMPLETED (live): {}", tc.content_text());
                    }
                }
                println!("update: {}", short(&update));
            }
            AcpEvent::Disconnected { reason } => {
                println!("disconnected: {reason}");
                return;
            }
            other => println!("event: {other:?}"),
        }
    }
}

/// The prompt for each shape. Both name the probe path and the content nonce,
/// because the point is a file whose existence and contents can be checked
/// afterwards from outside the transcript.
fn side_effect_prompt(shape: &str, probe: &str, content_nonce: &str) -> String {
    if shape == "batched" {
        format!(
            "In a SINGLE message, make both of these tool calls at once, in \
             parallel, without waiting for either result:\n\
             (a) use your text editor tool to create the file {probe} whose \
             entire contents are the one line {content_nonce}\n\
             (b) use your shell tool to run `uname -a`\n\
             Do not explain anything."
        )
    } else {
        format!(
            "Do exactly two things, strictly one at a time. Never call more \
             than one tool in a single message.\n\
             STEP 1: use your text editor tool to create the file {probe} \
             whose entire contents are the one line {content_nonce}\n\
             STEP 2: only after STEP 1 has come back with a result, use your \
             shell tool to run `uname -a`.\n\
             Do not explain anything."
        )
    }
}

/// Whether this run is capable of answering anything, said out loud before the
/// process parks — so a bad setup is visible in the log rather than quietly
/// producing a "result" that is really a repeat of the answered experiment.
fn report_setup(write_answered: bool, write_completed: bool) {
    if write_answered && write_completed {
        println!("SETUP OK: the write completed before this ask arrived");
        println!("SHAPE OBSERVED sequential");
    } else if write_answered {
        println!(
            "SETUP DEGRADED: the write was approved but never reported complete \
             — both calls came in one message, so nothing ran"
        );
        println!("SHAPE OBSERVED batched");
    } else {
        println!(
            "SETUP INVALID: the non-write ask came first, so no side effect was \
             ever produced. Discard this run."
        );
        println!("SHAPE OBSERVED useless");
    }
}

fn short(u: &SessionUpdate) -> String {
    let s = format!("{u:?}");
    s.chars().take(160).collect()
}

fn summarise<T: std::fmt::Debug>(item: &T) -> String {
    let s = format!("{item:?}");
    s.chars().take(300).collect()
}
