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
    /// Amber when it is a number the reader has to act on. Exactly one tile
    /// can be, and it is the one counting questions the agents are blocked on
    /// — the same fact `sidebar::Mark::Waiting` paints on a row.
    pub urgent: bool,
}

/// The Code half's counts, from `AppCtx` and nothing else.
pub(crate) fn code_tiles(ctx: &AppCtx) -> Vec<Tile> {
    let chats = (ctx.code_chats)();
    let running = chats.iter().filter(|c| c.is_running()).count();
    let waiting = (ctx.code_permissions)().len();
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
    vec![
        Tile {
            value: chats.len().to_string(),
            label: "working trees",
            urgent: false,
        },
        Tile {
            value: waiting.to_string(),
            label: "waiting on you",
            urgent: waiting > 0,
        },
        Tile {
            value: running.to_string(),
            label: "running now",
            urgent: false,
        },
        Tile {
            value: repos.to_string(),
            label: "repos",
            urgent: false,
        },
    ]
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
                h1 { class: "home-greeting", "{part_of_day(hour_now())}." }
                p { class: "home-standing", "{standing(plane, connected, count)}" }

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
                // a conversation count is already the line under the greeting,
                // and everything else the mockup tiles there is spend and
                // latency, which have no source.
                if plane == Plane::Code {
                    div { class: "home-tiles",
                        for tile in code_tiles(&ctx) {
                            div {
                                key: "{tile.label}",
                                class: if tile.urgent { "home-tile urgent" } else { "home-tile" },
                                div { class: "home-tile-value", "{tile.value}" }
                                div { class: "home-tile-label", "{tile.label}" }
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
        code_tiles, compose_chips, compose_placeholder, part_of_day, recent_for, standing, Home,
        RecentState,
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
        assert_eq!(by("running now"), "1");
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
