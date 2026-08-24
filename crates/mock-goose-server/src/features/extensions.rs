//! Extensions: goose's own catalogue, the persisted config store the
//! extensions screen reads and writes, and the session-level handshake that is
//! the only honest credential check.
//!
//! The point of mocking this rather than stubbing it per test is the pair of
//! failures the real feature is built around, and neither of them shows up in
//! a single call:
//!
//! - an `add` that returns OK and then reads back with no `available_tools`,
//!   which goose treats as "every tool this server publishes is allowed";
//! - a `session/extensions/add` that starts an MCP server whose `envKeys`
//!   name a secret nobody stored, which is the only honest credential check
//!   there is.
//!
//! So the store here is a real store: an `add` is visible to the next `list`,
//! a `set-enabled` sticks, and the handshake consults the secrets an earlier
//! `config/upsert` actually wrote.
//!
//! It is fed the JSON the real client produces, and the fidelity that matters
//! runs in one specific direction: the mock must reproduce goose's *failure*,
//! not an idealised version of it. If it silently kept a camelCase allowlist,
//! every test that ran against it would pass while the app shipped the bug
//! this whole surface exists to prevent.

use std::collections::BTreeSet;

use serde_json::{json, Value};

use crate::rpc::Out;
use crate::state::{Fixtures, Shared};

use super::Handled;

/// The `_goose/unstable/*` extension and config methods.
///
/// `out` goes unused: every method here answers in its own response frame.
/// Nothing on this surface pushes a `session/update`, because nothing on it
/// changes what a *session* is showing — the config plane is global, and the
/// screen that drives it re-reads by pull.
pub(crate) fn handle(method: &str, params: &Value, state: &Shared, _out: &Out) -> Handled {
    let result = match method {
        "_goose/unstable/extensions/available" => Ok(available()),
        "_goose/unstable/config/extensions/list" => list(state),
        "_goose/unstable/config/extensions/add" => add(params, state),
        "_goose/unstable/config/extensions/set-enabled" => set_enabled(params, state),
        "_goose/unstable/config/upsert" => upsert(params, state),
        "_goose/unstable/session/extensions/add" => session_add(params, state),
        _ => return None,
    };
    Some(result)
}

/// What the mock has been told about, and what it has been given.
///
/// It lives on [`crate::state::State`] but is declared here, so the feature's
/// storage arrives and leaves in one file rather than as a diff five branches
/// all land in. On `State` rather than in a `static` because `cargo test` runs
/// the unit tests below in one process and in parallel: a shared store would
/// make a secret written by one test visible to another, and the handshake
/// tests would pass or fail on ordering.
#[derive(Default)]
pub(crate) struct Store {
    /// `_goose/unstable/config/extensions/list` rows, as
    /// `{extension, enabled, configKey}`.
    configured: Vec<Value>,
    /// Config-file problems goose noticed while loading. Worth serving: an
    /// extension goose could not load is missing from `configured` entirely,
    /// so a screen that drew only what it got would say nothing about it.
    warnings: Vec<String>,
    /// Stored secrets, **by name only**. The real server keeps the value in
    /// `secrets.yaml`; this one deliberately does not keep it at all, so there
    /// is nothing here to leak.
    secrets: BTreeSet<String>,
    /// `MOCK_FIXTURES=broken`: the first list fails and every one after it is
    /// normal. One failure rather than a permanent one because the path worth
    /// exercising is the *retry* — a server that never recovered would only
    /// ever reach the error screen.
    fail_next_list: bool,
}

impl Store {
    /// The canned data a fixture set implies.
    pub(crate) fn new(fixtures: Fixtures) -> Self {
        Self {
            configured: seed(fixtures),
            // The warning names something that is *not* in the list, which is
            // exactly why it is worth putting on screen.
            warnings: match fixtures {
                Fixtures::Empty => Vec::new(),
                Fixtures::Full | Fixtures::Broken => vec![
                    "skipping extension `legacy-sse`: SSE is unsupported, migrate to \
                     streamable_http"
                        .to_string(),
                ],
            },
            secrets: BTreeSet::new(),
            fail_next_list: matches!(fixtures, Fixtures::Broken),
        }
    }
}

