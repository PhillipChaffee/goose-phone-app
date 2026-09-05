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

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use goose_acp_client::ConfigOption;

use crate::icons::Icon;
use crate::nav::Plane;
use crate::state::AppCtx;
use crate::views::session_settings::{option_choices, ChoicePickerSheet};

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

// THE LEDE IS GONE, AND IT WAS THE MOCKUP'S OWN THREE SENTENCES.
//
// The owner's call, against a real server: "I don't think this text is really
// necessary. Let's remove it." Recorded as a comment rather than dropped
// silently, because what it deletes is a deliberate divergence from the
// reference — `Lede::opening` carried "Nothing on this side touches a repo."
// verbatim from `10-home-chat.html` — and #148 should hold it as a decision
// rather than as a gap somebody later closes back the other way.
//
// WHAT WENT WITH IT. The only sentence on the screen saying what the Chat half
// is FOR: the plane badge and the segmented control both say "Chat" and neither
// says what that means. And 68px of the column — a 608x44 box plus its 24px
// bottom margin, measured on the captured `desktop-chats` at 1440x860.
//
// `host_of` survives because `compose_chips` still reads it: the host is a chip
// under the composer, which is the one place on this screen it is still named.

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

// AND THE FOOTNOTE, on the same call: "this doesn't seem necessary either. We
// can just remove that." Its first sentence was the mockups' too.
//
// THREE THINGS WENT WITH IT, and the first is the one to notice. The column's
// bottom hairline lived on `.home-footnote`'s `border-top`, so the chat home now
// ends at the dashed schedule pill that `margin-top: auto` pushes to the bottom
// edge — a rule with nothing under it would be a line to nowhere, so nothing
// replaces it. The second was the only cross-plane count anywhere on this
// screen: the band's counts are per-half, the segmented control carries none,
// and the sidebar shows one plane's list at a time. The third is 49px, a 712x35
// box plus its 14px top margin.

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
    /// fold of the 860pt window `src/main.rs:108` opens once the composer and
    /// a section heading are above it. It said 820 until this line was
    /// re-derived against the window the app actually opens; the count does
    /// not move, because the mockups' three is the binding half and a taller
    /// window can only fit more.
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

/// WHAT THE "WAYS TO START" HEADING SAYS ON ITS TRAILING EDGE.
///
/// `.home-section` is a two-item row — the name of the section on one side,
/// what it holds on the other — and this one shipped with the second item
/// missing, so the two headings on the chat home were asymmetric: one carried
/// a value and one carried nothing. The mockups' own is
/// `<span class="meta">recipes &amp; skills</span>`.
///
/// DERIVED FROM THE CARDS ON SCREEN rather than pasted from the mockup, and
/// the difference shows up on a real server. `starters_for` caps at four with
/// recipes first, so a server with nine recipes and three skills draws four
/// recipe cards — and a heading reading "recipes & skills" over four recipes
/// names a kind the reader cannot see. This says what is actually in the grid
/// underneath it, which is the same rule as the count beside "Pick up where
/// you left off": a section's meta is a fact about that section.
///
/// `None` for an empty list, which the caller never asks about — the section
/// does not render at all without starters. It is `Option` anyway rather than
/// an empty string, because an empty `span` inside a `space-between` row is
/// not nothing: it is a flex item, and it would hold the heading over on the
/// left while looking like a heading that had lost its value.
pub(crate) fn starter_kinds(starters: &[Starter]) -> Option<&'static str> {
    let recipes = starters.iter().any(|s| s.kind == "recipe");
    let skills = starters.iter().any(|s| s.kind == "skill");
    match (recipes, skills) {
        (true, true) => Some("recipes & skills"),
        (true, false) => Some("recipes"),
        (false, true) => Some("skills"),
        (false, false) => None,
    }
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

/// WHAT A CHIP OPENS, for the ones that are controls rather than facts.
///
/// `assets/desktop/40-home-chat.css` states the grammar this belongs to and
/// used to state it as a permanent rule: the mockups draw two visual classes in
/// a composer bar — a bordered box with a chevron for a thing you can CHANGE
/// and bare text for a thing you can only READ — and *"every chip this app puts
/// here is the second kind"*. That paragraph named its own expiry: *"the moment
/// either becomes a picker, the bordered form has to come back."* This is that
/// moment (#79, #194), and the form comes back with it.
///
/// Three, and no fourth. The mode belongs to a turn rather than to a session
/// about to exist, the host and the extension count are genuinely read-only,
/// and a context window is a fact about a model rather than a choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pick {
    /// The chat half's model, chosen before the session exists. #194.
    Model,
    /// The code half's repo, out of the manager's allowlist. #79.
    Repo,
    /// The ref the working tree's own branch is cut from. #79.
    Base,
}

impl Pick {
    /// The control's accessible name — what `views::press` locates it by, and
    /// the only name it has once the face is a value and a chevron.
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Model => "Model",
            Self::Repo => "Repository",
            Self::Base => "Base branch",
        }
    }
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
    /// What pressing it opens, or `None` for a fact. A fact is a `span`; a
    /// control is a `button` wearing `.picker`.
    pub pick: Option<Pick>,
    /// A leading glyph, and only the controls have one — it is what the
    /// mockups' `.picker` puts in front of the value, and what tells two
    /// bordered boxes apart at a glance before either is read.
    pub icon: Option<&'static str>,
}

impl Chip {
    /// A thing you can only read.
    const fn fact(text: String, mono: bool) -> Self {
        Self {
            text,
            mono,
            pick: None,
            icon: None,
        }
    }

