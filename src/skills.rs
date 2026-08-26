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
mod tests {
    use super::*;

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
}
