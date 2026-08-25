//! Protocol smoke test against a running server:
//!   cargo run -p goose-acp-client --example smoke -- <http://127.0.0.1:3284> SECRET [--prompt]
//!
//! `--config` instead opens a session, prints every `configOptions` entry it
//! arrived with, and sets one — the path the composer's session settings
//! sheet runs on. It also shows the refreshed set afterwards, including the
//! case where switching model collapses thinking effort to a single value.
//!
//! `--watch` instead connects and idles, reporting how long until the
//! connection is declared dead — used to verify ping-timeout detection
//! against a server that stops responding (see `MOCK_SILENT` in
//! mock-goose-server).

// Example code: `expect` on a fixture is a failing check, and stdout is how an
// example reports what it verified. Both are denied for shipped code. `expect`
// rather than `allow`: if a use goes away, so should its exception.
#![expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "example harness: assertions and progress output are the point"
)]

use std::time::Instant;

use goose_acp_client::{
    probe, AcpClient, AcpEvent, ConnectConfig, ContentBlock, SessionKind, SessionUpdate,
};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let base_url = args
        .next()
        .expect("usage: smoke <base_url> <secret> [--prompt|--watch]");
    let secret = args.next().unwrap_or_default();
    let mode = args.next();
    let run_prompt = mode.as_deref() == Some("--prompt");
    let run_config = mode.as_deref() == Some("--config");

    if mode.as_deref() == Some("--watch") {
        watch(base_url, secret).await;
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

    let list = client
        .session_list(&[SessionKind::User], None)
        .await
        .expect("session/list");
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

    if run_config {
        check_config_options(&client).await;
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
                            println!("\n[tool_call {} {:?}]", t.tool_call_id, t.title);
                        }
                        SessionUpdate::ToolCallUpdate(t) => {
                            println!("\n[tool_update {} {:?}]", t.tool_call_id, t.status);
                        }
                        SessionUpdate::SessionInfoUpdate(i) => {
                            println!("\n[title -> {:?}]", i.title);
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
        let prompt_task = tokio::spawn(async move {
            prompt_client
                .prompt(&sid, &[ContentBlock::text("smoke test: run a tool")])
                .await
        });

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

/// Connect and idle, reporting how long the connection survives — the
/// ping-timeout check.
async fn watch(base_url: String, secret: String) {
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
}

/// Open a session, show what it can be configured with, and change one thing.
///
/// Which option gets changed is picked off the array rather than named, for
/// the same reason the settings sheet names no ids: the agent decides what it
/// offers, and a client that hardcodes the list goes stale silently.
async fn check_config_options(client: &AcpClient) {
    let session = client.session_new("/tmp").await.expect("session/new");
    println!("new session: {}", session.session_id);
    report_options("on session/new", &session.config_options);

    let target = session
        .config_options
        .iter()
        .rev()
        .find(|o| o.is_adjustable())
        .expect("no adjustable config option offered");
    let next = target
        .options
        .iter()
        .find(|c| Some(c.value.as_str()) != target.current_value.as_deref())
        .expect("adjustable option with no alternative value");
    println!(
        "\nsetting {} = {} (was {:?})",
        target.config_id, next.value, target.current_value
    );
    let refreshed = client
        .set_config_option(&session.session_id, &target.config_id, &next.value)
        .await
        .expect("session/set_config_option");
    report_options("after the change", &refreshed);
}

/// Print an option set the way the settings sheet reads it: what it is, what
/// it is set to, and whether there is anything to choose between.
fn report_options(when: &str, options: &[goose_acp_client::ConfigOption]) {
    println!("configOptions {when}: {}", options.len());
    for option in options {
        println!(
            "  {} ({}) = {:?} — {} {}",
            option.config_id,
            option.name,
            option.current_value,
            option.options.len(),
            if option.is_adjustable() {
                "values: adjustable"
            } else {
                "value: a fact, not a control"
            }
        );
    }
}
