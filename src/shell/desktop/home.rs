//! What the content column shows when nothing is open.
//!
//! One file for both halves, and that is not an abstraction over them. The two
//! screens share no data, no vocabulary and no sections — they share a SHAPE:
//! a greeting, the thing you type into, and what you might pick up. Two files
//! would be two copies of that shape drifting apart, and the shape is the only
//! part that is common.
//!
//! WHAT IS NOT HERE, and why. The mockups' home screens are dense with numbers
//! this app has no source for — dollars spent today, containers warm of six,
//! round-trip p50/p99, server memory, session budget, queue depth.
//! `crates/opencode-client` has no endpoint for any of it. The owner's
//! instruction was to ship only what is real, so a row with no source is
//! ABSENT rather than zeroed: a tile reading `$0.00 / $4.00` is not honest
//! emptiness, it is a wrong number.
//!
//! CONTEXT-WINDOW SIZE IS NOT ON THAT LIST, though this file claimed twice
//! that it was. `Usage` is not "tokens in and out" — `src/state.rs:178`
//! declares it `(tokens used, context limit)` and the fill site reads goose's
//! own `used` and `contextLimit` off a `usage_update`. So the mockups' "1M
//! context" chip is honest, and `views::chat::crowding` was already computing
//! the percentage. Corrected here rather than left standing, because a wrong
//! comment about what the wire carries is how a real source stays unused.
//!
//! The consequence is that these are sparser than the picture, and that is the
//! intended outcome rather than an unfinished one. What is here is measured
//! from `AppCtx` and nothing else.

use dioxus::prelude::*;

use crate::icons::Icon;
use crate::nav::Plane;
use crate::state::AppCtx;

/// The part of the day, for the greeting.
///
/// `hour` is a parameter rather than read from the clock, for the reason
/// `sidebar::band_of` states: a function that asked the clock itself could
/// only be checked by a test that also asked it, which is an assertion
/// supplying its own needle.
///
/// The boundaries are the ordinary English ones. There is no "good night"
/// because the greeting is a salutation, not an observation about the hour —
/// someone working at 2am is being greeted, not told they should be asleep.
pub(crate) const fn part_of_day(hour: u32) -> &'static str {
    match hour {
        0..12 => "Good morning",
        12..18 => "Good afternoon",
        _ => "Good evening",
    }
}

/// One honest line under the greeting, or none.
///
/// It says what is TRUE right now and nothing else. Disconnected is the case
/// worth getting right: the mockup's line talks about threads and streaming,
/// which are facts about a server this app may not be talking to, and saying
/// "12 conversations" over a dead socket is the kind of confident wrongness
/// that makes a reader stop trusting the whole screen.
pub(crate) fn standing(plane: Plane, connected: bool, count: usize) -> String {
    if !connected {
        return match plane {
            Plane::Chat => "Not connected — set a server in Settings.".to_owned(),
            Plane::Code => "Not connected — set the code server in Settings.".to_owned(),
        };
    }
    match (plane, count) {
        (Plane::Chat, 0) => "No conversations yet. Ask goose anything.".to_owned(),
        (Plane::Chat, 1) => "One conversation on the server.".to_owned(),
        (Plane::Chat, n) => format!("{n} conversations on the server."),
        (Plane::Code, 0) => "No working trees yet.".to_owned(),
        (Plane::Code, 1) => "One working tree.".to_owned(),
        (Plane::Code, n) => format!("{n} working trees."),
    }
}

/// What the composer offers to do, per half.
pub(crate) const fn compose_placeholder(plane: Plane) -> &'static str {
    match plane {
        Plane::Chat => "Ask goose anything…",
        Plane::Code => "Describe a change — it runs on its own branch…",
    }
}

/// A count worth putting on the Code half's home screen.
///
/// Only four, and every one of them is read off a signal. The mockup has nine;
/// five of them (spend, containers warm, queue depth, cold-start seconds,
/// runners busy) have no source on this side of the wire at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tile {
    pub value: String,
    pub label: &'static str,
    /// The mockup's second line. `None` where there is no SECOND fact — a
    /// blank sub-line is a data slot pretending to be data.
    pub sub: Option<String>,
    /// Something is awake. The accent's one use in this block.
    pub live: bool,
    /// Amber when it is a number the reader has to act on. Exactly one tile
    /// can be, and it is the one counting questions the agents are blocked on
    /// — the same fact `sidebar::Mark::Waiting` paints on a row.
    pub urgent: bool,
}

/// The Code half's counts, from `AppCtx` and nothing else.
pub(crate) fn code_tiles(ctx: &AppCtx) -> Vec<Tile> {
    let chats = (ctx.code_chats)();
    let awake = chats.iter().filter(|c| c.is_running()).count();
    let asleep = chats.len().saturating_sub(awake);
    // TREES, NOT ASKS, and that is a correction. `code_permissions` is a FIFO
    // of asks tagged by chat, so two questions parked in one container read as
    // two trees waiting on you — while the board three inches below paints one
    // amber row. The tile and the rows have to agree.
    let waiting = (ctx.code_permissions)()
        .iter()
        .map(|(chat, _)| chat.clone())
        .collect::<std::collections::HashSet<String>>()
        .len();
    let repos = {
        let mut names: Vec<&str> = chats
            .iter()
            .map(|c| c.repo.as_str())
            .filter(|r| !r.trim().is_empty())
            .collect();
        names.sort_unstable();
        names.dedup();
        names.len()
    };
    let allowed = (ctx.code_repos)().len();
    vec![
        Tile {
            value: chats.len().to_string(),
            label: "working trees",
            sub: None,
            urgent: false,
            live: false,
        },
        Tile {
            value: waiting.to_string(),
            label: "waiting on you",
            sub: None,
            urgent: waiting > 0,
            live: false,
        },
        // "AWAKE", NOT "RUNNING NOW", and it is a correction rather than a
        // rewording. `ChatMeta.status` is the CONTAINER's lifecycle —
        // `running | stopped | absent` — and not a turn's; `code::status_label`
        // reads the same field and calls that state "idle" when no turn is in
        // flight. Nothing on the manager's index says whether an agent is
        // mid-turn, so a tile headed "running now" was over-claiming, directly
        // above rows that would have had to say "idle".
        Tile {
            value: awake.to_string(),
            label: "awake",
            sub: (asleep > 0).then(|| format!("{asleep} asleep")),
            urgent: false,
            live: awake > 0,
        },
        Tile {
            value: repos.to_string(),
            label: "repos",
            sub: (allowed > 0).then(|| format!("{allowed} allowed")),
            urgent: false,
            live: false,
        },
    ]
}

/// What a working tree is doing, in the one word a board has room for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TreeState {
    /// An ask is parked in it. Outranks awake, for `sidebar::Mark`'s reason.
    Waiting,
    Awake,
    Asleep,
}

impl TreeState {
    pub(crate) const fn class(self) -> &'static str {
        match self {
            Self::Waiting => "tree waiting",
            Self::Awake => "tree awake",
            Self::Asleep => "tree",
        }
    }

    /// The same words the tiles above use, deliberately: a board that headed a
    /// tile "awake" over a row reading "running" would be the app disagreeing
    /// with itself in one column.
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::Waiting => "waiting on you",
            Self::Awake => "awake",
            Self::Asleep => "asleep",
        }
    }
}

/// One working tree on the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tree {
    pub id: String,
    pub title: String,
    pub branch: Option<String>,
    /// `ChatMeta.base` is empty for every chat made before the base picker
    /// existed, which is a default rather than a migration — so an empty one
    /// renders no line instead of the word "from".
    pub base: Option<String>,
    /// The ask that is parked in it, or nothing. The mockup's own content here
    /// is the live shell command, which no index on this wire carries.
    pub say: Option<String>,
    pub state: TreeState,
    pub age: Option<String>,
}

/// A repo, and the trees cut from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoGroup {
    pub repo: String,
    /// Only when every tree in the group was cut from the same ref. Two bases
    /// in one repo is a fact about the trees, not about the repo, so the
    /// heading says nothing rather than picking one.
    pub base: Option<String>,
    pub awake: usize,
    pub waiting: usize,
    pub trees: Vec<Tree>,
}

