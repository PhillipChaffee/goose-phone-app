//! Convenience wrappers over the base ACP session methods.
//!
//! One method per RPC: they build the params object, apply the timeout that
//! suits that call, and type the reply. The transport itself is in the parent
//! module; nothing here touches the socket directly except
//! [`AcpClient::respond_permission`], which answers an agent-initiated request
//! and so has no reply to await.

use std::time::Duration;

use serde_json::{json, Map, Value};

use super::{AcpClient, Cmd, CLIENT_NAME};
use crate::error::AcpError;
use crate::goose::{LIST_TIMEOUT, MUTATE_TIMEOUT};
use crate::types::{
    config_options_from, ConfigOption, ContentBlock, NewSessionResponse, SessionKind,
    SessionListResponse,
};

/// The `_meta` object `session/new` sends: the client's own marker, with any
/// caller-supplied keys merged over it.
///
/// The merge is shallow and the caller wins, so a caller can override
/// `client` as well as add to it. A `meta` that is not a JSON object is
/// ignored rather than replacing the default — `_meta` has a shape, and
/// silently sending a non-object would be rejected by the server for a reason
/// that never reaches the caller.
fn new_session_meta(meta: Option<&Value>) -> Value {
    let mut merged = Map::new();
    merged.insert("client".to_string(), Value::String(CLIENT_NAME.to_string()));
    if let Some(Value::Object(extra)) = meta {
        for (key, value) in extra {
            merged.insert(key.clone(), value.clone());
        }
    }
    Value::Object(merged)
}

impl AcpClient {
    /// Create a session. `cwd` must be an absolute path on the *server*.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server rejects `cwd` (relative paths and paths
    /// it cannot open are refused), [`AcpError::Timeout`] after 60 s,
    /// [`AcpError::Closed`] if the connection drops, or
    /// [`AcpError::Transport`] if the reply is not a [`NewSessionResponse`].
    pub async fn session_new(&self, cwd: &str) -> Result<NewSessionResponse, AcpError> {
        self.session_new_with(cwd, None).await
    }

    /// Create a session, merging `meta` into the `_meta` object.
    ///
    /// `_meta` is how goose is told *why* a session exists — launching a
    /// recipe means starting a session with `_meta.recipeId` set — so the
    /// extra keys go alongside the client marker rather than replacing it.
    ///
    /// # Errors
    ///
    /// As [`AcpClient::session_new`].
    pub async fn session_new_with(
        &self,
        cwd: &str,
        meta: Option<&Value>,
    ) -> Result<NewSessionResponse, AcpError> {
        let result = self
            .request_with_timeout(
                "session/new",
                json!({
                    "cwd": cwd,
                    "mcpServers": [],
                    "_meta": new_session_meta(meta),
                }),
                Duration::from_secs(60),
            )
            .await?;
        serde_json::from_value(result).map_err(|e| AcpError::Transport(e.to_string()))
    }

    /// Load an existing session. The server replays its history as
    /// `session/update` events *before* this resolves.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server does not know `session_id` or refuses
    /// `cwd`, [`AcpError::Timeout`] if the replay takes longer than 120 s, or
    /// [`AcpError::Closed`] if the connection drops mid-replay.
    pub async fn session_load(&self, session_id: &str, cwd: &str) -> Result<Value, AcpError> {
        self.request_with_timeout(
            "session/load",
            json!({
                "sessionId": session_id,
                "cwd": cwd,
                "mcpServers": [],
            }),
            Duration::from_secs(120),
        )
        .await
    }

    /// List sessions of the given kinds, newest first.
    ///
    /// An empty `kinds` means all of them: that is the server's own reading of
    /// an absent or empty `types` filter, not a special case invented here.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server rejects the cursor,
    /// [`AcpError::Timeout`] after 30 s, [`AcpError::Closed`] if the
    /// connection drops, or [`AcpError::Transport`] if the reply is not a
    /// [`SessionListResponse`].
    pub async fn session_list(
        &self,
        kinds: &[SessionKind],
        cursor: Option<String>,
    ) -> Result<SessionListResponse, AcpError> {
        let types: Vec<&str> = kinds.iter().map(|kind| kind.as_wire()).collect();
        let mut params = json!({
            "_meta": {
                "types": types,
                "goose": {"includeLastMessageSnippet": true},
            }
        });
        if let Some(cursor) = cursor {
            params["cursor"] = Value::String(cursor);
        }
        let result = self
            .request_with_timeout("session/list", params, LIST_TIMEOUT)
            .await?;
        serde_json::from_value(result).map_err(|e| AcpError::Transport(e.to_string()))
    }

