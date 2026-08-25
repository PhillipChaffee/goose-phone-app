//! Navigation as data.
//!
//! Adding a destination used to mean editing five separate `match`es in
//! `app.rs` — the route arm, the dump-key arm, the `Destination` variant, the
//! drawer button and the `navigate` arm — every one of them a place two
//! branches touch the same lines. With five features arriving in parallel
//! that is the worst merge surface in the app, so the five `match`es are one
//! table instead: a destination is a row, and adding one is adding a row.
//!
//! The rules the table encodes, both of which predate it:
//!
//!   - Each destination keeps its own back stack, so leaving via the drawer
//!     and coming back lands you where you were.
//!   - A destination is "here" only when its stack is at its root. From a
//!     chat, Chats is somewhere to go *back* to, not where you are.

use dioxus::prelude::*;

use crate::code::CodeScreen;
use crate::state::{AppCtx, Screen, Tab};
use crate::views;

/// Where a destination sits in the drawer.
///
/// The groups are about *whose* thing it is: [`Group::Work`] is what you are
/// doing, [`Group::Library`] is what you have saved, [`Group::Server`] is the
/// machine's own configuration. Only the last two carry a header — the first
/// group needs no label, because everything above the first rule is obviously
/// the top of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    Work,
    Library,
    Server,
}

impl Group {
    /// Drawer order. Iterated rather than derived so the order is stated in
    /// one place instead of following from where a row happens to sit.
    pub(crate) const ALL: [Self; 3] = [Self::Work, Self::Library, Self::Server];

    /// The section header, or `None` for the group that does not take one.
    pub(crate) const fn header(self) -> Option<&'static str> {
        match self {
            Self::Work => None,
            Self::Library => Some("Library"),
            Self::Server => Some("Server"),
        }
    }
}

/// One drawer destination and everything the shell needs to know about it.
///
/// The four function pointers are what keep the table honest: a row that
/// cannot say how to reach itself, whether it is showing, what to render and
/// what to call the dump is not a destination, it is a button.
#[derive(Debug)]
pub(crate) struct Destination {
    /// Stable identifier, and the stem of the dump keys this destination's
    /// screens use (`code` → `code-list`, `code-new`, …). The keys are
    /// spelled out in [`Destination::key`] rather than built from `id`: the
    /// gallery and `docs/audit.js` have been keyed on the existing names
    /// since before this table existed.
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    pub group: Group,
    /// Make this destination the one on screen.
    ///
    /// It names a screen only where it has to. Chats and Settings share the
    /// Home `screen` signal, so entering either means saying which — and for
    /// Chats that is its root, since a pushed chat is not something the
    /// drawer navigates *to*. A destination that owns its own screen signal
    /// sets the tab and nothing else, which is what keeps its back stack
    /// across a trip through the drawer.
    pub go: fn(&AppCtx),
    /// True only when this destination's own stack is at its root — which is
    /// what the drawer marks as active.
    pub at_root: fn(&AppCtx) -> bool,
    /// Render whichever screen of this destination's stack is on top.
    pub view: fn(&AppCtx) -> Element,
    /// The dump key for the mounted screen, or `None` when this destination's
    /// stack is not the one on screen.
    ///
    /// One function answers both "is it mounted" and "what is it showing"
    /// because they are one lookup: two functions could disagree, and then
    /// the app would render one screen and file the dump under another.
    pub key: fn(&AppCtx) -> Option<&'static str>,
}

/// Chats: the goose session list and the chat pushed on top of it.
///
/// Named separately from the table so [`current`] has something total to fall
/// back to — there is always a screen on the phone, and picking the first
/// destination beats an index that could panic.
const CHATS: Destination = Destination {
    id: "chats",
    label: "Chats",
    icon: "message",
    group: Group::Work,
    go: |ctx| {
        let (mut tab, mut screen) = (ctx.tab, ctx.screen);
        tab.set(Tab::Home);
        screen.set(Screen::Sessions);
    },
    at_root: |ctx| (ctx.tab)() == Tab::Home && (ctx.screen)() == Screen::Sessions,
    view: |ctx| match (ctx.screen)() {
        Screen::Chat => rsx! { views::chat::ChatView {} },
        _ => rsx! { views::sessions::SessionsView {} },
    },
    key: |ctx| {
        ((ctx.tab)() == Tab::Home)
            .then(|| chats_key((ctx.screen)()))
            .flatten()
    },
};

