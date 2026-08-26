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
//! which is `desktop`, so a `feature`-gated mobile arm would compile nowhere
//! that gate can see.

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
#[cfg(all(
    feature = "desktop",
    not(any(target_os = "ios", target_os = "android"))
))]
pub(crate) use desktop::MIN_INNER;

/// Which shell is compiled in, as a VALUE.
///
/// The component above is `cfg`-gated, because only one shell is ever built.
/// The presentation rules that hang off the platform are not: they are total
/// functions of this enum, so `cargo test` runs BOTH arms in one process on
/// any host. A `#[cfg]` at each of those leaves would mean the mobile arm was
/// verified by nothing at all — `cargo test` takes the default features
/// (`desktop`), and the iOS gate is a `cargo check`, which proves only that
/// the arm parses.
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
                class: if (dest.at_root)(&ctx) { "drawer-item active" } else { "drawer-item" },
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
    use super::{nav_tooltip, Shell};

    /// The drawer's markup is frozen: `None` is how a Dioxus attribute is
    /// omitted entirely, so the phone's destination buttons gain nothing.
    ///
    /// Checked against the SHIPPED markup and not only against this file.
    /// `docs/gallery-states.json` is 49 states dumped out of the app on a
    /// device and never hand-edited; every destination button in it opens
    /// `class="drawer-item…" data-dioxus-id=` with nothing between, so an
    /// added attribute — which is precisely what the desktop arm adds — has
    /// nowhere to hide.
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
        for markup in states.values() {
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
}
