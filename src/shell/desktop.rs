//! The desktop shell: a pinned nav, a list, and what the list opens.
//!
//! Three columns — nav, list, detail — and `assets/desktop.css` reflows them
//! to two and then to one as the window narrows. NONE of that is decided here.
//! The breakpoints are `@media` rules in a stylesheet a phone binary does not
//! contain, so pane count costs zero Rust and nothing in this app listens to a
//! resize. What this file decides is what goes IN the columns, which is a
//! question about the destination rather than about the window.
//!
//! What it does not have, on purpose: no swipe tray (the rows carry their
//! actions inline, `src/views/chrome.rs`), no pull-to-refresh (a mount
//! re-fetch and ⌘R below), no hamburger (the nav is already on screen — the
//! rule is in `assets/desktop.css`), and no collapse or drag-resize on the nav.
//! Pinned means pinned.

use dioxus::prelude::*;

use crate::icons::Icon;
use crate::nav::{self, Destination, Group};
use crate::shell::render_group;

/// The smallest the window may be dragged to, in logical points.
///
/// A floor, not a preference, and every part of it is derived.
///
/// 480 wide is the rail plus 424 of content. 424 is wider than 402 — the frame
/// every state in `docs/style-gallery.html` is audited in — and wider than the
/// 360 `docs/measure-composer.js` is gated at, so the single content column at
/// the floor is still inside territory this app is measured in. It is also
/// exactly the `minWidth` goose's own desktop app ships
/// (`ui/desktop/src/main.ts`), which is a shipping precedent in this domain
/// arriving at the same number from the other direction.
///
/// 560 tall is the nav's own intrinsic height with slack: seven destinations
/// at `.drawer-item`'s 48px `min-height` is 336, plus the wordmark and the
/// padding is about 440. Shorter than that is survivable — `.drawer-nav` is
/// `overflow-y: auto` and simply scrolls — but it is the first thing to give,
/// so it is where the floor belongs. goose ships 400; the extra is the
/// composer, which that app does not put in a column.
/// `feature = "desktop"` as well as the module's own target gate: this is a
/// number about a WINDOW, and there is only a window under that feature.
/// Without it, `cargo check --no-default-features --features mobile` on a
/// Mac — a real thing to type, and a configuration no gate covers — compiles
/// this module for `target_os = "macos"` while `main.rs` takes the launch
/// arm that never asks for it.
#[cfg(feature = "desktop")]
pub(crate) const MIN_INNER: (f64, f64) = (480.0, 560.0);

/// Refresh what is on screen. ⌘R on a Mac, Ctrl+R everywhere else.
///
/// A JS listener and not a Rust `onkeydown`, for the reason written above
/// `CLOSE_OPEN_ROW` in `src/viewport.rs`: the native renderer sends every
/// listened-to event through a synchronous XHR, and a document-level keydown
/// handler in Rust would put a blocking round trip on every character typed
/// into the composer. JS owns the chord; Rust hears one message per press.
///
/// What it sends is the same `data-refresh` string the pull gesture sends, so
/// `crate::viewport::refresh_named` is one `match` serving both and a list
/// cannot be refreshable by one route and not the other.
///
/// Every mounted scroller, deduplicated: this shell has a list column and a
/// detail column on screen at once, and "refresh this view" means both. The
/// Scheduler is the case that proves it — its list and its job detail both
/// answer to `scheduler`, and the `Set` is what stops that being two fetches.
///
/// `preventDefault` is load-bearing. Left alone, ⌘R is the web view's own
/// reload, which would throw away the whole app — connection, drafts and all —
/// to re-fetch a list.
const REFRESH_KEY: &str = r"
(() => {
  if (window.__refreshKeyWired) return;
  window.__refreshKeyWired = true;
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'r' && e.key !== 'R') return;
    if (!e.metaKey && !e.ctrlKey) return;
    if (e.altKey || e.shiftKey) return;
    e.preventDefault();
    const names = new Set();
    for (const el of document.querySelectorAll('[data-refresh]')) {
      if (el.dataset.refresh) names.add(el.dataset.refresh);
    }
    for (const name of names) dioxus.send(name);
  });
})();
";

/// Wire ⌘R to the same dispatch the pull gesture uses.
fn use_refresh_key() {
    let ctx = crate::state::use_app_ctx();
    use_effect(move || {
        let mut eval = document::eval(REFRESH_KEY);
        spawn(async move {
            while let Ok(which) = eval.recv::<String>().await {
                crate::viewport::refresh_named(&ctx, &which);
            }
        });
    });
}

