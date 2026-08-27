//! Which shell the app wears.
//!
//! PLATFORM decides this, and only platform. A desktop window dragged narrow
//! is a narrow DESKTOP app: its rows keep their inline action icons, it still
//! has no swipe tray and no pull-to-refresh, and the nav stays pinned. It does
//! not become the phone. The mobile shell is not compiled into a Mac binary at
//! all, so it cannot appear there by accident, and the desktop shell cannot
//! appear on a phone.
//!
//! WIDTH decides one thing, and it is not decided here: how many columns
//! `assets/desktop.css` draws. That sheet is desktop-only (see [`crate::css`]),
//! so its `@media (min-width: …)` rules are physically unreachable from a phone
//! binary — which is why no Rust in this app observes a window resize. The rule
//! in `src/viewport.rs` (the native renderer sends every listened-to event
//! through a synchronous XHR) is honoured here by having nothing to listen to.
//!
//! `target_os` rather than `feature`, following `src/css.rs`: the
//! `cargo check --target aarch64-apple-ios` gate runs with DEFAULT features,
//! so a `feature`-gated mobile arm would compile nowhere that gate can see.
//! `Cargo.toml` now says the same thing about the renderer itself — the
//! `dioxus/desktop` feature is turned on by a target-conditional dependency
//! table rather than by a flag on the command line — so there is no
//! configuration in which the two disagree.

use dioxus::prelude::*;

use crate::icons::Icon;
use crate::nav::{Destination, Group, DESTINATIONS};
use crate::state::AppCtx;

#[cfg(any(target_os = "ios", target_os = "android"))]
mod mobile;
#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) use mobile::AppShell;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
mod desktop;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub(crate) use desktop::AppShell;
/// The window's floor, re-exported for `main.rs`, which is where a window is
/// built. The number and its derivation live beside the breakpoints it has to
/// agree with — and beside the test that checks that it does.
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub(crate) use desktop::MIN_INNER;

/// Which shell is compiled in, as a VALUE.
///
/// The component above is `cfg`-gated, because only one shell is ever built.
/// The presentation rules that hang off the platform are not: they are total
/// functions of this enum, so `cargo test` runs BOTH arms in one process on
/// any host. A `#[cfg]` at each of those leaves would mean the mobile arm was
/// verified by nothing at all — `cargo test` builds for the host, which is a
/// desktop target, and the iOS gate is a `cargo check`, which proves only
/// that the arm parses.
///
/// That matters more here than it usually would: the promise this whole branch
/// makes is that mobile rendering does not change, and a claim no test can
/// execute is a claim nobody can check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shell {
    Mobile,
    Desktop,
}

impl Shell {
    /// The one platform `cfg` in the shell.
    pub(crate) const CURRENT: Self = if cfg!(any(target_os = "ios", target_os = "android")) {
        Self::Mobile
    } else {
        Self::Desktop
    };
}

// Not decoration. `cargo check --target aarch64-apple-ios` proves the mobile
// arm compiles and nothing else in this repo proves it was SELECTED — and the
// list above is a list, so the failure mode is a target quietly missing from
// it. Android is exactly that case: nothing in CI builds for Android today, so
// if `target_os = "android"` were dropped here the Android build would take
// the desktop shell and no gate would say so. These make the check say it.
#[cfg(any(target_os = "ios", target_os = "android"))]
const _: () = assert!(matches!(Shell::CURRENT, Shell::Mobile));
#[cfg(not(any(target_os = "ios", target_os = "android")))]
const _: () = assert!(matches!(Shell::CURRENT, Shell::Desktop));

/// What this shell's DOM dumps are keyed under, prefixed onto the destination
/// key in `src/app.rs`.
///
/// `docs/gallery-states.json` is ONE store and both shells write into it. The
/// key `src/domdump.rs` emits is `Destination::key`, which is a fact about the
/// destination and identical on both — so a desktop `chats` and a phone
/// `chats` are the same string, and whichever log was read last would win.
/// Nothing would say so: the store would still hold 49 plausible states and
/// `docs/audit.js` would go on reporting them clean while auditing one shell's
/// markup against the other's list of screens.
///
/// It cannot be done in `DUMP_JS`. That string is installed by a hook `app.rs`
/// calls ABOVE the shell split — deliberately, because which markup a
/// destination produces is the shell's business and what the dump is called is
/// not — and the browser has no way to ask which binary it is running in. So
/// the one thing the JS cannot know is handed to it as a value, from the one
/// module that is `cfg`'d on exactly that question.
///
/// The phone's prefix is the empty string and that is load-bearing, not a
/// default: the 49 keys already in the store, the `LABELS` table in
/// `scripts/capture-gallery.py` and every phone finding `docs/audit.js` has
/// ever reported are all keyed on the bare name. A prefix on that side would
/// silently retire all of them.
///
/// `debug_assertions` only, following `src/domdump.rs` — a release build
/// contains no dump machinery and would carry this as dead code.
#[cfg(debug_assertions)]
#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) const DUMP_PREFIX: &str = "";

