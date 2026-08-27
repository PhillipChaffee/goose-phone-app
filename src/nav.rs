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

/// What to call the screen the detail column is showing.
///
/// A value rather than an `Element`, and that is the whole reason it exists.
/// Dioxus has no portal, so nothing rendered inside a pane can be moved into
/// the window's own bar — but a `String` travels anywhere. The desktop shell
/// paints this in `.shell-chrome` (`src/shell/desktop.rs`) and
/// `assets/desktop.css` takes the same heading back out of the pane below, so
/// there is one title per window rather than one per column.
///
/// The subtitle sits BESIDE the title rather than under it, which is a
/// constraint rather than a taste: `--chrome-h` is 52px measured off a real
/// macOS window, the toggle beside it is a 32px control centred on the traffic
/// lights at y 16, and a two-line group centred on that same 16 would start
/// above the top of the window. One line clears both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Crumb {
    pub title: String,
    /// Where the thing lives, what it is part of — the same string the pane's
    /// own `.subtitle` carries, from the same expression.
    pub subtitle: Option<String>,
}

impl Crumb {
    /// A name and nothing else.
    pub(crate) fn plain(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            subtitle: None,
        }
    }

    /// A name and where it lives. `None` is the same as [`Crumb::plain`] —
    /// several screens compute a subtitle that is legitimately absent.
    pub(crate) fn detailed(title: impl Into<String>, subtitle: Option<String>) -> Self {
        Self {
            title: title.into(),
            subtitle,
        }
    }
}

/// Whatever a destination has pushed on top of its root: the screen itself,
/// and what to call it.
///
/// ONE function returns both, deliberately. The pair could have been two
/// fields on [`Destination`] — a `detail` and a `detail_title`, each matching
/// the same screen enum — and then the only thing keeping them in step would
/// be whoever edited the row last: the app would render a diff and the window
/// bar would name the chat it came from, with nothing to say so. This is the
/// same argument the `root`/`detail` pair is already built on, one level down.
pub(crate) struct Detail {
    /// Dead on a phone, deliberately. One screen is on a phone at a time and
    /// it names itself in its own header; there is no window bar to hand a
    /// name to, so [`Destination::screen`] drops this on the floor. The
    /// `expect` is scoped to those two targets rather than blanket-allowed, so
    /// the day the phone does read it the exception fails the build instead of
    /// rotting.
    #[cfg_attr(
        any(target_os = "ios", target_os = "android"),
        expect(
            dead_code,
            reason = "the window bar this names is the desktop shell's alone"
        )
    )]
    pub crumb: Crumb,
    pub view: Element,
}

impl Detail {
    pub(crate) const fn new(crumb: Crumb, view: Element) -> Self {
        Self { crumb, view }
    }
}

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
/// The five function pointers are what keep the table honest: a row that
/// cannot say how to reach itself, whether it is showing, what it lists, what
/// it has open and what to call the dump is not a destination, it is a button.
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
    /// The screen at the BOTTOM of this destination's stack — the list the
    /// desktop shell keeps in its middle column while a detail fills the
    /// third.
    ///
    /// `None` is a destination with nothing to list. Settings is one screen, so
    /// on the desktop it takes the content area whole rather than half of it,
    /// and the shell draws two columns instead of three.
    pub root: Option<fn(&AppCtx) -> Element>,
    /// Whatever this destination has pushed ON TOP of its root, or `None` when
    /// it is at its root.
    ///
    /// The pair is deliberately a root plus a detail rather than a root plus a
    /// "render whichever is on top": those two would each carry their own copy
    /// of the root arm, and the only thing keeping the copies equal would be
    /// whoever edited the row last. Swapping a list component would then move
    /// the phone and leave the desktop's middle column rendering the old one,
    /// and no phone gate could see it. Here the root is written once and
    /// [`Destination::screen`] composes the two, so they cannot disagree.
    ///
    /// A phone reads it only through `screen`, which is a destination's whole
    /// stack collapsed to the one screen a phone shows — and drops the
    /// [`Crumb`], which is the desktop's window bar's alone.
    pub detail: fn(&AppCtx) -> Option<Detail>,
    /// The dump key for the mounted screen, or `None` when this destination's
    /// stack is not the one on screen.
    ///
    /// One function answers both "is it mounted" and "what is it showing"
    /// because they are one lookup: two functions could disagree, and then
    /// the app would render one screen and file the dump under another.
    pub key: fn(&AppCtx) -> Option<&'static str>,
}

