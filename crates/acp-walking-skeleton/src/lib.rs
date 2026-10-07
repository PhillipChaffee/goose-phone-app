//! PROTOTYPE — throwaway. Wayfinder ticket 10: prove the five walking-skeleton
//! beats from this repo's phone base against a real `goose serve`, using the
//! official `agent-client-protocol` 2.0.0 stack pinned to the server's rev
//! (`agentclientprotocol/rust-sdk@c97a520`). Nothing here is production code;
//! when the ticket closes, the answers live in the ticket and this crate dies.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use agent_client_protocol::schema::v1::{
    InitializeRequest, PermissionOptionKind, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SelectedPermissionOutcome, SessionNotification, SessionUpdate,
};
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::util::MatchDispatch;
use agent_client_protocol::{Client, SessionMessage};
use agent_client_protocol_http::HttpClient;
use tokio::sync::mpsc::UnboundedSender;

/// Where the skeleton points and what it asks. Hardcoded values are fine —
/// the whole crate is throwaway.
#[derive(Debug, Clone)]
pub struct SkeletonConfig {
    /// Origin of the server, no path: `https://box.tailnet.ts.net:9443`.
    /// The transport appends `/acp`.
    pub base_url: String,
    /// Sent as `X-Secret-Key` on every request (never `?token=`).
    pub secret: String,
    /// The cwd handed to `session/new` — a path on the SERVER machine.
    pub cwd: String,
    /// The one prompt the skeleton sends.
    pub prompt: String,
}

/// One log line for the on-screen beat log. `at_ms` is elapsed since the run
/// started, which is what makes "deltas arrive" observable in the log.
#[derive(Debug, Clone)]
pub struct Beat {
    /// Millis since the run started.
    pub at_ms: u128,
    /// Which beat (1-5), or 0 for preamble/summary.
    pub beat: u8,
    pub text: String,
}

