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
/// paints this in `.shell-chrome` (`src/shell/desktop/mod.rs`) and
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
///
/// THE PHONE'S TAXONOMY, and now only the phone's — the desktop arranges its
/// nav by [`Plane`]. The two are held equal by
/// `the_two_taxonomies_are_one_table` below, and this one is queued for
/// deletion: when the phone adopts the switch, `Group`, `Group::ALL`,
/// `Group::header`, [`Destination::group`] and `shell::render_group` go
/// together.
///
/// **Why the drawer-only items below carry `allow` and not `expect`**, which is
/// a deviation from this repo's policy and is deliberate. They are read by
/// `render_group`, compiled on phones only, and by the tests in this file,
/// which run on the host. So in a desktop build they are dead in the BINARY and
/// live in the TEST binary of the same invocation, and `#[expect]` cannot say
/// that: `cargo clippy --all-targets` builds both, the lint fires in one and
/// not the other, and the half where it does not fire then fails as
/// `unfulfilled_lint_expectation` under `-D warnings`. Measured rather than
/// assumed — `expect` was written first and that is exactly what it did. The
/// policy's reason for preferring `expect` is that an exception which stops
/// being needed should fail the build rather than rot; what answers that here
/// is the deletion above, which takes the attributes with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Group {
    Work,
    Library,
    Server,
}

impl Group {
    /// Drawer order. Iterated rather than derived so the order is stated in
    /// one place instead of following from where a row happens to sit.
    #[cfg_attr(
        not(any(target_os = "ios", target_os = "android")),
        allow(dead_code, reason = "the phone drawer's alone; see `Group`")
    )]
    pub(crate) const ALL: [Self; 3] = [Self::Work, Self::Library, Self::Server];

    /// The section header, or `None` for the group that does not take one.
    #[cfg_attr(
        not(any(target_os = "ios", target_os = "android")),
        allow(dead_code, reason = "the phone drawer's alone; see `Group`")
    )]
    pub(crate) const fn header(self) -> Option<&'static str> {
        match self {
            Self::Work => None,
            Self::Library => Some("Library"),
            Self::Server => Some("Server"),
        }
    }
}

/// Which half of the app a destination belongs to.
///
/// The desktop splits at the top level into a **Chat** half — goose's own
/// things, where nothing touches a repo — and a **Code** half, which is the
/// `OpenCode` plane and its working trees. The two are completely separate: own
/// list, own library, own vocabulary, no screen that shows both and no
/// abstraction over the pair. That is the decision this enum records, not a
/// limitation of it.
///
/// [`Group`] says the same thing in the shape the PHONE drawer needs, and the
/// two are held equal by `the_two_taxonomies_are_one_table` below for as long
/// as both exist. That test is the point of having both: the phone keeps its
/// flat drawer through this change, and when it adopts the switch `Group` is
/// DELETED rather than reconciled — nothing in the plane taxonomy reads it, so
/// there is no second derivation to redo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plane {
    Chat,
    Code,
}

impl Plane {
    /// Switch order, left to right. Iterated rather than derived, following
    /// [`Group::ALL`]: the order is stated in one place instead of following
    /// from where a row happens to sit.
    #[cfg_attr(
        any(target_os = "ios", target_os = "android"),
        expect(
            dead_code,
            reason = "the plane taxonomy is the desktop shell's until the phone \
                      adopts the switch, at which point this expectation fails \
                      and takes itself out"
        )
    )]
    pub(crate) const ALL: [Self; 2] = [Self::Chat, Self::Code];

    /// What the segment says. The segment is the ONE control that names a
    /// whole half of the app, so the word is a value here rather than a
    /// literal in the shell — same reason [`Group::header`] is.
    #[cfg_attr(
        any(target_os = "ios", target_os = "android"),
        expect(
            dead_code,
            reason = "the plane taxonomy is the desktop shell's until the phone \
                      adopts the switch, at which point this expectation fails \
                      and takes itself out"
        )
    )]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Code => "Code",
        }
    }

    /// The glyph beside the label, from `src/icons.rs`.
    ///
    /// Deliberately the same two icons the `chats` and `code` destinations
    /// already carry: the segment IS those destinations' plane, and a switch
    /// that used a third pair of glyphs would be naming the same two things
    /// twice in two alphabets.
    #[cfg_attr(
        any(target_os = "ios", target_os = "android"),
        expect(
            dead_code,
            reason = "the plane taxonomy is the desktop shell's until the phone \
                      adopts the switch, at which point this expectation fails \
                      and takes itself out"
        )
    )]
    pub(crate) const fn icon(self) -> &'static str {
        match self {
            Self::Chat => "message",
            Self::Code => "code",
        }
    }
}