/// Every drawer destination, in drawer order.
///
/// The blank-line-separated comments are placeholders, one per feature branch
/// that adds a destination. Each branch replaces its own line and nothing
/// else, so four concurrent branches merge without ever touching the same
/// hunk. Session history has no line: it is the Chats list growing kinds,
/// rename and search, and arrives inside a destination that already exists.
pub(crate) const DESTINATIONS: &[Destination] = &[
    CHATS,
    Destination {
        id: "code",
        label: "Code",
        icon: "code",
        group: Group::Work,
        // Nothing to name: Code owns `code_screen` outright, so it is left
        // exactly where it was left.
        go: |ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Code);
        },
        // Root only, the same reading Chats has always had: from a code chat
        // Code is where you came from. The drawer used to mark Code active
        // anywhere inside the tab, which said "here" about a screen two
        // pushes away.
        at_root: |ctx| (ctx.tab)() == Tab::Code && (ctx.code_screen)() == CodeScreen::List,
        view: |ctx| match (ctx.code_screen)() {
            CodeScreen::List => rsx! { views::code::CodeSessionsView {} },
            CodeScreen::New => rsx! { views::code::CodeNewView {} },
            CodeScreen::Chat => rsx! { views::code::CodeChatView {} },
            CodeScreen::Diff => rsx! { views::code::CodeDiffView {} },
            CodeScreen::Pulls => rsx! { views::code::CodePullsView {} },
        },
        key: |ctx| ((ctx.tab)() == Tab::Code).then(|| code_key((ctx.code_screen)())),
    },
    // recipes — PR 3 replaces this line
    Destination {
        id: "skills",
        label: "Skills",
        icon: "sparkle",
        group: Group::Library,
        // Nothing to name: Skills owns its own screen signal, so the drawer
        // leaves it where it was left — including on a skill's detail.
        go: |ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Skills);
        },
        at_root: |ctx| {
            (ctx.tab)() == Tab::Skills && (ctx.skills.screen)() == crate::skills::Screen::List
        },
        view: |ctx| match (ctx.skills.screen)() {
            crate::skills::Screen::List => rsx! { views::skills::SkillsView {} },
            crate::skills::Screen::Detail => rsx! { views::skills::SkillDetailView {} },
        },
        // The key mapping lives in `crate::skills` rather than here, so that
        // adding this destination is one row in this table and not a row plus
        // a function plus a test.
        key: |ctx| {
            ((ctx.tab)() == Tab::Skills).then(|| crate::skills::dump_key((ctx.skills.screen)()))
        },
    },
    // scheduler — PR 5 replaces this line

    // extensions — PR 6 replaces this line
    Destination {
        id: "settings",
        label: "Settings",
        icon: "gear",
        group: Group::Server,
        go: |ctx| {
            let (mut tab, mut screen) = (ctx.tab, ctx.screen);
            tab.set(Tab::Home);
            screen.set(Screen::Settings);
        },
        at_root: |ctx| (ctx.tab)() == Tab::Home && (ctx.screen)() == Screen::Settings,
        view: |_| rsx! { views::settings::SettingsView {} },
        key: |ctx| {
            ((ctx.tab)() == Tab::Home && (ctx.screen)() == Screen::Settings).then_some("settings")
        },
    },
];

/// The destination whose stack is on screen.
pub(crate) fn current(ctx: &AppCtx) -> &'static Destination {
    DESTINATIONS
        .iter()
        .find(|dest| (dest.key)(ctx).is_some())
        .unwrap_or(&CHATS)
}

/// The dump key for the Home tab, or `None` when Home is showing Settings —
/// which is a destination of its own, not a screen of this one.
///
/// A free function over the plain enum rather than a closure over signals, so
/// the resolution can be tested without a Dioxus runtime (the same reason
/// `SettingRow::select` and `code_chip_label` are free functions).
const fn chats_key(screen: Screen) -> Option<&'static str> {
    match screen {
        Screen::Sessions => Some("chats"),
        // Singular: the gallery and docs/audit.js have been keyed on these
        // names since before this table existed, and re-capturing every state
        // to rename one is not what a refactor is for.
        Screen::Chat => Some("chat"),
        Screen::Settings => None,
    }
}

const fn code_key(screen: CodeScreen) -> &'static str {
    match screen {
        CodeScreen::List => "code-list",
        CodeScreen::New => "code-new",
        CodeScreen::Chat => "code-chat",
        CodeScreen::Diff => "code-diff",
        CodeScreen::Pulls => "code-pulls",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings shares the Home `screen` signal with Chats but is a
    /// destination of its own, so the Chats stack has to disown it — or the
    /// drawer marks Chats as where you are while Settings is on screen.
    #[test]
    fn settings_is_not_a_screen_of_chats() {
        assert_eq!(chats_key(Screen::Settings), None);
        assert_eq!(chats_key(Screen::Sessions), Some("chats"));
        assert_eq!(chats_key(Screen::Chat), Some("chat"));
    }

    /// Two screens under one dump key means the second overwrites the first
    /// in the gallery, and whatever it was showing sits outside everything
    /// `docs/audit.js` checks — which has happened, to a whole branch of it.
    #[test]
    fn every_screen_dumps_under_a_key_of_its_own() {
        let mut keys: Vec<&str> = [Screen::Settings, Screen::Sessions, Screen::Chat]
            .into_iter()
            .filter_map(chats_key)
            .chain(["settings"])
            .chain(
                [
                    CodeScreen::List,
                    CodeScreen::New,
                    CodeScreen::Chat,
                    CodeScreen::Diff,
                    CodeScreen::Pulls,
                ]
                .into_iter()
                .map(code_key),
            )
            .collect();
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "two screens share a dump key");
    }

    /// Two destinations sharing a dump key means two screens overwriting each
    /// other in the gallery — which is how a whole branch of new UI once came
    /// to sit outside everything `docs/audit.js` checks.
    #[test]
    fn destination_ids_are_unique() {
        let mut ids: Vec<&str> = DESTINATIONS.iter().map(|dest| dest.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "two destinations share an id");
    }

    /// Every destination has to be reachable from the drawer: a row in a
    /// group the drawer does not paint is a screen with no way in.
    #[test]
    fn every_destination_sits_in_a_painted_group() {
        for dest in DESTINATIONS {
            assert!(
                Group::ALL.contains(&dest.group),
                "{} is in a group the drawer never renders",
                dest.id
            );
        }
    }
}
