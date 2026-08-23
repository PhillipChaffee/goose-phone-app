//! Client for the brain's code-agent plane (personal-ai-setup issue #17,
//! this repo issue #2): the session manager's API plus the per-chat `OpenCode`
//! servers it fronts, all behind one TLS + Basic-auth gateway on the tailnet.
//!
//! Two layers, one base URL:
//! - manager: `/api/...` — chat lifecycle + the metadata index
//! - per chat: `/chat/<id>/...` — that chat's own `opencode serve` HTTP
//!   API (sessions, messages, prompts,
//!   permissions, diff) + SSE at `/event`
//!
//! The gateway wakes a stopped chat on any request to it, so this client
//! never needs explicit wake logic beyond tolerating slow first responses.
//!
//! UI-framework agnostic: reqwest (rustls) + tokio, same stack as
//! `goose-acp-client`. The gateway's certificate is a real Let's Encrypt
//! cert on the tailnet name, so standard verification applies — no pinning.

use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum CodeError {
    #[error("{0}")]
    Http(#[from] reqwest::Error),
    #[error("server said {status}: {body}")]
    Status { status: u16, body: String },
    #[error("{0}")]
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CodeConfig {
    /// Gateway base, e.g. `https://brain.tailnet.ts.net:4300`.
    pub base_url: String,
    /// `OPENCODE_SERVER_PASSWORD` (username is free-form; we send `opencode`).
    pub password: String,
}

/// One repo from the manager's allowlist.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct RepoEntry {
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub edit_only: bool,
    #[serde(default)]
    pub allow_push: bool,
    #[serde(default)]
    pub public_throwaway: bool,
}

/// One chat from the manager's metadata index.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct ChatMeta {
    pub id: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub branch: String,
    /// `running` | `stopped` | `absent` (absent = recreated on next wake).
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub last_active: f64,
}

impl ChatMeta {
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.status == "running"
    }
}

/// A pending permission ask from a chat's `OpenCode` server. Answer with
/// [`CodeClient::reply_permission`] using `once` / `always` / `reject`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodePermission {
    pub id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub metadata: Value,
}

impl Default for CodePermission {
    fn default() -> Self {
        Self {
            id: String::new(),
            session_id: String::new(),
            title: String::new(),
            kind: String::new(),
            metadata: Value::Null,
        }
    }
}

/// One message part as `OpenCode`'s API ships it. Kept lenient: only the
/// fields the transcript fold needs are typed, everything else stays raw.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Part {
    pub id: String,
    #[serde(rename = "messageID")]
    pub message_id: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
    pub tool: Option<String>,
    #[serde(rename = "callID")]
    pub call_id: Option<String>,
    pub state: Option<Value>,
}

/// `{info, parts}` from `GET /session/:id/message`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct MessageWithParts {
    pub info: MessageInfo,
    pub parts: Vec<Part>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct MessageInfo {
    pub id: String,
    pub role: String,
    #[serde(rename = "sessionID")]
    pub session_id: String,
}

/// A session on a chat's server (`GET /session`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub directory: String,
}

/// Events off a chat's SSE stream, pre-dispatched on the `type` tag.
/// Unknown kinds are preserved so new server events degrade gracefully.
#[derive(Clone, Debug)]
pub enum CodeEvent {
    MessageUpdated {
        info: MessageInfo,
    },
    PartUpdated {
        part: Part,
        delta: Option<String>,
    },
    PermissionAsked(CodePermission),
    PermissionReplied {
        id: String,
    },
    SessionIdle {
        session_id: String,
    },
    SessionStatus(Value),
    Connected,
    Unknown {
        tag: String,
        raw: Value,
    },
    /// The stream ended (network drop, chat spin-down, gateway restart).
    Disconnected {
        reason: String,
    },
}

