//! Skills: the list, the one that is open, and the calls that fill them.
//!
//! Read-only by construction. goose has no `skills/*` namespace — a skill is
//! a *source*, and `sources/list` is the only method this screen touches.
//! Everything that writes one is deliberately absent, for the reasons
//! `goose_acp_client::goose::skills` sets out.
//!
//! # Payload discipline
//!
//! `sources/list` returns every skill's **entire `SKILL.md` inline**. Twenty
//! skills at 4KB each is ~80KB per refresh, over a tailnet that is often a
//! phone on cellular. Three rules follow, and they are why this module has no
//! polling and no cache:
//!
//!   - **Fetched once**, on the first visit that finds the list empty. A
//!     revisit re-renders what is already in memory.
//!   - **Refreshed only by the pull gesture.** There is no refresh button and
//!     no timer; skills are files on disk that change when someone edits
//!     them, which is not something the phone should be asking about every
//!     thirty seconds.
//!   - **Never written to disk.** The Code tab caches transcripts because a
//!     transcript is immutable once written; a skill is not, and a cached
//!     `SKILL.md` would go stale silently — the phone would show yesterday's
//!     instructions with no way to know it, which is worse than showing
//!     nothing.

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use goose_acp_client::SourceEntry;

use crate::state::{load_remote, show_toast, AppCtx, Remote, Screen as HomeScreen, Tab};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    List,
    Detail,
}

/// This feature's whole state, held as one field of [`AppCtx`].
#[derive(Clone, Copy)]
pub(crate) struct Ctx {
    pub screen: Signal<Screen>,
    pub list: Signal<Remote<SourceEntry>>,
    /// The skill the detail screen is showing. A clone rather than an index:
    /// a refresh that lands while the detail is open reorders the list under
    /// it, and an index would then be pointing at a different skill than the
    /// one whose name is in the title. That is the phone's pull, and on the
    /// desktop it is ⌘R and arriving here — where the list is on screen BESIDE
    /// the detail, so the reorder is watched rather than merely survived.
    pub open: Signal<Option<SourceEntry>>,
}

pub(crate) fn use_ctx() -> Ctx {
    Ctx {
        screen: use_signal(|| Screen::List),
        list: use_signal(Remote::new),
        open: use_signal(|| None),
    }
}

/// The dump key for each of this destination's screens.
///
/// A free function over the plain enum, so the mapping can be tested without
/// a Dioxus runtime — the same arrangement `nav::code_key` has, kept here so
/// that adding this feature is one line in `nav.rs` rather than three.
pub(crate) const fn dump_key(screen: Screen) -> &'static str {
    match screen {
        Screen::List => "skills",
        Screen::Detail => "skill",
    }
}

/// The `projectDir` a list call should carry, or `None` when Settings has no
/// working directory worth sending.
///
/// Absolute, because it is a path on the *server*: a relative one would be
/// resolved against whatever directory goose happens to be running in, which
/// is a different project than the user meant — the same check
/// `state::new_session_with` makes before creating a session.
///
/// `None` is not a failure. goose's `discover_skills` walks the global skill
/// directories whatever it is given (`crates/goose/src/skills/mod.rs`), so a
/// call with no `projectDir` still returns every skill under
/// `~/.agents/skills` — only the *project's* skills are missing, which is
/// what the screen's hint says.
pub(crate) fn project_dir(working_dir: &str) -> Option<&str> {
    let trimmed = working_dir.trim();
    trimmed.starts_with('/').then_some(trimmed)
}

/// Load the list if nothing is in it yet. The first visit's fetch.
pub(crate) fn ensure_loaded(ctx: &AppCtx) {
    let list = ctx.skills.list;
    let idle = {
        let remote = list.peek();
        remote.items.is_empty() && !remote.loading
    };
    if idle {
        refresh(ctx);
    }
}