/// The destination a plane opens on, and whose list is the plane's own.
///
/// A `match` and not a search of [`DESTINATIONS`], so the compiler makes it
/// total. The search version answers with an `Option`, and this file cannot
/// unwrap one — `unwrap_used` and `expect_used` are on in the workspace lint
/// table — so it would need [`current`]'s escape hatch instead: a fallback to
/// `CHATS` that is unreachable and that nothing can prove is unreachable. On
/// the Code plane that fallback would quietly render the chat list.
///
/// The cost is that the pairing is stated here rather than read off the table.
/// `every_plane_opens_on_a_list_of_its_own` and
/// `the_two_taxonomies_are_one_table` below are what keep the two in step.
#[cfg_attr(
    any(target_os = "ios", target_os = "android"),
    expect(
        dead_code,
        reason = "the plane taxonomy is the desktop shell's until the phone \
                  adopts the switch, at which point this expectation fails and \
                  takes itself out"
    )
)]
pub(crate) const fn primary(plane: Plane) -> &'static Destination {
    match plane {
        Plane::Chat => &CHATS,
        Plane::Code => &CODE,
    }
}

/// The rest of the plane: what you have saved, as against what you are doing.
///
/// Derived from [`primary`] rather than from [`Group::Library`], and that is
/// the whole design of this pair. The plane taxonomy reads no part of the group
/// taxonomy, so `Group` can be deleted whole when the phone adopts the switch.
///
/// Compared by `id` and not by pointer: `primary` returns a promoted `&CHATS`,
/// which is a different address from the `CHATS` copy inside `DESTINATIONS`,
/// so `ptr::eq` here would answer `false` for every row and put the primary in
/// its own library.
#[cfg_attr(
    any(target_os = "ios", target_os = "android"),
    expect(
        dead_code,
        reason = "the plane taxonomy is the desktop shell's until the phone \
                  adopts the switch, at which point this expectation fails and \
                  takes itself out"
    )
)]
pub(crate) fn library(plane: Plane) -> Vec<&'static Destination> {
    DESTINATIONS
        .iter()
        .filter(|dest| dest.plane == Some(plane) && dest.id != primary(plane).id)
        .collect()
}

/// The destinations that belong to no half, which the desktop keeps in the
/// sidebar's footer so they are reachable from either one.
///
/// A filter rather than a named `SETTINGS` const, so that the day a second
/// plane-free row arrives it is a row in the table and not an edit here. What
/// stops that from being a silent hole is `only_settings_belongs_to_neither_half`
/// below: today this is Settings and the test says so out loud.
#[cfg_attr(
    any(target_os = "ios", target_os = "android"),
    expect(
        dead_code,
        reason = "the plane taxonomy is the desktop shell's until the phone \
                  adopts the switch, at which point this expectation fails and \
                  takes itself out"
    )
)]
pub(crate) fn plane_free() -> Vec<&'static Destination> {
    DESTINATIONS
        .iter()
        .filter(|dest| dest.plane.is_none())
        .collect()
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
    /// Which drawer section the PHONE files this under. See [`Group`] for why
    /// this one is `allow` rather than `expect`, and what deletes it.
    #[cfg_attr(
        not(any(target_os = "ios", target_os = "android")),
        allow(dead_code, reason = "the phone drawer's alone; see `Group`")
    )]
    pub group: Group,
    /// Which half of the app this belongs to, or `None` for the one row that
    /// belongs to neither.
    ///
    /// Settings is that row: it configures the connection to BOTH servers, so
    /// filing it under a plane would hide the code gateway's fields behind the
    /// chat half. The desktop reaches it from the sidebar's footer, which is
    /// present in either plane, rather than from a plane's own list.
    #[cfg_attr(
        any(target_os = "ios", target_os = "android"),
        expect(
            dead_code,
            reason = "the plane taxonomy is the desktop shell's until the phone \
                      adopts the switch, at which point this expectation fails \
                      and takes itself out"
        )
    )]
    pub plane: Option<Plane>,
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
    plane: Some(Plane::Chat),
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

