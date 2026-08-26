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
/// `preventDefault` is load-bearing, and it is taken for the WHOLE
/// modifier+`r` family rather than just the plain chord. ⌘R and ⌘⇧R are both
/// the web view's own reload, which would throw away the whole app —
/// connection, drafts and all — to re-fetch a list; swallowing one and letting
/// the other through would mean a stray Shift reloaded the app. So the guard
/// on the other modifiers runs AFTER `preventDefault`, not before it, and the
/// case-insensitive compare is what makes one test cover ⇧ and Caps Lock
/// alike.
///
/// That is enough on macOS, which is what this app targets. On Windows it
/// would not be: `WebView2` handles its browser accelerator keys ahead of web
/// content, so Ctrl+R can reload before this listener is consulted, and the
/// fix there is `AreBrowserAcceleratorKeysEnabled(false)` in the wry config
/// rather than anything in this string. Written down rather than done, because
/// Windows is "if it falls out for free" and there is no Windows run to check
/// it against.
const REFRESH_KEY: &str = r"
(() => {
  if (window.__refreshKeyWired) return;
  window.__refreshKeyWired = true;
  document.addEventListener('keydown', (e) => {
    if (!e.key || e.key.toLowerCase() !== 'r') return;
    if (!e.metaKey && !e.ctrlKey) return;
    e.preventDefault();
    if (e.altKey || e.shiftKey) return;
    const names = new Set();
    for (const el of document.querySelectorAll('[data-refresh]')) {
      if (el.dataset.refresh) names.add(el.dataset.refresh);
    }
    for (const name of names) dioxus.send(name);
  });
})();
";

/// Escape closes what is open over the page.
///
/// The one keyboard expectation a Mac user brings to a dialog, and the app had
/// no answer for it: with "Delete this chat?" up, Escape did nothing at all.
///
/// Entirely in JS, with no message back to Rust, and that is the design rather
/// than a shortcut. Every sheet in this app owns its own `use_signal` in its
/// own view — `ctx.scheduler.sheet`, a `confirm_delete` local, a `menu` local —
/// so a Rust-side Escape would mean a registry of open dialogs for the shell to
/// close, which is state this shell does not have and should not grow. What it
/// does instead is press the control that is already on screen and already
/// means cancel, which is why a dialog that gains a third button gets this for
/// free and a dialog that removes its cancel correctly stops answering.
///
/// WHICH control is the whole care in this string. A `.modal-actions` button
/// cannot be picked by position: the permission prompt's buttons come from the
/// backend in the backend's order, and `views/chat.rs`'s
/// `permission_button_class` paints "Always allow" as `.btn.secondary` — so
/// "click the secondary" would answer a permission request with the broadest
/// possible grant. `p.modal-body` is rendered by `views::mod::Confirm` and by
/// nothing else in the app, so it is the discriminator: inside a Confirm,
/// Cancel is `.btn.secondary` and is the first action; anywhere else the only
/// safe move is the backdrop, which dismisses the sheets that opted into it
/// (rename, the overflow menu, the pickers) and correctly does nothing to a
/// permission prompt, which is a question that has to be answered.
///
/// `preventDefault` only when something was actually dismissed, so Escape
/// keeps whatever meaning it has elsewhere — a native `<select>`, an IME —
/// when no dialog is up.
const DISMISS_KEY: &str = r"
(() => {
  if (window.__dismissKeyWired) return;
  window.__dismissKeyWired = true;
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return;
    const back = document.querySelector('.modal-backdrop');
    if (!back) return;
    const dialog = back.querySelector('.modal');
    const cancel = dialog && dialog.querySelector('.modal-body')
      ? dialog.querySelector('.modal-actions > .btn.secondary')
      : null;
    e.preventDefault();
    if (cancel) cancel.click(); else back.click();
  });
})();
";

/// Wire Escape to the cancel that is already on screen.
fn use_dismiss_key() {
    use_effect(|| {
        document::eval(DISMISS_KEY);
    });
}

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
    use_arrival(dest.id, move |id| {
        if (ctx.conn)().is_connected() {
            crate::viewport::refresh_named(&ctx, id);
        }
    });
}