#[cfg(debug_assertions)]
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub(crate) const DUMP_PREFIX: &str = DUMP_PREFIX_DESKTOP;

/// What a DESKTOP state's key starts with, whatever this binary is.
///
/// [`DUMP_PREFIX`] answers "what do MY dumps get called", which is a different
/// question from "which of these captured states is the desktop's" — and the
/// tests that read `docs/gallery-states.json` ask the second one. They run on
/// this host, which is a desktop target, so asking them through `DUMP_PREFIX`
/// would work today and mean the wrong thing: it would resolve to `""` in a
/// build for a phone and filter the whole store away.
///
/// `scripts/capture-gallery.py` and `docs/audit.js` both partition on this
/// same string, and neither can be reached from here — so it is a literal in
/// three places, and the two of them that are checkable by a compiler are the
/// two in this file.
///
/// Dead on a phone, and it has to stay compiled there anyway. `DUMP_PREFIX` is
/// the only non-test reader and on iOS it resolves to `""` from the arm above,
/// so the constant is reachable from nothing but `#[cfg(test)]` code — which
/// `cargo check --package goose-mobile --target aarch64-apple-ios` does not
/// build. CI runs that command with `RUSTFLAGS: -D warnings`, so `dead_code`
/// there is a red build. Deleting it is not the fix: the tests that read
/// `docs/gallery-states.json` have to ask "which of these is the desktop's"
/// on every target, and the answer must not depend on which one they are
/// compiled for.
#[cfg(debug_assertions)]
#[cfg_attr(
    any(target_os = "ios", target_os = "android"),
    expect(
        dead_code,
        reason = "read only by tests and by the desktop arm of DUMP_PREFIX; \
                  the iOS check builds neither"
    )
)]
pub(crate) const DUMP_PREFIX_DESKTOP: &str = "desktop-";

// The phone's half of that claim, checked at COMPILE time on the targets it is
// about, in the shape `src/css.rs` already uses for the same kind of promise.
// A phone binary that started prefixing would rename all 49 captured states at
// once, and the only symptom would be an audit that has quietly stopped
// covering the screens it says it covers.
#[cfg(debug_assertions)]
#[cfg(any(target_os = "ios", target_os = "android"))]
const _: () = assert!(DUMP_PREFIX.is_empty());

