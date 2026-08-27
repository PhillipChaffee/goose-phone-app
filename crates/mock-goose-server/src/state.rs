//! The mock's whole world: the sessions it knows about, the session config
//! the client last set, and the fixture switches that decide which of those a
//! test run gets.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

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
    /// Set by `session/rename`, exactly as goose's `user_set_name` is: it is
    /// what stops the auto-titler writing over a name a person chose.
    pub(crate) user_set_name: bool,
    pub(crate) created_at: String,
    /// Last write of any kind, reported as `updatedAt`. A rename bumps it —
    /// goose's update statement ends `updated_at = datetime('now')`.
    pub(crate) updated_at: String,
    /// Last *message*, which is what the list is ordered by and what the
    /// cursor walks. Goose sorts on
    /// `COALESCE(MAX(message timestamp), updated_at)`, so the two come apart
    /// the moment a session is renamed: the badge says "now" and the row does
    /// not move. Keeping them apart here is the only way the app can be shown
    /// that before a user is.
    pub(crate) sort_at: String,
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
    /// Whether [`Fixtures::Broken`] has spent its one skill-listing failure.
    ///
    /// Broken fails the first `sources/list` for filesystem skills and answers
    /// normally after that, so the screen's recovery — the pull gesture — is a
    /// path a person can walk. A mock that failed forever would show the error
    /// state and nothing else, which proves only half of it.
    pub(crate) skills_broken_spent: bool,
    /// Whether [`Fixtures::Broken`] has already spent its one failure. The
    /// broken list fails *once* and then works, because an error a refresh
    /// cannot clear is a screen the app can never be driven past.
    pub(crate) session_list_failed: bool,
    /// Stand in for a goose started without `--enable-scheduler`, whose
    /// scheduler methods answer `-32601` with the reason in `data`. Same
    /// story: plumbed here so a scheduler branch has somewhere to read it.
    pub(crate) no_scheduler: bool,
    /// What happens to a round that is still running when its client goes
    /// away. See [`DieOnClose`].
    pub(crate) die_on_close: DieOnClose,
}

/// What the mock does with an in-flight round when the socket dies under it.
///
/// This exists because the default is *wrong in a way that hides a real bug*.
/// `Turn::ask_permission` waits on a oneshot whose sender lives in a map the
/// turn task itself keeps alive, so a dead socket leaves the turn parked
/// forever: the mock behaves the way we wish goose behaved, and a regression
/// test written against it passes on a server that never had the failure.
///
/// [`DieOnClose::Abort`] is the measured behaviour of goose 1.46.0
/// (`docs/permission-durability.md` section 0): the round is discarded, and
/// the prompt and the generated title are all that survive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum DieOnClose {
    /// Park the turn forever. Not faithful; kept as the default so every test
    /// written before this switch existed still means what it meant.
    #[default]
    Park,
    /// Drop the round: kill the turn, keep the prompt and the title.
    Abort,
}

impl DieOnClose {
    /// `MOCK_DIE_ON_CLOSE`, defaulting to [`DieOnClose::Park`]. Only `abort`
    /// is recognised; a `cancel` mode was designed as a hedge against the
    /// account that said goose answers its own ask with `Permission::Cancel`,
    /// and that account was falsified, so there is nothing for it to model.
    fn from_env() -> Self {
        match std::env::var("MOCK_DIE_ON_CLOSE").as_deref() {
            Ok("abort") => Self::Abort,
            _ => Self::Park,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Park => "park",
            Self::Abort => "abort",
        }
    }
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
            config: SessionConfig {
                provider: "anthropic".to_string(),
                mode: "auto".to_string(),
                model: "claude-sonnet-5".to_string(),
                thinking_effort: "off".to_string(),
            },
            fixtures: Fixtures::Full,
            extensions: crate::features::extensions::Store::default(),
            skills_broken_spent: false,
            session_list_failed: false,
            no_scheduler: false,
            die_on_close: DieOnClose::Park,
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
            die_on_close: DieOnClose::from_env(),
            ..Self::default()
        }
    }

    /// A session id in goose's shape: the day, then the next free number
    /// within that day.
    ///
    /// Goose mints these in SQL — `MAX(SUBSTR(id, 10)) + 1` for rows whose id
    /// starts with today — so the number is a property of the store, not a
    /// counter something has to remember to bump. Same here, which is why a
    /// seeded session and a freshly created one cannot collide.
    pub(crate) fn mint_session_id(&self, day: &str) -> String {
        let used = self.sessions.keys().filter_map(|id| {
            id.strip_prefix(day)
                .and_then(|rest| rest.strip_prefix('_'))
                .and_then(|n| n.parse::<u64>().ok())
        });
        format!("{day}_{}", used.max().unwrap_or(0) + 1)
    }
}

/// Seconds since the Unix epoch.
pub(crate) fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs().cast_signed())
}

/// An instant in the two spellings goose uses for it: the RFC3339 timestamp
/// that rides on the wire, and the `YYYYMMDD` day its session ids are
/// numbered within.
pub(crate) struct Stamp {
    pub(crate) rfc3339: String,
    pub(crate) day: String,
}

