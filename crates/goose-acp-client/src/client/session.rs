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
    config_options_from, ConfigOption, ContentBlock, NewSessionResponse, SessionListResponse,
    SessionQuery,
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

/// The `session/list` params for `query`.
///
/// Everything but the cursor rides in `_meta`: `types` and `query` are goose
/// extensions to the base ACP request, and `goose.includeLastMessageSnippet`
/// is what makes a list row able to show its last line without loading the
/// session. `query` is omitted rather than sent as `null` when there is no
/// search — an absent key and a null both mean "no keyword" to goose, but
/// only the absent one matches what a hand-written request looks like.
fn session_list_params(query: &SessionQuery) -> Value {
    let types: Vec<&str> = query.kinds().iter().map(|kind| kind.as_wire()).collect();
    let mut meta = json!({
        "types": types,
        "goose": {"includeLastMessageSnippet": true},
    });
    if let Some(keyword) = query.query() {
        meta["query"] = Value::String(keyword.to_string());
    }
    let mut params = json!({"_meta": meta});
    if let Some(cursor) = query.cursor() {
        params["cursor"] = Value::String(cursor.to_string());
    }
    params
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

    /// List sessions, newest first: the kinds, the keyword search and the page
    /// all come from one [`SessionQuery`], because the server binds the cursor
    /// to the filters and rejects a mismatched pair.
    ///
    /// The search is the server's, across the whole history rather than the
    /// page already loaded, and it reads the *messages*: goose splits the
    /// query on whitespace, ORs the words, and matches the text of every
    /// user-visible message. A session whose title says nothing still comes
    /// back if something in it was said.
    ///
    /// # Errors
    ///
    /// [`AcpError::Rpc`] if the server rejects the cursor,
    /// [`AcpError::Timeout`] after 30 s, [`AcpError::Closed`] if the
    /// connection drops, or [`AcpError::Transport`] if the reply is not a
    /// [`SessionListResponse`].
    pub async fn session_list(
        &self,
        query: &SessionQuery,
    ) -> Result<SessionListResponse, AcpError> {
        let result = self
            .request_with_timeout("session/list", session_list_params(query), LIST_TIMEOUT)
            .await?;
        serde_json::from_value(result).map_err(|e| AcpError::Transport(e.to_string()))
    }

    /// Send a user message; resolves at end of turn with the stop reason
    /// (`end_turn`, `max_tokens`, `refusal`, `cancelled`, …).
    ///
    /// Takes the whole `prompt` array rather than a string: ACP's message is
    /// a list of [`ContentBlock`]s, and an attached image or file is another
    /// block beside the text one, not something encoded into it.
    ///
    /// # Errors
    ///
    /// [`AcpError::Config`] if `blocks` is empty — a turn with nothing in it
    /// is a client bug, and the agent answers `invalid_params` to it anyway.
    /// [`AcpError::Rpc`] if the agent fails the turn (an unknown session id, a
    /// provider error), or [`AcpError::Closed`] if the connection drops before
    /// the turn ends. There is no timeout — a turn may legitimately run for
    /// minutes.
    pub async fn prompt(
        &self,
        session_id: &str,
        blocks: &[ContentBlock],
    ) -> Result<String, AcpError> {
        if blocks.is_empty() {
            return Err(AcpError::Config("prompt has no content".into()));
        }
        let result = self
            .request(
                "session/prompt",
                json!({
                    "sessionId": session_id,
                    "prompt": blocks,
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
#[expect(
    clippy::unwrap_used,
    clippy::panic,
    reason = "test assertions: a failing unwrap or a wrong-variant panic is the check"
)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::mpsc;

    use super::*;
    use crate::types::SessionKind;

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

    /// A `session/list` reply carrying `next_cursor`, parsed the way a real
    /// one would be so the tests below cannot invent a field name.
    fn page(next_cursor: Option<&str>) -> SessionListResponse {
        serde_json::from_value(json!({"sessions": [], "nextCursor": next_cursor})).unwrap()
    }

    /// The frame the app has always sent, now pinned: the kinds filter, the
    /// snippet opt-in, and no `query` and no `cursor` at all on page one.
    #[test]
    fn user_only_first_page_frame() {
        assert_eq!(
            session_list_params(&SessionQuery::new(&[SessionKind::User], None)),
            json!({"_meta": {
                "types": ["user"],
                "goose": {"includeLastMessageSnippet": true},
            }})
        );
    }

    /// All three kinds and a keyword. `query` is goose's spelling — the
    /// server reads `_meta.query`, not `_meta.keyword` or a top-level field.
    #[test]
    fn all_kinds_with_a_query_frame() {
        assert_eq!(
            session_list_params(&SessionQuery::new(&SessionKind::ALL, Some("deploy"))),
            json!({"_meta": {
                "types": ["user", "scheduled", "acp"],
                "query": "deploy",
                "goose": {"includeLastMessageSnippet": true},
            }})
        );
    }

    /// The cursor is a top-level `cursor`, beside `_meta`, and the filters go
    /// out again unchanged with it — the server re-hashes them to check.
    #[test]
    fn next_page_frame_repeats_the_filters() {
        let first = SessionQuery::new(&[SessionKind::Scheduled], Some("nightly"));
        let next = first.next_page(&page(Some("cursor-1"))).unwrap();
        assert_eq!(
            session_list_params(&next),
            json!({
                "cursor": "cursor-1",
                "_meta": {
                    "types": ["scheduled"],
                    "query": "nightly",
                    "goose": {"includeLastMessageSnippet": true},
                }
            })
        );
    }

    /// The invariant the type exists for: a cursor can only ever be paired
    /// with the filters it was minted under.
    ///
    /// There is no constructor and no setter that takes one, so the only way
    /// to get a cursor into a request is [`SessionQuery::next_page`], which
    /// copies its own filters across. Changing the search or the kinds means
    /// building a fresh query, and a fresh query starts at page one — the
    /// stale cursor is not dropped by a rule someone has to remember, it is
    /// unreachable.
    #[test]
    fn a_cursor_cannot_outlive_the_filters_that_minted_it() {
        let first = SessionQuery::new(&[SessionKind::User], Some("dep"));
        let next = first.next_page(&page(Some("cursor-1"))).unwrap();
        assert_eq!(next.kinds(), first.kinds());
        assert_eq!(next.query(), first.query());

        // The user types one more character: the only query value that can
        // exist for the new search is a first page.
        let widened = SessionQuery::new(&[SessionKind::User], Some("depl"));
        assert_eq!(widened.cursor(), None);
        assert!(session_list_params(&widened).get("cursor").is_none());

        // And the last page ends the chain rather than repeating itself.
        assert_eq!(next.next_page(&page(None)), None);
    }

    /// A search box mid-edit and an empty one are the same request: the
    /// server trims and drops a blank keyword, so doing it here keeps the
    /// filter hash — and therefore the cursor — stable across the difference.
    #[test]
    fn blank_searches_are_no_search() {
        let blank = SessionQuery::new(&[SessionKind::User], Some("   "));
        assert_eq!(blank, SessionQuery::new(&[SessionKind::User], None));
        assert_eq!(
            SessionQuery::new(&[SessionKind::User], Some(" deploy ")).query(),
            Some("deploy")
        );
    }

    // ---- what each wrapper puts on the wire --------------------------------
    //
    // A handle over a channel the test drains itself. Every wrapper below is
    // one method name and one params object, and both are goose's spelling
    // rather than this crate's — a mis-keyed field here is a call the server
    // answers `-32602` to, or worse answers happily while ignoring the half it
    // could not read.

    fn detached() -> (AcpClient, mpsc::UnboundedReceiver<Cmd>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (
            AcpClient {
                tx,
                unsupported: Arc::default(),
            },
            rx,
        )
    }

    /// Answer the one request the call under test sends, and hand back the
    /// method and params it went out with.
    async fn answer(rx: &mut mpsc::UnboundedReceiver<Cmd>, result: Value) -> (String, Value) {
        match rx.recv().await {
            Some(Cmd::Request {
                method,
                params,
                reply,
            }) => {
                let _ = reply.send(Ok(result));
                (method, params)
            }
            _ => panic!("the call should have sent exactly one request"),
        }
    }

    /// Opening an existing chat. The server replays the whole history as
    /// `session/update` events before this answers, so a key it cannot read is
    /// a chat that opens empty with no error anywhere — and `mcpServers` is
    /// required by the schema even though this client never sends one.
    #[tokio::test]
    async fn session_load_names_the_session_and_the_directory() {
        let (client, mut rx) = detached();
        let (loaded, sent) = tokio::join!(
            client.session_load("20260821_1", "/home/demo"),
            answer(&mut rx, json!({"configOptions": []})),
        );

        assert_eq!(sent.0, "session/load");
        assert_eq!(
            sent.1,
            json!({"sessionId": "20260821_1", "cwd": "/home/demo", "mcpServers": []})
        );
        assert_eq!(
            loaded.unwrap(),
            json!({"configOptions": []}),
            "the reply is handed back whole: the caller reads the options out of it"
        );
    }

    /// The discriminator goose reads is `type: "id"` with `value` flattened
    /// beside it, not nested under it — and the reply is the whole option set,
    /// which is what the sheet re-renders itself from. A wrong shape here
    /// changes nothing on the server and leaves the sheet showing the old
    /// value as if the change had taken.
    #[tokio::test]
    async fn set_config_option_sends_the_id_discriminator_and_returns_the_new_set() {
        let (client, mut rx) = detached();
        let reply = json!({"configOptions": [
            {"configId": "model", "name": "Model", "type": "select",
             "currentValue": "claude-opus-4",
             "options": [{"value": "claude-opus-4", "name": "Opus 4"},
                         {"value": "claude-sonnet-4", "name": "Sonnet 4"}]}
        ]});
        let (options, sent) = tokio::join!(
            client.set_config_option("20260821_1", "model", "claude-opus-4"),
            answer(&mut rx, reply),
        );

        assert_eq!(sent.0, "session/set_config_option");
        assert_eq!(
            sent.1,
            json!({"sessionId": "20260821_1", "configId": "model",
                   "type": "id", "value": "claude-opus-4"})
        );

        let options = options.unwrap();
        assert_eq!(options.len(), 1, "the caller re-renders from the reply");
        assert_eq!(options[0].config_id, "model");
        assert_eq!(options[0].current_label(), Some("Opus 4"));
    }

    /// A reply that carries no `configOptions` is an empty set rather than an
    /// error: goose has answered, the option took, and there is nothing for
    /// the sheet to redraw.
    #[tokio::test]
    async fn a_config_reply_with_nothing_in_it_is_not_a_failure() {
        let (client, mut rx) = detached();
        let (options, _sent) = tokio::join!(
            client.set_config_option("20260821_1", "mode", "auto"),
            answer(&mut rx, json!({})),
        );
        assert!(options.unwrap().is_empty());
    }

    /// Cancelling is a notification, and has to stay one: the request it is
    /// cancelling is the `session/prompt` still holding the turn open, so a
    /// `cancel` that waited for a reply would be waiting on the thing it is
    /// trying to stop.
    #[test]
    fn cancel_is_a_notification_naming_the_session() {
        let (client, mut rx) = detached();
        client.cancel("20260821_1");
        match rx.try_recv() {
            Ok(Cmd::Notify { method, params }) => {
                assert_eq!(method, "session/cancel");
                assert_eq!(params, json!({"sessionId": "20260821_1"}));
            }
            _ => panic!("cancel must go out as a notification, not a request"),
        }
    }

    /// Both answers to a permission prompt, in the shape ACP asks for: an
    /// outcome object inside an `outcome` key. A dismissed sheet has to send
    /// `cancelled` rather than nothing at all — the agent's turn is blocked on
    /// this reply, and silence is a chat that never finishes.
    #[test]
    fn a_permission_prompt_is_answered_either_way() {
        let (client, mut rx) = detached();
        client.respond_permission(json!("perm-1"), Some("allow_once".to_string()));
        client.respond_permission(json!(7), None);

        let mut answers = Vec::new();
        while let Ok(cmd) = rx.try_recv() {
            match cmd {
                Cmd::Respond {
                    id,
                    result: Ok(outcome),
                } => answers.push((id, outcome)),
                _ => panic!("a permission answer is a response, and never an error"),
            }
        }
        assert_eq!(
            answers,
            vec![
                (
                    json!("perm-1"),
                    json!({"outcome": {"outcome": "selected", "optionId": "allow_once"}})
                ),
                (json!(7), json!({"outcome": {"outcome": "cancelled"}})),
            ],
            "the id is echoed back as it arrived, and the outcome is nested twice"
        );
    }
}