impl Destination {
    /// This destination's whole stack collapsed to the one screen on top of it.
    ///
    /// The phone's answer, and only the phone's: the desktop reads `root` and
    /// `detail` separately, because its whole point is that the two are on
    /// screen at once and a closed detail is a column with a sentence in it
    /// rather than the list repeated. So this is `cfg`-gated to the targets
    /// that call it, the same way `desktop::MIN_INNER` is gated to the one
    /// that has a window — an ungated helper with one caller behind a `cfg` is
    /// dead code on every other target, and `cargo clippy -D warnings` is
    /// right to say so.
    ///
    /// `cargo check --target aarch64-apple-ios` is what compiles it. A
    /// destination with neither a detail nor a root would render nothing, and
    /// no row of the table is one: the six with lists are covered by
    /// `every_destination_but_settings_lists_something`, and the seventh is
    /// Settings, whose detail is unconditional.
    #[cfg(any(target_os = "ios", target_os = "android"))]
    pub(crate) fn screen(&self, ctx: &AppCtx) -> Element {
        (self.detail)(ctx)
            .map(|detail| detail.view)
            .or_else(|| self.root.map(|root| root(ctx)))
            .unwrap_or_else(|| rsx! {})
    }
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
    root: Some(|_| rsx! { views::sessions::SessionsView {} }),
    detail: |ctx| match (ctx.screen)() {
        Screen::Chat => Some(Detail::new(
            views::chat::crumb(ctx),
            rsx! { views::chat::ChatView {} },
        )),
        _ => None,
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
        root: Some(|_| rsx! { views::code::CodeSessionsView {} }),
        detail: |ctx| match (ctx.code_screen)() {
            CodeScreen::List => None,
            CodeScreen::New => Some(Detail::new(
                views::code::new_crumb(),
                rsx! { views::code::CodeNewView {} },
            )),
            CodeScreen::Chat => Some(Detail::new(
                views::code::chat_crumb(ctx),
                rsx! { views::code::CodeChatView {} },
            )),
            CodeScreen::Diff => Some(Detail::new(
                views::code::diff_crumb(ctx),
                rsx! { views::code::CodeDiffView {} },
            )),
            CodeScreen::Pulls => Some(Detail::new(
                views::code::pulls_crumb(ctx),
                rsx! { views::code::CodePullsView {} },
            )),
        },
        key: |ctx| ((ctx.tab)() == Tab::Code).then(|| code_key((ctx.code_screen)())),
    },
    Destination {
        id: "recipes",
        label: "Recipes",
        icon: "book",
        group: Group::Library,
        // Recipes owns `recipes.screen` outright, so — like Code — the
        // drawer leaves it wherever it was left.
        go: |ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Recipes);
        },
        at_root: |ctx| {
            (ctx.tab)() == Tab::Recipes && (ctx.recipes.screen)() == crate::recipes::Screen::List
        },
        root: Some(|_| rsx! { views::recipes::RecipesView {} }),
        detail: |ctx| match (ctx.recipes.screen)() {
            crate::recipes::Screen::List => None,
            crate::recipes::Screen::Detail => Some(Detail::new(
                views::recipes::crumb(ctx),
                rsx! { views::recipes::RecipeDetailView {} },
            )),
        },
        // Spelled inline rather than as a `recipes_key` beside `code_key`:
        // one destination is one hunk, and five branches each adding a
        // free function to the bottom of this file is five overlapping ones.
        key: |ctx| {
            ((ctx.tab)() == Tab::Recipes).then(|| match (ctx.recipes.screen)() {
                crate::recipes::Screen::List => "recipes-list",
                crate::recipes::Screen::Detail => "recipes-detail",
            })
        },
    },
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
        root: Some(|_| rsx! { views::skills::SkillsView {} }),
        detail: |ctx| match (ctx.skills.screen)() {
            crate::skills::Screen::List => None,
            crate::skills::Screen::Detail => Some(Detail::new(
                views::skills::crumb(ctx),
                rsx! { views::skills::SkillDetailView {} },
            )),
        },
        // The key mapping lives in `crate::skills` rather than here, so that
        // adding this destination is one row in this table and not a row plus
        // a function plus a test.
        key: |ctx| {
            ((ctx.tab)() == Tab::Skills).then(|| crate::skills::dump_key((ctx.skills.screen)()))
        },
    },
    Destination {
        id: "scheduler",
        label: "Scheduler",
        icon: "clock",
        // Library, beside Recipes, because a schedule is a thing you made out
        // of a recipe rather than a setting of the machine: Recipes is where
        // one is born, Scheduler is where it is watched.
        group: Group::Library,
        // Nothing to name: Scheduler owns its own screen signal, so the drawer
        // leaves it where it was left — including on a job's detail.
        go: |ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Scheduler);
        },
        at_root: |ctx| {
            (ctx.tab)() == Tab::Scheduler
                && (ctx.scheduler.screen)() == crate::scheduler::Screen::List
        },
        root: Some(|_| rsx! { views::scheduler::SchedulerView {} }),
        detail: |ctx| match (ctx.scheduler.screen)() {
            crate::scheduler::Screen::List => None,
            crate::scheduler::Screen::Detail => Some(Detail::new(
                views::scheduler::crumb(ctx),
                rsx! { views::scheduler::ScheduledJobView {} },
            )),
        },
        // The key mapping is a free function in `crate::scheduler`, next to the
        // enum it reads, so it can be tested without a Dioxus runtime.
        key: |ctx| {
            ((ctx.tab)() == Tab::Scheduler)
                .then(|| crate::scheduler::dump_key((ctx.scheduler.screen)()))
        },
    },
    Destination {
        id: "extensions",
        label: "Extensions",
        icon: "package",
        group: Group::Server,
        // Nothing to name: Extensions owns `extensions.screen` outright, so a
        // trip through the drawer leaves it exactly where it was.
        go: |ctx| {
            let mut tab = ctx.tab;
            tab.set(Tab::Extensions);
        },
        // Root only, the same reading Chats and Code have: from an
        // extension's detail screen, Extensions is where you came from.
        at_root: |ctx| {
            (ctx.tab)() == Tab::Extensions
                && (ctx.extensions.screen)() == crate::extensions::Screen::List
        },
        root: Some(|_| rsx! { views::extensions::ExtensionsView {} }),
        detail: |ctx| match (ctx.extensions.screen)() {
            crate::extensions::Screen::List => None,
            crate::extensions::Screen::Detail => Some(Detail::new(
                views::extensions::crumb(ctx),
                rsx! { views::extensions::ExtensionDetailView {} },
            )),
        },
        // The key mapping is a free function in `crate::extensions`, next to
        // the enum it reads, so it can be tested without a Dioxus runtime.
        key: |ctx| {
            ((ctx.tab)() == Tab::Extensions)
                .then(|| crate::extensions::dump_key((ctx.extensions.screen)()))
        },
    },
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
        // One screen, no list. On the desktop that is a two-column shell:
        // a placeholder column beside Settings would be a column of nothing.
        // The screen is the detail, then — the one row where the detail is
        // unconditional, which is the same fact said in the table's own terms.
        root: None,
        detail: |_| {
            Some(Detail::new(
                views::settings::crumb(),
                rsx! { views::settings::SettingsView {} },
            ))
        },
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

    /// A destination with no root screen is a blank middle column on the
    /// desktop, and nothing else would report it: the mobile shell never reads
    /// the field, so every phone gate passes with the hole in place. Settings
    /// is the one row that means it — one screen, nothing to list.
    #[test]
    fn every_destination_but_settings_lists_something() {
        for dest in DESTINATIONS {
            assert_eq!(
                dest.root.is_some(),
                dest.id != "settings",
                "{} has no root screen, so the desktop shell has nothing to \
                 keep in its list column while a detail is open",
                dest.id
            );
        }
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