/// [`Stamp`] for `epoch`, in UTC.
///
/// Doing calendar arithmetic by hand rather than taking a `chrono` dependency
/// on a test double: it is fifteen lines, and they are the same fifteen the
/// app already has in `src/state.rs` for turning a timestamp back into a date.
pub(crate) fn stamp(epoch: i64) -> Stamp {
    // Howard Hinnant's civil-from-days, shifted to an era starting 0000-03-01
    // so a leap day lands at the end of a year and needs no special case.
    let mut days = epoch.div_euclid(86_400) + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    days -= era * 146_097;
    let year_of_era = (days - days / 1460 + days / 36_524 - days / 146_096) / 365;
    let day_of_year = days - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_of_era = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_of_era + 2) / 5 + 1;
    let month = if month_of_era < 10 {
        month_of_era + 3
    } else {
        month_of_era - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);

    let secs = epoch.rem_euclid(86_400);
    let (hour, minute, second) = (secs / 3_600, (secs / 60) % 60, secs % 60);
    Stamp {
        rfc3339: format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"),
        day: format!("{year:04}{month:02}{day:02}"),
    }
}

pub(crate) type Shared = Arc<Mutex<State>>;

/// One seeded session, dated against the moment the process started rather
/// than a date written into the file.
///
/// A fixture with a hardcoded timestamp reads "20 Aug" three weeks later, and
/// the list row's age badge — "now", "2h", "3d" — is then never exercised by
/// the thing it exists for. Relative offsets keep the spread of ages the
/// screen was designed around true on any day the mock is run.
struct Fixture {
    ago: i64,
    kind: Kind,
    cwd: &'static str,
    title: &'static str,
    snippet: &'static str,
    conversation: Vec<Value>,
}

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;

/// The seeded sessions, oldest first so the per-day numbering in their ids
/// runs forward the way a real store's does.
///
/// Between them they cover what `session/list` can be asked: all three kinds,
/// enough sessions to need three pages, ages from minutes to a week, and one
/// title long enough to find out what the row does with it.
fn fixtures() -> Vec<Fixture> {
    vec![
        Fixture {
            ago: 6 * DAY + 2 * HOUR,
            kind: Kind::Scheduled,
            cwd: "/home/demo",
            title: "Weekly changelog digest",
            snippet: "Eleven pull requests landed this week; the session list is the headline.",
            conversation: vec![
                json!({"sessionUpdate":"user_message_chunk",
                       "content":{"type":"text","text":"Summarise the merged pull requests for the changelog."}}),
                json!({"sessionUpdate":"agent_message_chunk","messageId":"digest_m1",
                       "content":{"type":"text","text":"Eleven pull requests landed this week; the session list is the headline."}}),
            ],
        },
        Fixture {
            ago: 2 * DAY + 3 * HOUR,
            kind: Kind::User,
            cwd: "/home/demo",
            title: "Draft the release notes for 0.4",
            snippet: "Six bullets drafted. The pull-to-refresh change still wants a screenshot.",
            conversation: vec![
                json!({"sessionUpdate":"user_message_chunk",
                       "content":{"type":"text","text":"Draft the release notes for 0.4 from the merged PRs."}}),
                json!({"sessionUpdate":"agent_message_chunk","messageId":"notes_m1",
                       "content":{"type":"text","text":"Six bullets drafted. The pull-to-refresh change still wants a screenshot."}}),
            ],
        },
        Fixture {
            ago: DAY + HOUR,
            kind: Kind::Acp,
            cwd: "/home/demo/tools",
            title: "Sub-agent: summarise the audit",
            snippet: "Summary written to audit.md.",
            conversation: vec![json!({"sessionUpdate":"agent_message_chunk","messageId":"acp_m1",
                       "content":{"type":"text","text":"Summary written to audit.md."}})],
        },
        Fixture {
            ago: 9 * HOUR,
            kind: Kind::Scheduled,
            cwd: "/home/demo",
            title: "Nightly dependency audit",
            snippet: "No new advisories since yesterday.",
            conversation: vec![
                json!({"sessionUpdate":"user_message_chunk",
                       "content":{"type":"text","text":"Audit dependencies for advisories."}}),
                json!({"sessionUpdate":"agent_message_chunk","messageId":"sched_m1",
                       "content":{"type":"text","text":"No new advisories since yesterday."}}),
            ],
        },
        Fixture {
            ago: 6 * HOUR,
            kind: Kind::User,
            cwd: "/home/demo",
            title: "Seeded example chat",
            snippet: "Your project contains Cargo.toml, a src/ directory\u{2026}",
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
        },
        Fixture {
            ago: 2 * HOUR,
            kind: Kind::User,
            cwd: "/home/demo",
            title: "Tailscale certificate rotation",
            snippet: "Re-issued the certificate and restarted the listener on 3285.",
            conversation: vec![
                json!({"sessionUpdate":"user_message_chunk",
                       "content":{"type":"text","text":"The phone cannot reach the server since the certificate rotated."}}),
                json!({"sessionUpdate":"agent_message_chunk","messageId":"cert_m1",
                       "content":{"type":"text","text":"Re-issued the certificate and restarted the listener on 3285."}}),
            ],
        },
        // The long-text stress fixture: goose titles a session from its first
        // message, so a rambling question becomes a rambling title, and this
        // is the width every list row has to survive.
        Fixture {
            ago: 4 * MINUTE,
            kind: Kind::User,
            cwd: "/home/demo",
            title: "Why does the transcript jump to the bottom when a tool call finishes while I am scrolled up in a long session?",
            snippet: "The scroll anchor is re-attached on every update, so the view follows the newest node instead of staying where you left it.",
            conversation: vec![
                json!({"sessionUpdate":"user_message_chunk",
                       "content":{"type":"text","text":"Why does the transcript jump to the bottom when a tool call finishes while I am scrolled up in a long session?"}}),
                json!({"sessionUpdate":"agent_message_chunk","messageId":"scroll_m1",
                       "content":{"type":"text","text":"The scroll anchor is re-attached on every update, so the view follows the newest node instead of staying where you left it."}}),
            ],
        },
    ]
}