fn dispatch_event(raw: Value) -> CodeEvent {
    // Gateway `/global/event` wraps payloads as {directory, payload}; the
    // per-chat `/event` sends them bare. Accept both.
    let evt = raw.get("payload").cloned().unwrap_or(raw);
    let tag = evt
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let props = evt.get("properties").cloned().unwrap_or(Value::Null);
    match tag.as_str() {
        "server.connected" => CodeEvent::Connected,
        "server.heartbeat" => CodeEvent::Unknown {
            tag,
            raw: Value::Null,
        },
        "message.updated" => {
            let info = props
                .get("info")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            CodeEvent::MessageUpdated { info }
        }
        "message.part.updated" => {
            let part = props
                .get("part")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            let delta = props
                .get("delta")
                .and_then(Value::as_str)
                .map(str::to_string);
            CodeEvent::PartUpdated { part, delta }
        }
        "permission.updated" | "permission.asked" => {
            match serde_json::from_value::<CodePermission>(props.clone()) {
                Ok(p) if !p.id.is_empty() => CodeEvent::PermissionAsked(p),
                _ => CodeEvent::Unknown { tag, raw: props },
            }
        }
        "permission.replied" => CodeEvent::PermissionReplied {
            id: props
                .get("permissionID")
                .or_else(|| props.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
        "session.idle" => CodeEvent::SessionIdle {
            session_id: props
                .get("sessionID")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
        "session.status" => CodeEvent::SessionStatus(props),
        _ => CodeEvent::Unknown { tag, raw: evt },
    }
}

#[derive(Clone)]
pub struct CodeClient {
    http: reqwest::Client,
    base: String,
    password: String,
}

/// Hand-written so the gateway password never reaches a log line; the
/// derived form would print it verbatim.
impl std::fmt::Debug for CodeClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeClient")
            .field("base", &self.base)
            .field("password", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl CodeClient {
    /// Build a client for the gateway described by `cfg`.
    ///
    /// # Errors
    ///
    /// [`CodeError::Other`] if `cfg.base_url` is blank once trimmed, and
    /// [`CodeError::Http`] if reqwest cannot build its TLS-backed client
    /// (a broken rustls or root-certificate setup on the device).
    pub fn new(cfg: &CodeConfig) -> Result<Self, CodeError> {
        let base = cfg.base_url.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            return Err(CodeError::Other("code server URL is empty".into()));
        }
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            // Generous default: any request may transparently wake a stopped
            // chat (container start + server boot ≈ up to 90s gateway-side).
            .timeout(Duration::from_secs(150))
            .build()?;
        Ok(Self {
            http,
            base,
            password: cfg.password.clone(),
        })
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, format!("{}{}", self.base, path))
            .basic_auth("opencode", Some(&self.password))
    }

    async fn json_of(resp: reqwest::Response) -> Result<Value, CodeError> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            let body = body.chars().take(300).collect::<String>();
            return Err(CodeError::Status {
                status: status.as_u16(),
                body,
            });
        }
        if status.as_u16() == 204 {
            return Ok(Value::Null);
        }
        Ok(resp.json().await.unwrap_or(Value::Null))
    }

    // ------------------------------------------------------------ manager

    /// The manager's liveness probe (`GET /api/health`).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or the request
    /// outruns the client timeout, and [`CodeError::Status`] on a non-2xx
    /// answer — 401 when `password` is wrong, 502 while the manager behind
    /// the gateway is restarting.
    pub async fn health(&self) -> Result<Value, CodeError> {
        Self::json_of(self.req(reqwest::Method::GET, "/api/health").send().await?).await
    }

    /// The manager's repo allowlist (`GET /api/repos`).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or the request
    /// outruns the client timeout, and [`CodeError::Status`] on a non-2xx
    /// answer (401 for a wrong `password`). A 2xx body that does not decode
    /// as `{"repos": [...]}` yields an empty list rather than an error.
    pub async fn repos(&self) -> Result<Vec<RepoEntry>, CodeError> {
        let v = Self::json_of(self.req(reqwest::Method::GET, "/api/repos").send().await?).await?;
        Ok(serde_json::from_value(v.get("repos").cloned().unwrap_or_default()).unwrap_or_default())
    }

    /// The metadata index of every chat the manager knows (`GET /api/chats`).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or the request
    /// outruns the client timeout, and [`CodeError::Status`] on a non-2xx
    /// answer (401 for a wrong `password`). A 2xx body that does not decode
    /// as `{"chats": [...]}` yields an empty list rather than an error.
    pub async fn chats(&self) -> Result<Vec<ChatMeta>, CodeError> {
        let v = Self::json_of(self.req(reqwest::Method::GET, "/api/chats").send().await?).await?;
        Ok(serde_json::from_value(v.get("chats").cloned().unwrap_or_default()).unwrap_or_default())
    }

    /// Create a chat on `repo` with `task` as its opening instruction.
    /// `model` is `provider/model`; `None` leaves the manager's default.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout,
    /// [`CodeError::Status`] on a non-2xx answer — 400 for a repo outside the
    /// allowlist, 401 for a wrong `password` — and [`CodeError::Other`] if
    /// the created chat's payload does not decode as a [`ChatMeta`].
    pub async fn create_chat(
        &self,
        repo: &str,
        task: &str,
        model: Option<&str>,
    ) -> Result<ChatMeta, CodeError> {
        let mut body = json!({"repo": repo, "task": task});
        if let Some(m) = model {
            body["model"] = json!(m);
        }
        let v = Self::json_of(
            self.req(reqwest::Method::POST, "/api/chats")
                .json(&body)
                .send()
                .await?,
        )
        .await?;
        serde_json::from_value(v).map_err(|e| CodeError::Other(format!("bad chat payload: {e}")))
    }

    /// Start a stopped chat's container. Any request to the chat wakes it
    /// anyway; this is the explicit form, useful to pay the start cost early.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure, or if the container start
    /// outruns the client's 150s request timeout; [`CodeError::Status`] on a
    /// non-2xx answer (404 for a `chat_id` the manager does not know).
    pub async fn wake_chat(&self, chat_id: &str) -> Result<(), CodeError> {
        Self::json_of(
            self.req(reqwest::Method::POST, &format!("/api/chats/{chat_id}/wake"))
                .send()
                .await?,
        )
        .await
        .map(|_| ())
    }

    /// Stop a running chat's container; its workspace and branch survive.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for a `chat_id` the
    /// manager does not know).
    pub async fn stop_chat(&self, chat_id: &str) -> Result<(), CodeError> {
        Self::json_of(
            self.req(reqwest::Method::POST, &format!("/api/chats/{chat_id}/stop"))
                .send()
                .await?,
        )
        .await
        .map(|_| ())
    }

    /// Delete a chat. With `purge`, its workspace is discarded too.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for a `chat_id` the
    /// manager does not know).
    pub async fn delete_chat(&self, chat_id: &str, purge: bool) -> Result<(), CodeError> {
        let q = if purge { "?purge=1" } else { "" };
        Self::json_of(
            self.req(reqwest::Method::DELETE, &format!("/api/chats/{chat_id}{q}"))
                .send()
                .await?,
        )
        .await
        .map(|_| ())
    }

    // ----------------------------------------------------------- per chat

    fn chat_path(chat_id: &str, sub: &str) -> String {
        format!("/chat/{chat_id}{sub}")
    }

    /// The sessions on a chat's own `OpenCode` server (`GET /session`).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id`). A 2xx body that does not decode as a session list yields
    /// an empty list rather than an error.
    pub async fn sessions(&self, chat_id: &str) -> Result<Vec<SessionMeta>, CodeError> {
        let v = Self::json_of(
            self.req(reqwest::Method::GET, &Self::chat_path(chat_id, "/session"))
                .send()
                .await?,
        )
        .await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Create the chat's `OpenCode` session in its workspace.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout,
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id`), and [`CodeError::Other`] if the new session's payload does
    /// not decode as a [`SessionMeta`].
    pub async fn create_session(&self, chat_id: &str) -> Result<SessionMeta, CodeError> {
        let v = Self::json_of(
            self.req(
                reqwest::Method::POST,
                &Self::chat_path(chat_id, "/session?directory=/chat/workspace"),
            )
            .json(&json!({}))
            .send()
            .await?,
        )
        .await?;
        serde_json::from_value(v).map_err(|e| CodeError::Other(format!("bad session payload: {e}")))
    }

    /// Every message of a session with its parts (`GET .../message`), the
    /// transcript the UI folds.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id` or `session_id`). A 2xx body that does not decode as a
    /// message list yields an empty list rather than an error.
    pub async fn messages(
        &self,
        chat_id: &str,
        session_id: &str,
    ) -> Result<Vec<MessageWithParts>, CodeError> {
        let v = Self::json_of(
            self.req(
                reqwest::Method::GET,
                &Self::chat_path(chat_id, &format!("/session/{session_id}/message")),
            )
            .send()
            .await?,
        )
        .await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Fire-and-forget prompt: the turn runs server-side; progress arrives
    /// over the SSE stream. `model` is `provider/model`; one without a `/`
    /// is dropped and the session's own model is used.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer — 404 for an unknown
    /// `chat_id` or `session_id`, 400 for a model the server rejects. A turn
    /// that fails *after* this returns surfaces on the event stream, not
    /// here.
    pub async fn prompt_async(
        &self,
        chat_id: &str,
        session_id: &str,
        text: &str,
        model: Option<&str>,
    ) -> Result<(), CodeError> {
        let mut body = json!({"parts": [{"type": "text", "text": text}]});
        if let Some(m) = model {
            if let Some((provider, model_id)) = m.split_once('/') {
                body["model"] = json!({"providerID": provider, "modelID": model_id});
            }
        }
        Self::json_of(
            self.req(
                reqwest::Method::POST,
                &Self::chat_path(chat_id, &format!("/session/{session_id}/prompt_async")),
            )
            .json(&body)
            .send()
            .await?,
        )
        .await
        .map(|_| ())
    }

    /// Cancel the session's in-flight turn.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id` or `session_id`).
    pub async fn abort(&self, chat_id: &str, session_id: &str) -> Result<(), CodeError> {
        Self::json_of(
            self.req(
                reqwest::Method::POST,
                &Self::chat_path(chat_id, &format!("/session/{session_id}/abort")),
            )
            .send()
            .await?,
        )
        .await
        .map(|_| ())
    }

    /// The session's cumulative diff (`FileDiff[]`, kept raw for rendering).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id` or `session_id`).
    pub async fn diff(&self, chat_id: &str, session_id: &str) -> Result<Value, CodeError> {
        Self::json_of(
            self.req(
                reqwest::Method::GET,
                &Self::chat_path(chat_id, &format!("/session/{session_id}/diff")),
            )
            .send()
            .await?,
        )
        .await
    }

    /// Pending permission asks (reconnect catch-up).
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] if the gateway is unreachable or waking a stopped
    /// chat outruns the client's 150s request timeout, and
    /// [`CodeError::Status`] on a non-2xx answer (404 for an unknown
    /// `chat_id`). A 2xx body that does not decode as a permission list
    /// yields an empty list rather than an error.
    pub async fn permissions(&self, chat_id: &str) -> Result<Vec<CodePermission>, CodeError> {
        let v = Self::json_of(
            self.req(
                reqwest::Method::GET,
                &Self::chat_path(chat_id, "/permission"),
            )
            .send()
            .await?,
        )
        .await?;
        Ok(serde_json::from_value(v).unwrap_or_default())
    }

    /// Answer a pending permission ask. `response`: `once` | `always` |
    /// `reject`.
    ///
    /// # Errors
    ///
    /// [`CodeError::Http`] on transport failure or timeout, and
    /// [`CodeError::Status`] on a non-2xx answer — 404 for an ask that has
    /// already been answered or expired, 400 for a `response` outside the
    /// three accepted words.
    pub async fn reply_permission(
        &self,
        chat_id: &str,
        session_id: &str,
        permission_id: &str,
        response: &str,
    ) -> Result<(), CodeError> {
        Self::json_of(
            self.req(
                reqwest::Method::POST,
                &Self::chat_path(
                    chat_id,
                    &format!("/session/{session_id}/permissions/{permission_id}"),
                ),
            )
            .json(&json!({"response": response}))
            .send()
            .await?,
        )
        .await
        .map(|_| ())
    }

    /// Attach to a chat's SSE stream. Events (including a final
    /// [`CodeEvent::Disconnected`]) arrive on the returned receiver; drop it
    /// to detach. Reconnection policy belongs to the caller.
    #[must_use]
    pub fn events(&self, chat_id: &str) -> mpsc::Receiver<CodeEvent> {
        let (tx, rx) = mpsc::channel(256);
        let this = self.clone();
        let chat_id = chat_id.to_string();
        tokio::spawn(async move {
            let reason = match this.stream_events(&chat_id, &tx).await {
                Ok(()) => "stream ended".to_string(),
                Err(e) => e.to_string(),
            };
            let _ = tx.send(CodeEvent::Disconnected { reason }).await;
        });
        rx
    }

    async fn stream_events(
        &self,
        chat_id: &str,
        tx: &mpsc::Sender<CodeEvent>,
    ) -> Result<(), CodeError> {
        // No overall timeout on the SSE request: heartbeats every ~10s keep
        // it alive; a silent dead stream is caught by the read inactivity
        // window below.
        let resp = self
            .http
            .get(format!(
                "{}{}",
                self.base,
                Self::chat_path(chat_id, "/event")
            ))
            .basic_auth("opencode", Some(&self.password))
            .timeout(Duration::from_secs(60 * 60 * 24))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(CodeError::Status { status, body });
        }
        let mut stream = resp.bytes_stream();
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(60), stream.next()).await;
            let chunk = match chunk {
                Err(_) => return Err(CodeError::Other("stream silent for 60s".into())),
                Ok(None) => return Ok(()),
                Ok(Some(c)) => c?,
            };
            buf.extend_from_slice(&chunk);
            // SSE frames are separated by a blank line.
            while let Some(pos) = find_frame_end(&buf) {
                let frame: Vec<u8> = buf.drain(..pos + 2).collect();
                if let Some(evt) = parse_sse_frame(&frame) {
                    if tx.send(evt).await.is_err() {
                        return Ok(()); // receiver dropped — detach
                    }
                }
            }
        }
    }
}