/// Drive the five beats, reporting each arrival.
pub async fn run(cfg: SkeletonConfig, report: UnboundedSender<Beat>) {
    let start = Instant::now();
    let say = |beat: u8, text: String| {
        let _ = report.send(Beat {
            at_ms: start.elapsed().as_millis(),
            beat,
            text,
        });
    };

    say(0, format!("config: {} cwd={}", cfg.base_url, cfg.cwd));

    let mut headers = reqwest::header::HeaderMap::new();
    if let Ok(value) = reqwest::header::HeaderValue::from_str(&cfg.secret) {
        headers.insert("X-Secret-Key", value);
    }
    let http = match reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .default_headers(headers)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            say(1, format!("BEAT 1 FAIL: http client build: {e}"));
            return;
        }
    };

    // ---- Beat 1: reach a real goose serve ---------------------------------
    let origin = origin_of(&cfg.base_url);
    let status_url = format!("{origin}/status");
    match http.get(&status_url).send().await {
        Ok(resp) if resp.status().is_success() => {
            say(1, format!("BEAT 1 OK: GET /status -> {}", resp.status()));
        }
        Ok(resp) => {
            say(1, format!("BEAT 1 FAIL: GET /status -> {}", resp.status()));
            return;
        }
        Err(e) => {
            say(1, format!("BEAT 1 FAIL: GET /status: {e}"));
            return;
        }
    }

    // ---- Beats 2-5 live inside the connection the SDK owns ----------------
    let transport = match HttpClient::with_client(cfg.base_url.clone(), http) {
        Ok(t) => t,
        Err(e) => {
            say(2, format!("BEAT 2 FAIL: transport build: {e}"));
            return;
        }
    };

    let cwd = cfg.cwd.clone();
    let prompt_text = cfg.prompt.clone();
    let chunk_count = Arc::new(AtomicU32::new(0));
    let reporter = report.clone();

    let result: Result<(), agent_client_protocol::Error> = Client
        .builder()
        .name("skeleton-probe")
        .on_receive_request(
            async move |req: RequestPermissionRequest, responder, _connection| {
                let title = format!("{:?}", req.tool_call.fields.title);
                let options = req
                    .options
                    .iter()
                    .map(|o| format!("{}({:?})", o.name, o.kind))
                    .collect::<Vec<_>>()
                    .join(", ");
                say(5, format!("BEAT 5 sheet shown: {title} options: [{options}]"));
                // Let the sheet be visibly present, then answer it so the
                // roundtrip completes the turn.
                tokio::time::sleep(Duration::from_secs(2)).await;
                let picked = req
                    .options
                    .iter()
                    .find(|o| o.kind == PermissionOptionKind::AllowOnce)
                    .or_else(|| req.options.first());
                match picked {
                    Some(o) => {
                        say(5, format!("BEAT 5 respond: selected \"{}\"", o.name));
                        responder.respond(RequestPermissionResponse::new(
                            RequestPermissionOutcome::Selected(
                                SelectedPermissionOutcome::new(o.option_id.clone()),
                            ),
                        ))?;
                    }
                    None => {
                        say(5, "BEAT 5 respond: no options -> cancelled".to_string());
                        responder.respond(RequestPermissionResponse::new(
                            RequestPermissionOutcome::Cancelled,
                        ))?;
                    }
                }
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |connection| {
            // ---- Beat 2: initialize handshake --------------------------------
            let resp = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            say(2, format!("BEAT 2 OK: initialize handshake ({resp:?})"));

            // ---- Beat 3: session/new ------------------------------------------
            let mut session = connection
                .build_session(Path::new(&cwd))
                .block_task()
                .start_session()
                .await?;
            say(3, format!("BEAT 3 OK: session/new -> {}", session.session_id()));

            // The server's default approval mode is `auto`, which approves
            // every tool itself and never asks. Setting mode `approve` is what
            // makes the ask arrive — the same RPC the permission-durability
            // harness measured.
            let mode = connection
                .send_request(agent_client_protocol::schema::v1::SetSessionConfigOptionRequest::new(
                    session.session_id().clone(),
                    "mode",
                    "approve",
                ))
                .block_task()
                .await?;
            say(3, format!("mode set to approve ({mode:?})"));

            // ---- Beat 4: one prompt turn, deltas reported as they arrive ------
            session.send_prompt(prompt_text.clone())?;
            let reporter_stream = reporter.clone();
            loop {
                match session.read_update().await? {
                    SessionMessage::SessionMessage(dispatch) => {
                        MatchDispatch::new(dispatch)
                            .if_notification(async |n: SessionNotification| {
                                match n.update {
                                    SessionUpdate::AgentMessageChunk(chunk) => {
                                        let n = chunk_count.fetch_add(1, Ordering::Relaxed) + 1;
                                        if let agent_client_protocol::schema::v1::ContentBlock::Text(t) =
                                            chunk.content
                                        {
                                            reporter_stream.send(Beat {
                                                at_ms: start.elapsed().as_millis(),
                                                beat: 4,
                                                text: format!("delta#{n}: {}", t.text),
                                            })
                                            .ok();
                                        }
                                    }
                                    SessionUpdate::ToolCall(call) => {
                                        reporter_stream.send(Beat {
                                            at_ms: start.elapsed().as_millis(),
                                            beat: 4,
                                            text: format!(
                                                "tool call: {} ({:?}/{:?})",
                                                call.title, call.kind, call.status
                                            ),
                                        })
                                        .ok();
                                    }
                                    SessionUpdate::ToolCallUpdate(update) => {
                                        reporter_stream.send(Beat {
                                            at_ms: start.elapsed().as_millis(),
                                            beat: 4,
                                            text: format!(
                                                "tool update: {} {:?}",
                                                update.tool_call_id, update.fields.status
                                            ),
                                        })
                                        .ok();
                                    }
                                    other => {
                                        reporter_stream.send(Beat {
                                            at_ms: start.elapsed().as_millis(),
                                            beat: 4,
                                            text: format!("update: {other:?}"),
                                        })
                                        .ok();
                                    }
                                }
                                Ok(())
                            })
                            .await
                            .otherwise_ignore()?;
                    }
                    SessionMessage::StopReason(reason) => {
                        say(
                            4,
                            format!(
                                "BEAT 4 turn ended: {reason:?} ({} deltas)",
                                chunk_count.load(Ordering::Relaxed)
                            ),
                        );
                        break;
                    }
                    other => {
                        say(4, format!("unexpected session message: {other:?}"));
                    }
                }
            }
            Ok(())
        })
        .await;

    match result {
        Ok(()) => say(
            0,
            "SKELETON COMPLETE: all five beats in one sitting".to_string(),
        ),
        Err(e) => say(0, format!("SKELETON FAILED: {e}")),
    }
}