/// The three shapes the extensions screen has to draw.
///
/// Written as literal wire JSON rather than built through the client's types
/// on purpose — this is the *server's* side of the contract, and a fixture
/// that borrowed the client's serializer could not disagree with it.
///
/// [`Fixtures::Broken`] is the one worth explaining: its extension comes back
/// with **no** `available_tools`. That is not malformed JSON, it is the
/// dangerous reading — goose allows every tool the MCP server publishes — and
/// it is the state the screen exists to make visible.
fn seed(fixtures: Fixtures) -> Vec<Value> {
    match fixtures {
        Fixtures::Empty => Vec::new(),
        Fixtures::Broken => vec![json!({
            "extension": {
                "type": "mcp",
                "server": {"name": "mail-imap", "command": "uvx",
                           "args": ["mcp-email-server@1.4.2", "stdio"], "env": []},
                "envKeys": ["MCP_EMAIL_SERVER_PASSWORD"],
                "description": "IMAP/SMTP mail via a provider app password",
                "timeout": 300,
                "bundled": false
            },
            "enabled": true,
            "configKey": "mail-imap"
        })],
        Fixtures::Full => vec![
            json!({
                "extension": {
                    "type": "builtin",
                    "name": "developer",
                    "description": "Shell and file editing",
                    "display_name": "Developer",
                    "timeout": 300,
                    "bundled": true,
                    "available_tools": ["shell", "text_editor"]
                },
                "enabled": true,
                "configKey": "developer"
            }),
            json!({
                "extension": {
                    "type": "mcp",
                    "server": {"type": "http", "name": "todoist",
                               "url": "https://ai.todoist.net/mcp",
                               "headers": [{"name": "Authorization",
                                            "value": "Bearer ${TODOIST_API_KEY}"}]},
                    "envKeys": ["TODOIST_API_KEY"],
                    "description": "Todoist tasks via Doist's first-party remote MCP",
                    "timeout": 300,
                    "bundled": false,
                    "available_tools": ["find-tasks", "add-tasks"]
                },
                "enabled": false,
                "configKey": "todoist"
            }),
        ],
    }
}

/// goose's own catalogue, which is unrestricted: these come back with no
/// `available_tools` at all. Reproduced faithfully — a mock that invented
/// allowlists here would hide the fact that enabling a built-in is a
/// different decision from adding a scoped connector.
fn available() -> Value {
    json!({"extensions": [
        {"type": "builtin", "name": "developer", "display_name": "Developer",
         "description": "Shell, file editing and text tools", "bundled": true},
        {"type": "builtin", "name": "computercontroller", "display_name": "Computer Controller",
         "description": "Web scraping, automation and file caching", "bundled": true},
        {"type": "platform", "name": "memory", "display_name": "Memory",
         "description": "Remembers facts across sessions", "bundled": true},
    ]})
}

fn list(state: &Shared) -> Result<Value, (i64, String)> {
    let (extensions, warnings) = {
        let mut s = state.lock().unwrap();
        if s.extensions.fail_next_list {
            s.extensions.fail_next_list = false;
            return Err((
                -32603,
                "failed to load extensions: ~/.config/goose/config.yaml is not valid YAML \
                 at line 12, column 3"
                    .to_string(),
            ));
        }
        (
            s.extensions.configured.clone(),
            s.extensions.warnings.clone(),
        )
    };
    Ok(json!({"extensions": extensions, "warnings": warnings}))
}