/// Code: the working trees on the `OpenCode` plane, and whatever one has open.
///
/// Named separately from the table for [`primary`]'s sake, which is the same
/// reason [`CHATS`] is: a plane's opening destination has to be nameable from
/// a `match`, or the pairing becomes a search that answers with an `Option`
/// this file is not allowed to unwrap.
const CODE: Destination = Destination {
    id: "code",
    label: "Code",
    icon: "code",
    group: Group::Work,
    plane: Some(Plane::Code),
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
    CODE,
    Destination {
        id: "recipes",
        label: "Recipes",
        icon: "book",
        group: Group::Library,
        plane: Some(Plane::Chat),
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
        plane: Some(Plane::Chat),
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
        plane: Some(Plane::Chat),
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
        // Chat, not Server, and this is the one row where the two taxonomies
        // genuinely disagree about shape rather than about spelling. An
        // extension is a goose extension — it changes what the chat half can
        // do and nothing about a working tree — so on the desktop it sits in
        // the Chat library. The phone drawer goes on filing it under Server,
        // because `Group` is a fact about the drawer and this is a fact about
        // the plane; `the_two_taxonomies_are_one_table` covers exactly this by
        // comparing Library and Server together against both libraries.
        plane: Some(Plane::Chat),
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
        // The one row that belongs to neither half — see the field's own
        // comment. It configures both servers at once.
        plane: None,
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

    use std::cell::Cell;

    // The only way the table's row functions can be run at all.
    //
    // `go`, `at_root`, `root`, `detail` and `key` each take an `&AppCtx`, and
    // an `AppCtx` is a bundle of Dioxus signals that exists only inside a
    // mounted component. `crate::testkit::render_seeded` mounts one — but the
    // view it takes is a bare `fn() -> Element` with nothing to capture with,
    // so the question goes in through a thread-local and the answer comes back
    // as the markup the mount rendered. One `#[test]` is one thread, so there
    // is nothing here for two tests to collide over.
    /// Something to ask the live context, answered as one line of text.
    type Question = fn(&AppCtx) -> String;
    /// Something to build out of the live context and render, the way the
    /// shell renders a `root` or a `detail`.
    type Scene = fn(&AppCtx) -> Element;

    thread_local! {
        static ASKED: Cell<Option<Question>> = const { Cell::new(None) };
        static SHOWN: Cell<Option<Scene>> = const { Cell::new(None) };
    }

    fn asked_view() -> Element {
        let ctx = crate::state::use_app_ctx();
        let answer = ASKED
            .take()
            .map_or_else(String::new, |question| question(&ctx));
        rsx! { "{answer}" }
    }

    fn shown_view() -> Element {
        let ctx = crate::state::use_app_ctx();
        SHOWN.take().map_or_else(|| rsx! {}, |view| view(&ctx))
    }

    /// Run `question` against a live `AppCtx` and hand back what it answered.
    ///
    /// Every caller asserts on an EXACT expected string rather than on a
    /// substring, and that is not fussiness: Dioxus swallows a panic thrown
    /// during render and renders nothing, so a mount that never reached the
    /// question answers `""`. Against an exact expectation that is a loud
    /// failure; against `contains` it would be a quiet pass over a test that
    /// ran no production code at all.
    fn ask(question: fn(&AppCtx) -> String) -> String {
        ASKED.set(Some(question));
        crate::testkit::render_seeded(|_| {}, asked_view)
    }

    /// Render whatever `view` builds out of a live `AppCtx`, and hand back the
    /// markup — the half of the harness that can run the table's `root` and
    /// `detail` screens rather than only ask about them.
    fn show(view: fn(&AppCtx) -> Element) -> String {
        SHOWN.set(Some(view));
        crate::testkit::render_seeded(|_| {}, shown_view)
    }

    /// One answer per row, in table order, joined into a single line — so a
    /// failure prints the whole table with the broken row named in it instead
    /// of `false != true`.
    fn per_destination(mut line: impl FnMut(&'static Destination) -> String) -> String {
        DESTINATIONS
            .iter()
            .map(&mut line)
            .collect::<Vec<_>>()
            .join(" | ")
    }

    /// Push whatever this destination stacks on top of its root.
    ///
    /// Settings is deliberately absent: it has no stack: one screen, which is
    /// its own detail. The tests that call this say so in their expectations.
    fn push_detail(ctx: &AppCtx, id: &str) {
        let (mut screen, mut code) = (ctx.screen, ctx.code_screen);
        let (mut recipes, mut skills) = (ctx.recipes.screen, ctx.skills.screen);
        let (mut scheduler, mut extensions) = (ctx.scheduler.screen, ctx.extensions.screen);
        match id {
            "chats" => screen.set(Screen::Chat),
            "code" => code.set(CodeScreen::Chat),
            "recipes" => recipes.set(crate::recipes::Screen::Detail),
            "skills" => skills.set(crate::skills::Screen::Detail),
            "scheduler" => scheduler.set(crate::scheduler::Screen::Detail),
            "extensions" => extensions.set(crate::extensions::Screen::Detail),
            _ => {}
        }
    }

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

    /// Ids, sorted and deduplicated — the shape every set comparison below is
    /// written in, so a failure prints two readable lists rather than two
    /// `Vec<&Destination>` debug dumps.
    fn ids(dests: impl IntoIterator<Item = &'static Destination>) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = dests.into_iter().map(|dest| dest.id).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    fn ids_in(group: Group) -> Vec<&'static str> {
        ids(DESTINATIONS.iter().filter(|dest| dest.group == group))
    }

    /// THE DRIFT GUARD, and the reason [`Plane`] and [`Group`] are allowed to
    /// coexist at all.
    ///
    /// Two taxonomies over one table is a standing invitation for a feature
    /// branch to add a row to whichever one it happened to be reading and
    /// leave the other short — and the failure is silent on both shells, since
    /// each renders only its own. Then the phone's adoption of the switch stops
    /// being a deletion of `Group` and becomes a re-derivation of whatever the
    /// two have drifted into.
    ///
    /// `Library` and `Server` are compared TOGETHER against both libraries
    /// because the two taxonomies legitimately cut that region differently:
    /// Extensions is `Group::Server` for the drawer and `Plane::Chat`'s library
    /// for the desktop. What may not differ is the membership — which rows are
    /// "something you have saved or configured" as against "the thing you are
    /// doing" — and that is what this compares.
    ///
    /// Shown to fail: give `recipes` `plane: None` and the second assertion
    /// reports `recipes` missing from the libraries; mark `skills` as
    /// `Group::Work` and the first reports it as a third primary.
    #[test]
    fn the_two_taxonomies_are_one_table() {
        assert_eq!(
            ids_in(Group::Work),
            ids(Plane::ALL.map(primary)),
            "the drawer's Work group and the planes' opening destinations have \
             drifted apart"
        );

        let saved = {
            let mut both = ids_in(Group::Library);
            both.extend(ids_in(Group::Server));
            both.retain(|id| *id != "settings");
            both.sort_unstable();
            both
        };
        assert_eq!(
            saved,
            ids(Plane::ALL.into_iter().flat_map(library)),
            "a destination is in one taxonomy's library and not the other's"
        );
    }

    /// A plane whose opening destination has nothing to list is a sidebar with
    /// nothing in it. `every_destination_but_settings_lists_something` does not
    /// catch this: it proves every row except Settings has a root, and says
    /// nothing about WHICH row a plane opens on.
    ///
    /// Shown to fail: set `CODE.root` to `None` — the other test then also
    /// fails, which is the point, but this one names the plane.
    #[test]
    fn every_plane_opens_on_a_list_of_its_own() {
        for plane in Plane::ALL {
            let dest = primary(plane);
            assert!(
                dest.root.is_some(),
                "{plane:?} opens on {}, which has no list for the sidebar to show",
                dest.id
            );
            assert_eq!(
                dest.plane,
                Some(plane),
                "{plane:?} opens on {}, which says it belongs to {:?}",
                dest.id,
                dest.plane
            );
        }
    }

    /// A destination in no plane is unreachable on the desktop, and only
    /// Settings means it — the footer is its way in. Anything else that lands
    /// here is a screen the sidebar cannot open, on a shell whose whole
    /// navigation is the sidebar.
    #[test]
    fn only_settings_belongs_to_neither_half() {
        for dest in DESTINATIONS {
            assert_eq!(
                dest.plane.is_none(),
                dest.id == "settings",
                "{} is in no plane, so nothing in the desktop sidebar reaches it",
                dest.id
            );
        }
    }

    /// The primary is not in its own library, or the sidebar would list the
    /// chat list underneath itself.
    #[test]
    fn a_plane_does_not_file_its_own_list_under_library() {
        for plane in Plane::ALL {
            assert!(
                !library(plane)
                    .iter()
                    .any(|dest| dest.id == primary(plane).id),
                "{plane:?} lists its opening destination in its own library"
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

    /// A subtitle that arrives as `None` has to come out as a crumb with no
    /// subtitle, not as one carrying an empty string: the window bar renders
    /// `.subtitle` whenever it is `Some`, so an empty `Some` is a blank
    /// second element pushing the title off its 52px band.
    #[test]
    fn a_crumb_with_nothing_to_add_is_a_plain_one() {
        assert_eq!(
            Crumb::detailed("Review", None),
            Crumb::plain("Review"),
            "a detail that computed no subtitle must be indistinguishable from \
             a name on its own, or the window bar lays out a second line for \
             nothing"
        );
        let named = Crumb::detailed("Review", Some("4 of 9 files".to_owned()));
        assert_eq!(named.title, "Review");
        assert_eq!(
            named.subtitle.as_deref(),
            Some("4 of 9 files"),
            "the crumb dropped the half that says where the screen lives, so \
             the window bar would name a diff with no repo behind it"
        );
    }

    /// The drawer draws a rule and a header before a group that has one and
    /// nothing before the group that does not. Give `Work` a header and the
    /// top of the list gains a label above the first row; drop `Library`'s and
    /// Recipes, Skills and Scheduler run on from Chats with no break.
    #[test]
    fn only_the_groups_that_want_a_label_carry_one() {
        assert_eq!(
            Group::ALL.map(Group::header),
            [None, Some("Library"), Some("Server")],
            "the drawer's section headers moved"
        );
    }

    /// The switch is the ONE control that names a whole half of the app, and
    /// its glyphs are deliberately the same two the `chats` and `code` rows
    /// already carry — a third pair would be naming the same two things twice
    /// in two alphabets. Change either row's icon without changing the plane's
    /// and the segment stops matching the destination it opens.
    #[test]
    fn the_switch_wears_the_glyphs_of_the_destinations_it_opens() {
        assert_eq!(Plane::ALL.map(Plane::label), ["Chat", "Code"]);
        for plane in Plane::ALL {
            assert_eq!(
                plane.icon(),
                primary(plane).icon,
                "the {plane:?} segment and the destination it opens ({}) draw \
                 different glyphs",
                primary(plane).id
            );
        }
    }

    /// The sidebar reaches a destination through exactly one of three routes:
    /// a plane's opening row, a plane's library, or the footer. A row in none
    /// of them is a screen the desktop cannot open at all, and a row in two is
    /// one the sidebar lists twice.
    ///
    /// Shown to fail: give `settings` a plane and the footer empties, so its
    /// id goes missing from the left side.
    #[test]
    fn the_sidebar_reaches_every_destination_exactly_once() {
        let mut reached: Vec<&'static str> = Plane::ALL
            .into_iter()
            .flat_map(|plane| {
                let mut rows = vec![primary(plane)];
                rows.extend(library(plane));
                rows
            })
            .chain(plane_free())
            .map(|dest| dest.id)
            .collect();
        let listed = reached.len();
        reached.sort_unstable();
        reached.dedup();
        assert_eq!(
            reached.len(),
            listed,
            "the sidebar lists a destination twice: {reached:?}"
        );
        assert_eq!(
            reached,
            ids(DESTINATIONS.iter()),
            "the sidebar's three routes do not add up to the table"
        );
        assert_eq!(
            ids(plane_free()),
            ["settings"],
            "the sidebar footer is not Settings alone"
        );
    }

    /// The drawer's whole promise: tapping a row puts you on that row's root,
    /// and the row then marks itself as where you are.
    ///
    /// Every `go` in the table is a different amount of work — Chats and
    /// Settings name a screen because they share the Home signal, the other
    /// five only set a tab because they own their screen signal — and a row
    /// that sets the wrong one lands somewhere and lights up nothing. Change
    /// Code's `go` to `Tab::Home` and its answer here turns `false` while the
    /// drawer would go on showing the chat list under a highlighted Code.
    #[test]
    fn opening_a_destination_lands_on_its_own_root() {
        assert_eq!(
            ask(|ctx| per_destination(|dest| {
                (dest.go)(ctx);
                format!("{}={}", dest.id, (dest.at_root)(ctx))
            })),
            "chats=true | code=true | recipes=true | skills=true | \
             scheduler=true | extensions=true | settings=true",
            "a destination the drawer opened does not report itself as where \
             you are"
        );
    }

    /// Exactly one row is highlighted at a time. Two would be the drawer
    /// claiming you are in two places; none would be a drawer with nothing lit
    /// while a screen of its own is up.
    ///
    /// Shown to fail: drop `(ctx.tab)() == Tab::Home &&` from Chats' `at_root`
    /// and every count from `code` onward becomes 2, because the Home screen
    /// signal is still parked on `Sessions` underneath the tab you left it for.
    #[test]
    fn arriving_somewhere_leaves_everywhere_else() {
        assert_eq!(
            ask(|ctx| per_destination(|dest| {
                (dest.go)(ctx);
                let lit = DESTINATIONS.iter().filter(|d| (d.at_root)(ctx)).count();
                format!("{}={lit}", dest.id)
            })),
            "chats=1 | code=1 | recipes=1 | skills=1 | scheduler=1 | \
             extensions=1 | settings=1",
            "the drawer highlights something other than exactly one row"
        );
    }

    /// The second of the two rules this module exists to hold: a destination
    /// is "here" only when its stack is at its root. From a chat, Chats is
    /// somewhere to go BACK to — but it is still the destination whose stack
    /// is on screen, which is why `at_root` and `current` have to disagree
    /// here rather than being one function.
    ///
    /// Shown to fail: drop the screen half of any row's `at_root` and that
    /// row answers `true` from two pushes deep, so the drawer says "here"
    /// about a screen you would need the back chevron to reach.
    ///
    /// Settings is the row with no stack to push — one screen, which is its
    /// own detail — and its expectation says so out loud.
    #[test]
    fn a_pushed_screen_is_not_where_the_drawer_says_you_are() {
        assert_eq!(
            ask(|ctx| per_destination(|dest| {
                (dest.go)(ctx);
                push_detail(ctx, dest.id);
                format!("{}={},{}", dest.id, (dest.at_root)(ctx), current(ctx).id)
            })),
            "chats=false,chats | code=false,code | recipes=false,recipes | \
             skills=false,skills | scheduler=false,scheduler | \
             extensions=false,extensions | settings=true,settings",
            "a destination two screens deep either marks itself active or \
             stops owning the stack that is on screen"
        );
    }

    /// THE GALLERY'S KEYS, walked over every screen the app has.
    ///
    /// `docs/style-gallery.html` and `docs/audit.js` are keyed on these exact
    /// names, and a screen whose key changed silently drops out of every check
    /// they run — which has happened to a whole branch of new UI before. The
    /// answers are joined with `+`, so a state claimed by two destinations
    /// prints as `chats+settings` and a state claimed by none prints as an
    /// empty slot: one function per row answers both "is it mounted" and
    /// "what is it showing", and this is what says the answers do not overlap.
    #[test]
    fn every_screen_reports_the_key_the_gallery_captured_it_under() {
        assert_eq!(
            ask(|ctx| {
                let (mut tab, mut screen, mut code) = (ctx.tab, ctx.screen, ctx.code_screen);
                let mut keys = Vec::new();
                let claim = |ctx: &AppCtx| {
                    DESTINATIONS
                        .iter()
                        .filter_map(|dest| (dest.key)(ctx))
                        .collect::<Vec<_>>()
                        .join("+")
                };
                tab.set(Tab::Home);
                for at in [Screen::Sessions, Screen::Chat, Screen::Settings] {
                    screen.set(at);
                    keys.push(claim(ctx));
                }
                tab.set(Tab::Code);
                for at in [
                    CodeScreen::List,
                    CodeScreen::New,
                    CodeScreen::Chat,
                    CodeScreen::Diff,
                    CodeScreen::Pulls,
                ] {
                    code.set(at);
                    keys.push(claim(ctx));
                }
                let mut recipes = ctx.recipes.screen;
                tab.set(Tab::Recipes);
                for at in [crate::recipes::Screen::List, crate::recipes::Screen::Detail] {
                    recipes.set(at);
                    keys.push(claim(ctx));
                }
                let mut skills = ctx.skills.screen;
                tab.set(Tab::Skills);
                for at in [crate::skills::Screen::List, crate::skills::Screen::Detail] {
                    skills.set(at);
                    keys.push(claim(ctx));
                }
                let mut scheduler = ctx.scheduler.screen;
                tab.set(Tab::Scheduler);
                for at in [
                    crate::scheduler::Screen::List,
                    crate::scheduler::Screen::Detail,
                ] {
                    scheduler.set(at);
                    keys.push(claim(ctx));
                }
                let mut extensions = ctx.extensions.screen;
                tab.set(Tab::Extensions);
                for at in [
                    crate::extensions::Screen::List,
                    crate::extensions::Screen::Detail,
                ] {
                    extensions.set(at);
                    keys.push(claim(ctx));
                }
                keys.join(" ")
            }),
            "chats chat settings code-list code-new code-chat code-diff \
             code-pulls recipes-list recipes-detail skills skill scheduler \
             scheduler-detail extensions extensions-detail",
            "a screen's dump key moved, so the gallery and docs/audit.js stop \
             seeing the screen they were keyed on"
        );
    }

    /// A destination sitting on its root has nothing pushed, so the desktop's
    /// third column shows its empty sentence rather than the list again.
    /// Settings is the one row whose detail is unconditional — it has no list,
    /// so the screen IS the detail and the shell draws two columns.
    ///
    /// Shown to fail: make Code's `CodeScreen::List` arm return a `Detail` and
    /// `code=true` here, which on the desktop is the working-tree list drawn
    /// in both columns at once.
    #[test]
    fn a_destination_at_its_root_has_nothing_pushed() {
        assert_eq!(
            ask(|ctx| per_destination(|dest| {
                (dest.go)(ctx);
                format!("{}={}", dest.id, (dest.detail)(ctx).is_some())
            })),
            "chats=false | code=false | recipes=false | skills=false | \
             scheduler=false | extensions=false | settings=true",
            "a destination at its root disagrees with the shell about whether \
             the detail column has anything in it"
        );
    }

    /// The window bar names what the detail column has open, and each row has
    /// to reach for its OWN screen's name.
    ///
    /// This is the failure `Detail`'s own doc comment describes: wire a row's
    /// crumb to a neighbour's expression and the app renders one screen while
    /// the bar names another, with nothing to say so — the phone drops the
    /// crumb on the floor, so no phone gate can see it either. The seeded
    /// titles are ones nothing but the seed can put on screen.
    #[test]
    fn every_detail_names_its_own_screen_in_the_window_bar() {
        assert_eq!(
            ask(|ctx| {
                let (mut chat, mut code_chat) = (ctx.chat, ctx.code_chat);
                chat.write().title = "Rotating the Tailscale cert".to_owned();
                code_chat.write().title = "Waking the worktree".to_owned();
                per_destination(|dest| {
                    (dest.go)(ctx);
                    push_detail(ctx, dest.id);
                    let named = (dest.detail)(ctx)
                        .map_or_else(|| "nothing open".to_owned(), |open| open.crumb.title);
                    format!("{}={named}", dest.id)
                })
            }),
            "chats=Rotating the Tailscale cert | code=Waking the worktree | \
             recipes=Recipe | skills=Skill | scheduler=Scheduled job | \
             extensions=Extension | settings=Settings",
            "the window bar names a screen other than the one the detail \
             column is showing"
        );
    }

    /// Code is the row with five screens rather than two, so it is the one
    /// where the crumb and the view could most easily drift apart — and the
    /// four names below are the whole of what the window bar can say about
    /// the Code plane.
    ///
    /// Shown to fail: swap the `Diff` and `Pulls` arms and the bar reads
    /// "Pull requests" over a diff.
    #[test]
    fn the_code_plane_names_each_of_its_four_pushed_screens() {
        assert_eq!(
            ask(|ctx| {
                let mut code_chat = ctx.code_chat;
                code_chat.write().title = "Waking the worktree".to_owned();
                let mut code = ctx.code_screen;
                [
                    CodeScreen::List,
                    CodeScreen::New,
                    CodeScreen::Chat,
                    CodeScreen::Diff,
                    CodeScreen::Pulls,
                ]
                .into_iter()
                .map(|at| {
                    code.set(at);
                    (CODE.detail)(ctx)
                        .map_or_else(|| "nothing open".to_owned(), |open| open.crumb.title)
                })
                .collect::<Vec<_>>()
                .join(" | ")
            }),
            "nothing open | New code session | Waking the worktree | Review | \
             Pull requests",
            "a Code screen is named after a different Code screen in the \
             window bar"
        );
    }

    /// The heading of the first `<h1 class="title...">` in `markup`, which is
    /// what every list screen in this app calls itself.
    fn heading(markup: &str) -> &str {
        markup
            .split_once("<h1 class=\"title")
            .and_then(|(_, rest)| rest.split_once('>'))
            .and_then(|(_, rest)| rest.split_once("</h1>"))
            .map_or("no heading", |(text, _)| text)
    }

    /// Every list column renders the list that belongs to the row holding it.
    ///
    /// This is the reason the table carries a `root` at all rather than one
    /// "render whatever is on top": the desktop draws the root and the detail
    /// AT ONCE, so a `root` pointed at the wrong view is a middle column
    /// showing another destination's list beside the right detail. The phone
    /// never renders `root` while a detail is up, so no phone gate sees it —
    /// this mounts all six at once and reads back what each one called itself.
    #[test]
    fn every_list_column_heads_itself_with_its_own_destinations_name() {
        let markup = show(|ctx| {
            let ctx = *ctx;
            rsx! {
                for dest in DESTINATIONS {
                    if let Some(root) = dest.root {
                        section { "data-dest": "{dest.id}", {root(&ctx)} }
                    }
                }
            }
        });
        let named: Vec<String> = markup
            .split("data-dest=\"")
            .skip(1)
            .map(|chunk| {
                let (id, rest) = chunk.split_once('"').unwrap_or(("unlabelled", chunk));
                format!("{id}={}", heading(rest))
            })
            .collect();
        assert_eq!(
            named.join(" | "),
            "chats=Chats | code=Code | recipes=Recipes | skills=Skills | \
             scheduler=Scheduler | extensions=Extensions",
            "a destination's list column is headed by another destination's \
             name, so the desktop shows one list beside the other's detail. \
             Rendered {} bytes.",
            markup.len()
        );
    }

    /// The detail column renders the screen its own crumb names.
    ///
    /// The pair is what `Detail` exists to keep together: ONE function returns
    /// both, so that "the app renders a diff and the window bar names the chat
    /// it came from" cannot happen. This mounts every row's detail at once and
    /// checks each pane says the same word the bar was handed — which is also
    /// the desktop's stated rule that there is one title per window, taken out
    /// of the pane by CSS rather than by rendering something else.
    #[test]
    fn each_detail_pane_says_the_same_name_its_crumb_does() {
        let markup = show(|ctx| {
            let ctx = *ctx;
            let (mut chat, mut code_chat) = (ctx.chat, ctx.code_chat);
            chat.write().title = "Rotating the Tailscale cert".to_owned();
            code_chat.write().title = "Waking the worktree".to_owned();
            for dest in DESTINATIONS {
                push_detail(&ctx, dest.id);
            }
            rsx! {
                for dest in DESTINATIONS {
                    if let Some(open) = (dest.detail)(&ctx) {
                        section { "data-dest": "{dest.id}", "data-crumb": "{open.crumb.title}",
                            {open.view}
                        }
                    }
                }
            }
        });
        let checked: Vec<String> = markup
            .split("data-dest=\"")
            .skip(1)
            .map(|chunk| {
                let (id, rest) = chunk.split_once('"').unwrap_or(("unlabelled", chunk));
                let crumb = rest
                    .split_once("data-crumb=\"")
                    .and_then(|(_, after)| after.split_once('"'))
                    .map_or("no crumb", |(title, _)| title);
                format!("{id}: bar {crumb} / pane {}", heading(rest))
            })
            .collect();
        assert_eq!(
            checked.join(" | "),
            "chats: bar Rotating the Tailscale cert / pane Rotating the \
             Tailscale cert | code: bar Waking the worktree / pane Waking the \
             worktree | recipes: bar Recipe / pane Recipe | skills: bar Skill \
             / pane Skill | scheduler: bar Scheduled job / pane Scheduled job \
             | extensions: bar Extension / pane Extension | settings: bar \
             Settings / pane Settings",
            "a detail pane and the crumb handed to the window bar name \
             different screens ({} bytes of markup)",
            markup.len()
        );
    }
}