/// Re-fetch a destination's list when you arrive at it.
///
/// This is the desktop's whole answer to refresh, and it is goose's own
/// desktop app's answer too: `ui/desktop/src/components/sessions/
/// SessionListView.tsx` has no refresh control anywhere and does not poll the
/// session list — it re-fetches from an effect on mount. A button in the bar
/// is a control that is wrong most of the time it is on screen, which is the
/// policy `src/views/extensions.rs` already states for the phone; the phone
/// answers it with a gesture and the desktop answers it with arrival.
///
/// Keyed on the destination and gated on the connection, which is the same
/// shape four of the six lists already use for their own mount effects
/// (`views/skills.rs`, `views/recipes.rs`, `views/extensions.rs`,
/// `views/scheduler.rs`): re-run when you arrive somewhere new, and re-run
/// when a connection finally arrives under a screen that was already up.
///
/// One rule rather than a list of the destinations that need it. Skills and
/// Scheduler reach the server through `ensure_loaded`, which is a cache and
/// deliberately does NOT re-fetch, so for those this is the only re-fetch
/// there is; Extensions fetches on its own mount as well and will therefore
/// ask twice on arrival. A duplicate GET over Tailscale is cheaper than a
/// hand-kept list of exceptions that goes stale the first time a feature adds
/// a destination.
fn use_arrival_refresh(dest: &'static Destination) {
    let ctx = crate::state::use_app_ctx();
    let id = dest.id;
    use_effect(move || {
        if (ctx.conn)().is_connected() {
            crate::viewport::refresh_named(&ctx, id);
        }
    });
}

/// The desktop shell.
#[component]
pub(crate) fn AppShell() -> Element {
    let ctx = crate::state::use_app_ctx();
    let dest = nav::current(&ctx);

    use_arrival_refresh(dest);
    use_refresh_key();

    // `at_root` already means "nothing is selected" — it is the predicate the
    // drawer has always highlighted with, and it needs no new state to serve
    // as the third column's empty test. A destination with no list of its own
    // (Settings) is one screen, so it takes the content area whole and the
    // shell draws two columns rather than three.
    let detail_open = dest.root.is_none() || !(dest.at_root)(&ctx);

    rsx! {
        // `data-detail` is the one thing the CSS cannot work out for itself,
        // and it is a fact about the app rather than about the window: below
        // the three-column breakpoint the list and the detail share a column,
        // so the sheet has to know which of them has something in it. Two
        // columns is what goose's own desktop app is at every width
        // (`ui/desktop/src/components/schedules/SchedulesView.tsx` returns the
        // detail INSTEAD of the list), so this is the reference layout, not a
        // degraded one.
        div {
            class: "shell",
            "data-detail": if detail_open { "open" } else { "empty" },

            // NOT `.drawer.open`. Three reasons, all load-bearing: `.drawer`
            // is `position: absolute` with `translateX(-100%)` and would have
            // to be fought rather than reused; `src/domdump.rs` files any dump
            // containing `.drawer.open` under a `-drawer` suffix, so every
            // desktop capture would be named as if a panel were over it; and
            // nothing has to move out of `assets/main.css`, because
            // `.drawer-brand`, `.drawer-nav`, `.drawer-item` and
            // `.drawer-group` are all independent of `.drawer` itself.
            aside { class: "navpane",
                h2 { class: "drawer-brand", "goose" }
                nav { class: "drawer-nav",
                    for group in Group::ALL {
                        {render_group(&ctx, group)}
                    }
                }
            }

            if let Some(root) = dest.root {
                section { class: "pane pane-list", {root(&ctx)} }
            }

            section { class: "pane pane-detail",
                if detail_open {
                    {(dest.view)(&ctx)}
                } else {
                    {empty_detail(dest)}
                }
            }
        }
    }
}

