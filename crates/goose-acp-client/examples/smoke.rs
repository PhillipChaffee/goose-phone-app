//! Protocol smoke test against a running server:
//!   cargo run -p goose-acp-client --example smoke -- http://127.0.0.1:3284 SECRET [--prompt]
//!
//! `--watch` instead connects and idles, reporting how long until the
//! connection is declared dead — used to verify ping-timeout detection
//! against a server that stops responding (see MOCK_SILENT in
//! mock-goose-server).

// Test/example code: unwrapping a fixture is a failing check, and stdout is
// how an example reports what it verified. Both are denied for shipped code.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "test/example harness: assertions and progress output are the point"
)]


use std::time::Instant;

use goose_acp_client::{probe, AcpClient, AcpEvent, ConnectConfig, SessionUpdate};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let base_url = args
        .next()
        .expect("usage: smoke <base_url> <secret> [--prompt|--watch]");
    let secret = args.next().unwrap_or_default();
    let mode = args.next();
    let run_prompt = mode.as_deref() == Some("--prompt");

    if mode.as_deref() == Some("--watch") {
        let cfg = ConnectConfig {
            base_url,
            secret,
            fingerprint: None,
        };
        let started = Instant::now();
        let (_client, mut events, info) = AcpClient::connect(&cfg).await.expect("connect");
        println!("connected to {} — idling", info.agent_name);
        while let Some(event) = events.recv().await {
            if let AcpEvent::Disconnected { reason } = event {
                println!(
                    "disconnected after {:.0}s: {reason}",
                    started.elapsed().as_secs_f64()
                );
                return;
            }
        }
        println!("event stream ended without a Disconnected event");
        return;
    }

    println!("probe: {:?}", probe(&base_url, &secret, false).await);

    let cfg = ConnectConfig {
        base_url,
        secret,
        fingerprint: None,
    };
    let (client, mut events, info) = AcpClient::connect(&cfg).await.expect("connect");
    println!("connected: {} {}", info.agent_name, info.agent_version);

    let list = client.session_list(None).await.expect("session/list");
    println!("sessions: {}", list.sessions.len());
    for s in &list.sessions {
        println!(
            "  {} title={:?} msgs={:?} snippet={:?}",
            s.session_id,
            s.title,
            s.message_count(),
            s.last_message_snippet()
        );
    }

    if run_prompt {
        let session = client.session_new("/tmp").await.expect("session/new");
        println!("new session: {}", session.session_id);
        let sid = session.session_id.clone();

        let pump = tokio::spawn(async move {
            while let Some(ev) = events.recv().await {
                match ev {
                    AcpEvent::Update { update, .. } => match update {
                        SessionUpdate::AgentMessageChunk(c) => {
                            print!("{}", c.content.text_repr());
                        }
                        SessionUpdate::AgentThoughtChunk(_) => print!("·"),
                        SessionUpdate::ToolCall(t) => {
                            println!("\n[tool_call {} {:?}]", t.tool_call_id, t.title)
                        }
                        SessionUpdate::ToolCallUpdate(t) => {
                            println!("\n[tool_update {} {:?}]", t.tool_call_id, t.status)
                        }
                        SessionUpdate::SessionInfoUpdate(i) => {
                            println!("\n[title -> {:?}]", i.title)
                        }
                        _ => {}
                    },
                    AcpEvent::Permission(p) => {
                        println!(
                            "\n[permission request for {:?} -> allowing once]",
                            p.tool_call.title
                        );
                        // client handle unavailable here; answered by main task below
                        return Some(p);
                    }
                    AcpEvent::Disconnected { reason } => {
                        println!("\n[disconnected: {reason}]");
                        return None;
                    }
                    _ => {}
                }
            }
            None
        });

        let prompt_client = client.clone();
        let prompt_task =
            tokio::spawn(async move { prompt_client.prompt(&sid, "smoke test: run a tool").await });

        if let Ok(Some(p)) = pump.await {
            client.respond_permission(p.request_id, Some("allow_once".into()));
        }
        match prompt_task.await.expect("join") {
            Ok(stop) => println!("\nstopReason: {stop}"),
            Err(e) => println!("\nprompt error: {e}"),
        }
    }
    client.close();
}