/// Fetch the list again. The pull gesture, and nothing else.
pub(crate) fn refresh(ctx: &AppCtx) {
    let ctx = *ctx;
    spawn_forever(async move {
        let Some(client) = ctx.client.peek().clone() else {
            // Not an error to report: the screen already says it is offline,
            // and a toast on top of that sentence says it twice.
            return;
        };
        let working_dir = ctx.settings.peek().working_dir.clone();
        load_remote(&ctx, ctx.skills.list, async move {
            // `includeProjectSources: false` on purpose: `true` makes goose
            // walk every project in its registry, so asking for it would pull
            // down the full text of skills belonging to projects that are not
            // the one the phone is pointed at.
            let (entries, partial) = client.skills_list(project_dir(&working_dir), false).await?;
            if let Some(error) = partial {
                // The list arrived, so this is a gap in it and not a failure
                // of it — a toast over something readable, per `Remote`'s
                // sticky-versus-toast rule.
                show_toast(&ctx, format!("Some skills are missing: {error}"));
            }
            Ok(entries)
        })
        .await;
    });
}

/// Push the detail screen for one skill.
pub(crate) fn open(ctx: &AppCtx, entry: SourceEntry) {
    let (mut open, mut screen) = (ctx.skills.open, ctx.skills.screen);
    open.set(Some(entry));
    screen.set(Screen::Detail);
}

/// Back to the list. The open skill is dropped with it — it is the largest
/// thing this feature holds, and nothing off-screen needs a copy of it.
pub(crate) fn close(ctx: &AppCtx) {
    let (mut open, mut screen) = (ctx.skills.open, ctx.skills.screen);
    screen.set(Screen::List);
    open.set(None);
}

/// What the phone is here to do that goose Desktop's skill browser does not:
/// turn a skill into a message.
///
/// The draft is filled rather than sent. What the skill needs — which file,
/// which environment, which branch — is the part only the user knows, so the
/// composer opens with the invocation already typed and the cursor where the
/// user's half goes.
pub(crate) fn use_skill(ctx: &AppCtx, name: &str) {
    let (mut draft, mut tab, mut screen) = (ctx.chat_draft, ctx.tab, ctx.screen);
    draft.set(draft_for(name));
    tab.set(Tab::Home);
    if ctx.chat.peek().session_id.is_some() {
        // A chat is already open: this belongs in it, rather than in a new
        // session that has none of the context the user just built up.
        screen.set(HomeScreen::Chat);
        return;
    }
    // `new_session` navigates on success and toasts on failure, and the Chats
    // list is where a failure should leave you — not on a detail screen whose
    // button apparently did nothing.
    screen.set(HomeScreen::Sessions);
    crate::state::new_session(ctx);
}

/// The line the composer opens with.
///
/// A sentence rather than a bare name: goose loads a skill when the prompt
/// asks for it by name, and the trailing space and colon are the invitation
/// to finish the thought.
fn draft_for(name: &str) -> String {
    format!("Use the {name} skill: ")
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test scaffolding: a fixture that will not parse, or a call the \
              assertion is about that was never made, is the failing check"
)]
mod tests {
    use super::*;

    use serde_json::{json, Value};

    use crate::serverkit::{ok, rpc_error, Harness, Reply};

    /// Two screens under one dump key means the second overwrites the first
    /// in the gallery, and whatever it was showing sits outside everything
    /// `docs/audit.js` checks.
    #[test]
    fn each_screen_dumps_under_a_key_of_its_own() {
        assert_ne!(dump_key(Screen::List), dump_key(Screen::Detail));
    }

    /// The degraded case. A relative path is the one worth pinning: goose
    /// would resolve it against its own process directory and answer with a
    /// straight face, so the screen would show a different project's skills
    /// and say nothing about it.
    #[test]
    fn only_an_absolute_working_directory_becomes_a_project_dir() {
        assert_eq!(
            project_dir("/Users/me/work/pilot"),
            Some("/Users/me/work/pilot")
        );
        assert_eq!(
            project_dir("  /Users/me/work/pilot  "),
            Some("/Users/me/work/pilot")
        );
        assert_eq!(project_dir(""), None);
        assert_eq!(project_dir("   "), None);
        assert_eq!(project_dir("work/pilot"), None);
        assert_eq!(project_dir("~/work/pilot"), None);
    }

