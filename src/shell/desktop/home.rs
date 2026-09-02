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
//! round-trip p50/p99, server memory, session budget, context-window
//! percentage. `Usage` is `(u64, u64)` and `crates/opencode-client` has no
//! endpoint for any of it. The owner's instruction was to ship only what is
//! real, so a row with no source is ABSENT rather than zeroed: a tile reading
//! `$0.00 / $4.00` is not honest emptiness, it is a wrong number.
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

/// One thing the reader could start with that is not a blank prompt.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Starter {
    pub name: String,
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
            kind: "recipe",
            icon: "book",
            tab: crate::state::Tab::Recipes,
        })
        .collect();
    out.extend((ctx.skills.list)().items.iter().map(|skill| Starter {
        name: skill.name.clone(),
        kind: "skill",
        icon: "sparkle",
        tab: crate::state::Tab::Skills,
    }));
    out.retain(|s| !s.name.trim().is_empty());
    out.truncate(MOST);
    out
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
                        button {
                            class: "btn primary",
                            // A name of its own, so the control is findable by
                            // something other than the words inside it — which
                            // is what a screen reader needs and what
                            // `views::press` locates by.
                            title: match plane {
                                Plane::Chat => "Start a chat",
                                Plane::Code => "Start a session",
                            },
                            // AN EMPTY COMPOSER STARTS NOTHING. Without this a
                            // stray press makes a session on the server — a
                            // real object with a real id that nobody asked for
                            // and someone now has to delete.
                            disabled: draft().trim().is_empty(),
                            onclick: move |_| start(),
                            Icon { name: "plus" }
                            {
                                match plane {
                                    Plane::Chat => "Start a chat",
                                    Plane::Code => "Start a session",
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
                //
                // The mockup also has "Pick up where you left off", listing
                // recent conversations. That is deliberately NOT here: the
                // sidebar is showing exactly that list, permanently, three
                // inches to the left. The mockup's sidebar was drawn before
                // the list moved into it.
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
                                        span { class: "home-starter-kind", "{starter.kind}" }
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
    use super::{code_tiles, compose_placeholder, part_of_day, standing, Home};
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