/// Run `arrive` with the destination's id whenever the destination changes —
/// and whenever anything `arrive` itself reads changes.
///
/// The mirror into a signal is the whole hook, and it is not ceremony.
/// `AppShell` is an unkeyed child of `.app` (`src/app.rs`), so it mounts once
/// for the life of the process and navigating only re-renders it; a Dioxus
/// effect re-runs only when a signal READ INSIDE THE CLOSURE changes
/// (`dioxus-hooks-0.7.10/src/use_effect.rs` subscribes a `ReactiveContext` to
/// exactly those reads). A `&'static str` captured by move is not a signal, so
/// an effect that closed over the id directly would fire once, at launch, on
/// whichever destination happened to be up — `state.rs` starts every run on
/// Settings, which is the one id `refresh_named` has no arm for — and then
/// never again. Arrival would refresh nothing, ever, with nothing to say so.
///
/// `src/domdump.rs` carries the same three lines for the same reason, and its
/// comment is the one this hook was written from. The four views with mount
/// effects of their own get away with capturing by move because those effects
/// live in the VIEW, which really is unmounted and remounted on arrival.
///
/// It takes the closure rather than returning the signal so that the whole
/// mechanism is in one place: the test below drives this function through a
/// mount and three navigations with a recording closure, which is the only
/// thing that can notice the bug above coming back.
fn use_arrival(id: &'static str, mut arrive: impl FnMut(&'static str) + 'static) {
    let mut current = use_signal(|| id);
    if *current.peek() != id {
        current.set(id);
    }
    use_effect(move || arrive(current()));
}

/// The desktop shell.
#[component]
pub(crate) fn AppShell() -> Element {
    let ctx = crate::state::use_app_ctx();
    let dest = nav::current(&ctx);

    use_arrival_refresh(dest);
    use_refresh_key();
    use_dismiss_key();

    // The destination's own answer to "is anything open", which is the fact
    // the third column needs and needs no new state to hold. A destination
    // with no list of its own (Settings) is one screen, so its detail is
    // unconditional: it takes the content area whole and the shell draws two
    // columns rather than three.
    let detail = (dest.detail)(&ctx);
    let detail_open = detail.is_some();

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
                if let Some(detail) = detail {
                    {detail}
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
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test scaffolding: a harness that cannot start is the failing check"
)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use dioxus::prelude::*;

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

    /// The list column's FLOOR, from the mockups.
    ///
    /// The column is `clamp(330px, 30%, 460px)` in `assets/desktop.css` — at a
    /// flat 330 the widest window was the one a list read worst in, with every
    /// title ellipsised beside a detail pane that was mostly empty. 330 is
    /// still the number the breakpoint is built from, because the clamp is at
    /// its floor everywhere near it.
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

    // ---- the arrival effect, driven through a real VirtualDom ----------
    //
    // The test above proves every id has somewhere to land. This one proves
    // anything is ever thrown: the two halves are independent, and the bug
    // this shell shipped its first draft with was entirely in the second.

    thread_local! {
        /// Every id `use_arrival` handed its closure, in order.
        static ARRIVALS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
        /// The probe's "where am I", so the test can navigate it.
        static WHEREABOUTS: RefCell<Option<Signal<&'static str>>> = const {
            RefCell::new(None)
        };
    }

    /// `AppShell` in miniature: one long-lived component that re-renders on
    /// navigation and never unmounts, which is the whole reason the effect
    /// needs a reactive dependency.
    #[component]
    fn ArrivalProbe() -> Element {
        let here = use_signal(|| "chats");
        WHEREABOUTS.with(|slot| *slot.borrow_mut() = Some(here));
        super::use_arrival(here(), |id| {
            ARRIVALS.with(|log| log.borrow_mut().push(id));
        });
        rsx! { div { "{here}" } }
    }

    /// Let the queued effects run. Dioxus queues an effect and re-runs it from
    /// a task, so nothing fires without an executor to poll one — which is why
    /// a plain `rebuild_in_place` sees the mount and no navigation at all.
    fn settle(dom: &mut VirtualDom) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        rt.block_on(async {
            for _ in 0..6 {
                let _ = tokio::time::timeout(Duration::from_millis(20), dom.wait_for_work()).await;
                dom.render_immediate_to_vec();
            }
        });
    }

    /// Arriving somewhere has to actually fire — every time, not once.
    ///
    /// This is the one thing no static check in this file can see. A Dioxus
    /// effect re-runs only when a signal it READ changes, `AppShell` mounts
    /// once for the life of the process, and a destination id is a
    /// `&'static str`; close over it directly and the desktop's automatic
    /// refresh runs at launch, on whichever screen happened to be up, and
    /// never again. No panic, no warning, no failing test — just lists as old
    /// as the window.
    #[test]
    fn arriving_somewhere_new_dispatches_every_time() {
        let mut dom = VirtualDom::new(ArrivalProbe);
        dom.rebuild_in_place();
        settle(&mut dom);
        let here = WHEREABOUTS
            .with(|slot| *slot.borrow())
            .expect("the probe rendered, so it published its signal");

        for dest in ["code", "recipes", "skills"] {
            dom.in_runtime(|| {
                let mut here = here;
                here.set(dest);
            });
            settle(&mut dom);
        }

        let seen = ARRIVALS.with(|log| log.borrow().clone());
        assert_eq!(
            seen,
            vec!["chats", "code", "recipes", "skills"],
            "the arrival effect fired {} time(s) across a mount and three \
             navigations — it is not re-running on arrival, so the desktop's \
             re-fetch-on-mount is dead and ⌘R is the only refresh there is",
            seen.len()
        );
    }
}