    #[test]
    fn the_draft_names_the_skill_and_leaves_room() {
        assert_eq!(draft_for("code-review"), "Use the code-review skill: ");
    }

    // ------------------------------------------------------------ the server
    //
    // Everything above is a pure function. Everything below is the module's
    // other half — one `let Some(client) = ... else { return }` away from
    // doing nothing — and the rules that live there are about WHAT WENT OUT
    // and HOW OFTEN: fetched once, never polled, `includeProjectSources`
    // false, `projectDir` only when it is a real path. None of those is
    // visible in `Ctx` afterwards, so these run against `crate::serverkit`'s
    // loopback JSON-RPC server and read its request log.

    /// One `sources/list` entry, in the shape goose sends one.
    fn wire_source(name: &str, path: &str) -> Value {
        json!({
            "type": "skill",
            "name": name,
            "description": format!("What {name} does."),
            "content": format!("---\nname: {name}\n---\n\nDo the thing."),
            "path": path,
            "global": true,
        })
    }

    fn source(name: &str, path: &str) -> SourceEntry {
        serde_json::from_value(wire_source(name, path)).unwrap()
    }

    /// A goose with two filesystem skills and one built-in — deliberately out
    /// of order, and deliberately across the two calls, so the merge has
    /// something to get wrong.
    fn two_halves(_method: &str, params: &Value) -> Reply {
        match params["type"].as_str() {
            Some("skill") => ok(json!({
                "sources": [wire_source("zebra", "/z"), wire_source("Apple", "/a")],
            })),
            _ => ok(json!({ "sources": [wire_source("beta", "/b")] })),
        }
    }

    /// The built-ins fail and the user's own skills arrive.
    fn half_a_list(method: &str, params: &Value) -> Reply {
        if params["type"] == json!("builtinSkill") {
            return rpc_error(-32603, "could not decode a shipped skill");
        }
        two_halves(method, params)
    }

    /// A goose with no `sources/list` at all. `-32601` is its own signal for
    /// "this feature is absent", not merely "this server is old".
    fn no_sources_plane(_method: &str, _params: &Value) -> Reply {
        rpc_error(-32601, "Method not found")
    }

    /// A goose that hands back a session, for the `use_skill` tests. The tests
    /// count `session/new` rather than trusting the script to be selective.
    fn hands_back_a_session(_method: &str, _params: &Value) -> Reply {
        ok(json!({ "sessionId": "s-new", "configOptions": [] }))
    }

    fn names(h: &Harness) -> Vec<String> {
        h.with(|ctx| {
            ctx.skills
                .list
                .peek()
                .items
                .iter()
                .map(|entry| entry.name.clone())
                .collect()
        })
    }

    // ---- the fetch ----

    /// The payload rule, which is the reason this module has no polling and no
    /// cache: `sources/list` returns every skill's entire `SKILL.md` inline, so
    /// a second visit must re-render what is in memory rather than pull ~80KB
    /// down a cellular tailnet again.
    #[test]
    fn the_first_visit_fetches_and_a_second_one_re_reads_what_it_already_has() {
        let (mut h, server) = Harness::connected(two_halves);
        h.act(ensure_loaded);
        assert_eq!(
            server.count("sources/list"),
            2,
            "a skills fetch is two calls — filesystem skills and built-ins"
        );
        assert_eq!(
            names(&h),
            ["Apple", "beta", "zebra"],
            "the two halves did not arrive merged and sorted case-insensitively"
        );

        h.act(ensure_loaded);
        assert_eq!(
            server.count("sources/list"),
            2,
            "revisiting the tab pulled every SKILL.md down the wire again"
        );

        // The pull gesture is the one thing that may.
        h.act(refresh);
        assert_eq!(server.count("sources/list"), 4);
    }