/// What the third column says when nothing is selected.
///
/// The state with no precedent to copy: goose's own desktop app never has it,
/// because there the detail replaces the list rather than sitting beside it.
/// It is also the state a three-column app is in the moment it launches, so a
/// blank column is not an option.
///
/// Worded off the destination table rather than per screen, so a feature that
/// adds a row gets this for free and cannot forget it.
fn empty_detail(dest: &'static Destination) -> Element {
    rsx! {
        div { class: "pane-empty",
            Icon { name: dest.icon }
            p { class: "pane-empty-line", "Nothing open" }
            p { class: "pane-empty-hint", "Pick something from {dest.label} to see it here." }
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "desktop")]
    use super::MIN_INNER;
    use crate::nav::DESTINATIONS;

    // The layout's numbers. None of them appears in the binary — the
    // columns are drawn by `assets/desktop.css` and nothing else, which is
    // exactly what keeps every resize listener out of this app. They live
    // here because being checkable is the only job they have.
    /// The pinned nav's width, and the mockups' number.
    ///
    /// Inside goose's own desktop app's band for the same panel — it defaults to
    /// 240 and clamps to 160..400 (`ui/desktop/src/components/Layout/constants.ts`,
    /// `NavigationContext.tsx`) — so 212 is a measured width rather than a taste.
    const NAV: u32 = 212;

    /// The list column's width, from the mockups.
    const LIST: u32 = 330;

    /// The narrowest a content column is allowed to get.
    ///
    /// The app's own floor, not a guess: `docs/design.md` records that "320pt is a
    /// defensive width rather than a device; the narrowest this app really meets
    /// is 360", and `node docs/measure-composer.js 360` is a gate. Below it the
    /// composer's chip row is documented to fail.
    const CONTENT_MIN: u32 = 360;

    /// The nav's width once it drops its labels.
    const RAIL: u32 = 56;

    /// Three columns need all three of them, so the breakpoint is their sum.
    ///
    /// The mockups say "900". This is that number with the arithmetic done, which
    /// is the difference between a breakpoint and a round number: raise `LIST` and
    /// this follows.
    pub(crate) const THREE_PANE: u32 = NAV + LIST + CONTENT_MIN;

    /// Two columns need the nav and one content column.
    pub(crate) const TWO_PANE: u32 = NAV + CONTENT_MIN;

    /// The window minimum and the stylesheet's breakpoints are one decision
    /// held in two languages, and only one of them can be compiled. Raise the
    /// floor above the two-column breakpoint and the narrowest layout becomes
    /// unreachable — designed, styled, and impossible to see; lower it and the
    /// window opens onto a width no tier covers.
    #[cfg(feature = "desktop")]
    #[test]
    fn the_window_floor_lands_inside_the_narrowest_tier() {
        let (width, height) = MIN_INNER;
        assert!(
            width < f64::from(TWO_PANE),
            "a {width}pt floor is at or above the {TWO_PANE}pt two-column \
             breakpoint, so the one-column tier can never be reached"
        );
        assert!(
            width >= f64::from(RAIL + CONTENT_MIN),
            "a {width}pt floor leaves the one content column narrower than \
             the {CONTENT_MIN}pt this app is measured at"
        );
        assert!(height > 0.0);
    }

    /// Pane count is decided entirely inside `assets/desktop.css`, which is
    /// exactly what keeps every resize listener out of this app — and it is
    /// also what puts the two numbers out of the compiler's reach. This is the
    /// only thing that notices when the arithmetic above and the sheet stop
    /// agreeing.
    #[test]
    fn the_stylesheet_breaks_where_the_arithmetic_says() {
        let sheet = include_str!("../../assets/desktop.css");
        for edge in [THREE_PANE, TWO_PANE] {
            let rule = format!("@media (max-width: {}px)", edge - 1);
            assert!(
                sheet.contains(&rule),
                "assets/desktop.css has no `{rule}`, so the columns do not \
                 reflow where src/shell/desktop.rs says they do"
            );
        }
    }

    /// Arriving at a destination is the desktop's whole refresh story, and it
    /// goes through the same name-keyed `match` the pull gesture uses. A
    /// destination whose id has no arm there simply never re-fetches — no
    /// error, no warning, because the fallthrough arm is legitimately there
    /// for the scrollers that name nothing. This is the same trap
    /// `src/viewport.rs`'s own test guards from the other end.
    #[test]
    fn every_destination_with_a_list_can_be_refreshed_by_name() {
        let dispatch = include_str!("../viewport.rs");
        for dest in DESTINATIONS {
            if dest.root.is_none() {
                continue;
            }
            assert!(
                dispatch.contains(&format!("\"{}\" =>", dest.id)),
                "arriving at {} on the desktop refreshes nothing: \
                 refresh_named has no arm for that name",
                dest.id
            );
        }
    }
}