/// How many of a conversation's updates are messages a person would count.
///
/// Derived rather than written down beside each fixture: goose counts rows in
/// its message table, so a fixture that claimed a different number would be
/// stating something the real server cannot.
fn visible_messages(conversation: &[Value]) -> u64 {
    conversation
        .iter()
        .filter(|update| {
            matches!(
                update.get("sessionUpdate").and_then(Value::as_str),
                Some("user_message_chunk" | "agent_message_chunk")
            )
        })
        .count()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(crate) fn seed(state: &Shared) {
    let mut s = state.lock().unwrap();
    // `empty` is a store with nothing in it, not a handler that lies about
    // what it holds: the zero state a screen draws is the one a fresh install
    // actually produces, and a session created during the run still lists.
    if matches!(s.fixtures, Fixtures::Empty) {
        return;
    }

    let now = now_epoch();
    for fixture in fixtures() {
        let at = stamp(now - fixture.ago);
        let id = s.mint_session_id(&at.day);
        s.sessions.insert(
            id,
            SessionData {
                cwd: fixture.cwd.to_string(),
                title: fixture.title.to_string(),
                message_count: visible_messages(&fixture.conversation),
                conversation: fixture.conversation,
                snippet: fixture.snippet.to_string(),
                kind: fixture.kind,
                user_set_name: false,
                created_at: at.rfc3339.clone(),
                updated_at: at.rfc3339.clone(),
                sort_at: at.rfc3339,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The calendar arithmetic is hand-rolled, so it gets the two dates that
    /// break hand-rolled calendar arithmetic: a leap day, and the last second
    /// of a year.
    #[test]
    fn stamps_are_utc_calendar_dates() {
        assert_eq!(stamp(0).rfc3339, "1970-01-01T00:00:00Z");
        assert_eq!(stamp(1_756_000_000).rfc3339, "2025-08-24T01:46:40Z");
        assert_eq!(stamp(1_709_164_800).rfc3339, "2024-02-29T00:00:00Z");
        assert_eq!(stamp(1_767_225_599).rfc3339, "2025-12-31T23:59:59Z");
        assert_eq!(stamp(1_709_164_800).day, "20240229");
    }

    /// Ids are numbered within their day and never reuse a number, which is
    /// what stops a session created during a run from landing on top of a
    /// seeded one dated the same day.
    #[test]
    fn session_ids_are_numbered_within_their_day() {
        let mut state = State::default();
        assert_eq!(state.mint_session_id("20260824"), "20260824_1");

        state
            .sessions
            .insert("20260824_1".to_string(), SessionData::default());
        state
            .sessions
            .insert("20260824_7".to_string(), SessionData::default());
        state
            .sessions
            .insert("20260823_9".to_string(), SessionData::default());
        assert_eq!(state.mint_session_id("20260824"), "20260824_8");
        assert_eq!(state.mint_session_id("20260825"), "20260825_1");
    }

    /// Every seeded session has to be listable, and `session/list` only shows
    /// sessions with messages — a fixture with none would be seeded and then
    /// invisible, which is a fixture that does not exist.
    #[test]
    fn every_fixture_has_messages_and_a_snippet() {
        for fixture in fixtures() {
            assert!(
                visible_messages(&fixture.conversation) > 0,
                "{} has no messages",
                fixture.title
            );
            assert!(
                !fixture.snippet.is_empty(),
                "{} has no snippet",
                fixture.title
            );
        }
    }

    /// `empty` is the zero state, and it is a store with nothing in it rather
    /// than a handler that hides what it holds.
    #[test]
    fn the_empty_fixture_seeds_nothing() {
        let state: Shared = Arc::new(Mutex::new(State {
            fixtures: Fixtures::Empty,
            ..State::default()
        }));
        seed(&state);
        assert!(state.lock().unwrap().sessions.is_empty());

        let state: Shared = Arc::new(Mutex::new(State::default()));
        seed(&state);
        assert_eq!(state.lock().unwrap().sessions.len(), fixtures().len());
    }
}