/// The tooltip on a destination button.
///
/// `None` on the phone, where nothing hovers and the label is right there.
///
/// `Some` on the desktop, and it is not decoration there: at the narrowest
/// width the pinned nav collapses to a 56px icon rail, and a column of glyphs
/// with no way to ask what they are is a navigation you have to learn. The
/// label itself stays in the DOM at that width — `assets/desktop.css`
/// collapses it to a zero-size box rather than removing it — so the button's
/// accessible name is unchanged and this is the pointer's copy of it.
///
/// Returned as data so the phone's answer is a thing a test can hold. `None`
/// is what keeps the drawer's markup byte-identical: Dioxus omits an attribute
/// whose value is `None`, so the phone's button gains nothing at all.
pub(crate) const fn nav_tooltip(shell: Shell, label: &'static str) -> Option<&'static str> {
    match shell {
        Shell::Mobile => None,
        Shell::Desktop => Some(label),
    }
}

/// Whether a destination paints itself as where you are.
///
/// Two shells, two questions, because the two shells put the nav in different
/// relationships to the screen.
///
/// On the PHONE the drawer is an overlay over one screen, and the rule
/// `src/nav.rs` states is that a destination is "here" only when its stack is
/// at its root: from a chat, Chats is somewhere to go *back* to. Unchanged —
/// the mobile arm is `at_root` and nothing else, which is the expression this
/// replaces, verbatim.
///
/// On the DESKTOP the nav is pinned beside the columns it navigates, and
/// `at_root` said something false there. Open anything and the pill went out:
/// Chats' own list was still on screen, the chat it opened was in the column
/// beside it, and NO destination in the nav was marked — while the list, one
/// column over, was marking the open row. Highlight where you are not, no
/// highlight where you are. So the desktop asks the question its layout can
/// answer: is this destination's stack the one on screen, at whatever depth.
/// That is `Destination::key`, which is already the app's answer to "which
/// destination is showing" (`nav::current` finds the current one with it), so
/// the pill cannot disagree with the columns.
///
/// Two `bool`s rather than the destination and the context, for
/// [`nav_tooltip`]'s reason: a rule taken as data is a rule a test can hold,
/// and the promise this branch is under is about the arm that no test run on
/// this host ever executes.
pub(crate) const fn nav_is_active(shell: Shell, at_root: bool, on_screen: bool) -> bool {
    match shell {
        Shell::Mobile => at_root,
        Shell::Desktop => on_screen,
    }
}

/// One labelled band of destinations.
///
/// Moved here from `app.rs` unchanged, because the overlay drawer and the
/// pinned desktop nav paint the same list off the same table: two copies would
/// disagree the first time a feature adds a row to `nav::DESTINATIONS`.
///
/// An empty group renders nothing at all, header included: until the features
/// land there is no Library to head, and a heading over a gap is the app
/// promising something it does not have.
///
/// The `drawer_open.set(false)` at the end is the mobile shell's — the desktop
/// nav has no drawer to close and the write lands on a signal nothing there
/// reads. It stays because this function is the mobile drawer's body verbatim,
/// and "verbatim" is what makes the no-change-to-mobile promise checkable by
/// reading the diff.
pub(crate) fn render_group(ctx: &AppCtx, group: Group) -> Element {
    let items: Vec<&'static Destination> = DESTINATIONS
        .iter()
        .filter(|dest| dest.group == group)
        .collect();
    if items.is_empty() {
        return rsx! {};
    }
    let ctx = *ctx;

    rsx! {
        if let Some(header) = group.header() {
            div { class: "drawer-group", "{header}" }
        }
        for dest in items {
            button {
                key: "{dest.id}",
                class: if nav_is_active(
                    Shell::CURRENT,
                    (dest.at_root)(&ctx),
                    (dest.key)(&ctx).is_some(),
                ) { "drawer-item active" } else { "drawer-item" },
                title: nav_tooltip(Shell::CURRENT, dest.label),
                onclick: move |_| {
                    (dest.go)(&ctx);
                    let mut open = ctx.drawer_open;
                    open.set(false);
                },
                Icon { name: dest.icon }
                "{dest.label}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{nav_is_active, nav_tooltip, Shell, DUMP_PREFIX_DESKTOP};

    /// The phone's rule is `at_root` and only `at_root` — a destination one
    /// push deep is somewhere to go back to, not where you are — and the
    /// desktop's is "this stack is the one on screen", at any depth. The
    /// interesting row is the last one: one push into a destination, which is
    /// where the two shells part company and where the desktop's pill used to
    /// go out.
    #[test]
    fn the_phone_marks_a_root_and_the_desktop_marks_a_section() {
        // Somewhere else entirely: neither marks it.
        assert!(!nav_is_active(Shell::Mobile, false, false));
        assert!(!nav_is_active(Shell::Desktop, false, false));

        // At this destination's root: both mark it.
        assert!(nav_is_active(Shell::Mobile, true, true));
        assert!(nav_is_active(Shell::Desktop, true, true));

        // Here, one push deep. The phone says "back", the desktop says "here".
        assert!(!nav_is_active(Shell::Mobile, false, true));
        assert!(nav_is_active(Shell::Desktop, false, true));
    }

    /// The drawer's markup is frozen: `None` is how a Dioxus attribute is
    /// omitted entirely, so the phone's destination buttons gain nothing.
    ///
    /// Checked against the SHIPPED markup and not only against this file.
    /// `docs/gallery-states.json` is 49 phone states dumped out of the app on
    /// a device and never hand-edited; every destination button in it opens
    /// `class="drawer-item…" data-dioxus-id=` with nothing between, so an
    /// added attribute — which is precisely what the desktop arm adds — has
    /// nowhere to hide.
    ///
    /// The PHONE's half of the store, since it now holds both shells. The
    /// desktop's own states are full of exactly the attribute this forbids,
    /// which is the whole reason the desktop arm returns `Some`: reading them
    /// here would fail the test by finding the feature working.
    #[test]
    fn the_phone_drawer_gains_no_attribute() {
        assert_eq!(nav_tooltip(Shell::Mobile, "Chats"), None);
        assert_eq!(nav_tooltip(Shell::Desktop, "Chats"), Some("Chats"));

        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/gallery-states.json");
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let states: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&raw).unwrap_or_default();
        assert!(!states.is_empty(), "cannot read {}", path.display());

        let mut seen = 0_usize;
        for (_, markup) in states
            .iter()
            .filter(|(key, _)| !key.starts_with(DUMP_PREFIX_DESKTOP))
        {
            for tail in markup.split("<button class=\"drawer-item").skip(1) {
                seen += 1;
                let head: String = tail.chars().take_while(|c| *c != '>').collect();
                assert!(
                    head.contains("data-dioxus-id=\"") && !head.contains("title="),
                    "a captured destination button reads `<button \
                     class=\"drawer-item{head}>` — the phone's drawer has \
                     gained an attribute it did not ship with"
                );
            }
        }
        assert!(seen > 0, "no captured state contains a destination button");
    }

    /// The host runs the desktop arm, so every rule keyed off the shell has to
    /// take it as a PARAMETER rather than read `Shell::CURRENT` — otherwise
    /// the mobile assertions in `views::chrome` silently assert about desktop
    /// and pass. This test is here to say that out loud where the enum is
    /// defined; it is the one place `CURRENT` is read in a test on purpose.
    #[test]
    fn a_test_run_is_the_desktop_arm_so_mobile_rules_must_be_passed_the_shell() {
        assert_eq!(Shell::CURRENT, Shell::Desktop);
    }

    /// `Cargo.toml`'s `[features]` table is a MARKER for `dx`, not a switch,
    /// and it is enabled in every build — phones included. Measured two ways:
    /// a `compile_error!` under `cfg(all(feature = <marker>, target_os =
    /// "ios"))` fires under `cargo check --target aarch64-apple-ios`, and
    /// `dx build --platform ios --verbose` reports
    /// `features: ["desktop", "dioxus/mobile"]`. So a `cfg` on that feature
    /// name is TRUE on an iPhone: it says the opposite of what it reads as.
    ///
    /// That is a trap with precedent baked in. Until this commit, eight
    /// `cfg`s in this crate were spelled with exactly that feature name, so
    /// `git log` makes the wrong thing look like house style — and the arm it
    /// would silently enable is the window-building one. The name cannot be
    /// changed out of the trap either: `dioxus-cli-0.7.10/src/platform.rs:188`
    /// matches only `web`/`desktop`/`mobile`/`native`/`liveview`/`server`, and
    /// renaming the key to anything else breaks `dx check` outright. A test is
    /// the only ceiling on offer, and this is it — platform is `target_os`,
    /// full stop.
    #[test]
    fn no_cfg_reads_the_dx_marker_feature() {
        // Spelled in halves so this file is not its own first hit; comment
        // lines are dropped below so the prose above is free to say it plainly.
        let quote = '"';
        let needles = [
            format!("feature = {quote}desktop{quote}"),
            format!("feature = {quote}mobile{quote}"),
        ];

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![root.clone()];
        let mut scanned = 0_usize;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                scanned += 1;
                let source = std::fs::read_to_string(&path).unwrap_or_default();
                let code = source
                    .lines()
                    .filter(|line| !line.trim_start().starts_with("//"))
                    .collect::<Vec<_>>()
                    .join("\n");
                for needle in &needles {
                    assert!(
                        !code.contains(needle.as_str()),
                        "{} reads `{needle}`. Cargo.toml's [features] table is \
                         a marker for dx that is enabled in EVERY build, \
                         phones included, so that cfg is true on an iPhone. \
                         Gate on `target_os` instead.",
                        path.display(),
                    );
                }
            }
        }
        assert!(
            scanned > 10,
            "only walked {scanned} files under {} — the scan found nothing to \
             guard, which is a broken test rather than a clean tree",
            root.display(),
        );
    }

    /// The two manifest lines that only `dx` reads, held in place by the one
    /// gate that can see them. Neither has a compiler behind it: every cargo
    /// gate in CI passes with both deleted, and the damage only shows up in
    /// `dx check` — which CI does not run and a contributor meets as a broken
    /// dev loop.
    ///
    /// Both failures were reproduced rather than reasoned about. Delete the
    /// featureless `dioxus` entry and `dx check` says "Could not autodetect
    /// mobile platform. Use --ios or --android instead.", because
    /// `dioxus-cli-0.7.10/src/build/renderer.rs:17` does
    /// `dependencies.iter().find(|dep| dep.name == "dioxus")` with no target
    /// filter and lands on the `mobile` table instead. Delete `[features]` and
    /// it says "Could not automatically detect target triple". The first cost
    /// an earlier pass a round; the second is the one that reads as dead
    /// weight, since it enables nothing.
    #[test]
    fn the_manifest_lines_that_only_dx_reads_are_still_there() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&path).unwrap_or_default();
        let code = manifest
            .lines()
            .filter(|line| !line.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n");

        let first_dioxus = code
            .lines()
            .find(|line| line.trim_start().starts_with("dioxus ="))
            .unwrap_or_default();
        assert!(
            first_dioxus.contains("features = []"),
            "the first `dioxus =` entry in {} is `{first_dioxus}` — dx reads \
             that one entry to autodetect a platform and must find it \
             featureless, or `dx check` and bare `dx build` stop working",
            path.display(),
        );

        let quote = '"';
        assert!(
            code.contains(&format!("default = [{quote}desktop{quote}"))
                && code.contains("desktop = []"),
            "{} has lost the dx marker feature; `dx check` now fails with \
             \"Could not automatically detect target triple\"",
            path.display(),
        );
    }
}