/// Persist an extension, reproducing the one behaviour that makes the client's
/// read-back necessary: only the `snake_case` `available_tools` key is read.
///
/// A camelCase `availableTools` is left in the stored object and ignored,
/// exactly as goose ignores it — goose sets no `deny_unknown_fields` — so the
/// extension ends up with an empty allowlist, which means every tool is
/// allowed. Setting `MOCK_DROP_ALLOWLIST=1` drops the correct spelling too,
/// simulating a server that has moved the field, so the app's hard error can
/// be exercised by hand.
fn add(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let mut extension = params.get("extension").cloned().unwrap_or(Value::Null);
    let enabled = params
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let Some(obj) = extension.as_object_mut() else {
        return Err((-32602, "extension must be an object".to_string()));
    };
    if std::env::var("MOCK_DROP_ALLOWLIST").is_ok_and(|v| v == "1") {
        obj.remove("available_tools");
    }
    // goose stores the allowlist as `Vec<String>`, so an absent field and an
    // empty one are the same thing on the way back out: omitted entirely.
    let tools = obj
        .get("available_tools")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if tools.is_empty() {
        obj.remove("available_tools");
    } else {
        obj.insert("available_tools".to_string(), Value::Array(tools));
    }

    let name = extension_name(&extension);
    if name.is_empty() {
        return Err((-32602, "extension has no name".to_string()));
    }
    let row = json!({
        "extension": extension,
        "enabled": enabled,
        "configKey": name_to_key(&name),
    });

    let mut s = state.lock().unwrap();
    s.extensions
        .configured
        .retain(|e| extension_name(&e["extension"]) != name);
    s.extensions.configured.push(row);
    drop(s);
    Ok(json!({}))
}

fn set_enabled(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let key = params
        .get("configKey")
        .and_then(Value::as_str)
        .unwrap_or("");
    let enabled = params
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let found = {
        let mut s = state.lock().unwrap();
        s.extensions
            .configured
            .iter_mut()
            .find(|e| e["configKey"].as_str() == Some(key))
            .map(|entry| entry["enabled"] = Value::Bool(enabled))
            .is_some()
    };
    if found {
        Ok(json!({}))
    } else {
        Err((-32602, format!("Extension '{key}' not found")))
    }
}

/// Write one config value. Only the secret path is modelled, because it is the
/// only one this app uses.
fn upsert(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let key = params.get("key").and_then(Value::as_str).unwrap_or("");
    if key.is_empty() {
        return Err((-32602, "key is required".to_string()));
    }
    // Real goose logs "Secret value is not a string; skipping" and starts the
    // extension WITHOUT the credential when the value is, say, a numeric app
    // password that got parsed as a number. Same here: the key is only
    // remembered when a string arrives, so a client with that bug fails the
    // handshake below instead of appearing to work.
    if params.get("value").and_then(Value::as_str).is_some() {
        state
            .lock()
            .unwrap()
            .extensions
            .secrets
            .insert(key.to_string());
    }
    Ok(json!({}))
}