/// THE CODE HALF'S BOARD: per-repo groups of working trees.
///
/// `now` is a parameter for `sidebar::band_of`'s reason — a function that read
/// the clock could only be checked by a test that also read it.
pub(crate) fn code_board(ctx: &AppCtx, now: i64) -> Vec<RepoGroup> {
    // The FRONT of each chat's queue, which is what `views::code` already does
    // with the same list: a board row is not the place to work through a
    // backlog, it is the place to say there is one.
    let asks = (ctx.code_permissions)();

    let mut chats = (ctx.code_chats)();
    // Newest first, `sidebar::code_rows`' own comparator.
    chats.sort_by(|a, b| b.last_active.total_cmp(&a.last_active));

    let mut groups: Vec<RepoGroup> = Vec::new();
    for meta in &chats {
        let repo = meta.repo.trim();
        // The manager can send a chat with no repo at all — the field is
        // `serde(default)`. Bucketed under a name rather than dropped, because
        // a tree missing from the board is worse than one filed oddly.
        let repo = if repo.is_empty() { "no repo" } else { repo };
        let ask = asks.iter().find(|(chat, _)| chat == &meta.id);
        let state = if ask.is_some() {
            TreeState::Waiting
        } else if meta.is_running() {
            TreeState::Awake
        } else {
            TreeState::Asleep
        };
        let tree = Tree {
            id: meta.id.clone(),
            title: if meta.title.trim().is_empty() {
                meta.id.clone()
            } else {
                meta.title.clone()
            },
            branch: (!meta.branch.trim().is_empty()).then(|| meta.branch.clone()),
            base: (!meta.base.trim().is_empty()).then(|| format!("from {}", meta.base)),
            say: ask
                .map(|(_, p)| p.title.trim().to_owned())
                .filter(|t| !t.is_empty()),
            state,
            // Guarded against a stamp in the FUTURE, which is a clock skew
            // between this machine and the manager and would otherwise render
            // as "in 3 hours". `now` is an `i64` of seconds and `last_active`
            // an `f64` of the same; the cast is scoped to this one comparison
            // and is exact for any epoch second this century.
            age: {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "an epoch second is ~2^31, three orders of \
                              magnitude below where f64 stops representing \
                              integers exactly"
                )]
                let now = now as f64;
                (meta.last_active > 0.0 && meta.last_active <= now)
                    .then(|| crate::state::relative_time_secs(meta.last_active))
            },
        };
        if let Some(group) = groups.iter_mut().find(|g| g.repo == repo) {
            group.trees.push(tree);
        } else {
            groups.push(RepoGroup {
                repo: repo.to_owned(),
                base: None,
                awake: 0,
                waiting: 0,
                trees: vec![tree],
            });
        }
    }
    for group in &mut groups {
        group.awake = group
            .trees
            .iter()
            .filter(|t| t.state == TreeState::Awake)
            .count();
        group.waiting = group
            .trees
            .iter()
            .filter(|t| t.state == TreeState::Waiting)
            .count();
        // One base or none — see the field.
        let mut bases: Vec<&str> = group
            .trees
            .iter()
            .filter_map(|t| t.base.as_deref())
            .collect();
        bases.sort_unstable();
        bases.dedup();
        group.base = match bases.as_slice() {
            [one] if group.trees.iter().all(|t| t.base.is_some()) => Some((*one).to_owned()),
            _ => None,
        };
    }
    groups
}

/// The chat home's three-sentence lede.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lede {
    /// A statement about what the plane IS, true by construction and needing
    /// no source. The mockup's own first sentence, verbatim.
    pub opening: &'static str,
    pub before: String,
    /// The host, which the mockup sets in bold.
    pub host: Option<String>,
    pub after: String,
}

/// The lede, or `None` over a dead socket.
///
/// `None` rather than a disconnected variant, because `standing` already says
/// that sentence and saying it twice in two voices is worse than saying it
/// once — the component falls back to `.home-standing`.
///
/// TWO CLAUSES OF THE MOCKUP'S ARE DROPPED and both for the same reason. "one
/// still streaming SINCE 09:14" needs a start time: `running_sessions` is a
/// bare `HashSet<String>` with no stamp anywhere near it, and
/// `SessionInfo.updated_at` is when the session last CHANGED, not when the
/// current turn began. "When a thread needs to change files, hand it across to
/// Code" describes a feature that does not exist — grepped, there is no
/// hand-across in `src/` — and a lede that instructs the reader to do a thing
/// the app cannot do is worse than one clause shorter.
pub(crate) fn lede(ctx: &AppCtx, connected: bool) -> Option<Lede> {
    if !connected {
        return None;
    }
    let count = (ctx.sessions)().len();
    let host = host_of(&ctx.settings.peek().server_url);
    let running = (ctx.running_sessions)().len();
    let waiting = (ctx.permission)()
        .iter()
        .map(|p| p.session_id.clone())
        .collect::<std::collections::HashSet<String>>()
        .len();

    let head = match count {
        0 => "No conversations yet".to_owned(),
        1 => "One conversation".to_owned(),
        n => format!("{n} conversations"),
    };
    let before = if host.is_some() {
        format!("{head} with the goose server on ")
    } else {
        format!("{head} with the goose server")
    };
    let tail = match (running, waiting) {
        (0, 0) => " \u{2014} nothing running, and none of them waiting on you.".to_owned(),
        (0, w) => format!(" \u{2014} nothing running, and {w} waiting on you."),
        (r, 0) => format!(" \u{2014} {r} still streaming, and none of them waiting on you."),
        (r, w) => format!(" \u{2014} {r} still streaming, and {w} waiting on you."),
    };
    Some(Lede {
        opening: "Nothing on this side touches a repo.",
        before,
        host,
        after: tail,
    })
}

/// The scheduled recipe worth naming, and how many others there are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sched {
    pub name: String,
    pub what: String,
    pub more: usize,
}

/// NOT "Next on this side", and that is the one deviation from the mockup in
/// this block.
///
/// `ScheduledJob` carries `cron`, `last_run` and `job_start_time` and NO
/// next-run field; goose's `schedules/list` does not compute one, and this app
/// cannot — `hour_now` above records that there is no timezone source anywhere
/// in the tree, so a cron expression cannot be turned into a wall-clock "next
/// at 09:30" that would be right for the reader. What IS real is the cadence,
/// which `cron::summary` already says in a sentence, and a run happening right
/// now, which `job_start_time` dates.
///
/// A running job outranks the server's order because it is the only fact on
/// this row that is about this moment.
pub(crate) fn sched_line(ctx: &AppCtx, now: i64) -> Option<Sched> {
    let jobs = (ctx.scheduler.list)().items;
    let live = jobs.iter().filter(|j| !j.paused).count();
    let job = jobs
        .iter()
        .find(|j| j.currently_running)
        .or_else(|| jobs.iter().find(|j| !j.paused))?;
    Some(Sched {
        name: crate::scheduler::title_for(&job.id),
        what: if job.currently_running {
            crate::scheduler::running_for(job.job_start_time.as_deref(), now)
        } else {
            crate::cron::summary(&job.cron)
        },
        more: live.saturating_sub(1),
    })
}

/// The hairline at the bottom of the chat column.
///
/// Sentence one is the mockup's and needs no source: it is a statement about
/// what this half is. Sentence two is its "⌘2 for Code and its six working
/// trees" with the keycap taken out and the count kept — the desktop wires
/// three chords and ⌘2 is not one of them, so the keycap would be a promise
/// nothing keeps. The count is gated on the code plane having answered: until
/// this window has been to the Code half once, `code_chats` is empty for want
/// of a fetch rather than for want of trees, and "Code has 0 working trees"
/// would be a wrong number rather than an empty one.
pub(crate) fn footnote(ctx: &AppCtx) -> (&'static str, Option<String>) {
    let second = (ctx.code_conn)()
        .is_connected()
        .then(|| match (ctx.code_chats)().len() {
            1 => "Code has 1 working tree.".to_owned(),
            n => format!("Code has {n} working trees."),
        });
    (
        "Chat talks only to the goose server \u{2014} no containers, no branches, no diffs.",
        second,
    )
}

/// WHAT THE READER WAS LAST DOING, offered back.
///
/// This block was cut once, on the reasoning that the sidebar three inches to
/// the left is already listing the same sessions. That reasoning was wrong and
/// the mockups say so directly: `10-home-chat.html` renders BOTH — twelve rows
/// in `.side` AND three `.crow`s here — because they are not the same row. The
/// sidebar's is a title, a message count and an age, compacted to fit 252px.
/// This one carries the last thing that was actually SAID in the conversation,
/// which is the only piece of real conversation content anywhere on the screen
/// and the thing that makes "pick up where you left off" a question the reader
/// can answer without opening anything.
///
/// Three, not twelve. The sidebar is the index; this is the shortcut, and a
/// shortcut as long as the index is just the index again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recent {
    pub id: String,
    pub title: String,
    /// The last message, as the server summarised it. `None` renders no quote
    /// rather than an empty one — a session with nothing said in it is a row
    /// with two lines, not a row with a blank third.
    pub quote: Option<String>,
    /// "6 turns", or `None` where the server sent no count.
    pub turns: Option<String>,
    pub age: Option<String>,
    pub state: RecentState,
}

/// What the row's dot and its trailing word say.
///
/// The same three states `sidebar::Mark` paints, deliberately — a conversation
/// that is streaming is streaming in both columns, and two enums would be two
/// chances for the sidebar and the home screen to disagree about it in the
/// same window. It is a separate type only because this one has a WORD as well
/// as a dot, which a 252px row has no room for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecentState {
    /// An agent is mid-turn.
    Running,
    /// An agent asked something and is blocked on the answer. Outranks
    /// running, for `sidebar::Mark`'s reason: a question waiting on the reader
    /// is the only one of the three states that is about THEM.
    Waiting,
    Idle,
}

impl RecentState {
    /// The word beside the dot, or `None` for a conversation that is merely
    /// finished — which is most of them, and which needs no label.
    pub(crate) const fn word(self) -> Option<&'static str> {
        match self {
            Self::Running => Some("streaming"),
            Self::Waiting => Some("needs you"),
            Self::Idle => None,
        }
    }

    pub(crate) const fn class(self) -> &'static str {
        match self {
            Self::Running => "recent running",
            Self::Waiting => "recent waiting",
            Self::Idle => "recent",
        }
    }
}