    /// `includeProjectSources: true` makes goose walk every project in its
    /// registry and return the full text of skills belonging to projects the
    /// phone is not pointed at. It must be false in both halves of every call.
    #[test]
    fn the_fetch_never_asks_for_other_projects_skills() {
        let (mut h, server) = Harness::connected(two_halves);
        h.set_working_dir("/Users/me/work/pilot");
        h.act(ensure_loaded);

        for n in 0..2 {
            let params = server.params("sources/list", n);
            assert_eq!(
                params["includeProjectSources"],
                json!(false),
                "call {n} asked goose to walk every project it knows about"
            );
            assert_eq!(
                params["projectDir"],
                json!("/Users/me/work/pilot"),
                "call {n} did not carry the configured project"
            );
        }
        assert_eq!(
            server.params("sources/list", 0)["type"],
            json!("skill"),
            "the filesystem half is asked for first — it is the one that \
             teaches the client whether the method exists"
        );
        assert_eq!(
            server.params("sources/list", 1)["type"],
            json!("builtinSkill")
        );
    }

    /// A relative working directory must arrive as no `projectDir` at all.
    /// goose would resolve one against its own process directory and answer
    /// with a straight face, so the screen would show a different project's
    /// skills and say nothing about it.
    #[test]
    fn a_relative_working_directory_is_sent_as_no_project_at_all() {
        let (mut h, server) = Harness::connected(two_halves);
        h.set_working_dir("work/pilot");
        h.act(refresh);
        assert_eq!(
            server.params("sources/list", 0)["projectDir"],
            Value::Null,
            "a relative path was sent as the project directory"
        );
        assert_eq!(
            names(&h),
            ["Apple", "beta", "zebra"],
            "no project directory is not a failure: the global skills still load"
        );
    }

    /// Half a list is still a list. goose Desktop `Promise.all`s the two calls
    /// and shows an empty screen when either fails; this shows what arrived and
    /// says the rest is missing — a toast, because there is something readable
    /// underneath it.
    #[test]
    fn one_half_failing_shows_the_other_half_and_says_so() {
        let (mut h, _server) = Harness::connected(half_a_list);
        h.act(ensure_loaded);

        assert_eq!(
            names(&h),
            ["Apple", "zebra"],
            "one failing half threw the other half's skills away"
        );
        let toast = h.toast().expect("the missing half was never mentioned");
        assert!(
            toast.contains("Some skills are missing"),
            "the toast does not say part of the list is absent: {toast}"
        );
        h.with(|ctx| {
            assert_eq!(
                ctx.skills.list.peek().sticky,
                None,
                "a partial list is not a failure to keep on screen over the \
                 skills that did arrive"
            );
        });
    }

    /// A server without the feature costs ONE round trip, not two: the first
    /// `-32601` is cached, so the second half is refused without touching the
    /// socket — and both halves failing is what puts the screen on its
    /// unsupported arm rather than on an empty list that reads as "you have no
    /// skills".
    #[test]
    fn a_server_without_sources_is_asked_once_and_told_apart_from_an_empty_list() {
        let (mut h, server) = Harness::connected(no_sources_plane);
        h.act(ensure_loaded);

        assert_eq!(
            server.count("sources/list"),
            1,
            "the client asked a server that had already refused the method"
        );
        h.with(|ctx| {
            let list = ctx.skills.list.peek();
            assert!(
                list.unsupported,
                "a server with no skills feature reads as an empty list, which \
                 says the user has no skills"
            );
            assert!(!list.loading, "the spinner is still turning");
        });
    }

    /// Offline is already on the screen in words, so a refresh must add
    /// nothing — no toast saying it twice, and above all no spinner, because
    /// nothing would ever stop it.
    #[test]
    fn a_refresh_without_a_connection_says_nothing_and_arms_nothing() {
        let mut h = Harness::offline();
        h.act(ensure_loaded);
        assert_eq!(h.toast(), None, "being offline was reported twice");
        h.with(|ctx| {
            let list = ctx.skills.list.peek();
            assert!(!list.loading, "a disconnected fetch armed a spinner");
            assert!(list.items.is_empty() && list.sticky.is_none());
        });
    }

    // ---- the detail ----