    /// A thing you can change.
    const fn control(text: String, pick: Pick, icon: Option<&'static str>) -> Self {
        Self {
            text,
            mono: false,
            pick: Some(pick),
            icon,
        }
    }
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
/// `picked` is what the reader chose on this screen and the server has not been
/// told about yet — the chat half's model, which cannot be written through
/// [`crate::state::set_config_option`] because that needs a session that does
/// not exist. It outranks `current_value` for exactly as long as that is true.
pub(crate) fn compose_chips(ctx: &AppCtx, plane: Plane, picked: Option<&str>) -> Vec<Chip> {
    let mut out = Vec::new();
    match plane {
        Plane::Chat => {
            let option = (ctx.config_options)()
                .iter()
                .find(|o| o.config_id == "model")
                .cloned();
            // THE MODEL IS A CONTROL NOW, and only where choosing would decide
            // something: `is_adjustable` is false for a one-value select, and
            // offering that as a menu is design rule 11's control that does
            // nothing. With no option at all — a cold launch, before any
            // session has said what the agent offers — there is no chip, which
            // is what shipped and stays true.
            let adjustable = option.as_ref().is_some_and(ConfigOption::is_adjustable);
            let label = picked
                .map(|value| model_label(value, option.as_ref()))
                .or_else(|| {
                    option
                        .as_ref()
                        .and_then(ConfigOption::current_label)
                        .map(str::to_owned)
                });
            if let Some(label) = label {
                out.push(if adjustable {
                    Chip::control(label, Pick::Model, None)
                } else {
                    Chip::fact(label, false)
                });
            }
            if let Some(host) = host_of(&ctx.settings.peek().server_url) {
                out.push(Chip::fact(host, true));
            }
            let loaded = (ctx.extensions.list)().items.len();
            if loaded > 0 {
                out.push(Chip::fact(format!("{loaded} extensions"), false));
            }
            if let Some((_, limit)) = (ctx.usage)().filter(|(_, limit)| *limit > 0) {
                out.push(Chip::fact(
                    format!("{} context", crate::views::chat::format_tokens(limit)),
                    true,
                ));
            }
        }
        Plane::Code => {
            // WHERE THE TREE GETS CUT, as two controls — #79. They lead the row
            // because they are the only two facts on it the reader decides, and
            // because the send button below cannot act on the sentence without
            // them: `views::code::can_start` wants a repo before it wants
            // anything else.
            //
            // AND ONLY WITH AN ALLOWLIST TO PICK FROM. A picker over an empty
            // manager list is a control that opens a sheet saying there is
            // nothing in it, on a screen whose standing line is already saying
            // the gateway is not connected. `N repos` was the fact these two
            // replace, and it was gated on the same count.
            let where_ = (ctx.new_where)();
            if !(ctx.code_repos)().is_empty() {
                out.push(Chip::control(
                    if where_.repo.is_empty() {
                        Pick::Repo.title().to_owned()
                    } else {
                        crate::views::code::repo_chip_label(&where_.repo).to_owned()
                    },
                    Pick::Repo,
                    Some("repo"),
                ));
                out.push(Chip::control(
                    crate::views::code::branch_chip_label(where_.base.as_deref()).to_owned(),
                    Pick::Base,
                    Some("git-branch"),
                ));
            }
            if let Some(host) = host_of(&ctx.settings.peek().code_server_url) {
                out.push(Chip::fact(host, true));
            }
        }
    }
    out
}

/// A model reference as the catalogue names it, or the reference itself.
///
/// [`ConfigOption::current_label`] does exactly this for the value the SERVER
/// holds, and cannot be asked about one it has not been told yet — which is
/// every model picked on the home screen, right up until the session exists.
fn model_label(value: &str, option: Option<&ConfigOption>) -> String {
    option
        .and_then(|o| o.options.iter().find(|c| c.value == value))
        .map_or(value, |c| c.name.as_str())
        .to_owned()
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

/// DIAL THE CODE GATEWAY ON ARRIVAL.
///
/// Its own component with nothing to render, because MOUNTING is the trigger
/// and a `use_effect` on `Home` could not be. `Home` is one component for both
/// halves, so switching plane changes a PROP rather than remounting — and a
/// `use_effect` does not re-run for a prop, only for a signal it read. Worse,
/// the first draft read `plane` before it read `code_conn` and returned early
/// on the chat half, so it subscribed to nothing at all and could never re-run
/// for any reason. Measured: the Code plane sat at "Not connected" with the
/// gateway answering, and the mock server's log had not one request in it.
///
/// `use_hook` and not `use_effect`, which is `views::code::CodeSessionsView`'s
/// own shape for the same job, down to the two guards: nothing to do if a
/// client already exists, and nothing to dial if no gateway is configured.
#[component]
fn CodeDial() -> Element {
    let ctx = crate::state::use_app_ctx();
    use_hook(|| {
        if ctx.code_client.peek().is_none()
            && !ctx.settings.peek().code_server_url.trim().is_empty()
        {
            spawn_forever(async move {
                if crate::code::code_connect(&ctx).await {
                    crate::code::start_code_poll(&ctx);
                }
            });
        }
    });
    rsx! {}
}

/// The plane's home screen.
///
/// Its own component rather than rsx inside `AppShell`, for `SidebarList`'s
/// reason: `AppShell` calls `dioxus::desktop::window()` and cannot be mounted
/// in a test, and this reads `AppCtx` and nothing else.
#[component]
pub(crate) fn Home(plane: Plane) -> Element {
    let ctx = crate::state::use_app_ctx();

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
    //
    // THE SCHEDULER IS ON THIS LIST NOW, and it was the last block on the
    // screen that was code nobody could reach. `sched_line` reads
    // `ctx.scheduler.list`, and the only writer of that signal is
    // `scheduler::ensure_loaded`, called from exactly one place in `src/` —
    // `views::scheduler`'s own mount. So the dashed row at the bottom of this
    // column appeared only once the reader had been to Library -> Scheduler in
    // that process, which is precisely the click the row exists to save. It is
    // the same defect the paragraph above records for "Ways to start", one
    // block further down the page, and it survived that fix because the fix
    // listed the signals it knew about rather than the ones this screen reads.
    //
    // THE SIGNAL IS READ BEFORE THE PROP, and that is load-bearing rather than
    // tidy. A `use_effect` subscribes to what it actually READS, and `plane`
    // is a prop: `plane == Plane::Chat && (ctx.conn)()…` short-circuits on the
    // code half, so on a window that opened there the closure subscribed to
    // NOTHING and could never run again — including after the reader switched
    // to Chat, because `Home` is one component for both halves and a prop
    // change is not a signal change. `CodeDial` below carries the same lesson
    // in its own words, measured: "it subscribed to nothing at all and could
    // never re-run for any reason". Reading `conn` unconditionally is what
    // keeps the connection arriving under an already-open screen a trigger.
    use_effect(move || {
        let connected = (ctx.conn)().is_connected();
        if plane == Plane::Chat && connected {
            crate::recipes::refresh(&ctx);
            crate::skills::ensure_loaded(&ctx);
            crate::scheduler::ensure_loaded(&ctx);
            let ctx = ctx;
            spawn(async move { crate::extensions::refresh(&ctx).await });
        }
    });

    rsx! {
        // `home-code` ON THE `main` THAT ALREADY CARRIES `home` — #76, and the
        // whole of it. Seven rules in `assets/desktop/97-home-code.css` have
        // been correct and emitted by nothing since `c5db8a0`: the code half
        // wore the chat half's geometry because it had no class of its own to
        // hang a rule on, and no CSS could fix that.
        //
        // TWO TOP-LEVEL BRANCHES, and it is the shape rather than a preference.
        // The mockup's code home is three rows — a board that scrolls, and a
        // composer welded to the bottom edge that never does — and the chat
        // home is one column with the composer THIRD. rsx has no conditional
        // wrapper, so the composer is a component (`HomeCompose`) and each half
        // says where it goes. What the two branches share is the shape this
        // file's header names: a thing you type into, and what you might pick
        // up. They never shared an order.
        main {
            class: if plane == Plane::Code { "scroll home home-code" } else { "scroll home" },
            div { class: "home-inner",
                if plane == Plane::Code {
                    // THE BOARD IS ITS OWN SCROLL REGION, which is what the
                    // dock is for: with nine trees the column measured 1133px
                    // in an 826px pane and the last working-tree row sat 307px
                    // below the fold, while the box you type into was the first
                    // thing on screen. Now the rows scroll under a composer
                    // that does not move.
                    div { class: "home-board",
                        // THE CODE HALF HAS TO DIAL ITSELF, and nothing on the
                        // desktop ever did: `views::code::CodeSessionsView` is
                        // the only thing in the app that calls `code_connect`,
                        // and this shell renders `Home` where that view would
                        // be. So the board, the sidebar's tree list and every
                        // tile read an empty `code_chats` forever, with the
                        // standing line correctly reporting a socket nobody had
                        // tried to open.
                        CodeDial {}
                        // Disconnected, the reason — and on this half the sheet
                        // draws it as the amber banner `.home-code
                        // .home-standing` has been holding for it, rather than
                        // as 22px of grey body text.
                        if !connected {
                            p { class: "home-standing", "{standing(plane, connected, count)}" }
                        }
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

                        // THE BOARD: what is actually in each repo. Every child
                        // of a row is a span — the mockup puts an "Answer"
                        // button on three of its six rows, and a button inside a
                        // button makes the parser hoist the inner one, which
                        // re-parents everything after it. That shape produced
                        // 1600 audit findings the one time it shipped here. The
                        // whole row is the target and it opens the chat, where
                        // the permission modal offers the same two answers with
                        // the ask beside them.
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
                                        // Enter the plane first —
                                        // `open_code_chat` sets `code_screen`
                                        // and not `tab`, and `nav::current`
                                        // reads the tab first. The bug
                                        // `sidebar.rs` writes up at length.
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

                    // WELDED TO THE BOTTOM EDGE, and LAST. `flex: 0 0 auto`
                    // over a board that takes the slack, which is the mockup's
                    // three-row `.main`. On the chat half the composer is third
                    // in one scrolling column; here it is a region.
                    div { class: "home-dock",
                        HomeCompose { plane }
                    }
                } else {
                    // NO GREETING ON THE CODE HALF, which is why this is inside
                    // the chat branch rather than guarded. The mockup has none
                    // — that side opens on a board, because what a working tree
                    // is doing is the question, and `hour_now` is UTC and says
                    // so.
                    h1 { class: "home-greeting", "{part_of_day(hour_now())}." }
                    // ONE SENTENCE UNDER THE GREETING, AND ONLY WHEN IT IS OWED.
                    //
                    // This was the `else` of the lede's `if let`; with the lede
                    // gone it stands on its own, and the branch it lost is the
                    // decision #200 left open. CONNECTED, BOTH HALVES SAY
                    // NOTHING HERE, which was already the code half's rule and
                    // its reason carries over: the tiles and the board are that
                    // half's standing line, and on this one the count is on the
                    // section heading three inches down — `.home-section-meta`
                    // reads "12 threads" beside "Pick up where you left off". A
                    // sentence here would be the same number said twice, which
                    // is what the owner called unnecessary. Disconnected, both
                    // halves still owe the reader the reason, and nothing else
                    // on the screen gives it.
                    if !connected {
                        p { class: "home-standing", "{standing(plane, connected, count)}" }
                    }

                    // THE COMPOSER IS THE NEW-SESSION AFFORDANCE, which is why
                    // the sidebar's New button hides while this is on screen.
                    // The owner's words: "I don't think we need a new chat
                    // button when the big chat box is visible in the middle."
                    HomeCompose { plane }

                    // PICK UP WHERE YOU LEFT OFF.
                    //
                    // See `Recent` for why this is here after being cut once.
                    // The short version: the sidebar's row and this one are not
                    // the same row, and the mockups render both.
                    if !recent.is_empty() {
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
                                    // The sidebar row's sequence, and it has to
                                    // be this order: `open_session` sets
                                    // `screen`, not `tab`, and `nav::current`
                                    // reads the tab first — so a row pressed
                                    // from any other destination would open a
                                    // session the window was not looking at.
                                    // Entering the plane first is what makes the
                                    // press land.
                                    onclick: {
                                        let id = row.id;
                                        move |_| {
                                            (crate::nav::primary(Plane::Chat).go)(&ctx);
                                            // Looked up rather than carried:
                                            // `Recent` is what the row DRAWS,
                                            // and `open_session` needs the whole
                                            // `SessionInfo` — its cwd, its kind,
                                            // the fields a summary has no
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
                    // with that are not a blank prompt, and the sidebar keeps
                    // them behind the Library disclosure — so this is the one
                    // place they are visible without a click. Named from the
                    // server's own list rather than invented.
                    if !starters.is_empty() {
                        div { class: "home-starters",
                            h2 { class: "home-section",
                                "Ways to start"
                                if let Some(kinds) = starter_kinds(&starters) {
                                    span { class: "home-section-meta", "{kinds}" }
                                }
                            }
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
                                            // The taxonomy word AND the
                                            // sentence, in that order and
                                            // separated by a middot — the
                                            // mockups' own "recipe · summarises
                                            // overnight runs". The word alone
                                            // was the whole second line, which
                                            // told the reader what the card IS
                                            // and nothing about what it would
                                            // do.
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

                    // THE BLOCK THAT CLOSES THE CHAT COLUMN, and it is one now
                    // rather than two. The mockups end with a dashed schedule
                    // row pushed to the bottom and a hairline footnote under it;
                    // the footnote is the owner's cut (#200), so the pill on the
                    // column's bottom edge is the whole ending. `margin-top:
                    // auto` in `40-home-chat.css` is what puts it there, and
                    // that only works because `.home-inner` is
                    // `min-height: 100%`.
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
                }
            }
        }
    }
}

/// THE THING YOU TYPE INTO, on either half.
///
/// Its own component because #76 moved it: the chat home renders it third, in
/// one scrolling column, and the code home renders it LAST, inside a
/// `.home-dock` that does not scroll. rsx has no conditional wrapper, so the
/// two orders are two branches, and a block that appears in both has to be a
/// component rather than a copy.
///
/// It owns the draft as well, which `Home` used to. Nothing outside the
/// composer ever read that signal, and a lifted first message has to be given
/// back to the field it was lifted from — a scope that unmounts under the
/// round trip is the one thing `state::give_back`'s `try_write` exists for.
#[component]
fn HomeCompose(plane: Plane) -> Element {
    let ctx = crate::state::use_app_ctx();
    let mut draft = use_signal(String::new);
    // THE MODEL, HELD HERE UNTIL THERE IS A SESSION TO PUT IT ON — #194.
    //
    // `state::set_config_option` needs a `session_id` and the home screen's
    // whole premise is that there is not one, so a picker that wrote through it
    // would have been a control that silently did nothing. The choice waits in
    // this signal, the chip shows it, and `state::new_session_sending` applies
    // it to the session it creates before the first word goes out.
    let model = use_signal(|| None::<String>);
    let mut sheet = use_signal(|| None::<Pick>);

    // WHERE THE TREE GETS CUT, seeded the way `CodeNewView` seeds its own
    // pills, and for its reason rather than by copying it: the chip has to be
    // able to say `goose-phone-app` and `main` before either is opened, or the
    // reader is told to choose something that already has an answer.
    //
    // A write during render, which `CodeNewView` also does and which cannot
    // loop here: `choose_new_repo` returns early when the repo is unchanged,
    // and the base is only written once, when a fetched default arrives for the
    // repo now chosen.
    if plane == Plane::Code {
        let where_ = (ctx.new_where)();
        if where_.repo.is_empty() {
            if let Some(first) = (ctx.code_repos)().first() {
                crate::code::choose_new_repo(&ctx, &first.name);
            }
        } else if where_.base.is_none() {
            let branches = (ctx.code_branches)();
            if branches.repo == where_.repo {
                if let Some(default) = branches.default {
                    crate::code::choose_new_base(&ctx, Some(default));
                }
            }
        }
    }

    // START, and on the chat half it SENDS.
    //
    // It did not, and this file argued that it could not: "`send_prompt`
    // returns false without a `session_id`, and the session's id only arrives
    // after a round trip, so 'type here and it is sent' would mean a task
    // waiting on a signal to change." The first clause is true and the
    // conclusion does not follow. The round trip is made inside
    // `state::new_session_with`'s own task, which is already holding the id
    // after `chat.set(…)` — so the send happens there, on the next line, with
    // nothing waiting on anything. What the reader got in the meantime was
    // their own sentence sitting unsent in a composer they had already pressed
    // the arrow on, which is the owner's report. `state::new_session_sending`
    // carries the full disagreement.
    //
    // AND THE DRAFT IS NOT BLANKED HERE ANY MORE. It used to be, at the end of
    // this closure, unconditionally — before anything could know whether a
    // session had been made. `new_session_with` refuses synchronously when the
    // working directory is unset or relative and asynchronously when there is
    // no client, and in both cases the text was gone: cleared here, parked in a
    // signal nothing on screen renders, and wiped by the next `open_session`.
    // The lift-and-give-back now belongs to the one function that knows which
    // of those happened.
    let mut start = move || {
        match plane {
            Plane::Chat => {
                // The composer itself, handed over: it is emptied at once so a
                // second press cannot make a second session while the create is
                // in flight, and refilled by whichever path fails. The model
                // goes with it, because it is the one thing about the session
                // that has to be decided before its first turn runs and the
                // only channel for it is this call (#194).
                crate::state::new_session_sending(&ctx, draft, model.peek().clone());
            }
            Plane::Code => {
                // `new_task`, AND IT USED TO BE `code_draft`, WHICH DESTROYED
                // THE SENTENCE. `code_draft` is the code CHAT's composer:
                // `CodeNewView` never reads it — it seeded a fresh
                // `use_signal(String::new)` from nothing — and `open_code_chat`
                // then blanked it, because its guard is true for every newly
                // created chat. So the text was written to a signal with no
                // reader and wiped, and the composer on this screen was a box
                // whose only function was to enable a button.
                //
                // Not fixed by making `CodeNewView` read `code_draft` either:
                // that would carry a half-typed correction out of one
                // conversation and into a new session pointed at a different
                // repo, which is the line `open_code_chat` and
                // `new_attachments` both already draw. `new_task` is the tray's
                // own shape for the tray's own reason — see the field.
                let mut new_task = ctx.new_task;
                new_task.set(draft.peek().trim().to_owned());
                // AND IT STILL OPENS THE NEW-SESSION SCREEN, which is #79's one
                // open decision taken and written down rather than left.
                //
                // The chips above now say WHERE — repo and base travel on
                // `ctx.new_where` and `CodeNewView` seeds from them, so the
                // thing this issue actually reported ("the destination the
                // session will land in is not visible while you type", and a
                // second screen asking again for what you already chose) is
                // gone. What the arrow does NOT do is create the tree outright,
                // and that was measured before it was decided:
                //
                //   - `views::code::can_start` wants a MODEL, and its own note
                //     says why it may not be defaulted into — "the one parameter
                //     that decides what the work costs, how good it is, and,
                //     through privacy hard rule 1, who gets to see the code". A
                //     one-step send would need a fourth picker here or a send
                //     that refuses with three pills' worth of reasons and no
                //     room to say which.
                //   - `CodeNewView` is reachable from NOWHERE else on this
                //     shell. `grep -rn 'CodeScreen::New' src` finds three
                //     writers: the sessions list's FAB and the code chat's
                //     topbar, neither of which the desktop mounts, and this
                //     line. It owns the only attach tray a new code session has
                //     and the only mode picker, so creating from here would take
                //     both off the desktop with nothing to replace them.
                //
                // So this half is two steps on purpose, and the second one now
                // opens on the answers the first gave.
                let mut screen = ctx.code_screen;
                screen.set(crate::code::CodeScreen::New);
                // Blanked here and not by a give-back, because this arm cannot
                // fail: setting a screen is not a round trip, and the sentence
                // is already on `new_task` where the next screen will take it.
                draft.set(String::new());
            }
        }
    };

    rsx! {
        div { class: "home-compose",
            textarea {
                class: "input",
                placeholder: compose_placeholder(plane),
                value: "{draft}",
                // TWO, AND NOT THREE, AND NOT ONE.
                //
                // Three made the resting composer a 148px slab with two empty
                // lines under the placeholder, against the mockups' 100px —
                // measured, `.home-compose` was 640x148 where `.launch.calm` is
                // 712x100, with the 17/18/15 field padding identical on both
                // sides and only the line count different. It is 48px of the
                // 110px by which this column ran off the bottom of the window.
                //
                // One would be the mockups' own number and cannot ship: nothing
                // in this app grows a textarea. There is no `field-sizing`, no
                // `scrollHeight` read and no resize hook anywhere in `src/`, and
                // `assets/shared.css`'s `max-height` only CAPS a field that
                // grows by this attribute. At one row a second line of a prompt
                // would scroll inside a box the height of a single line, which
                // is worse than the slab. Two is the smallest count that still
                // shows the reader the line they just wrapped.
                rows: 2,
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
                // WHAT THE SESSION WILL BE, before it exists — and two of these
                // are now controls rather than facts.
                //
                // The mockup puts six facts here; three of them have no source
                // on the wire and are absent rather than guessed. Of the ones
                // that are left, the model (#194) and the code half's repo and
                // base (#79) are things you CHANGE, and `assets/desktop/40-home-
                // chat.css` has always said what that means: a bordered box with
                // a chevron, because a chip that is pressable and drawn like the
                // read-only chips beside it is worse than either.
                div { class: "home-chips",
                    for chip in compose_chips(&ctx, plane, model().as_deref()) {
                        if let Some(pick) = chip.pick {
                            button {
                                key: "{chip.text}",
                                class: "home-chip picker",
                                // The control's only name once its face is a
                                // value: `views::press` locates by this, and a
                                // screen reader reads it.
                                title: pick.title(),
                                "aria-label": pick.title(),
                                onclick: move |_| {
                                    // Asked on the press rather than on the
                                    // render, exactly as `CodeNewView`'s branch
                                    // pill asks: the manager answers this from
                                    // GitHub with its own credential, so it
                                    // wakes no container, but it is still a
                                    // round trip nobody has asked for until the
                                    // sheet is opened. `ensure_code_branches`
                                    // returns at once when the list is in hand.
                                    if pick == Pick::Base {
                                        let repo = ctx.new_where.peek().repo.clone();
                                        crate::code::ensure_code_branches(&ctx, &repo);
                                    }
                                    sheet.set(Some(pick));
                                },
                                if let Some(icon) = chip.icon {
                                    Icon { name: icon }
                                }
                                "{chip.text}"
                                Icon { name: "chevron-down" }
                            }
                        } else {
                            span {
                                key: "{chip.text}",
                                class: if chip.mono { "home-chip mono" } else { "home-chip" },
                                "{chip.text}"
                            }
                        }
                    }
                }
                // `.send`, the circle, and NOT `.btn primary`.
                //
                // It is the same control the transcript's composer already uses
                // (`assets/shared.css`), which is the point: the thing you press
                // to send is one shape everywhere in the app, and the reader
                // meets it here first. A 131x40 rectangle reading "Start a chat"
                // was saying out loud what an arrow in a circle says by being
                // where it is — and, being `--bg-inverse` when live and
                // `--bg-tertiary` when not, it was also the reason a resting
                // home screen had no accent on it anywhere.
                button {
                    class: "send",
                    // A name of its own, so the control is findable by something
                    // other than the words inside it — which is what a screen
                    // reader needs and what `views::press` locates by. It
                    // carries the whole label now that the face is a glyph.
                    title: match plane {
                        Plane::Chat => "Start a chat",
                        Plane::Code => "Start a session",
                    },
                    "aria-label": match plane {
                        Plane::Chat => "Start a chat",
                        Plane::Code => "Start a session",
                    },
                    // AN EMPTY COMPOSER STARTS NOTHING. Without this a stray
                    // press makes a session on the server — a real object with a
                    // real id that nobody asked for and someone now has to
                    // delete.
                    disabled: draft().trim().is_empty(),
                    onclick: move |_| start(),
                    Icon { name: "arrow-up" }
                }
            }
        }
        {home_sheet(&ctx, sheet, model)}
    }
}

/// Whichever chip's sheet is open.
///
/// [`ChoicePickerSheet`] and nothing of this screen's own, which is #194's
/// instruction in its own words: *"reuse it rather than inventing a second one,
/// so the two screens cannot drift."* The repo and branch rows are
/// `views::code`'s too — the same `repo_choices` and `branch_choices` the
/// new-session screen builds, so a repo is described the same way in both
/// places and the filter that arrived for one arrives for both.
fn home_sheet(
    ctx: &AppCtx,
    mut sheet: Signal<Option<Pick>>,
    mut model: Signal<Option<String>>,
) -> Element {
    let ctx = *ctx;
    match sheet() {
        None => rsx! {},
        Some(Pick::Model) => {
            let option = (ctx.config_options)()
                .iter()
                .find(|o| o.config_id == "model")
                .cloned();
            // What this screen has settled outranks what the server holds, for
            // as long as the two can differ — which is until the session exists.
            let current = model().or_else(|| option.as_ref().and_then(|o| o.current_value.clone()));
            let choices = option.as_ref().map(option_choices).unwrap_or_default();
            rsx! {
                ChoicePickerSheet {
                    title: "Select model",
                    backend: "goose",
                    // NOT "applies from your next message", which is the default
                    // and is false here: there is no next message on this screen,
                    // there is a first one, and this is the same distinction
                    // `ChoicePickerSheet::subtitle` was added for.
                    subtitle: "the chat you start runs on this from its first message",
                    choices,
                    current,
                    // Unreachable while the chip renders only for an adjustable
                    // option, and stated anyway, by the rule `views::chat`'s mode
                    // sheet states it under: an empty picker with nothing in it
                    // and nothing to say is the one outcome a reader cannot act
                    // on.
                    empty: "This agent offers no other model.",
                    onchoose: move |value: String| {
                        model.set(Some(value));
                        sheet.set(None);
                    },
                    onclose: move |()| sheet.set(None),
                }
            }
        }
        Some(Pick::Repo) => {
            let repos = (ctx.code_repos)();
            let count = repos.len();
            rsx! {
                ChoicePickerSheet {
                    title: "Repositories ({count})",
                    backend: "code agent",
                    subtitle: "from the brain's allowlist",
                    choices: crate::views::code::repo_choices(&repos),
                    current: Some((ctx.new_where)().repo),
                    empty: "The manager's allowlist is empty — nothing to start a session on.",
                    onchoose: move |value: String| {
                        crate::code::choose_new_repo(&ctx, &value);
                        sheet.set(None);
                    },
                    onclose: move |()| sheet.set(None),
                }
            }
        }
        Some(Pick::Base) => {
            let branches = (ctx.code_branches)();
            rsx! {
                ChoicePickerSheet {
                    title: "Choose base branch",
                    backend: "code agent",
                    subtitle: "the session's own branch is cut from this one",
                    // The manager stops at 500. Said above the rows, because
                    // with a filter over a list that has been cut short,
                    // "Nothing matches" about a branch that exists is a lie the
                    // reader has no way to catch.
                    note: branches.truncated.then(|| {
                        format!(
                            "{} branches — this repo has more than the manager will \
                             read, so one that is missing here may still exist.",
                            branches.names.len(),
                        )
                    }),
                    choices: crate::views::code::branch_choices(&branches),
                    current: (ctx.new_where)().base,
                    empty: if branches.loading {
                        "Asking GitHub for this repo's branches…"
                    } else {
                        "This manager cannot list branches — the session starts on the repo's default."
                    },
                    onchoose: move |value: String| {
                        crate::code::choose_new_base(&ctx, Some(value));
                        sheet.set(None);
                    },
                    onclose: move |()| sheet.set(None),
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
        code_board, code_tiles, compose_chips, compose_placeholder, part_of_day, recent_for,
        sched_line, standing, starter_kinds, Home, RecentState, Starter, TreeState,
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
            |ctx| compose_chips(ctx, Plane::Chat, None),
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
            |ctx| compose_chips(ctx, Plane::Chat, None),
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
            |ctx| compose_chips(ctx, Plane::Chat, None),
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
            |ctx| compose_chips(ctx, Plane::Chat, None),
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

    #[component]
    fn CodeHome() -> Element {
        rsx! { Home { plane: Plane::Code } }
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

    /// A CREATE THAT WAS REFUSED LEAVES THE TEXT WHERE THE READER CAN SEE IT.
    ///
    /// This is `what_you_type_on_the_home_screen_reaches_the_new_chat`
    /// re-pointed rather than a new test, and the old assertion had to go: it
    /// read `ctx.chat_draft` and passed on a harness with no connection,
    /// because `start()` wrote the sentence there and blanked the composer
    /// whether or not a session was ever made. The name promised the
    /// destination and the body checked the envelope.
    ///
    /// The destination is asserted where a destination can be — `state`'s
    /// `the_home_composers_first_message_is_sent_with_the_session`, against a
    /// loopback goose, which is the only place a `session/prompt` really goes
    /// out. What THIS harness has is the unhappy path, and it is the one that
    /// silently destroyed what you typed: the seeded settings carry no working
    /// directory, so `new_session_sending` refuses before a frame goes out and
    /// the sentence has to come back to the field it was lifted from.
    ///
    /// REPRODUCED: put `draft.set(String::new())` back at the end of `start()`
    /// and the first assertion fails with an empty composer over a toast.
    #[test]
    fn a_refused_create_leaves_the_typed_text_on_the_home_screen() {
        let _guard = crate::views::press::alone();
        let mut screen = Pressable::mount(|_| {}, ChatHome);

        screen.type_into("Ask goose anything", "rotate the tailscale cert");
        screen.settle();
        screen.press("Start a chat");
        screen.settle();

        assert!(
            screen.markup().contains("rotate the tailscale cert"),
            "the create was refused and the composer was blanked anyway, so the \
             sentence is gone with nothing to paste and no undo: {}",
            screen.markup()
        );
        assert_eq!(
            screen.with(|ctx| (ctx.chat_draft)()),
            "",
            "the sentence was parked in the chat's draft, which nothing on this \
             screen renders and the next `open_session` wipes"
        );
    }

    /// …AND SO DOES THE CODE HALF'S, which is the arm nothing covered.
    ///
    /// The test above mounts `ChatHome` and there was no counterpart, which is
    /// how a signal write with no reader survived: `start()`'s `Plane::Code`
    /// arm wrote `ctx.code_draft`, the code CHAT's composer, which
    /// `CodeNewView` does not read and `open_code_chat` then blanks. The
    /// sentence was destroyed between two screens with nothing on either
    /// saying so.
    ///
    /// The second assertion is the half that would not have been enough on its
    /// own. `code_draft` staying empty is the cross-conversation line
    /// `open_code_chat` and `new_attachments` already draw: a correction typed
    /// in one chat has no business in a new session pointed at another repo,
    /// and pointing the home composer at that signal would have carried it
    /// there.
    ///
    /// REPRODUCED: point the Code arm back at `ctx.code_draft` and both
    /// assertions fail.
    #[test]
    fn what_you_type_on_the_code_home_screen_reaches_the_new_session() {
        let _guard = crate::views::press::alone();
        let mut screen = Pressable::mount(|_| {}, CodeHome);

        screen.type_into("Describe a change", "add a retry to the poll");
        screen.settle();
        screen.press("Start a session");
        screen.settle();

        assert_eq!(
            screen.with(|ctx| (ctx.new_task)()),
            "add a retry to the poll",
            "the text typed on the code home did not reach the carrier the \
             new-session screen seeds its field from, so it is lost the moment \
             that screen opens"
        );
        assert_eq!(
            screen.with(|ctx| (ctx.code_draft)()),
            "",
            "the code home wrote into the code CHAT's draft, which is a \
             conversation's and not a session-about-to-exist's"
        );
        assert!(
            screen.with(|ctx| (ctx.code_screen)() == crate::code::CodeScreen::New),
            "the press did not open the screen that can act on the sentence"
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

    /// THE RESTING COMPOSER IS TWO LINES TALL, and it is pinned because
    /// nothing else in the repo can see it.
    ///
    /// `rows` is the only thing that sets a textarea's height here — there is
    /// no auto-grow in this app — so this one attribute is 22px of the column,
    /// and `docs/audit.js` cannot notice it changing: the audit measures the
    /// markup in `docs/gallery-states.json`, which still holds the `rows="3"`
    /// this screen shipped with, and reports Clean either way.
    ///
    /// Measured against the captured `desktop-chats` at 1440x860 with the
    /// attribute patched to 2: `.home-compose` 712x148 -> 712x126, and
    /// `.home-inner` 815px -> 792px inside an 808px pane — which is the
    /// difference between the footnote being on the screen and being 19px
    /// below it.
    #[test]
    fn the_home_composer_rests_at_two_lines() {
        let html = crate::testkit::render(|| rsx! { Home { plane: Plane::Chat } });
        assert!(
            html.contains("rows=2"),
            "the home composer is not two rows tall. Three is 148px at rest \
             against the mockups' 100, and one cannot ship: nothing in this \
             app grows a textarea, so a second line would scroll inside a \
             box one line high. Markup: {}",
            &html[..html.len().min(600)]
        );
    }

    /// THE TWO HOME SCREENS TAKE TWO DIFFERENT MEASURES, and they may not be
    /// told apart by which file sorts first.
    ///
    /// `--measure` is declared on `.pane-main` (`80-measure.css`) and
    /// `--gutter` substitutes there, so both overrides have to be written on
    /// that element — which means both match the SAME `.pane-main` whenever a
    /// home screen is up. Written the obvious way they would be two rules at
    /// equal specificity resolved by `40-` sorting before `97-`: a cascade
    /// decided by a filename, which no gate in this repo measures and which
    /// #76 is about to make worse by putting a second class on the element
    /// `:has(.home)` already matches.
    ///
    /// So they are made mutually exclusive instead, and this is the assertion
    /// that keeps them that way. It is not a style check — delete the `:not()`
    /// and the code home silently takes the chat home's 712px column, 64px
    /// narrower than the board was derived for, with nothing failing and the
    /// audit still Clean.
    #[test]
    fn the_two_home_screens_cannot_take_each_others_measure() {
        let chat = sheet("40-home-chat.css");
        let code = sheet("97-home-code.css");

        assert!(
            chat.contains(".pane-main:has(.home):not(:has(.home-tiles)) {\n  --measure: 44.5rem;"),
            "the chat home's measure is not 44.5rem on `.pane-main`, excluding \
             the code half. 44.5rem is 712px at this shell's 16px root, which \
             is the mockups' own column, and it must be set on `.pane-main` \
             because that is where `--gutter` substitutes it"
        );
        assert!(
            code.contains(".pane-main:has(.home-tiles) {\n  --measure: 48.5rem;"),
            "the code home's measure is not 48.5rem on `.pane-main`. 48.5rem \
             is 776px, the mockups' board inside a 26px gutter"
        );
        // The needle carries the newline and the two-space indent a DECLARATION
        // has, so the write-up of this trap in the same file — which quotes the
        // dead value inside backticks — cannot satisfy the check that the value
        // is gone.
        assert!(
            !code.contains("\n  --measure: 64rem;"),
            "`.home.home-code` has its `--measure: 64rem` back. That rule is a \
             no-op wherever it is emitted: a custom property in another \
             property's value substitutes at the element that DECLARES it, and \
             `--gutter` is declared on `.pane-main`"
        );
    }

    /// THIS SCREEN ASKS FOR THE SCHEDULER, and until now it did not.
    ///
    /// The dashed row at the foot of this column reads `ctx.scheduler.list`,
    /// and the only writer of that signal was `views::scheduler`'s own mount —
    /// so the block rendered nothing until the reader had been to
    /// Library -> Scheduler in that process, which is the click it exists to
    /// save.
    ///
    /// ASSERTED ON THE CALL SITE, and that is a limitation stated rather than
    /// hidden. The fetch itself needs a live `AcpClient`, which arrives from a
    /// loopback goose in `serverkit::Harness` — and that harness mounts its own
    /// probe component rather than a caller's, so there is no way in this tree
    /// to mount `Home` in front of a server and count the request. What CAN be
    /// checked is that the mount effect names the scheduler at all, which is
    /// the thing that was missing and the thing a future edit to this list
    /// would drop. `code_of` strips the comments, so the paragraph beside the
    /// effect explaining this cannot satisfy it.
    #[test]
    fn the_chat_home_fetches_every_list_it_draws() {
        let code = crate::selfscan::code_of("home.rs", include_str!("home.rs"));
        for fetch in [
            "recipes::refresh",
            "skills::ensure_loaded",
            "scheduler::ensure_loaded",
            "extensions::refresh",
        ] {
            assert!(
                code.contains(fetch),
                "the chat home's mount effect does not call `{fetch}`, so the \
                 block that reads its signal renders only for a reader who has \
                 already been to the Library screen that fetches it"
            );
        }
    }

    /// …AND THE EFFECT ACTUALLY FIRES on a connected chat home.
    ///
    /// The source check above says the fetches are written down; this says the
    /// closure they are written in runs. Nothing observable comes back —
    /// `refresh` peeks `ctx.client`, which is `None` without a loopback goose —
    /// so what is asserted is the guard: the screen branches on the same
    /// `connected` the effect does, so a rendering that differs between the two
    /// fixtures is that boolean being read. `render_settled` rather than
    /// `render`, because an effect has not run when the first pass ends.
    ///
    /// RE-POINTED FOR #200, and it is the load-bearing half of that change.
    /// The needle was `html.contains("home-lede")` and the lede is gone, so the
    /// assertion would have passed over an effect that never ran — the only
    /// guard on four fetches, and the fix for #91.
    ///
    /// BOTH FIXTURES ARE RENDERED, which the old test did not do and which is
    /// what keeps the new needle from being vacuous. `.home-standing` is absent
    /// on a screen that rendered nothing at all, so its absence proves nothing
    /// on its own; the disconnected render is the control that says this
    /// component emits it whenever the flag is false, and the greeting is what
    /// says a chat home was drawn at all.
    #[test]
    fn a_connected_chat_home_runs_its_mount_fetch() {
        let connected = crate::testkit::render_settled(
            |ctx| {
                let mut conn = ctx.conn;
                conn.set(crate::state::ConnState::Connected {
                    agent: "goose".to_owned(),
                });
            },
            || rsx! { Home { plane: Plane::Chat } },
        );
        let offline =
            crate::testkit::render_settled(|_| {}, || rsx! { Home { plane: Plane::Chat } });

        assert!(
            offline.contains("home-standing"),
            "the disconnected home does not say why it is empty, so the \
             assertion below has no control and proves nothing: {}",
            &offline[..offline.len().min(400)]
        );
        assert!(
            connected.contains("home-greeting"),
            "the connected home rendered nothing at all, so the absence below \
             is vacuous: {}",
            &connected[..connected.len().min(400)]
        );
        assert!(
            !connected.contains("home-standing"),
            "the connected home is still showing the disconnected sentence, so \
             the effect's own guard was false and nothing in it ran: {}",
            &connected[..connected.len().min(400)]
        );
    }

    /// …AND IT SUBSCRIBES TO THE CONNECTION WHICHEVER HALF IS ON SCREEN.
    ///
    /// `use_effect` re-runs for the signals it READ. `plane` is a prop, so
    /// `plane == Plane::Chat && (ctx.conn)()…` short-circuits on the code half
    /// and subscribes to nothing at all — the closure can then never run again
    /// for any reason, including the reader switching to Chat, because `Home`
    /// is one component for both halves and a prop change is not a signal
    /// change. `CodeDial` two hundred lines up carries the same lesson, and it
    /// was measured there: the Code plane sat at "Not connected" with the
    /// gateway answering and not one request in the server's log.
    #[test]
    fn the_mount_fetch_reads_the_connection_before_it_reads_the_prop() {
        let code = crate::selfscan::code_of("home.rs", include_str!("home.rs"));
        let effect = code
            .split("use_effect(move || {")
            .nth(1)
            .expect("the chat home still has a mount effect");
        let conn = effect
            .find("ctx.conn")
            .expect("the effect still reads conn");
        let prop = effect
            .find("plane == Plane::Chat")
            .expect("the effect is still scoped to the chat half");
        assert!(
            conn < prop,
            "the mount effect tests the `plane` prop before it reads `conn`, \
             so on the code half it short-circuits, subscribes to no signal at \
             all and can never re-run — including after the reader switches to \
             Chat"
        );
    }

    /// A starter of one kind, for the heading's meta.
    fn starter_of(kind: &'static str) -> Starter {
        Starter {
            name: "whatever".to_owned(),
            description: None,
            kind,
            icon: "book",
            tab: crate::state::Tab::Recipes,
        }
    }

    /// THE HEADING NAMES WHAT IS UNDER IT, and only that.
    ///
    /// The mockup's meta is the literal string "recipes & skills", and pasting
    /// it would be wrong on a real server: `starters_for` caps at four with
    /// recipes first, so nine recipes and three skills draw four recipe cards
    /// under a heading naming a kind that is not on the screen.
    #[test]
    fn the_ways_to_start_heading_says_which_kinds_are_in_it() {
        assert_eq!(
            starter_kinds(&[starter_of("recipe"), starter_of("skill")]),
            Some("recipes & skills")
        );
        assert_eq!(
            starter_kinds(&[starter_of("recipe")]),
            Some("recipes"),
            "a grid of recipes is headed as though it held skills too"
        );
        assert_eq!(
            starter_kinds(&[starter_of("skill")]),
            Some("skills"),
            "a grid of skills is headed as though it held recipes too"
        );
        assert_eq!(
            starter_kinds(&[]),
            None,
            "an empty grid still writes a meta, which in a `space-between` row \
             is a flex item — a heading holding itself over to the left with a \
             value that is not there"
        );
    }

    /// THE HEADING RENDERS ITS META, which is the half a unit test cannot see.
    ///
    /// `starter_kinds` being right is not the same as the `h2` carrying it:
    /// this section shipped for months with `h2 { "Ways to start" }` and no
    /// child at all, beside a "Pick up where you left off" that had one.
    #[test]
    fn the_ways_to_start_heading_carries_its_meta_into_the_markup() {
        let html = crate::testkit::render_seeded(
            |ctx| {
                let mut list = ctx.recipes.list;
                list.write().items = vec![recipe("Morning brief")];
            },
            || rsx! { Home { plane: Plane::Chat } },
        );
        assert!(
            html.contains("Ways to start"),
            "the starter section did not render at all, so what follows proves \
             nothing: {}",
            &html[..html.len().min(400)]
        );
        assert!(
            html.contains(r#"<span class="home-section-meta">recipes</span>"#),
            "the \"Ways to start\" heading has no meta, so the two section \
             headings on this screen are asymmetric — one carries a value and \
             one carries nothing: {html}"
        );
    }

    /// One allowlisted repo, as the manager sends it.
    fn allowed(name: &str) -> opencode_client::RepoEntry {
        opencode_client::RepoEntry {
            name: name.to_owned(),
            url: String::new(),
            edit_only: false,
            allow_push: false,
            public_throwaway: false,
        }
    }

    /// One goose config option with `values` to choose between.
    fn option(id: &str, current: &str, values: &[(&str, &str)]) -> goose_acp_client::ConfigOption {
        serde_json::from_value(serde_json::json!({
            "configId": id,
            "name": id,
            "type": "select",
            "currentValue": current,
            "options": values
                .iter()
                .map(|(value, name)| serde_json::json!({"value": value, "name": name}))
                .collect::<Vec<_>>(),
        }))
        .expect("a config option this test wrote")
    }

    /// THE CODE HOME HAS A SHELL OF ITS OWN, AND THE COMPOSER IS UNDER THE
    /// BOARD — #76, which is the whole reason this wave exists.
    ///
    /// Seven rules in `assets/desktop/97-home-code.css` were correct and
    /// emitted by nothing from `c5db8a0` until now, because `home.rs` rendered
    /// one `main { class: "scroll home" }` for both halves: the code half wore
    /// the chat half's geometry and no CSS could fix that, since there was no
    /// class on the element to hang a rule on.
    ///
    /// ORDER IS HALF THE ASSERTION and it is the half a `contains` check would
    /// miss. `.home-dock` is `flex: 0 0 auto` under a `.home-board` that takes
    /// the slack, so the dock is only welded to the bottom edge if it comes
    /// LAST. Emitted first — which is where the composer was — it is a region
    /// pinned to the top with the board under it, which is the shipped layout
    /// wearing new class names.
    ///
    /// REPRODUCED: swap the two `div`s in the `Plane::Code` branch and the
    /// third assertion fails; drop the `home-code` from the `main`'s class and
    /// the first does.
    #[test]
    fn the_code_home_docks_its_composer_under_its_own_board() {
        let html = crate::testkit::render(|| rsx! { Home { plane: Plane::Code } });
        for needle in ["home-board", "home-dock", "home-tiles", "home-compose"] {
            assert!(
                html.contains(needle),
                "the code home renders no `{needle}`, so the ordering \
                 assertions below would be about nothing: {html}"
            );
        }
        // Safe against the loop above, which is why it runs first: a needle
        // that is not there fails there rather than comparing two sentinels.
        let at = |needle: &str| html.find(needle).unwrap_or(usize::MAX);
        assert!(
            html.contains("scroll home home-code"),
            "the code half's `main` does not carry `home-code`, so every rule \
             in `97-home-code.css` that scopes to it paints nothing: {html}"
        );
        assert!(
            at("home-board") < at("home-tiles"),
            "the counts are outside the board, so they do not scroll with the \
             rows they count"
        );
        assert!(
            at("home-board") < at("home-dock"),
            "the dock is emitted before the board, so `flex: 0 0 auto` pins the \
             composer to the TOP of the pane and the board scrolls under it — \
             which is the layout this issue is about, wearing new names"
        );
        assert!(
            at("home-dock") < at("home-compose"),
            "the composer is not inside the dock"
        );
    }

    /// AND THE CHAT HOME WEARS NONE OF IT.
    ///
    /// The two screens share a shape and not an order — the chat column is one
    /// scroller with the composer third, and the code half is a board over a
    /// dock. A `home-board` on the chat half would put `overflow-y: auto` round
    /// a greeting.
    #[test]
    fn the_chat_home_is_one_column_and_says_so() {
        let html = crate::testkit::render(|| rsx! { Home { plane: Plane::Chat } });
        for class in ["home-code", "home-board", "home-dock"] {
            assert!(
                !html.contains(class),
                "the chat home rendered the code half's `{class}`, so it takes \
                 that half's geometry: {html}"
            );
        }
        assert!(
            html.contains("home-compose"),
            "no composer on the chat home"
        );
    }

    /// THE CODE COMPOSER NAMES WHERE THE TREE GETS CUT — #79.
    ///
    /// Two bare facts became two controls: the repo the session runs on and
    /// the ref its own branch is cut from. The `N repos` chip they replace was
    /// a count of a list the reader could not open — it said how many choices
    /// there were and offered none of them.
    ///
    /// `render_settled` rather than `render`, because the first pass is where
    /// the composer writes the default repo into the carrier: rendered once,
    /// the chip still reads `Repository`.
    #[test]
    fn the_code_composer_offers_the_repo_and_the_base_rather_than_counting_them() {
        let html = crate::testkit::render_settled(
            |ctx| {
                let mut repos = ctx.code_repos;
                repos.set(vec![allowed("acme/infra"), allowed("acme/web")]);
            },
            || rsx! { Home { plane: Plane::Code } },
        );
        assert!(
            html.contains("home-chip picker"),
            "the composer's chips are all still facts, so nothing on this \
             screen says where the session will be cut: {html}"
        );
        assert!(
            html.contains(r#"aria-label="Repository""#)
                && html.contains(r#"aria-label="Base branch""#),
            "one of the two controls #79 asks for is missing: {html}"
        );
        assert!(
            html.contains("infra"),
            "the repo control does not name the repo it is pointed at, so the \
             destination is still invisible while you type: {html}"
        );
        assert!(
            !html.contains("2 repos"),
            "the count chip the two controls replace is still on the row, so \
             the same fact is said twice and one of the two is not pressable"
        );
    }

    /// WITH NO ALLOWLIST THERE ARE NO CONTROLS, only the reason.
    ///
    /// A picker over an empty manager list opens a sheet saying there is
    /// nothing in it, on a screen whose standing line is already saying the
    /// gateway is not connected. `N repos` was gated on the same count.
    #[test]
    fn an_empty_allowlist_offers_nothing_to_pick() {
        let html = crate::testkit::render_settled(|_| {}, || rsx! { Home { plane: Plane::Code } });
        assert!(
            !html.contains("picker"),
            "the code home offered a repo picker with no repos to pick: {html}"
        );
        assert!(
            html.contains("home-standing"),
            "and it did not say why either: {html}"
        );
    }

    /// WHAT YOU PICK ON THE HOME SCREEN IS WHERE THE NEW SESSION OPENS — the
    /// half of #79 that makes the two-step honest.
    ///
    /// The issue's own objection to a picker that still routes onward is that
    /// it "leaves two places to choose a repo and ignores the one you used".
    /// `ctx.new_where` is what stops that being true, and this is the
    /// assertion on it: `views::code::CodeNewView` seeds from the carrier and
    /// only falls back to the first allowlisted repo when it is empty.
    ///
    /// The needle is `class="choice"` WITH its closing quote, which is the
    /// only way to name the second row: the first is `class="choice selected"`
    /// because it is the repo the composer defaulted to, and a press on that
    /// would assert nothing.
    ///
    /// REPRODUCED: point the sheet's `onchoose` at a local signal instead of
    /// `choose_new_repo` and the first assertion fails with `acme/infra`.
    #[test]
    fn the_repo_picked_on_the_home_screen_is_the_one_carried_forward() {
        let _guard = crate::views::press::alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut repos = ctx.code_repos;
                repos.set(vec![allowed("acme/infra"), allowed("acme/web")]);
            },
            CodeHome,
        );
        screen.settle();

        screen.press("Repository");
        screen.settle();
        assert!(
            screen.markup().contains("Repositories (2)"),
            "the repo control did not open a picker: {}",
            screen.markup()
        );

        screen.press(r#"class="choice""#);
        screen.settle();

        assert_eq!(
            screen.with(|ctx| (ctx.new_where)().repo),
            "acme/web",
            "the repo chosen on the home screen was not written to the carrier \
             the new-session screen seeds from, so that screen will open on the \
             first repo of the allowlist and ask again"
        );
        assert!(
            !screen.markup().contains("Repositories (2)"),
            "the sheet stayed open over the choice it had just taken"
        );
        assert!(
            screen.markup().contains("web"),
            "the chip still names the old repo: {}",
            screen.markup()
        );
    }

    /// CHANGING THE REPO DROPS THE BASE WITH IT.
    ///
    /// `views::code::choose_repo` draws the same line for the same reason: a
    /// branch of the repo you just left does not exist on the one you just
    /// chose, and carrying the name over would put a base on the wire that the
    /// manager has to refuse.
    #[test]
    fn choosing_another_repo_does_not_keep_the_old_ones_branch() {
        let (repo, base) = crate::testkit::with_ctx(
            |ctx| {
                crate::code::choose_new_repo(ctx, "acme/infra");
                crate::code::choose_new_base(ctx, Some("release-4".to_owned()));
                crate::code::choose_new_repo(ctx, "acme/web");
            },
            |ctx| {
                let held = (ctx.new_where)();
                (held.repo, held.base)
            },
        );
        assert_eq!(repo, "acme/web");
        assert_eq!(
            base, None,
            "the base branch of the repo the reader just left is still selected"
        );
    }

    /// THE MODEL IS A CONTROL ON THE HOME COMPOSER — #194, reported by the
    /// owner against a real server: "i can't actually change the model or
    /// anything else from this thing."
    ///
    /// And it is drawn as one. `assets/desktop/40-home-chat.css` states the
    /// rule this satisfies: a chip that is clickable but drawn like the
    /// read-only chips beside it is worse than either, because nothing on the
    /// row says what responds.
    #[test]
    fn the_model_can_be_changed_before_the_conversation_exists() {
        let html = crate::testkit::render_settled(
            |ctx| {
                let mut config = ctx.config_options;
                config.set(vec![option(
                    "model",
                    "gpt-5.2",
                    &[("gpt-5.2", "GPT-5.2"), ("claude-opus-5", "Claude Opus 5")],
                )]);
            },
            || rsx! { Home { plane: Plane::Chat } },
        );
        assert!(
            html.contains(r#"aria-label="Model""#) && html.contains("home-chip picker"),
            "the model is still a read-only chip on the one screen where \
             choosing one is the natural thing to do: {html}"
        );
        assert!(html.contains("GPT-5.2"), "the chip does not name the model");
    }

    /// A SETTING WITH ONE VALUE IS A FACT, not a menu.
    ///
    /// `ConfigOption::is_adjustable` is the same guard `views::chat`'s mode
    /// chip applies, for design rule 11's reason: offering a list of one is a
    /// control that decides nothing.
    #[test]
    fn a_model_with_nothing_to_choose_between_stays_bare() {
        let chips = crate::testkit::with_ctx(
            |ctx| {
                let mut config = ctx.config_options;
                config.set(vec![option("model", "gpt-5.2", &[("gpt-5.2", "GPT-5.2")])]);
            },
            |ctx| compose_chips(ctx, Plane::Chat, None),
        );
        assert_eq!(chips.len(), 1);
        assert_eq!(chips[0].text, "GPT-5.2");
        assert!(
            chips[0].pick.is_none(),
            "a one-value select was offered as a picker"
        );
    }

    /// AND THE CHIP SAYS WHAT WAS JUST PICKED, not what the server still holds.
    ///
    /// The two can differ for exactly as long as there is no session to write
    /// the choice to — which is the whole of the home screen. A chip that went
    /// on naming `currentValue` after a pick would be a control the reader
    /// pressed with nothing visibly happening, which is the defect #194 is
    /// about arriving one layer down.
    #[test]
    fn the_chip_names_the_model_the_reader_just_chose() {
        let chips = crate::testkit::with_ctx(
            |ctx| {
                let mut config = ctx.config_options;
                config.set(vec![option(
                    "model",
                    "gpt-5.2",
                    &[("gpt-5.2", "GPT-5.2"), ("claude-opus-5", "Claude Opus 5")],
                )]);
            },
            |ctx| compose_chips(ctx, Plane::Chat, Some("claude-opus-5")),
        );
        assert_eq!(
            chips[0].text, "Claude Opus 5",
            "the chip is still reporting the server's model over the one the \
             reader chose, and the catalogue's own name is what it should say"
        );
    }

    /// A REFERENCE THE CATALOGUE DOES NOT CARRY IS PRINTED AS IT IS.
    ///
    /// `current_label` falls back the same way for the server's own value.
    /// Empty is not a special case: nothing offers it, and inventing a word
    /// for it here would be a name for a model that has none.
    #[test]
    fn a_model_the_server_has_not_described_is_named_by_its_reference() {
        let chips = crate::testkit::with_ctx(
            |ctx| {
                let mut config = ctx.config_options;
                config.set(vec![option(
                    "model",
                    "gpt-5.2",
                    &[("gpt-5.2", "GPT-5.2"), ("claude-opus-5", "Claude Opus 5")],
                )]);
            },
            |ctx| compose_chips(ctx, Plane::Chat, Some("qwen3-coder-480b")),
        );
        assert_eq!(chips[0].text, "qwen3-coder-480b");
    }

    /// BEFORE ANYTHING IS CHOSEN THE CONTROL ASKS FOR THE THING IT SETS.
    ///
    /// The one render where the carrier is still empty — the composer seeds it
    /// on its first pass — and the one place a chip in this app may name its
    /// own control rather than a value. `views::code`'s model pill draws the
    /// same distinction in the same words.
    ///
    /// The host is on this row too, and it is the one fact the code half keeps:
    /// which gateway the tree will be built on is not something this screen can
    /// change, and it is the only thing left saying which brain is answering.
    #[test]
    fn the_code_controls_name_themselves_until_they_have_a_value() {
        let chips = crate::testkit::with_ctx(
            |ctx| {
                let mut repos = ctx.code_repos;
                repos.set(vec![allowed("acme/infra")]);
                let mut settings = ctx.settings;
                settings.write().code_server_url = "http://brain.ts.net:4399".to_owned();
            },
            |ctx| compose_chips(ctx, Plane::Code, None),
        );
        let text: Vec<&str> = chips.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(text, ["Repository", "Default", "brain.ts.net:4399"]);
        assert!(
            chips[2].mono && chips[2].pick.is_none(),
            "the gateway is not something this screen can change, and an \
             address is compared character by character"
        );
    }

    /// THE MODEL PICKER OPENS AND WHAT IT PICKS STICKS — #194 end to end.
    ///
    /// The sheet is `ChoicePickerSheet`, which is `views::chat`'s and
    /// `views::code`'s, so this asserts the wiring rather than the widget: that
    /// the chip opens it, that it is built from the agent's own catalogue, and
    /// that choosing writes somewhere the send button will read.
    ///
    /// The needle is `class="choice"` WITH its closing quote, which is the only
    /// way to name a row that is not the current one: the model the server
    /// holds renders as `class="choice selected"`.
    #[test]
    fn the_model_picked_on_the_home_screen_is_the_one_the_chip_then_names() {
        let _guard = crate::views::press::alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut config = ctx.config_options;
                config.set(vec![option(
                    "model",
                    "gpt-5.2",
                    &[("gpt-5.2", "GPT-5.2"), ("claude-opus-5", "Claude Opus 5")],
                )]);
            },
            ChatHome,
        );
        screen.settle();

        screen.press("Model");
        screen.settle();
        assert!(
            screen.markup().contains("Select model") && screen.markup().contains("Claude Opus 5"),
            "the model chip did not open a picker over the agent's own \
             catalogue: {}",
            screen.markup()
        );

        screen.press(r#"class="choice""#);
        screen.settle();
        assert!(
            !screen.markup().contains("Select model"),
            "the sheet stayed open over the choice it had just taken"
        );
        assert!(
            screen
                .markup()
                .contains(r#"aria-label="Model">Claude Opus 5"#),
            "the chip is still naming the model the server holds after a \
             choice was made, so the control looks like it did nothing: {}",
            screen.markup()
        );
    }

    /// AND SO DOES THE BASE BRANCH — the second half of #79.
    ///
    /// The sheet is `views::code`'s own: the same `branch_choices`, the same
    /// `Default` marking, and the same note about a list the manager cut short
    /// at 500. With a filter over a truncated list, "Nothing matches" about a
    /// branch that exists is a lie the reader has no way to catch, so it is
    /// said above the rows on both screens or on neither.
    #[test]
    fn the_base_branch_can_be_changed_from_the_home_composer() {
        let _guard = crate::views::press::alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut repos = ctx.code_repos;
                repos.set(vec![allowed("acme/infra")]);
                let mut branches = ctx.code_branches;
                branches.set(crate::code::BranchList {
                    repo: "acme/infra".to_owned(),
                    default: Some("main".to_owned()),
                    names: vec!["main".to_owned(), "release-4".to_owned()],
                    truncated: true,
                    loading: false,
                });
            },
            CodeHome,
        );
        screen.settle();
        assert!(
            screen.markup().contains(r#"aria-label="Base branch">"#),
            "no base control: {}",
            screen.markup()
        );

        screen.press("Base branch");
        screen.settle();
        assert!(
            screen.markup().contains("Choose base branch") && screen.markup().contains("release-4"),
            "the base chip did not open the repo's branch list: {}",
            screen.markup()
        );
        assert!(
            screen.markup().contains("more than the manager will"),
            "a list the manager cut short is being presented as complete, so a \
             filter over it would report a branch that exists as missing: {}",
            screen.markup()
        );

        screen.press(r#"class="choice""#);
        screen.settle();
        assert_eq!(
            screen.with(|ctx| (ctx.new_where)().base),
            Some("release-4".to_owned()),
            "the base chosen on the home screen was not written to the carrier \
             the new-session screen seeds from"
        );
        assert!(
            !screen.markup().contains("Choose base branch"),
            "the sheet stayed open over the choice it had just taken"
        );
    }

    /// One region file of `assets/desktop/`, by name.
    ///
    /// Through `SHELL_PARTS` rather than `include_str!` here, so a file this
    /// lane edits but `src/css.rs` has stopped concatenating cannot pass: the
    /// sheet that ships is the one the assertions above are about.
    fn sheet(name: &str) -> &'static str {
        crate::css::SHELL_PARTS
            .iter()
            .find(|&&(file, _)| file == name)
            .map(|&(_, body)| body)
            .expect("no such region file in `SHELL_PARTS`")
    }
}