/// The three most recent conversations, newest first.
///
/// `now` is a parameter for `sidebar::band_of`'s reason: a function that read
/// the clock could only be checked by a test that also read it.
pub(crate) fn recent_for(ctx: &AppCtx, now: i64) -> Vec<Recent> {
    /// Three is the mockups' own count, and it is also what fits above the
    /// fold of the 820pt window `src/main.rs` opens once the composer and a
    /// section heading are above it.
    const MOST: usize = 3;

    let running = (ctx.running_sessions)();
    let waiting: std::collections::HashSet<String> = (ctx.permission)()
        .iter()
        .map(|p| p.session_id.clone())
        .collect();

    let mut sessions = (ctx.sessions)();
    sessions.sort_by_key(|s| {
        std::cmp::Reverse(
            s.updated_at
                .as_deref()
                .and_then(crate::state::rfc3339_to_epoch)
                .unwrap_or(i64::MIN),
        )
    });

    sessions
        .iter()
        .take(MOST)
        .map(|info| Recent {
            id: info.session_id.clone(),
            title: info
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| "Untitled chat".to_owned()),
            quote: info
                .last_message_snippet()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty()),
            turns: info.message_count().map(|n| {
                if n == 1 {
                    "1 turn".to_owned()
                } else {
                    format!("{n} turns")
                }
            }),
            age: info
                .updated_at
                .as_deref()
                .and_then(crate::state::rfc3339_to_epoch)
                .filter(|stamp| *stamp <= now)
                .map(crate::state::relative_time),
            state: if waiting.contains(&info.session_id) {
                RecentState::Waiting
            } else if running.contains(&info.session_id) {
                RecentState::Running
            } else {
                RecentState::Idle
            },
        })
        .collect()
}

/// One thing the reader could start with that is not a blank prompt.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Starter {
    pub name: String,
    /// The server's own one-line description of it. This is what makes a card
    /// worth reading: `.start .s` in the mockups carries "summarises overnight
    /// runs", not the word "recipe". `None` where the server sent none.
    pub description: Option<String>,
    /// "recipe" or "skill" — the word the server uses, so the screen says what
    /// the thing IS rather than inventing a category for it.
    pub kind: &'static str,
    pub icon: &'static str,
    /// Where pressing it goes. The starter does not RUN anything: a recipe
    /// with parameters needs a form, and launching one blind would start a
    /// session against arguments nobody chose. It opens the library screen
    /// that knows how to ask.
    pub tab: crate::state::Tab,
}

/// What the chat half offers to start with, from the server's own lists.
///
/// Capped, because this is a shortcut rather than an index — the Library has
/// the full lists and is one click away. Recipes first: a recipe is a thing
/// to RUN and a skill is a thing the agent knows, so a recipe is the closer
/// answer to "what could I do now".
pub(crate) fn starters_for(ctx: &AppCtx) -> Vec<Starter> {
    const MOST: usize = 4;
    let mut out: Vec<Starter> = (ctx.recipes.list)()
        .items
        .iter()
        .map(|entry| Starter {
            name: entry.recipe.title.clone(),
            description: one_line(&entry.recipe.description),
            kind: "recipe",
            icon: "book",
            tab: crate::state::Tab::Recipes,
        })
        .collect();
    out.extend((ctx.skills.list)().items.iter().map(|skill| Starter {
        name: skill.name.clone(),
        description: one_line(&skill.description),
        kind: "skill",
        icon: "sparkle",
        tab: crate::state::Tab::Skills,
    }));
    out.retain(|s| !s.name.trim().is_empty());
    out.truncate(MOST);
    out
}

/// The first line of a description, trimmed, or `None` if there is not one.
///
/// A recipe's description is free text the author wrote and some of them run
/// to a paragraph. A card in a four-across grid has room for one line, and the
/// CSS clamps — but clamping a paragraph mid-sentence reads as a bug, whereas
/// its first line reads as a summary, which is what the author put there.
fn one_line(text: &str) -> Option<String> {
    let line = text.trim().lines().next().unwrap_or("").trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// One fact under the composer, said in as few characters as it takes.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Chip {
    pub text: String,
    /// Set where the value is an address or an identifier rather than a word —
    /// a host, a path, a model reference. Those are things you COMPARE
    /// character by character, and a proportional face makes that harder for
    /// no gain. It is the one place this shell reaches for `--font-mono`.
    pub mono: bool,
}

/// What the composer knows about the session it is about to start.
///
/// The mockup's row is `Opus 5 · goose server · tail-mini:3285 · 7 extensions
/// · 1M context · $0.41 today`. Five of those six have a source here; the
/// spend does not, and no cost figure exists on either wire, so it is absent
/// rather than zeroed.
///
/// The context size took two tries. It was called sourceless on the strength
/// of a comment saying `Usage` is tokens in and out; it is
/// `(used, context limit)`, so the limit is exactly the mockup's "1M context"
/// and `views::chat::format_tokens` already knows how to say it. It appears
/// only once a `usage_update` has arrived, which is honest: before the first
/// turn the app genuinely does not know the window, and a guessed 200k would
/// be the wrong kind of confident.
pub(crate) fn compose_chips(ctx: &AppCtx, plane: Plane) -> Vec<Chip> {
    let mut out = Vec::new();
    match plane {
        Plane::Chat => {
            if let Some(model) = (ctx.config_options)()
                .iter()
                .find(|o| o.config_id == "model")
                .and_then(goose_acp_client::ConfigOption::current_label)
            {
                out.push(Chip {
                    text: model.to_owned(),
                    mono: false,
                });
            }
            if let Some(host) = host_of(&ctx.settings.peek().server_url) {
                out.push(Chip {
                    text: host,
                    mono: true,
                });
            }
            let loaded = (ctx.extensions.list)().items.len();
            if loaded > 0 {
                out.push(Chip {
                    text: format!("{loaded} extensions"),
                    mono: false,
                });
            }
            if let Some((_, limit)) = (ctx.usage)().filter(|(_, limit)| *limit > 0) {
                out.push(Chip {
                    text: format!("{} context", crate::views::chat::format_tokens(limit)),
                    mono: true,
                });
            }
        }
        Plane::Code => {
            if let Some(host) = host_of(&ctx.settings.peek().code_server_url) {
                out.push(Chip {
                    text: host,
                    mono: true,
                });
            }
            let repos = (ctx.code_repos)().len();
            if repos > 0 {
                out.push(Chip {
                    text: format!("{repos} repos"),
                    mono: false,
                });
            }
        }
    }
    out
}

/// The host and port out of a configured URL, or `None` if there is not one.
///
/// The scheme and any path are dropped: what identifies the server on a
/// tailnet is the name and the port, and `https://` in front of it is six
/// characters of a chip that has about thirty.
pub(crate) fn host_of(url: &str) -> Option<String> {
    let rest = url
        .trim()
        .split_once("://")
        .map_or_else(|| url.trim(), |(_, rest)| rest);
    let host = rest.split(['/', '?']).next().unwrap_or("");
    (!host.is_empty()).then(|| host.to_owned())
}

/// Now's hour, local. Split out so the component has one clock and everything
/// above it has none.
fn hour_now() -> u32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // UTC. The app has no timezone source — no `chrono`, no `libc` binding —
    // and inventing one would be a dependency for a salutation. Written down
    // because it IS wrong for anyone far from UTC, and it is the kind of wrong
    // that looks like a bug rather than a limitation.
    u32::try_from((secs / 3_600) % 24).unwrap_or(0)
}