    /// The open skill is a CLONE, not an index: a refresh that lands while the
    /// detail is open reorders the list under it, and an index would then point
    /// at a different skill than the one whose name is in the title. Backing
    /// out drops it, because it is the largest thing this feature holds.
    #[test]
    fn the_open_skill_survives_the_list_being_reordered_under_it() {
        let (mut h, _server) = Harness::connected(two_halves);
        h.act(|ctx| open(ctx, source("zebra", "/z")));
        h.act(ensure_loaded);

        h.with(|ctx| {
            assert!(matches!(*ctx.skills.screen.peek(), Screen::Detail));
            assert_eq!(
                ctx.skills.open.peek().as_ref().map(|e| e.name.clone()),
                Some("zebra".to_owned()),
                "the fetch that reordered the list moved the detail onto \
                 another skill"
            );
        });

        h.act(close);
        h.with(|ctx| {
            assert!(matches!(*ctx.skills.screen.peek(), Screen::List));
            assert!(
                ctx.skills.open.peek().is_none(),
                "backing out kept a copy of the whole SKILL.md off screen"
            );
        });
    }

    // ---- using one ----

    /// The point of the feature: the composer opens with the invocation typed
    /// and the cursor where the user's half goes. Into the chat that is already
    /// open, because a new session would have none of the context just built up
    /// — and creating one would be a session nobody asked for.
    #[test]
    fn using_a_skill_from_an_open_chat_fills_that_chats_composer() {
        let (mut h, server) = Harness::connected(hands_back_a_session);
        h.set_working_dir("/srv/goose");
        h.with(|ctx| {
            ctx.chat.clone().set(crate::state::ChatState {
                session_id: Some("s-live".to_owned()),
                ..crate::state::ChatState::default()
            });
        });
        h.act(|ctx| use_skill(ctx, "code-review"));

        h.with(|ctx| {
            assert_eq!(*ctx.chat_draft.peek(), "Use the code-review skill: ");
            assert!(matches!(*ctx.tab.peek(), Tab::Home));
            assert!(
                matches!(*ctx.screen.peek(), HomeScreen::Chat),
                "the draft was filled into a chat the user was not taken to"
            );
        });
        assert_eq!(
            server.count("session/new"),
            0,
            "a second session was created beside the chat that was already open"
        );
    }

    /// With no chat open the Chats list is where a failure should leave you —
    /// `new_session` navigates on success and toasts on failure, so landing on
    /// a detail screen would look like a button that did nothing.
    #[test]
    fn using_a_skill_with_no_chat_open_starts_one_from_the_chats_list() {
        let (mut h, server) = Harness::connected(hands_back_a_session);
        h.set_working_dir("/srv/goose");

        // Synchronously — before the session call has even gone out.
        h.with(|ctx| {
            use_skill(ctx, "deploy");
            assert!(
                matches!(*ctx.screen.peek(), HomeScreen::Sessions),
                "the tap left the user on a screen the failure path cannot \
                 report onto"
            );
        });
        h.settle();

        assert_eq!(server.count("session/new"), 1);
        assert_eq!(server.params("session/new", 0)["cwd"], json!("/srv/goose"));
        h.with(|ctx| {
            assert_eq!(*ctx.chat_draft.peek(), "Use the deploy skill: ");
            assert!(
                matches!(*ctx.screen.peek(), HomeScreen::Chat),
                "the session was created and nothing navigated to it"
            );
            assert_eq!(ctx.chat.peek().session_id.as_deref(), Some("s-new"));
        });
    }

    /// Settings half filled in: the draft is still prepared, but the failure is
    /// said out loud rather than leaving a button that quietly does nothing.
    #[test]
    fn using_a_skill_without_a_working_directory_says_why_it_could_not() {
        let (mut h, server) = Harness::connected(hands_back_a_session);
        h.act(|ctx| use_skill(ctx, "deploy"));

        assert_eq!(
            server.count("session/new"),
            0,
            "a session was attempted with a working directory goose refuses"
        );
        let toast = h.toast().expect("the tap did nothing and said nothing");
        assert!(
            toast.contains("absolute working directory"),
            "the toast does not say what to fix: {toast}"
        );
        h.with(|ctx| {
            assert!(
                matches!(*ctx.screen.peek(), HomeScreen::Sessions),
                "the user is left on a screen with no sign of the failure"
            );
        });
    }
}