/// Origin (scheme + authority) of a base URL that may or may not carry `/acp`,
/// with the WebSocket schemes mapped back to HTTP so the `/status` probe can
/// still run when the transport is `wss://`. Any `?query` is dropped first:
/// the WebSocket probe carries its secret as `?token=` (the SDK's WS route
/// cannot send headers), and a query left on the base would push `/status`
/// into that query, turning beat 1 into `GET /` — measured as a 404.
fn origin_of(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    let trimmed = trimmed
        .replacen("wss://", "https://", 1)
        .replacen("ws://", "http://", 1);
    let trimmed = match trimmed.find('?') {
        Some(i) => &trimmed[..i],
        None => trimmed.as_str(),
    };
    match trimmed.rfind("/acp") {
        Some(i) => trimmed[..i].to_string(),
        None => trimmed.to_string(),
    }
}

/// PROBE for the 07 bet: persist the secret to the platform keystore and
/// read it back. Uses keyring-core's `Entry` directly against the platform
/// store. The `keyring` crate's v1 `Entry` shim is unusable on iOS and
/// Android by construction — its auto-default store (`set_credential_store`
/// in keyring-4.2.0/src/v1.rs) returns Err on both, and `Entry::new` bails
/// with `NoDefaultStore` before ever consulting a store the app registered
/// itself. Measured on the simulator: through the shim the probe fails with
/// "No default store has been set" even after `keyring_core::set_default_store`.
///
/// Android runtime contract (compile-proven, runtime unproven here — the NDK
/// is a prerequisite no leg of this prototype had): the store reads the
/// Android application context from `ndk-context`, which the app glue must
/// initialize first; the crate ships a JNI entry point
/// (`initializeNdkContext`) for exactly that.
pub fn keystore_probe() -> Result<String, String> {
    let service = "acp-skeleton-probe";
    let user = "goose-secret";
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
    {
        use keyring_core::api::CredentialStoreApi;
        #[cfg(target_os = "ios")]
        // iOS compiles only the `protected` module (the `keychain` module is
        // macOS-gated, and the crate compile_error!s without `protected`).
        let store = apple_native_keyring_store::protected::Store::new()
            .map_err(|e| format!("apple protected store: {e}"))?;
        #[cfg(all(target_os = "macos", not(target_os = "ios")))]
        let store = apple_native_keyring_store::keychain::Store::new()
            .map_err(|e| format!("apple keychain store: {e}"))?;
        #[cfg(target_os = "android")]
        let store = android_native_keyring_store::Store::new()
            .map_err(|e| format!("android keystore store: {e}"))?;
        let entry = store
            .build(service, user, None)
            .map_err(|e| format!("build entry: {e}"))?;
        entry
            .set_password("skeleton-paste-once-secret")
            .map_err(|e| format!("set_password: {e}"))?;
        let read_back = entry
            .get_password()
            .map_err(|e| format!("get_password: {e}"))?;
        if read_back != "skeleton-paste-once-secret" {
            return Err(format!("round-trip mismatch: {read_back:?}"));
        }
        entry
            .delete_credential()
            .map_err(|e| format!("delete_credential: {e}"))?;
        Ok(format!(
            "keystore round-trip OK ({}): set -> read -> delete",
            std::env::consts::OS
        ))
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
    {
        let _ = (service, user);
        Err("no platform keystore wired in this prototype for this target".to_string())
    }
}