/// The plane's home screen.
///
/// Its own component rather than rsx inside `AppShell`, for `SidebarList`'s
/// reason: `AppShell` calls `dioxus::desktop::window()` and cannot be mounted
/// in a test, and this reads `AppCtx` and nothing else.
#[component]
pub(crate) fn Home(plane: Plane) -> Element {
    let ctx = crate::state::use_app_ctx();
    let mut draft = use_signal(String::new);

    let connected = match plane {
        Plane::Chat => (ctx.conn)().is_connected(),
        Plane::Code => (ctx.code_conn)().is_connected(),
    };
    let count = match plane {
        Plane::Chat => (ctx.sessions)().len(),
        Plane::Code => (ctx.code_chats)().len(),
    };

    let starters = if plane == Plane::Chat {
        starters_for(&ctx)
    } else {
        Vec::new()
    };
    let recent = if plane == Plane::Chat {
        recent_for(&ctx, crate::state::now_secs())
    } else {
        Vec::new()
    };

    // THIS SCREEN FETCHES WHAT IT SHOWS, and until now it did not.
    //
    // `starters_for` reads `ctx.recipes.list` and `ctx.skills.list`, and the
    // only things that ever filled those were `RecipesView` and `SkillsView`'s
    // own mount effects — so "Ways to start" appeared only after the reader had
    // visited the Library, which is precisely the click the section exists to
    // save. Same for the extensions chip. On a cold launch the captured home
    // screen rendered a placeholder comment where the section should be.
    //
    // Reactive rather than a one-shot, and the reason is `RecipesView`'s
    // verbatim: the fetch is worth starting the moment there is a connection to
    // make it over, including one that arrives after this screen is already up
    // — which on the desktop is the common case, because the home screen is
    // what the window opens on.
    // THE CODE HALF HAS TO CONNECT ITSELF, and nothing on the desktop ever
    // did. `views::code::CodeSessionsView` is the only thing in the app that
    // calls `code_connect`, and this shell renders THIS component where that
    // view would be — so the code plane was never dialled: the board, the
    // sidebar's tree list and every tile read an empty `code_chats` forever,
    // with the standing line correctly reporting a socket nobody had tried.
    //
    // Fires only out of `Disconnected`, and that guard is the whole of it.
    // Reading `code_conn` is what re-arms the effect; `Connecting` is this
    // effect's own write coming back, `Connected` is done, and `Failed` must
    // NOT retry on its own — a gateway that is switched off would otherwise be
    // dialled in a tight loop for as long as the window is open.
    use_effect(move || {
        if plane != Plane::Code {
            return;
        }
        if !matches!((ctx.code_conn)(), crate::state::ConnState::Disconnected) {
            return;
        }
        let ctx = ctx;
        spawn(async move {
            if crate::code::code_connect(&ctx).await {
                crate::code::start_code_poll(&ctx);
            }
        });
    });

    use_effect(move || {
        if plane == Plane::Chat && (ctx.conn)().is_connected() {
            crate::recipes::refresh(&ctx);
            crate::skills::ensure_loaded(&ctx);
            let ctx = ctx;
            spawn(async move { crate::extensions::refresh(&ctx).await });
        }
    });

    // START, and it is the same sequence `recipes::run` uses: put the text
    // where the conversation will find it, then make the conversation. The
    // draft lives on `AppCtx` precisely so something can fill it in before its
    // chat exists — that comment is `open_session`'s, and this is the second
    // caller of the pattern it was written for.
    //
    // It does NOT send. `send_prompt` returns false without a `session_id`,
    // and the session's id only arrives after a round trip, so "type here and
    // it is sent" would mean a task waiting on a signal to change. The text
    // lands in the new chat's composer with the cursor in it instead, which is
    // one keystroke rather than none — stated plainly rather than hidden,
    // because the mockup implies none.
    let mut start = move || {
        let text = draft.peek().trim().to_owned();
        match plane {
            Plane::Chat => {
                let mut chat_draft = ctx.chat_draft;
                chat_draft.set(text);
                crate::state::new_session(&ctx);
            }
            Plane::Code => {
                let mut code_draft = ctx.code_draft;
                code_draft.set(text);
                // The code half has no "create and send": a session needs a
                // repo and a base branch before it can exist, which is what
                // `CodeNewView` is for. So this carries the text and opens
                // that screen — the same shape as the chat side, one screen
                // further along.
                let mut screen = ctx.code_screen;
                screen.set(crate::code::CodeScreen::New);
            }
        }
        draft.set(String::new());
    };

    rsx! {
        main { class: "scroll home",
            div { class: "home-inner",
                // NO GREETING ON THE CODE HALF. The mockup has none — that
                // side opens on a board, because what a working tree is doing
                // is the question, and `hour_now` is UTC and says so.
                if plane == Plane::Chat {
                    h1 { class: "home-greeting", "{part_of_day(hour_now())}." }
                }
                // THE LEDE, three clauses of it, in the reading face. Over a
                // dead socket `lede` answers `None` and the one honest
                // sentence takes its place — see `standing`.
                if let Some(lede) = lede(&ctx, connected) {
                    p { class: "home-lede",
                        "{lede.opening} {lede.before}"
                        if let Some(host) = lede.host.clone() {
                            b { class: "home-lede-host", "{host}" }
                        }
                        "{lede.after}"
                    }
                } else {
                    p { class: "home-standing", "{standing(plane, connected, count)}" }
                }

                // THE COMPOSER IS THE NEW-SESSION AFFORDANCE, which is why the
                // sidebar's New button hides while this is on screen. The
                // owner's words: "I don't think we need a new chat button when
                // the big chat box is visible in the middle."
                div { class: "home-compose",
                    textarea {
                        class: "input",
                        placeholder: compose_placeholder(plane),
                        value: "{draft}",
                        rows: 3,
                        oninput: move |e| draft.set(e.value()),
                        onkeydown: move |e: Event<KeyboardData>| {
                            if e.key() == Key::Enter && !e.modifiers().contains(Modifiers::SHIFT) {
                                e.prevent_default();
                                if !draft.peek().trim().is_empty() {
                                    start();
                                }
                            }
                        },
                    }
                    div { class: "home-compose-row",
                        // WHAT THE SESSION WILL BE, before it exists.
                        //
                        // The mockup puts six facts here; three of them have
                        // no source on the wire and are absent rather than
                        // guessed. See `compose_chips`.
                        div { class: "home-chips",
                            for chip in compose_chips(&ctx, plane) {
                                span {
                                    key: "{chip.text}",
                                    class: if chip.mono { "home-chip mono" } else { "home-chip" },
                                    "{chip.text}"
                                }
                            }
                        }
                        // `.send`, the circle, and NOT `.btn primary`.
                        //
                        // It is the same control the transcript's composer
                        // already uses (`assets/main.css`), which is the point:
                        // the thing you press to send is one shape everywhere
                        // in the app, and the reader meets it here first. A
                        // 131x40 rectangle reading "Start a chat" was saying
                        // out loud what an arrow in a circle says by being
                        // where it is — and, being `--bg-inverse` when live and
                        // `--bg-tertiary` when not, it was also the reason a
                        // resting home screen had no accent on it anywhere.
                        button {
                            class: "send",
                            // A name of its own, so the control is findable by
                            // something other than the words inside it — which
                            // is what a screen reader needs and what
                            // `views::press` locates by. It carries the whole
                            // label now that the face is a glyph.
                            title: match plane {
                                Plane::Chat => "Start a chat",
                                Plane::Code => "Start a session",
                            },
                            "aria-label": match plane {
                                Plane::Chat => "Start a chat",
                                Plane::Code => "Start a session",
                            },
                            // AN EMPTY COMPOSER STARTS NOTHING. Without this a
                            // stray press makes a session on the server — a
                            // real object with a real id that nobody asked for
                            // and someone now has to delete.
                            disabled: draft().trim().is_empty(),
                            onclick: move |_| start(),
                            Icon { name: "arrow-up" }
                        }
                    }
                }

                // PICK UP WHERE YOU LEFT OFF.
                //
                // See `Recent` for why this is here after being cut once. The
                // short version: the sidebar's row and this one are not the
                // same row, and the mockups render both.
                if plane == Plane::Chat && !recent.is_empty() {
                    div { class: "home-recent",
                        h2 { class: "home-section",
                            "Pick up where you left off"
                            span { class: "home-section-meta",
                                {
                                    let n = (ctx.sessions)().len();
                                    if n == 1 { "1 thread".to_owned() } else { format!("{n} threads") }
                                }
                            }
                        }
                        for row in recent {
                            button {
                                key: "{row.id}",
                                class: row.state.class(),
                                title: "{row.title}",
                                // The sidebar row's sequence, and it has to be
                                // this order: `open_session` sets `screen`, not
                                // `tab`, and `nav::current` reads the tab
                                // first — so a row pressed from any other
                                // destination would open a session the window
                                // was not looking at. Entering the plane first
                                // is what makes the press land.
                                onclick: {
                                    let id = row.id;
                                    move |_| {
                                        (crate::nav::primary(Plane::Chat).go)(&ctx);
                                        // Looked up rather than carried: `Recent`
                                        // is what the row DRAWS, and `open_session`
                                        // needs the whole `SessionInfo` — its cwd,
                                        // its kind, the fields a summary has no
                                        // business holding a stale copy of.
                                        if let Some(info) =
                                            (ctx.sessions)().iter().find(|s| s.session_id == id)
                                        {
                                            crate::state::open_session(&ctx, info.clone());
                                        }
                                    }
                                },
                                span { class: "recent-dot" }
                                span { class: "recent-text",
                                    span { class: "recent-title", "{row.title}" }
                                    if let Some(quote) = row.quote.clone() {
                                        span { class: "recent-quote", "{quote}" }
                                    }
                                }
                                span { class: "recent-facts",
                                    if let Some(turns) = row.turns.clone() {
                                        span { class: "recent-fact", "{turns}" }
                                    }
                                    if let Some(word) = row.state.word() {
                                        span { class: "recent-state", "{word}" }
                                    } else if let Some(age) = row.age.clone() {
                                        span { class: "recent-fact", "{age}" }
                                    }
                                }
                            }
                        }
                    }
                }

                // WAYS TO START, and only on the chat half.
                //
                // Recipes and skills are the two things this half can begin
                // with that are not a blank prompt, and the sidebar keeps them
                // behind the Library disclosure — so this is the one place
                // they are visible without a click. Named from the server's
                // own list rather than invented.
                if plane == Plane::Chat && !starters.is_empty() {
                    div { class: "home-starters",
                        h2 { class: "home-section", "Ways to start" }
                        div { class: "home-starter-grid",
                            for starter in starters {
                                button {
                                    key: "{starter.kind}-{starter.name}",
                                    class: "home-starter",
                                    title: "{starter.name}",
                                    onclick: move |_| {
                                        let mut tab = ctx.tab;
                                        tab.set(starter.tab);
                                    },
                                    Icon { name: starter.icon }
                                    span { class: "home-starter-text",
                                        span { class: "home-starter-name", "{starter.name}" }
                                        // The taxonomy word AND the sentence,
                                        // in that order and separated by a
                                        // middot — the mockups' own
                                        // "recipe · summarises overnight
                                        // runs". The word alone was the whole
                                        // second line, which told the reader
                                        // what the card IS and nothing about
                                        // what it would do.
                                        span { class: "home-starter-kind",
                                            "{starter.kind}"
                                            if let Some(what) = starter.description.clone() {
                                                span { class: "home-starter-what", " · {what}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // THE CODE HALF'S COUNTS. Chat has no equivalent worth a tile:
                // a conversation count is already the lede's first clause, and
                // everything else the mockup tiles there is spend and latency,
                // which have no source.
                if plane == Plane::Code {
                    div { class: "home-tiles",
                        for tile in code_tiles(&ctx) {
                            div {
                                key: "{tile.label}",
                                class: if tile.urgent {
                                    "home-tile urgent"
                                } else if tile.live {
                                    "home-tile live"
                                } else {
                                    "home-tile"
                                },
                                div { class: "home-tile-value", "{tile.value}" }
                                div { class: "home-tile-label", "{tile.label}" }
                                if let Some(sub) = tile.sub.clone() {
                                    div { class: "home-tile-sub", "{sub}" }
                                }
                            }
                        }
                    }

                    // THE BOARD: what is actually in each repo. Every child of
                    // a row is a span — the mockup puts an "Answer" button on
                    // three of its six rows, and a button inside a button makes
                    // the parser hoist the inner one, which re-parents
                    // everything after it. That shape produced 1600 audit
                    // findings the one time it shipped here. The whole row is
                    // the target and it opens the chat, where the permission
                    // modal offers the same two answers with the ask beside
                    // them.
                    for group in code_board(&ctx, crate::state::now_secs()) {
                        div { key: "{group.repo}", class: "repo-group",
                            div { class: "repo-head",
                                Icon { name: "repo" }
                                span { class: "repo-head-name", "{group.repo}" }
                                span { class: "repo-head-base",
                                    if let Some(base) = group.base.clone() {
                                        "{base} \u{b7} {group.trees.len()} trees"
                                    } else {
                                        "{group.trees.len()} trees"
                                    }
                                }
                                span { class: "repo-head-facts",
                                    if group.waiting > 0 {
                                        span { class: "warn", "{group.waiting} waiting" }
                                    }
                                    if group.awake > 0 {
                                        span { class: "live", "{group.awake} awake" }
                                    }
                                }
                            }
                            for tree in group.trees {
                                button {
                                    key: "{tree.id}",
                                    class: tree.state.class(),
                                    title: "{tree.title}",
                                    // Enter the plane first — `open_code_chat`
                                    // sets `code_screen` and not `tab`, and
                                    // `nav::current` reads the tab first. The
                                    // bug `sidebar.rs` writes up at length.
                                    onclick: {
                                        let id = tree.id.clone();
                                        move |_| {
                                            (crate::nav::primary(Plane::Code).go)(&ctx);
                                            if let Some(meta) =
                                                (ctx.code_chats)().iter().find(|c| c.id == id)
                                            {
                                                crate::code::open_code_chat(&ctx, meta.clone());
                                            }
                                        }
                                    },
                                    span {
                                        class: "tree-mark",
                                        "aria-label": tree.state.word(),
                                    }
                                    span { class: "tree-text",
                                        span { class: "tree-title", "{tree.title}" }
                                        if let Some(say) = tree.say.clone() {
                                            span { class: "tree-say q", "{say}" }
                                        }
                                    }
                                    span { class: "tree-branch",
                                        if let Some(branch) = tree.branch.clone() {
                                            span { class: "tree-branch-name", "{branch}" }
                                        }
                                        if let Some(base) = tree.base.clone() {
                                            span { class: "tree-branch-base", "{base}" }
                                        }
                                    }
                                    span { class: "tree-state", {tree.state.word()} }
                                    if let Some(age) = tree.age.clone() {
                                        span { class: "tree-age", "{age}" }
                                    }
                                }
                            }
                        }
                    }
                }

                // THE TWO BLOCKS THAT CLOSE THE CHAT COLUMN. Without them the
                // page simply stops after its last card; the mockups end with
                // a dashed schedule row pushed to the bottom and a hairline
                // footnote under it, which is what gives the column an edge.
                if plane == Plane::Chat {
                    if let Some(sched) = sched_line(&ctx, crate::state::now_secs()) {
                        button {
                            class: "home-sched",
                            title: "Open the scheduler",
                            onclick: move |_| {
                                let mut tab = ctx.tab;
                                tab.set(crate::state::Tab::Scheduler);
                            },
                            Icon { name: "clock" }
                            span { class: "home-sched-name", "{sched.name}" }
                            span { class: "home-sched-what", "{sched.what}" }
                            if sched.more > 0 {
                                span { class: "home-sched-more", "+{sched.more} more" }
                            }
                        }
                    }
                    {
                        let (first, second) = footnote(&ctx);
                        rsx! {
                            p { class: "home-footnote",
                                "{first}"
                                if let Some(second) = second {
                                    " {second}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "test scaffolding: a fixture this file wrote that will not parse \
              is a broken test rather than a runtime condition"
)]
mod tests {
    use super::{
        code_board, code_tiles, compose_chips, compose_placeholder, footnote, lede, part_of_day,
        recent_for, sched_line, standing, Home, RecentState, TreeState,
    };
    use crate::nav::Plane;
    use crate::views::press::Pressable;
    use dioxus::prelude::*;

    /// The greeting covers the whole clock, and every hour gets one.
    ///
    /// A `match` with a gap would render an empty `h1` at whatever hour it
    /// missed — visible for an hour a day, and only to whoever is working
    /// then. Walking all 24 is cheaper than trusting the ranges.
    #[test]
    fn every_hour_of_the_day_has_a_greeting() {
        for hour in 0..24 {
            assert!(
                !part_of_day(hour).is_empty(),
                "hour {hour} has no greeting, so the home screen renders a \
                 blank heading for an hour a day"
            );
        }
        assert_eq!(part_of_day(0), "Good morning");
        assert_eq!(part_of_day(11), "Good morning");
        assert_eq!(part_of_day(12), "Good afternoon");
        assert_eq!(part_of_day(17), "Good afternoon");
        assert_eq!(part_of_day(18), "Good evening");
        assert_eq!(part_of_day(23), "Good evening");
    }

    /// DISCONNECTED IS SAID OUT LOUD, and it outranks the count.
    ///
    /// The mockup's standing line talks about threads and streaming, which are
    /// facts about a server this app may not be talking to. A count rendered
    /// over a dead socket is confident wrongness — the reader has no way to
    /// tell it from a live one, and it is the sentence that decides whether
    /// they trust the rest of the screen.
    #[test]
    fn a_dead_socket_is_reported_rather_than_counted() {
        for plane in Plane::ALL {
            let line = standing(plane, false, 12);
            assert!(
                line.contains("Not connected"),
                "{plane:?} reported a count while disconnected: {line}"
            );
            assert!(
                !line.contains("12"),
                "{plane:?} printed a stale count over a dead socket: {line}"
            );
        }
    }

    /// One, and more than one, read as English rather than as a template.
    #[test]
    fn the_standing_line_counts_in_words_a_person_would_use() {
        assert!(standing(Plane::Chat, true, 1).starts_with("One conversation"));
        assert!(standing(Plane::Chat, true, 7).starts_with("7 conversations"));
        assert!(standing(Plane::Code, true, 1).starts_with("One working tree"));
        assert!(standing(Plane::Code, true, 3).starts_with("3 working trees"));
        // Zero says what to do next rather than reporting a nought.
        assert!(standing(Plane::Chat, true, 0).contains("Ask goose"));
    }

    /// The two halves keep their own vocabulary in the composer too.
    #[test]
    fn each_half_asks_for_its_own_kind_of_work() {
        assert_ne!(
            compose_placeholder(Plane::Chat),
            compose_placeholder(Plane::Code)
        );
        assert!(compose_placeholder(Plane::Code).contains("branch"));
    }

    /// The tiles count what is there, and exactly one of them can be urgent.
    ///
    /// Urgency is the reader's cue to act, so spending it on more than the
    /// blocked-agent count is spending it on nothing.
    #[test]
    fn the_code_tiles_count_what_the_context_holds() {
        let tiles = crate::testkit::with_ctx(
            |ctx| {
                let mut chats = ctx.code_chats;
                chats.set(vec![
                    opencode_client::ChatMeta {
                        id: "a".to_owned(),
                        repo: "one".to_owned(),
                        title: "a".to_owned(),
                        branch: String::new(),
                        base: String::new(),
                        status: "running".to_owned(),
                        model: None,
                        last_active: 0.0,
                    },
                    opencode_client::ChatMeta {
                        id: "b".to_owned(),
                        repo: "one".to_owned(),
                        title: "b".to_owned(),
                        branch: String::new(),
                        base: String::new(),
                        status: "stopped".to_owned(),
                        model: None,
                        last_active: 0.0,
                    },
                    opencode_client::ChatMeta {
                        id: "c".to_owned(),
                        repo: "two".to_owned(),
                        title: "c".to_owned(),
                        branch: String::new(),
                        base: String::new(),
                        status: "stopped".to_owned(),
                        model: None,
                        last_active: 0.0,
                    },
                ]);
            },
            code_tiles,
        );
        let by = |label: &str| {
            tiles
                .iter()
                .find(|t| t.label == label)
                .map(|t| t.value.clone())
                .unwrap_or_default()
        };
        assert_eq!(by("working trees"), "3");
        // "awake" and not "running now": `ChatMeta.status` is the container's
        // lifecycle and not a turn's, and `code::status_label` calls the same
        // state "idle". The tile was over-claiming — see `code_tiles`.
        assert_eq!(by("awake"), "1");
        assert_eq!(
            tiles
                .iter()
                .find(|t| t.label == "awake")
                .and_then(|t| t.sub.clone()),
            Some("2 asleep".to_owned()),
            "the tile's second line should account for the trees that are not awake"
        );
        assert_eq!(by("repos"), "2", "two trees in one repo counted twice");
        assert_eq!(by("waiting on you"), "0");
        assert_eq!(
            tiles.iter().filter(|t| t.urgent).count(),
            0,
            "nothing is blocked, so nothing should be shouting"
        );
    }

    /// And with something blocked, exactly that tile is the urgent one.
    #[test]
    fn only_the_blocked_count_is_urgent() {
        let tiles = crate::testkit::with_ctx(
            |ctx| {
                let mut perms = ctx.code_permissions;
                perms.set(vec![(
                    "chat-1".to_owned(),
                    opencode_client::CodePermission::default(),
                )]);
            },
            code_tiles,
        );
        let urgent: Vec<&str> = tiles.iter().filter(|t| t.urgent).map(|t| t.label).collect();
        assert_eq!(urgent, ["waiting on you"]);
    }

    /// THE QUOTE IS THE POINT OF THIS BLOCK, and it is what the sidebar has
    /// no room for.
    ///
    /// The section was cut once on the reasoning that the sidebar lists the
    /// same sessions. This is the assertion that reasoning could not have
    /// survived: the sidebar's row is a title, a count and an age, and the
    /// last thing SAID in the conversation appears nowhere else in the
    /// desktop shell. `last_message_snippet` is on the wire, the mock serves
    /// it, and `views/sessions.rs` has been rendering it on the phone the
    /// whole time.
    ///
    /// REPRODUCED: drop `quote` from `recent_for` and this fails; drop the
    /// whole section and `the_recent_rows_are_pressable` fails as well.
    #[test]
    fn a_recent_row_carries_what_was_last_said_in_it() {
        let rows = crate::testkit::with_ctx(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![said(
                    "s1",
                    "Certificate rotation",
                    "2026-09-02T10:00:00Z",
                    "Re-issued the certificate and restarted the listener.",
                    6,
                )]);
            },
            |ctx| recent_for(ctx, 2_000_000_000),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].quote.as_deref(),
            Some("Re-issued the certificate and restarted the listener."),
            "the row is not carrying the last message"
        );
        assert_eq!(rows[0].turns.as_deref(), Some("6 turns"));
    }

    /// Three, newest first, whatever order the server sent them in.
    ///
    /// The cap is what keeps this a shortcut rather than a second copy of the
    /// sidebar; the sort is because `session/list` makes no ordering promise
    /// and "pick up where you left off" is a claim about recency.
    #[test]
    fn the_recent_rows_are_the_three_newest() {
        let rows = crate::testkit::with_ctx(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![
                    said("old", "Old", "2026-08-01T10:00:00Z", "x", 1),
                    said("newest", "Newest", "2026-09-02T10:00:00Z", "x", 1),
                    said("mid", "Mid", "2026-09-01T10:00:00Z", "x", 1),
                    said("older", "Older", "2026-07-01T10:00:00Z", "x", 1),
                ]);
            },
            |ctx| recent_for(ctx, 2_000_000_000),
        );
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            ["newest", "mid", "old"],
            "not the three newest, in order"
        );
    }

    /// A QUESTION WAITING ON THE READER OUTRANKS AN AGENT THAT IS BUSY.
    ///
    /// The same precedence `sidebar::Mark` applies, and deliberately the same:
    /// one conversation shows a mark in two columns of one window, and the two
    /// disagreeing about what it is would be worse than either being wrong.
    /// Of the three states it is the only one that is about the READER.
    #[test]
    fn a_blocked_conversation_says_so_even_while_it_is_running() {
        let rows = crate::testkit::with_ctx(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![said("s1", "Busy", "2026-09-02T10:00:00Z", "x", 2)]);
                let mut running = ctx.running_sessions;
                running.write().insert("s1".to_owned());
                let mut perms = ctx.permission;
                perms.set(vec![goose_acp_client::PermissionRequest {
                    request_id: serde_json::Value::from(7),
                    session_id: "s1".to_owned(),
                    tool_call: goose_acp_client::ToolCallUpdate {
                        tool_call_id: "call-1".to_owned(),
                        ..goose_acp_client::ToolCallUpdate::default()
                    },
                    options: Vec::new(),
                }]);
            },
            |ctx| recent_for(ctx, 2_000_000_000),
        );
        assert_eq!(rows[0].state, RecentState::Waiting);
        assert_eq!(rows[0].state.word(), Some("needs you"));
    }

    /// An idle conversation gets no word at all.
    ///
    /// Most of them are idle. A row that labelled every finished conversation
    /// "done" would spend a colour and a word on the default case, which is
    /// what makes the two states that matter stop being visible.
    #[test]
    fn a_finished_conversation_is_not_labelled() {
        assert_eq!(RecentState::Idle.word(), None);
        assert_eq!(RecentState::Running.word(), Some("streaming"));
    }

    /// With nothing on the server there is no section, by `no_starters_means_
    /// no_section`'s rule: a heading over an empty list is a promise the app
    /// cannot keep.
    #[test]
    fn no_conversations_means_no_pick_up_section() {
        let html = crate::testkit::render(|| rsx! { Home { plane: Plane::Chat } });
        assert!(
            !html.contains("Pick up where you left off"),
            "an empty server still offered somewhere to pick up from"
        );
    }

    /// AND THE ROWS ACTUALLY GO SOMEWHERE.
    ///
    /// The sidebar shipped its rows with no `onclick` at all, and every test
    /// on them passed — they all asked what the list PAINTS. This asks what a
    /// press DOES, and it asks it of the same two-step the sidebar needed:
    /// `open_session` sets `ctx.screen` and not `ctx.tab`, and `nav::current`
    /// reads the tab first, so a row pressed from anywhere but the chat plane
    /// would set a screen the window was not looking at and do visibly
    /// nothing.
    ///
    /// REPRODUCED: delete the `(nav::primary)(...)` line and this fails on the
    /// tab; delete the `open_session` call and it fails on the session id.
    #[test]
    fn the_recent_rows_are_pressable() {
        let _guard = crate::views::press::alone();
        let mut screen = crate::views::press::Pressable::mount(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![said(
                    "s1",
                    "Certificate rotation",
                    "2026-09-02T10:00:00Z",
                    "x",
                    2,
                )]);
                // Somewhere else entirely, which is the case the two-step is
                // for and the one a chat-plane-only harness cannot see.
                let mut tab = ctx.tab;
                tab.set(crate::state::Tab::Skills);
            },
            ChatHome,
        );

        screen.press("Certificate rotation");
        screen.settle();

        assert!(
            screen.with(|ctx| (ctx.tab)() == crate::state::Tab::Home),
            "the press did not enter the chat plane, so `nav::current` kept \
             answering Skills and the row did visibly nothing"
        );
        assert_eq!(
            screen.with(|ctx| (ctx.chat)().session_id),
            Some("s1".to_owned()),
            "the press did not open the session it names"
        );
    }

    /// A description is one LINE, because a card has room for one.
    ///
    /// Recipe descriptions are free text and some run to a paragraph. Clamping
    /// a paragraph mid-sentence reads as a bug; its first line reads as a
    /// summary, which is what the author put there.
    #[test]
    fn a_starters_description_is_its_first_line() {
        assert_eq!(
            super::one_line("  Summarises overnight runs.\n\nThen files them.  "),
            Some("Summarises overnight runs.".to_owned())
        );
        assert_eq!(super::one_line("   \n  "), None);
        assert_eq!(super::one_line(""), None);
    }

    /// A `SessionInfo` with a snippet and a turn count on it, built through
    /// serde for `recipe`'s reason — `_meta` is a JSON bag and a literal here
    /// would be a second place to spell its keys.
    fn said(
        id: &str,
        title: &str,
        updated: &str,
        snippet: &str,
        turns: u64,
    ) -> goose_acp_client::SessionInfo {
        serde_json::from_value(serde_json::json!({
            "sessionId": id,
            "title": title,
            "updatedAt": updated,
            "_meta": { "messageCount": turns, "lastMessageSnippet": snippet },
        }))
        .expect("a session row this test wrote")
    }

    /// THE BOARD GROUPS BY REPO, and a question outranks a running container.
    ///
    /// The mockup's board is per-repo groups of working trees, which is the
    /// shape a fleet has: a tree belongs to a repo and you look for it there.
    /// `TreeState`'s precedence is `sidebar::Mark`'s, deliberately — one tree
    /// shows a mark in the sidebar and a word on the board, and the two
    /// disagreeing about it in one window would be worse than either being
    /// wrong.
    ///
    /// REPRODUCED: swap the `Waiting`/`Awake` arms in `code_board` and the
    /// third assertion fails; drop the grouping and the first two do.
    #[test]
    fn the_board_groups_by_repo_and_a_question_outranks_a_container() {
        let groups = crate::testkit::with_ctx(
            |ctx| {
                let mut chats = ctx.code_chats;
                chats.set(vec![
                    meta("c1", "goose-phone-app", "running", 300.0),
                    meta("c2", "goose-phone-app", "stopped", 200.0),
                    meta("c3", "personal-ai-setup", "running", 100.0),
                ]);
                // c1's container is up AND it is asking. The ask wins.
                let mut perms = ctx.code_permissions;
                perms.set(vec![(
                    "c1".to_owned(),
                    opencode_client::CodePermission {
                        title: "Run cargo clippy?".to_owned(),
                        ..opencode_client::CodePermission::default()
                    },
                )]);
            },
            |ctx| code_board(ctx, 2_000_000_000),
        );
        assert_eq!(groups.len(), 2, "two repos, two groups");
        assert_eq!(groups[0].repo, "goose-phone-app");
        assert_eq!(groups[0].trees.len(), 2);
        assert_eq!(
            groups[0].trees[0].state,
            TreeState::Waiting,
            "a tree whose container is up AND which is blocked on the reader \
             read as merely awake — the one state that is about THEM"
        );
        assert_eq!(groups[0].waiting, 1);
        assert_eq!(
            groups[0].awake, 0,
            "the waiting tree is not also counted awake"
        );
        assert_eq!(groups[1].awake, 1);
        assert_eq!(
            groups[0].trees[0].say.as_deref(),
            Some("Run cargo clippy?"),
            "the row should carry the question it is blocked on"
        );
    }

    /// A repo heading names a base only when every tree in it shares one.
    ///
    /// Two bases in one repo is a fact about the trees and not about the repo,
    /// so the heading says nothing rather than picking one of them.
    #[test]
    fn a_repo_heading_claims_a_base_only_when_they_all_share_it() {
        let same = crate::testkit::with_ctx(
            |ctx| {
                let mut chats = ctx.code_chats;
                chats.set(vec![
                    based("c1", "repo", "main"),
                    based("c2", "repo", "main"),
                ]);
            },
            |ctx| code_board(ctx, 2_000_000_000),
        );
        assert_eq!(same[0].base.as_deref(), Some("from main"));

        let mixed = crate::testkit::with_ctx(
            |ctx| {
                let mut chats = ctx.code_chats;
                chats.set(vec![
                    based("c1", "repo", "main"),
                    based("c2", "repo", "release"),
                ]);
            },
            |ctx| code_board(ctx, 2_000_000_000),
        );
        assert_eq!(
            mixed[0].base, None,
            "the heading picked one of two bases and presented it as the repo's"
        );
    }

    /// A tree the manager sent with no repo is filed rather than dropped.
    ///
    /// `ChatMeta.repo` is `serde(default)`, so an empty one is reachable, and
    /// a working tree missing from the board is worse than one filed oddly.
    #[test]
    fn a_tree_with_no_repo_still_appears() {
        let groups = crate::testkit::with_ctx(
            |ctx| {
                let mut chats = ctx.code_chats;
                chats.set(vec![meta("c1", "  ", "stopped", 1.0)]);
            },
            |ctx| code_board(ctx, 2_000_000_000),
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].repo, "no repo");
        assert_eq!(groups[0].trees.len(), 1);
    }

    /// THE LEDE SAYS NOTHING OVER A DEAD SOCKET, and never says "since".
    ///
    /// The mockup's third clause is "one still streaming since 09:14", and
    /// there is no source for the "since": `running_sessions` is a bare
    /// `HashSet<String>` with no stamp near it, and `updated_at` is when the
    /// session last CHANGED rather than when the turn began.
    ///
    /// REPRODUCED: drop the `connected` guard and the first assertion fails
    /// with a count over a socket nobody opened.
    #[test]
    fn the_lede_is_silent_when_there_is_nothing_to_report() {
        let offline = crate::testkit::with_ctx(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![bare("s1"), bare("s2")]);
            },
            |ctx| lede(ctx, false),
        );
        assert!(offline.is_none(), "the lede counted over a dead socket");

        let online = crate::testkit::with_ctx(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![bare("s1"), bare("s2")]);
                let mut settings = ctx.settings;
                settings.write().server_url = "http://tail-mini:3285".to_owned();
                let mut running = ctx.running_sessions;
                running.write().insert("s1".to_owned());
            },
            |ctx| lede(ctx, true),
        );
        let lede = online.expect("connected, so there is a lede");
        assert_eq!(lede.host.as_deref(), Some("tail-mini:3285"));
        assert!(lede.before.contains("2 conversations"), "{}", lede.before);
        assert!(lede.after.contains("1 still streaming"), "{}", lede.after);
        assert!(
            !lede.after.contains("since"),
            "the lede claimed to know when a turn started, which nothing \
             records: {}",
            lede.after
        );
    }

    /// A RUNNING JOB OUTRANKS THE SERVER'S ORDER, because it is the only fact
    /// on that row about this moment. And the row never claims a next-run
    /// time: `ScheduledJob` has no such field and this app has no timezone to
    /// turn a cron expression into one.
    #[test]
    fn the_schedule_row_prefers_the_job_that_is_running() {
        let sched = crate::testkit::with_ctx(
            |ctx| {
                let mut list = ctx.scheduler.list;
                list.write().items = vec![job("first", false), job("second", true)];
            },
            |ctx| sched_line(ctx, 2_000_000_000),
        );
        let sched = sched.expect("two live jobs, so there is a row");
        assert!(
            sched.name.to_lowercase().contains("second"),
            "the row named a queued job while another was running: {}",
            sched.name
        );
        assert_eq!(sched.more, 1, "the other live job should be counted");
        assert!(
            !sched.what.contains("next"),
            "the row claimed a next-run time, which goose does not compute and \
             this app has no timezone to derive: {}",
            sched.what
        );
    }

    /// Nothing scheduled means no row, by `no_starters_means_no_section`'s
    /// rule.
    #[test]
    fn nothing_scheduled_means_no_row() {
        let sched = crate::testkit::with_ctx(|_| {}, |ctx| sched_line(ctx, 2_000_000_000));
        assert!(sched.is_none());
    }

    /// THE FOOTNOTE COUNTS THE OTHER HALF ONLY ONCE IT HAS ANSWERED.
    ///
    /// Until this window has been to the Code half, `code_chats` is empty for
    /// want of a fetch rather than for want of trees — so "Code has 0 working
    /// trees" would be a wrong number rather than an empty one.
    ///
    /// REPRODUCED: drop the `is_connected()` gate and the first assertion
    /// fails.
    #[test]
    fn the_footnote_counts_the_other_half_only_once_it_has_answered() {
        let (first, second) = crate::testkit::with_ctx(|_| {}, footnote);
        assert!(!first.is_empty());
        assert_eq!(
            second, None,
            "the footnote reported the code half's tree count over a socket \
             nothing has dialled"
        );

        let (_, second) = crate::testkit::with_ctx(
            |ctx| {
                let mut conn = ctx.code_conn;
                conn.set(crate::state::ConnState::Connected {
                    agent: "opencode".to_owned(),
                });
                let mut chats = ctx.code_chats;
                chats.set(vec![meta("c1", "r", "stopped", 1.0)]);
            },
            footnote,
        );
        assert_eq!(second.as_deref(), Some("Code has 1 working tree."));
    }

    fn meta(id: &str, repo: &str, status: &str, last: f64) -> opencode_client::ChatMeta {
        opencode_client::ChatMeta {
            id: id.to_owned(),
            repo: repo.to_owned(),
            title: id.to_owned(),
            branch: String::new(),
            base: String::new(),
            status: status.to_owned(),
            model: None,
            last_active: last,
        }
    }

    fn based(id: &str, repo: &str, base: &str) -> opencode_client::ChatMeta {
        opencode_client::ChatMeta {
            base: base.to_owned(),
            ..meta(id, repo, "stopped", 1.0)
        }
    }

    fn bare(id: &str) -> goose_acp_client::SessionInfo {
        goose_acp_client::SessionInfo {
            session_id: id.to_owned(),
            cwd: None,
            title: Some(id.to_owned()),
            updated_at: None,
            meta: None,
        }
    }

    fn job(id: &str, running: bool) -> goose_acp_client::ScheduledJob {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "source": "/x.yaml",
            "cron": "0 30 9 * * *",
            "currentlyRunning": running,
            "paused": false,
        }))
        .expect("a scheduled job this test wrote")
    }

    /// The starters are named from the server's lists, capped, and recipes
    /// come first.
    ///
    /// A recipe is a thing to RUN and a skill is a thing the agent knows, so
    /// a recipe is the closer answer to "what could I do now". The cap is
    /// because this is a shortcut and the Library holds the full lists — a
    /// home screen that grew with the server is one that stops being scannable
    /// on the machine that has most to offer.
    #[test]
    fn the_starters_are_the_servers_own_and_recipes_lead() {
        let starters = crate::testkit::with_ctx(
            |ctx| {
                let mut recipes = ctx.recipes.list;
                recipes.write().items = (0..3).map(|i| recipe(&format!("Recipe {i}"))).collect();
                let mut skills = ctx.skills.list;
                skills.write().items = (0..3).map(|i| skill(&format!("Skill {i}"))).collect();
            },
            super::starters_for,
        );
        assert_eq!(starters.len(), 4, "the cap is not being applied");
        assert_eq!(starters[0].kind, "recipe");
        assert_eq!(starters[3].kind, "skill", "skills should follow recipes");
        assert!(starters.iter().all(|s| !s.name.trim().is_empty()));
    }

    /// A nameless entry is dropped rather than rendered as a blank card.
    #[test]
    fn an_unnamed_starter_is_not_offered() {
        let starters = crate::testkit::with_ctx(
            |ctx| {
                let mut recipes = ctx.recipes.list;
                recipes.write().items = vec![recipe("   ")];
            },
            super::starters_for,
        );
        assert!(
            starters.is_empty(),
            "a recipe with no title was offered as a blank card"
        );
    }

    /// With nothing on the server the section does not appear at all.
    ///
    /// A heading over an empty grid is the app promising something it does not
    /// have — the rule `shell::render_group` and `sidebar`'s bands both state.
    #[test]
    fn no_starters_means_no_section() {
        let html = crate::testkit::render(|| rsx! { Home { plane: Plane::Chat } });
        assert!(
            !html.contains("Ways to start"),
            "an empty server still advertised ways to start"
        );
    }

    /// Built through serde, following `recipes::tests::entry_id`: these DTOs
    /// gain fields as the protocol does, and a struct literal here would be a
    /// second place to edit every time one arrives.
    fn recipe(title: &str) -> goose_acp_client::RecipeListEntry {
        serde_json::from_value(serde_json::json!({
            "id": title,
            "recipe": { "title": title, "description": "", "instructions": "x" },
            "file_path": "/x.yaml",
            "last_modified": "2026-08-23T18:42:11+00:00",
            "schedule_cron": null,
            "slash_command": null,
        }))
        .expect("a recipe row this test wrote")
    }

    fn skill(name: &str) -> goose_acp_client::SourceEntry {
        // The same shape `skills::tests` builds, which is the shape the mock
        // server sends.
        serde_json::from_value(serde_json::json!({
            "type": "skill",
            "name": name,
            "description": "",
            "content": "",
            "path": format!("/skills/{name}"),
            "global": true,
        }))
        .expect("a skill row this test wrote")
    }

    /// The chips say what the session WILL be, from sources that exist.
    ///
    /// The "no context window" half of this test was WRONG and is corrected
    /// here: it forbade the string "context" on the strength of a comment
    /// saying `Usage` is tokens in and out. `src/state.rs:178` declares it
    /// `(tokens used, context limit)`, so the limit is exactly the mockup's
    /// "1M context" — and a test asserting a real source stays unused is worse
    /// than no test, because it makes the next reader believe the source is
    /// gone. What survives is the half that was right: money, which neither
    /// wire reports and which must never appear.
    #[test]
    fn the_composer_chips_come_from_real_sources_only() {
        let chips = crate::testkit::with_ctx(
            |ctx| {
                let mut settings = ctx.settings;
                settings.write().server_url = "https://tail-mini.ts.net:3285/acp".to_owned();
                let mut ext = ctx.extensions.list;
                ext.write().items = vec![extension("developer"), extension("memory")];
            },
            |ctx| compose_chips(ctx, Plane::Chat),
        );
        let text: Vec<&str> = chips.iter().map(|c| c.text.as_str()).collect();
        assert!(
            text.contains(&"tail-mini.ts.net:3285"),
            "the host chip is missing or still carries its scheme and path: {text:?}"
        );
        assert!(
            text.contains(&"2 extensions"),
            "extensions not counted: {text:?}"
        );
        for chip in &chips {
            assert!(
                !chip.text.contains('$'),
                "a chip is quoting money, which no endpoint reports: {}",
                chip.text
            );
        }
        assert!(
            !text.iter().any(|t| t.contains("context")),
            "a context chip appeared with no `usage_update` received — before the \
             first turn the app does not know the window, and a guessed one is \
             the wrong kind of confident: {text:?}"
        );
    }

    /// AND THE CONTEXT WINDOW IS ON THE WIRE, which this file said twice that
    /// it was not.
    ///
    /// The companion to the correction above, and the one that would catch the
    /// mistake coming back: it fails if the chip is dropped OR if it is
    /// rendered from something other than the limit. `format_tokens` is
    /// `views::chat`'s, so the composer and the transcript say a token figure
    /// the same way.
    #[test]
    fn the_context_window_is_quoted_once_the_server_has_said_what_it_is() {
        let chips = crate::testkit::with_ctx(
            |ctx| {
                let mut usage = ctx.usage;
                usage.set(Some((83_000, 1_000_000)));
            },
            |ctx| compose_chips(ctx, Plane::Chat),
        );
        let text: Vec<&str> = chips.iter().map(|c| c.text.as_str()).collect();
        assert!(
            text.contains(&"1.0M context"),
            "the context window is on the wire and the composer is not saying it: {text:?}"
        );
    }

    /// A LIMIT OF ZERO IS NOT A CONTEXT WINDOW.
    ///
    /// `Usage` is two `u64`s and nothing stops a server sending `contextLimit:
    /// 0`. Rendered, that is "0 context", which reads as a window with no room
    /// in it rather than as a server that did not say — and it is also the
    /// value `crowding` already guards against for the same reason.
    #[test]
    fn a_zero_limit_is_treated_as_no_answer_rather_than_as_no_room() {
        let chips = crate::testkit::with_ctx(
            |ctx| {
                let mut usage = ctx.usage;
                usage.set(Some((0, 0)));
            },
            |ctx| compose_chips(ctx, Plane::Chat),
        );
        assert!(
            !chips.iter().any(|c| c.text.contains("context")),
            "a zero context limit was printed as though it were a window"
        );
    }

    /// An address gets the monospace face; a word does not.
    #[test]
    fn only_the_address_is_set_in_mono() {
        let chips = crate::testkit::with_ctx(
            |ctx| {
                let mut settings = ctx.settings;
                settings.write().server_url = "http://127.0.0.1:3285".to_owned();
                let mut ext = ctx.extensions.list;
                ext.write().items = vec![extension("developer")];
            },
            |ctx| compose_chips(ctx, Plane::Chat),
        );
        for chip in &chips {
            assert_eq!(
                chip.mono,
                chip.text.contains(':'),
                "{:?} is set in the wrong face — mono is for addresses",
                chip.text
            );
        }
    }

    /// A blank or unset server contributes no chip rather than an empty one.
    #[test]
    fn an_unset_server_adds_no_chip() {
        assert_eq!(super::host_of(""), None);
        assert_eq!(super::host_of("   "), None);
        assert_eq!(super::host_of("https://"), None);
        assert_eq!(
            super::host_of("https://brain.ts.net:4300/x?y=1").as_deref(),
            Some("brain.ts.net:4300")
        );
        // No scheme at all is what a half-typed field holds.
        assert_eq!(
            super::host_of("localhost:3285").as_deref(),
            Some("localhost:3285")
        );
    }

    /// Built the way `extensions::tests::entry` builds one — the chips only
    /// count the list, so nothing here needs to be more than a real row.
    fn extension(name: &str) -> goose_acp_client::GooseExtensionEntry {
        goose_acp_client::GooseExtensionEntry {
            extension: goose_acp_client::GooseExtension::mcp(
                goose_acp_client::McpServer::Stdio(goose_acp_client::StdioMcpServer::new(
                    name,
                    "uvx",
                    Vec::new(),
                )),
                Vec::new(),
                "test",
                Vec::new(),
            ),
            enabled: true,
            config_key: Some(name.to_owned()),
            extra: serde_json::Map::new(),
        }
    }

    #[component]
    fn ChatHome() -> Element {
        rsx! { Home { plane: Plane::Chat } }
    }

    /// The screen renders its three parts.
    #[test]
    fn the_home_screen_greets_and_offers_the_composer() {
        let html = crate::testkit::render(|| rsx! { Home { plane: Plane::Chat } });
        assert!(html.contains("home-greeting"), "no greeting");
        assert!(html.contains("home-compose"), "no composer");
        assert!(
            html.contains("Ask goose anything"),
            "the composer has no placeholder saying what it is for"
        );
        assert!(
            !html.contains("home-tiles"),
            "the chat half rendered the code half's count tiles"
        );
    }

    /// The code half gets the tiles and its own words.
    #[test]
    fn the_code_home_counts_and_says_branch() {
        let html = crate::testkit::render(|| rsx! { Home { plane: Plane::Code } });
        assert!(html.contains("home-tiles"), "the code half has no tiles");
        assert!(html.contains("working trees"));
        assert!(
            html.contains("branch"),
            "the composer does not say what it does"
        );
    }

    /// TYPING AND STARTING CARRIES THE TEXT, which is the whole point of the
    /// composer being the new-session affordance.
    ///
    /// It lands in `ctx.chat_draft` — the signal that lives on the context
    /// precisely so something can fill it in before its chat exists, which is
    /// `open_session`'s own comment and the pattern `recipes::run` already
    /// uses. A composer that dropped the text on the floor would look
    /// identical until you got to the empty chat.
    #[test]
    fn what_you_type_on_the_home_screen_reaches_the_new_chat() {
        let _guard = crate::views::press::alone();
        let mut screen = Pressable::mount(|_| {}, ChatHome);

        screen.type_into("Ask goose anything", "rotate the tailscale cert");
        screen.settle();
        screen.press("Start a chat");
        screen.settle();

        assert_eq!(
            screen.with(|ctx| (ctx.chat_draft)()),
            "rotate the tailscale cert",
            "the text typed on the home screen did not reach the draft the new \
             chat reads from, so it is lost the moment the session opens"
        );
    }

    /// An empty composer starts nothing, and says so before it is pressed.
    ///
    /// Without the guard a stray press makes a session on the server — a real
    /// object, with a real id, that nobody asked for and someone now has to
    /// delete. Asserted as `disabled` in the markup rather than by pressing:
    /// a disabled button is the state a reader can SEE, and a test that
    /// pressed it would be asserting the browser honours `disabled` rather
    /// than that this screen sets it.
    #[test]
    fn an_empty_composer_offers_nothing_to_press() {
        let empty = crate::testkit::render(|| rsx! { Home { plane: Plane::Chat } });
        assert!(
            empty.contains("disabled=true"),
            "the start button is live over an empty composer, so a stray press \
             makes a session nobody asked for: {}",
            &empty[..empty.len().min(600)]
        );

        let typed = crate::testkit::render_seeded(
            |ctx| {
                let mut draft = ctx.chat_draft;
                draft.set("something".to_owned());
            },
            || rsx! { Home { plane: Plane::Chat } },
        );
        // The home composer holds its own draft, so seeding `chat_draft` must
        // NOT enable it — that signal belongs to the conversation, and a home
        // screen lit up by whatever was last typed in a chat would be lying
        // about what it is about to send.
        assert!(
            typed.contains("disabled=true"),
            "the home composer is driven by the chat's draft rather than its \
             own, so it lights up over text the reader cannot see"
        );
    }
}
