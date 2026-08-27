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
        .expect("usage: perm_loss <url> <secret> ask|inspect [sid]");
    let secret = args.next().unwrap_or_default();
    let mode = args.next().unwrap_or_else(|| "ask".to_owned());
    let want_sid = args.next();

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
        // A reconnect is a fresh agent built from the persisted session, so
        // this is exactly what the phone sees when it comes back.
        // session/load replays the persisted transcript. Whatever the server
        // kept is what the phone would see on coming back.
        match client.session_load(&sid, "/tmp").await {
            Ok(v) => {
                println!("--- session/load raw ---");
                println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            Err(e) => println!("session_load failed: {e}"),
        }
        // And the update stream that load replays, which is where tool calls
        // and their statuses actually appear.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_secs(2), events.recv()).await {
                Ok(Some(AcpEvent::Update { update, .. })) => {
                    println!("REPLAY {}", short(&update));
                }
                Ok(Some(other)) => println!("REPLAY event: {}", summarise(&other)),
                Ok(None) | Err(_) => break,
            }
        }
        return;
    }

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

fn short(u: &SessionUpdate) -> String {
    let s = format!("{u:?}");
    s.chars().take(160).collect()
}

fn summarise<T: std::fmt::Debug>(item: &T) -> String {
    let s = format!("{item:?}");
    s.chars().take(300).collect()
}
