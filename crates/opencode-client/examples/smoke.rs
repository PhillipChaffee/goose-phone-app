//! Smoke test against a live code-agent plane — either the real brain or the
//! containerless test stack from personal-ai-setup:
//!
//! ```bash
//! # terminal 1 (in personal-ai-setup):
//! scripts/verify/test-code-agent-manager.sh --serve
//! # terminal 2 (here):
//! CODE_BASE_URL=http://127.0.0.1:4399 CODE_PASSWORD=<printed> \
//!   cargo run -p opencode-client --example smoke
//! ```
//!
//! Drives the full client path the app uses: health → repos → create chat →
//! create session → SSE attach → prompt (a push+PR task) → answer the
//! blocking `git push` permission ask → collect streamed deltas → wait for
//! idle → verify the PR URL in the transcript → diff → delete. Exits
//! non-zero on any failure.

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


use opencode_client::{CodeClient, CodeConfig, CodeEvent};

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("set {name}"))
}

struct Score {
    pass: u32,
    fail: u32,
}

impl Score {
    fn check(&mut self, what: &str, ok: bool, detail: &str) {
        if ok {
            self.pass += 1;
            println!("PASS  {what}");
        } else {
            self.fail += 1;
            println!("FAIL  {what} — {detail}");
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut s = Score { pass: 0, fail: 0 };
    let client = CodeClient::new(&CodeConfig {
        base_url: env("CODE_BASE_URL"),
        password: env("CODE_PASSWORD"),
    })
    .expect("client build");

    let health = client.health().await;
    s.check("manager health", health.is_ok(), &format!("{health:?}"));

    let repos = client.repos().await.unwrap_or_default();
    s.check(
        "repo allowlist non-empty",
        !repos.is_empty(),
        "no repos — is the test stack up?",
    );
    let repo = repos
        .first().map_or_else(|| "testrepo".into(), |r| r.name.clone());

    let chat = match client.create_chat(&repo, "smoke: push and PR", None).await {
        Ok(c) => c,
        Err(e) => {
            println!("FAIL  create chat — {e}");
            std::process::exit(1);
        }
    };
    s.check("chat created", !chat.id.is_empty(), "empty id");
    s.check(
        "chat branch is agent/-prefixed",
        chat.branch.starts_with("agent/"),
        &chat.branch,
    );

    let session = client.create_session(&chat.id).await.expect("session");
    s.check("session created", !session.id.is_empty(), "empty id");

    let mut events = client.events(&chat.id);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    client
        .prompt_async(
            &chat.id,
            &session.id,
            "push the branch and open a pull request",
            None,
        )
        .await
        .expect("prompt_async");

    // Consume the stream: answer the push ask when it arrives, accumulate
    // deltas, stop at idle.
    let mut saw_permission = false;
    let mut deltas = String::new();
    let mut idle = false;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    while let Ok(Some(event)) = tokio::time::timeout_at(deadline, events.recv()).await {
        match event {
            CodeEvent::PermissionAsked(p) => {
                saw_permission = true;
                client
                    .reply_permission(&chat.id, &p.session_id, &p.id, "once")
                    .await
                    .expect("permission reply");
            }
            CodeEvent::PartUpdated { delta: Some(d), .. } => deltas.push_str(&d),
            CodeEvent::SessionIdle { session_id } if session_id == session.id => {
                idle = true;
                break;
            }
            CodeEvent::Disconnected { reason } => {
                println!("FAIL  stream dropped — {reason}");
                break;
            }
            _ => {}
        }
    }
    s.check(
        "blocking git-push permission ask arrived and was answered",
        saw_permission,
        "no ask on the stream",
    );
    s.check(
        "streamed deltas accumulated",
        deltas.contains("checking the workspace"),
        &format!("got: {deltas:?}"),
    );
    s.check("turn reached session.idle", idle, "no idle before timeout");

    let msgs = client
        .messages(&chat.id, &session.id)
        .await
        .unwrap_or_default();
    let transcript: String = msgs
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| p.text.clone())
        .collect();
    s.check(
        "PR URL in the transcript",
        transcript.contains("/pull/"),
        &transcript.chars().take(200).collect::<String>(),
    );

    let diff = client.diff(&chat.id, &session.id).await;
    s.check(
        "diff endpoint",
        diff.as_ref().map(serde_json::value::Value::is_array).unwrap_or(false),
        &format!("{diff:?}"),
    );

    let deleted = client.delete_chat(&chat.id, true).await;
    s.check(
        "chat deleted (purged)",
        deleted.is_ok(),
        &format!("{deleted:?}"),
    );

    println!("\n== smoke: {} passed, {} failed ==", s.pass, s.fail);
    if s.fail > 0 {
        std::process::exit(1);
    }
}