/// Bring an extension up in a live session — the handshake the app uses to
/// prove a credential without ever reading one back.
fn session_add(params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
    let extension = params.get("extension").cloned().unwrap_or(Value::Null);

    // The handshake needs a live session, which is why the app makes a
    // throwaway one when no chat is open. An unknown id is an error here, as
    // it is on the real server — otherwise skipping the session would look
    // like a passing credential check.
    let sid = params
        .get("sessionId")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !state.lock().unwrap().sessions.contains_key(sid) {
        return Err((-32002, format!("session not found: {sid}")));
    }

    // Inline env values are refused at the session level by the real server,
    // with this wording.
    let has_inline_env = extension
        .pointer("/server/env")
        .and_then(Value::as_array)
        .is_some_and(|env| !env.is_empty());
    if has_inline_env {
        return Err((
            -32602,
            "extension env values must be passed via envKeys referencing stored \
             secrets, not inline env"
                .to_string(),
        ));
    }

    // goose launches the MCP server here, so a declared env key with no stored
    // secret is a hard startup failure.
    let declared: Vec<String> = extension
        .get("envKeys")
        .and_then(Value::as_array)
        .map(|keys| {
            keys.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let missing: Vec<String> = {
        let s = state.lock().unwrap();
        declared
            .into_iter()
            .filter(|k| !s.extensions.secrets.contains(k))
            .collect()
    };
    if missing.is_empty() {
        Ok(json!({}))
    } else {
        // -32603 rather than a spec code with a canned message: goose reports
        // a failed launch as an internal error whose sentence rides in `data`,
        // and that sentence is what the app shows.
        Err((
            -32603,
            format!(
                "failed to start extension `{}`: missing env {}",
                extension_name(&extension),
                missing.join(", ")
            ),
        ))
    }
}

/// The name goose reads off an extension: the server's name for `mcp`, the
/// top-level `name` for a builtin or platform extension.
fn extension_name(extension: &Value) -> String {
    extension
        .pointer("/server/name")
        .or_else(|| extension.get("name"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// goose's `name_to_key`: lowercase, whitespace **dropped**, anything else
/// outside `[A-Za-z0-9_-]` folded to `_`.
///
/// Reproduced rather than echoed back from the request, so a client that
/// reimplemented the folding would disagree with the mock in exactly the way
/// it disagrees with goose.
fn name_to_key(name: &str) -> String {
    name.chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use goose_acp_client::{GooseExtension, McpServer, StdioMcpServer};
    use tokio::sync::mpsc;

    use super::*;
    use crate::state::SessionData;

    /// Call the handler the way [`super::super::dispatch`] would, asserting on
    /// the way past that the method is one this feature owns — a `None` here
    /// is a routing bug, and saying so beats a confusing unwrap downstream.
    fn call(method: &str, params: &Value, state: &Shared) -> Result<Value, (i64, String)> {
        let (out, _rx) = mpsc::unbounded_channel();
        handle(method, params, state, &out).expect("the extension handler owns this method")
    }

    fn mail() -> GooseExtension {
        GooseExtension::mcp(
            McpServer::Stdio(StdioMcpServer::new(
                "mail-imap",
                "uvx",
                vec!["stdio".into()],
            )),
            vec!["MCP_EMAIL_SERVER_PASSWORD".into()],
            "IMAP mail",
            vec!["list_mailboxes".into(), "get_emails_content".into()],
        )
    }

    fn add_mail(state: &Shared, extension: &Value) -> Result<Value, (i64, String)> {
        call(
            "_goose/unstable/config/extensions/add",
            &json!({"extension": extension, "enabled": true}),
            state,
        )
    }

    /// A live session for the handshake to run in, as `session/new` would
    /// have left behind.
    fn open_session(state: &Shared, sid: &str) {
        state
            .lock()
            .unwrap()
            .sessions
            .insert(sid.to_string(), SessionData::default());
    }

    fn listed(state: &Shared) -> Value {
        call(
            "_goose/unstable/config/extensions/list",
            &Value::Null,
            state,
        )
        .unwrap()
    }

    #[test]
    fn a_snake_case_allowlist_is_kept() {
        let state: Shared = Arc::default();
        add_mail(&state, &serde_json::to_value(mail()).unwrap()).unwrap();

        let rows = listed(&state);
        assert_eq!(rows["extensions"][0]["configKey"], json!("mail-imap"));
        assert_eq!(rows["extensions"][0]["enabled"], json!(true));
        assert_eq!(
            rows["extensions"][0]["extension"]["available_tools"],
            json!(["list_mailboxes", "get_emails_content"])
        );
    }

    #[test]
    fn a_camel_case_allowlist_is_dropped_exactly_as_goose_drops_it() {
        let state: Shared = Arc::default();
        let mut extension = serde_json::to_value(mail()).unwrap();
        let obj = extension.as_object_mut().unwrap();
        let tools = obj.remove("available_tools").unwrap();
        obj.insert("availableTools".to_string(), tools);

        add_mail(&state, &extension).unwrap();

        let rows = listed(&state);
        let stored = &rows["extensions"][0]["extension"];
        assert!(
            stored.get("available_tools").is_none(),
            "the camelCase spelling must not become an allowlist: {stored}"
        );
    }

    /// A credential is proved by starting the extension, never by reading the
    /// value back — so a missing secret has to be an error here.
    #[test]
    fn a_session_add_fails_until_the_secret_is_stored() {
        let state: Shared = Arc::default();
        open_session(&state, "20260821_1");
        let extension = serde_json::to_value(mail()).unwrap();
        let params = json!({"sessionId": "20260821_1", "extension": extension});

        let err = call("_goose/unstable/session/extensions/add", &params, &state).unwrap_err();
        assert!(
            err.1.contains("MCP_EMAIL_SERVER_PASSWORD"),
            "got: {}",
            err.1
        );

        call(
            "_goose/unstable/config/upsert",
            &json!({"key": "MCP_EMAIL_SERVER_PASSWORD", "value": "pw", "isSecret": true}),
            &state,
        )
        .unwrap();
        call("_goose/unstable/session/extensions/add", &params, &state).unwrap();
    }

    /// goose logs "Secret value is not a string; skipping" for a value that
    /// arrived as a number — an app password of all digits is the realistic
    /// case — and then starts the extension with no credential at all.
    #[test]
    fn a_non_string_secret_is_skipped_and_the_extension_will_not_start() {
        let state: Shared = Arc::default();
        open_session(&state, "s");
        call(
            "_goose/unstable/config/upsert",
            &json!({"key": "MCP_EMAIL_SERVER_PASSWORD", "value": 12_345_678, "isSecret": true}),
            &state,
        )
        .unwrap();

        let err = call(
            "_goose/unstable/session/extensions/add",
            &json!({"sessionId": "s", "extension": serde_json::to_value(mail()).unwrap()}),
            &state,
        )
        .unwrap_err();
        assert!(err.1.contains("missing env"), "got: {}", err.1);
    }

    /// The handshake is worth nothing without a session to run it in, so an
    /// unknown session id must not look like a passing credential check.
    #[test]
    fn a_session_add_needs_a_live_session() {
        let state: Shared = Arc::default();
        let err = call(
            "_goose/unstable/session/extensions/add",
            &json!({"sessionId": "never-opened",
                    "extension": serde_json::to_value(mail()).unwrap()}),
            &state,
        )
        .unwrap_err();
        assert_eq!(err.0, -32002);
        assert!(err.1.contains("session not found"), "got: {}", err.1);
    }

    #[test]
    fn toggling_an_unknown_extension_is_an_error() {
        let state: Shared = Arc::default();
        assert!(call(
            "_goose/unstable/config/extensions/set-enabled",
            &json!({"configKey": "nope", "enabled": true}),
            &state,
        )
        .is_err());
    }

    /// An `mcp` extension's name lives on the server, not beside it — a mock
    /// that read the wrong one would key everything it stored as `""`, and the
    /// client's verified add would never find what it had just written.
    #[test]
    fn an_mcp_extension_is_named_by_its_server() {
        let mcp = json!({"type": "mcp", "server": {"name": "todoist", "url": "u", "headers": []}});
        assert_eq!(extension_name(&mcp), "todoist");
        let builtin = json!({"type": "builtin", "name": "developer"});
        assert_eq!(extension_name(&builtin), "developer");
    }

    /// goose *drops* whitespace rather than folding it to `_`, so a display
    /// name with a space is one underscore shorter than the obvious reading.
    #[test]
    fn a_config_key_is_folded_the_way_goose_folds_it() {
        assert_eq!(name_to_key("Mail (IMAP)"), "mail_imap_");
        assert_eq!(name_to_key("mail-imap"), "mail-imap");
    }

    /// The zero state has to be reachable, and the fixture switch is what
    /// makes it reachable without deleting anybody's config.
    #[test]
    fn the_empty_fixture_configures_nothing() {
        assert!(Store::new(Fixtures::Empty).configured.is_empty());
        assert!(!Store::new(Fixtures::Full).configured.is_empty());
    }

    /// `broken` fails the *first* list and then behaves, so the retry after
    /// the error screen has something to show — and what it shows is an
    /// extension with no allowlist, which is the dangerous state rather than a
    /// malformed one.
    #[test]
    fn the_broken_fixture_fails_once_and_then_serves_an_unrestricted_extension() {
        let state: Shared = Arc::default();
        state.lock().unwrap().extensions = Store::new(Fixtures::Broken);

        let err = call(
            "_goose/unstable/config/extensions/list",
            &Value::Null,
            &state,
        )
        .unwrap_err();
        assert_eq!(err.0, -32603);
        assert!(err.1.contains("config.yaml"), "got: {}", err.1);

        let rows = listed(&state);
        assert!(rows["extensions"][0]["extension"]
            .get("available_tools")
            .is_none());
        assert!(!rows["warnings"].as_array().unwrap().is_empty());
    }
}
