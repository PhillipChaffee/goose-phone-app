//! The mock's whole world: the sessions it knows about, the session config
//! the client last set, and the fixture switches that decide which of those a
//! test run gets.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

#[derive(Clone, Default)]
pub(crate) struct SessionData {
    pub(crate) cwd: String,
    pub(crate) title: String,
    /// Stored `session/update` payloads, replayed verbatim on session/load.
    pub(crate) conversation: Vec<Value>,
    pub(crate) message_count: u64,
    pub(crate) snippet: String,
}

/// Which set of canned data the mock serves.
///
/// Every screen in the app has three shapes worth testing and only one of
/// them is the interesting-looking one, so the choice is a process-level
/// switch (`MOCK_FIXTURES`) rather than a per-call one: a test spawns the
/// server it wants and then talks normal protocol to it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Fixtures {
    /// Populated lists — recipes, skills, schedules, extensions all present.
    Full,
    /// Every list answers `[]`, which is the zero-state each screen draws.
    Empty,
    /// Lists answer with errors or malformed entries, for the error paths.
    Broken,
}

impl Fixtures {
    /// `MOCK_FIXTURES`, defaulting to [`Fixtures::Full`]. An unrecognised
    /// value is `full` rather than a hard exit: this is a test double, and
    /// failing to start is a worse diagnostic than a banner that disagrees
    /// with what was asked for.
    fn from_env() -> Self {
        match std::env::var("MOCK_FIXTURES").as_deref() {
            Ok("empty") => Self::Empty,
            Ok("broken") => Self::Broken,
            _ => Self::Full,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Empty => "empty",
            Self::Broken => "broken",
        }
    }
}

pub(crate) struct State {
    pub(crate) sessions: HashMap<String, SessionData>,
    pub(crate) next_session: u64,
    /// Whatever the client last selected on each option, so a switch actually
    /// sticks and a reload shows it.
    pub(crate) config: SessionConfig,
    /// Which canned data to serve. Read by the startup banner today; the
    /// feature handlers that consume it land with the features themselves.
    pub(crate) fixtures: Fixtures,
    /// The configured extensions and the secret-store key names. Its shape
    /// lives with the handler that owns it, so the feature's storage arrives
    /// and leaves in one file.
    pub(crate) extensions: crate::features::extensions::Store,
    /// Stand in for a goose started without `--enable-scheduler`, whose
    /// scheduler methods answer `-32601` with the reason in `data`. Same
    /// story: plumbed here so a scheduler branch has somewhere to read it.
    pub(crate) no_scheduler: bool,
}

/// The four options goose routes in `session/set_config_option`. Anything
/// outside them is an `invalid_params` error there and here.
#[derive(Clone)]
pub(crate) struct SessionConfig {
    pub(crate) provider: String,
    pub(crate) mode: String,
    pub(crate) model: String,
    pub(crate) thinking_effort: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            sessions: HashMap::new(),
            next_session: 0,
            config: SessionConfig {
                provider: "anthropic".to_string(),
                mode: "auto".to_string(),
                model: "claude-sonnet-5".to_string(),
                thinking_effort: "off".to_string(),
            },
            fixtures: Fixtures::Full,
            extensions: crate::features::extensions::Store::default(),
            no_scheduler: false,
        }
    }
}

impl State {
    /// The default state with the environment's fixture switches applied.
    /// Reading them once at startup — rather than per request — is what makes
    /// them describable in the banner.
    pub(crate) fn from_env() -> Self {
        let fixtures = Fixtures::from_env();
        Self {
            fixtures,
            // Fixture-driven canned data is built here rather than in
            // `default`, so a unit test constructing a bare `State` starts
            // from nothing and says what it wants out loud.
            extensions: crate::features::extensions::Store::new(fixtures),
            no_scheduler: std::env::var("MOCK_NO_SCHEDULER").is_ok_and(|v| v == "1"),
            ..Self::default()
        }
    }
}

pub(crate) type Shared = Arc<Mutex<State>>;

pub(crate) fn seed(state: &Shared) {
    let seeded = SessionData {
        cwd: "/home/demo".to_string(),
        title: "Seeded example chat".to_string(),
        conversation: vec![
            json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"What files are in my project?"}}),
            json!({"sessionUpdate":"tool_call","toolCallId":"seed_tc1","title":"shell: ls","kind":"execute","status":"pending",
                   "rawInput":{"command":"ls"},
                   "_meta":{"goose":{"toolCall":{"toolName":"developer__shell","extensionName":"developer"}}}}),
            json!({"sessionUpdate":"tool_call_update","toolCallId":"seed_tc1","status":"completed",
                   "content":[{"type":"content","content":{"type":"text","text":"Cargo.toml\nsrc\nREADME.md"}}]}),
            json!({"sessionUpdate":"agent_message_chunk","messageId":"seed_m1",
                   "content":{"type":"text","text":"Your project contains **Cargo.toml**, a `src/` directory, and a README."}}),
        ],
        message_count: 2,
        snippet: "Your project contains Cargo.toml, a src/ directory…".to_string(),
    };

    let mut s = state.lock().unwrap();
    s.next_session = 2;
    s.sessions.insert("20260820_1".to_string(), seeded);
}