    /// Send a user message; resolves at end of turn with the stop reason
    /// (`end_turn`, `max_tokens`, `refusal`, `cancelled`, …).
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the agent fails the turn (an unknown session id, a
    /// provider error), or [`AcpError::Closed`] if the connection drops before
    /// the turn ends. There is no timeout — a turn may legitimately run for
    /// minutes.
    pub async fn prompt(&self, session_id: &str, text: &str) -> Result<String, AcpError> {
        let result = self
            .request(
                "session/prompt",
                json!({
                    "sessionId": session_id,
                    "prompt": [ContentBlock::text(text)],
                }),
            )
            .await?;
        Ok(result
            .get("stopReason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn")
            .to_string())
    }

    /// Cancel the running turn; the pending `prompt` resolves with
    /// `cancelled`.
    pub fn cancel(&self, session_id: &str) {
        self.notify("session/cancel", json!({"sessionId": session_id}));
    }

    /// Change one session config option — `provider`, `model`, `mode` or
    /// `thinking_effort` — and get the full option set back.
    ///
    /// Takes effect on the session immediately; the next `session/prompt`
    /// uses it. The agent also pushes a `config_option_update` notification,
    /// so a second client watching the same session stays in step.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server rejects the option or its value,
    /// [`AcpError::Timeout`], or [`AcpError::Closed`] if the connection
    /// drops.
    pub async fn set_config_option(
        &self,
        session_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<Vec<ConfigOption>, AcpError> {
        // `type: "id"` is the discriminator every id-based option kind uses;
        // `value` is flattened alongside it, not nested under it.
        let raw = self
            .request(
                "session/set_config_option",
                json!({
                    "sessionId": session_id,
                    "configId": config_id,
                    "type": "id",
                    "value": value,
                }),
            )
            .await?;
        Ok(config_options_from(&raw))
    }

    /// Delete a session on the server.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server does not know `session_id`,
    /// [`AcpError::Timeout`] after 30 s, or [`AcpError::Closed`] if the
    /// connection drops.
    pub async fn session_delete(&self, session_id: &str) -> Result<(), AcpError> {
        self.request_with_timeout(
            "session/delete",
            json!({"sessionId": session_id}),
            MUTATE_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    /// # Errors
    ///
    /// [`AcpError::Unsupported`] if this goose server does not implement the
    /// unstable rename extension, [`AcpError::Rpc`] if it does not know
    /// `session_id`, [`AcpError::Timeout`] after 30 s, or
    /// [`AcpError::Closed`] if the connection drops.
    pub async fn session_rename(&self, session_id: &str, title: &str) -> Result<(), AcpError> {
        self.goose_request(
            "_goose/unstable/session/rename",
            json!({"sessionId": session_id, "title": title}),
            MUTATE_TIMEOUT,
        )
        .await
        .map(|_| ())
    }

    /// Answer a `session/request_permission` request. `option_id = None`
    /// reports the prompt as cancelled.
    pub fn respond_permission(&self, request_id: Value, option_id: Option<String>) {
        let outcome = match option_id {
            Some(id) => json!({"outcome": {"outcome": "selected", "optionId": id}}),
            None => json!({"outcome": {"outcome": "cancelled"}}),
        };
        let _ = self.tx.send(Cmd::Respond {
            id: request_id,
            result: Ok(outcome),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A caller's keys go *alongside* the client marker, and win where they
    /// collide — the recipe launch path in a later PR depends on both halves.
    #[test]
    fn session_meta_merges_over_the_default() {
        assert_eq!(new_session_meta(None), json!({"client": CLIENT_NAME}));
        assert_eq!(
            new_session_meta(Some(&json!({"recipeId": "abc"}))),
            json!({"client": CLIENT_NAME, "recipeId": "abc"})
        );
        assert_eq!(
            new_session_meta(Some(&json!({"client": "someone-else"}))),
            json!({"client": "someone-else"})
        );
        // Not an object: the default survives rather than being replaced.
        assert_eq!(
            new_session_meta(Some(&json!("recipe"))),
            json!({"client": CLIENT_NAME})
        );
    }
}
