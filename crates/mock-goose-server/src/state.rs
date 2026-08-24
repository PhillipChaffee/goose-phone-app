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
    /// What produced this session — the thing `_meta.types` filters on.
    pub(crate) kind: Kind,
}

/// The three session kinds goose's `session/list` will filter by.
///
/// Spelled out here rather than borrowed from `goose-acp-client` on purpose:
/// the mock is the *other side* of the wire, and a shared enum could not
/// disagree with the client about a wire string. The wire tests are what
/// prove the two agree.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    #[default]
    User,
    Scheduled,
    Acp,
}

impl Kind {
    pub(crate) const ALL: [Self; 3] = [Self::User, Self::Scheduled, Self::Acp];

    pub(crate) const fn as_wire(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Scheduled => "scheduled",
            Self::Acp => "acp",
        }
    }

    pub(crate) fn from_wire(wire: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_wire() == wire)
    }
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
            no_scheduler: false,
        }
    }
}

impl State {
    /// The default state with the environment's fixture switches applied.
    /// Reading them once at startup — rather than per request — is what makes
    /// them describable in the banner.
    pub(crate) fn from_env() -> Self {
        Self {
            fixtures: Fixtures::from_env(),
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
        kind: Kind::User,
    };

    // A scheduler run and another agent client's session. The app filtered
    // `session/list` down to `user` for its whole life, so these two are the
    // fixtures for the sessions it used to hide — and, at a page size of two,
    // for the second page as well.
    let scheduled = SessionData {
        cwd: "/home/demo".to_string(),
        title: "Nightly dependency audit".to_string(),
        conversation: vec![
            json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Audit dependencies for advisories."}}),
            json!({"sessionUpdate":"agent_message_chunk","messageId":"sched_m1",
                   "content":{"type":"text","text":"No new advisories since yesterday."}}),
        ],
        message_count: 2,
        snippet: "No new advisories since yesterday.".to_string(),
        kind: Kind::Scheduled,
    };
    let agent = SessionData {
        cwd: "/home/demo/tools".to_string(),
        title: "Sub-agent: summarise the audit".to_string(),
        conversation: vec![
            json!({"sessionUpdate":"agent_message_chunk","messageId":"acp_m1",
                   "content":{"type":"text","text":"Summary written to audit.md."}}),
        ],
        message_count: 1,
        snippet: "Summary written to audit.md.".to_string(),
        kind: Kind::Acp,
    };

    let mut s = state.lock().unwrap();
    s.next_session = 2;
    s.sessions.insert("20260820_1".to_string(), seeded);
    s.sessions.insert("20260819_1".to_string(), scheduled);
    s.sessions.insert("20260818_1".to_string(), agent);
}