fn find_frame_end(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

/// Parse one SSE frame: concatenated `data:` lines → JSON → [`CodeEvent`].
fn parse_sse_frame(frame: &[u8]) -> Option<CodeEvent> {
    let text = String::from_utf8_lossy(frame);
    let mut data = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.trim_start());
        }
    }
    if data.is_empty() {
        return None;
    }
    let raw: Value = serde_json::from_str(&data).ok()?;
    let evt = dispatch_event(raw);
    // Heartbeats parse as Unknown with a null payload — drop them here.
    if matches!(&evt, CodeEvent::Unknown { tag, raw } if tag == "server.heartbeat" && raw.is_null())
    {
        return None;
    }
    Some(evt)
}

#[cfg(test)]
#[expect(
    clippy::panic,
    reason = "test assertions: an unexpected event kind is a test failure, and panic! carries the offending value into the report"
)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dispatches_part_updated_with_delta() {
        let raw = json!({
            "type": "message.part.updated",
            "properties": {
                "part": {
                    "id": "prt_1", "messageID": "msg_1", "sessionID": "ses_1",
                    "type": "text", "text": "Hello wor"
                },
                "delta": "ld"
            }
        });
        match dispatch_event(raw) {
            CodeEvent::PartUpdated { part, delta } => {
                assert_eq!(part.id, "prt_1");
                assert_eq!(part.kind, "text");
                assert_eq!(delta.as_deref(), Some("ld"));
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn dispatches_global_envelope() {
        let raw = json!({
            "directory": "/chat/workspace",
            "payload": {"type": "session.idle", "properties": {"sessionID": "ses_9"}}
        });
        match dispatch_event(raw) {
            CodeEvent::SessionIdle { session_id } => assert_eq!(session_id, "ses_9"),
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn dispatches_permission() {
        let raw = json!({
            "type": "permission.updated",
            "properties": {
                "id": "perm_1", "sessionID": "ses_1", "type": "bash",
                "title": "Run git push", "metadata": {"command": "git push"}
            }
        });
        match dispatch_event(raw) {
            CodeEvent::PermissionAsked(p) => {
                assert_eq!(p.id, "perm_1");
                assert_eq!(p.title, "Run git push");
            }
            other => panic!("wrong event: {other:?}"),
        }
    }

    #[test]
    fn sse_frame_parsing_joins_data_lines() {
        let frame =
            b"event: message\ndata: {\"type\":\"server.connected\",\ndata: \"properties\":{}}\n\n";
        match parse_sse_frame(frame) {
            Some(CodeEvent::Connected) => {}
            other => panic!("wrong parse: {other:?}"),
        }
    }

    #[test]
    fn unknown_events_are_preserved() {
        let raw = json!({"type": "todo.updated", "properties": {"x": 1}});
        match dispatch_event(raw) {
            CodeEvent::Unknown { tag, .. } => assert_eq!(tag, "todo.updated"),
            other => panic!("wrong event: {other:?}"),
        }
    }
}
