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
//! re-fetch and ⌘R below), no hamburger (the phone's is hidden by
//! `assets/desktop.css`; the nav's own toggle is below), and no drag-resize on
//! the nav.
//!
//! The nav DOES collapse, and that is the one thing about it the window gets a
//! say in. It starts open every launch and nothing persists the choice: the
//! default is the feature, and a nav that remembered being shut would open one
//! day into a window with no navigation in it and no explanation.

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
/// No gate of its own: this module is compiled only where a window exists,
/// because `src/shell/mod.rs` declares it under
/// `not(any(target_os = "ios", target_os = "android"))` and `Cargo.toml`
/// gives `dioxus` its `desktop` feature under that same predicate. There is
/// no longer a flag that can put the two out of step.
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

/// ⌘/ shows and hides the nav. Ctrl+/ everywhere else.
///
/// The chord is goose's own: `toggleNavigation: 'CommandOrControl+/'`
/// (`ui/desktop/src/utils/settings.ts`), bound there to a View ▸ Toggle
/// Navigation menu item. Two apps that look like one product should not
/// disagree about the key that hides the same panel.
///
/// JS rather than a Rust `onkeydown`, for `REFRESH_KEY`'s reason — a
/// document-level keydown handler in Rust puts a blocking synchronous XHR on
/// every character typed into the composer.
///
/// `e.code` is not consulted and `e.key` is, deliberately: on a US layout the
/// slash is `Slash`, but on AZERTY it is Shift+colon and on a German layout it
/// is Shift+7. Matching the CHARACTER is what makes the chord the same chord
/// on every keyboard, and it is why the shift guard the other two listeners
/// carry is absent here — reaching `/` at all requires Shift on most of
/// Europe.
///
/// `preventDefault` only after the chord has been recognised, which is the
/// same shape `REFRESH_KEY` uses and for a weaker reason: nothing in a web
/// view claims ⌘/ today, so there is probably nothing to swallow. It is taken
/// anyway because the alternative is a key that both toggles the nav and does
/// whatever the platform decides ⌘/ means next.
const NAV_KEY: &str = r"
(() => {
  if (window.__navKeyWired) return;
  window.__navKeyWired = true;
  document.addEventListener('keydown', (e) => {
    if (e.key !== '/') return;
    if (!e.metaKey && !e.ctrlKey) return;
    if (e.altKey) return;
    e.preventDefault();
    dioxus.send('toggle');
  });
})();
";

/// Wire ⌘/ to the nav's own open signal.
fn use_nav_key(mut nav_open: Signal<bool>) {
    use_effect(move || {
        let mut eval = document::eval(NAV_KEY);
        spawn(async move {
            while eval.recv::<String>().await.is_ok() {
                let now = *nav_open.peek();
                nav_open.set(!now);
            }
        });
    });
}

/// Whether the window is fullscreen, as the window itself reports it.
///
/// `src/main.rs` hides the macOS titlebar, so `assets/platform/macos.css`
/// reserves a 52pt band with a 76pt indent for the traffic lights. Fullscreen
/// takes those lights away — `AppKit` hides them and re-draws them in an overlay
/// on hover near the top edge — so the reservation becomes 122pt of nothing at
/// the top of a window someone has just asked to be as large as possible.
///
/// REPORTED, not inferred, and that is the whole change. This was a JS test of
/// `innerHeight >= screen.height - 2`, on the reasoning that a fullscreen
/// window covers the display exactly while a maximised one is short by the
/// menu bar. Driven into real fullscreen it never once fired, so the block it
/// exists to switch on was dead for the life of the feature — and a heuristic
/// that is always wrong looks exactly like one that works, because both spend
/// all their time saying "not fullscreen". `tao` knows the answer; asking it
/// cannot be wrong.
///
/// A `Resized` and not a fullscreen event, because tao publishes no such
/// event: the transition arrives as a resize and the flag is read back off the
/// window. So this is a trigger, not a measurement — the 2px slack that made
/// the old inference a guess has nothing to be slack about here.
///
/// NOT A BREACH OF THE RULE IN `src/viewport.rs`. That rule is about DOM
/// events, each of which costs a synchronous XHR to reach Rust, which is why a
/// resize stream must be JS-owned. A `tao` event is not a DOM event: it is
/// already in the event loop, in this process, and reaches this closure by a
/// function call. The stream is free; only the writes are not, and `peek`
/// means the signal is written on the transition rather than on the frame.
fn use_fullscreen() -> Signal<bool> {
    use dioxus::desktop::tao::event::{Event, WindowEvent};
    use dioxus::desktop::use_wry_event_handler;

    let mut full = use_signal(|| dioxus::desktop::window().fullscreen().is_some());
    use_wry_event_handler(move |event, _| {
        if matches!(
            event,
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            }
        ) {
            let now = dioxus::desktop::window().fullscreen().is_some();
            if *full.peek() != now {
                full.set(now);
            }
        }
    });
    full
}

/// What the collapse control says it will do, which is never what it is.
///
/// A toggle's name is its ACTION, not its state — "Hide sidebar" while the
/// sidebar is showing. Both `title` (the pointer's tooltip) and `aria-label`
/// (the screen reader's name) take it, because with the nav shut the button
/// is an unlabelled glyph in an empty corner and it is the only thing on
/// screen that can bring the navigation back.
///
/// Data rather than a branch inside the component, following [`nav_tooltip`]
/// and [`nav_is_active`] in `src/shell/mod.rs`: a rule taken as a value is a
/// rule a test can hold.
///
/// [`nav_tooltip`]: crate::shell::nav_tooltip
/// [`nav_is_active`]: crate::shell::nav_is_active
const fn nav_toggle_label(open: bool) -> &'static str {
    if open {
        "Hide sidebar"
    } else {
        "Show sidebar"
    }
}

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

    // The nav's own state, held HERE rather than on `AppCtx`, and not
    // `ctx.drawer_open`.
    //
    // Reusing the phone's signal was the obvious move and it is wrong twice.
    // `shell::render_group` closes the drawer on every destination click, so
    // the nav would shut itself the moment you navigated with it; and
    // `views/scheduler.rs` treats an open drawer as an overlay and stops
    // polling underneath it, so a pinned-open nav would silently freeze the
    // schedule list for the life of the window. Both are correct readings of
    // what that signal means on a phone — it is a panel COVERING the screen —
    // and neither is true of a column beside one.
    //
    // A local `use_signal` and not a field on `AppCtx` because `AppShell`
    // mounts once for the life of the process (`src/app.rs`) and nothing
    // outside this file reads it: the toggle, the keyboard chord and the
    // attribute the sheet keys off are all below.
    let mut nav_open = use_signal(|| true);

    use_arrival_refresh(dest);
    use_refresh_key();
    use_nav_key(nav_open);
    use_dismiss_key();
    // The chrome strip is only real while the traffic lights are on screen;
    // fullscreen takes them away. Read off the window rather than guessed at —
    // see `use_fullscreen`.
    let fullscreen = use_fullscreen();

    // The destination's own answer to "is anything open", which is the fact
    // the third column needs and needs no new state to hold. A destination
    // with no list of its own (Settings) is one screen, so its detail is
    // unconditional: it takes the content area whole and the shell draws two
    // columns rather than three.
    //
    // It arrives with its own name (`nav::Detail`), which is the whole of what
    // the window's bar needs: Dioxus has no portal, so no header rendered
    // inside a pane can be MOVED into `.shell-chrome` — but a `String` travels
    // anywhere. So the title is carried as data and painted in the bar below,
    // and `assets/desktop.css` takes the pane's own copy of it back out.
    let detail = (dest.detail)(&ctx);
    let detail_open = detail.is_some();
    let crumb = detail.as_ref().map(|detail| detail.crumb.clone());

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
            // The whole of the collapse, as far as Rust is concerned. Width
            // still decides nothing here and neither does this: the sheet
            // slides the column shut and slides the button across, and the
            // only fact it cannot work out for itself is which way the toggle
            // was last pressed.
            "data-nav": if nav_open() { "open" } else { "closed" },
            // ON `.shell`, beside the other two facts the sheet reads, and not
            // on `.app` where the JS used to put it. `.app` is the phone's
            // element too, and an attribute set from a desktop-only hook had
            // no business there; here it sits with `data-detail` and
            // `data-nav`, set the same way, by the same render.
            "data-fullscreen": if fullscreen() { "true" } else { "false" },

            // THE WINDOW'S OWN BAR, full width, above the columns.
            //
            // The first draft did not have one: the traffic lights were left
            // where macOS puts them, at logical (9, 9), and the nav card was
            // inset 8 — so the lights landed ON the card's 16px corner radius,
            // tangled in the curve. There is no arrangement of paddings that
            // fixes that, because two things were claiming the same corner.
            // A band that spans the window is what gives the lights a place of
            // their own, and it is also the structure the reference has:
            // goose's own app puts the lights, the sidebar toggle AND the
            // current view's title in one strip across the top
            // (`ui/desktop/src/components/Layout/AppLayout.tsx:99-122`).
            //
            // It exists on every platform, not only where the titlebar is
            // hidden: `--traffic-w` is zero elsewhere, so the bar simply
            // starts with the toggle. That is what keeps the toggle in one
            // place — the property that matters most about the control that
            // brings the navigation back — instead of it moving per platform.
            header { class: "shell-chrome",
                // Empty, sized, and not a spacer for spacing's sake: it is the
                // room the traffic lights are painted in. They are drawn by
                // AppKit over the web view, so nothing here renders them and
                // nothing here may cover them.
                div { class: "traffic-slot" }

                button {
                    class: "nav-toggle",
                    title: nav_toggle_label(nav_open()),
                    "aria-label": nav_toggle_label(nav_open()),
                    "aria-expanded": if nav_open() { "true" } else { "false" },
                    onclick: move |_| {
                        let now = *nav_open.peek();
                        nav_open.set(!now);
                    },
                    Icon { name: "sidebar" }
                }

                // WHAT THE WINDOW HAS OPEN, in the window's own bar.
                //
                // The reference does exactly this and nothing more: its band
                // is an empty drag strip, and the one thing painted into the
                // middle of it is the open session's name, contributed by
                // whatever is mounted (`SessionActionsHeader.tsx` returns
                // `null` when there is no session). A LIST keeps its heading
                // on the canvas there, and it keeps it here — so the list
                // column never moves, at any width, in any state, and the one
                // thing that travels is the name of the thing you opened.
                //
                // Leading-aligned rather than centred on the window, and that
                // is measured rather than preferred. The reference's window
                // centre IS its content centre because it has no list column;
                // ours is not. At a 902pt window the detail column's own
                // measure centres 271pt from the window's centre, and with the
                // nav shut at 1400 it is 210pt out — so a centred title names
                // the pane it is not over. Anchored to the toggle it is in one
                // place at every width, which is the same property the toggle
                // itself is placed for.
                //
                // `if let` and not a `title` prop with an empty string: an
                // empty flex item still takes its gap, and the bar with
                // nothing open should be exactly what it was before this —
                // the lights, the toggle, and a drag strip.
                if let Some(crumb) = crumb {
                    div { class: "chrome-title",
                        h1 { class: "chrome-heading", "{crumb.title}" }
                        if let Some(subtitle) = crumb.subtitle {
                            span { class: "chrome-sub", "{subtitle}" }
                        }
                    }
                }

                // Everything to the right of the toggle drags the window.
                //
                // `src/main.rs` takes the titlebar away, and that takes
                // AppKit's own drag region with it: with `fullsize_content_view`
                // the web view owns every pixel, so a window with no
                // replacement cannot be moved at all.
                // `-webkit-app-region: drag` is the web answer and is not
                // available to us — that is a Chromium property, and wry on
                // macOS is WKWebView. `drag_window()` is, and dioxus-desktop
                // documents this exact call site (`desktop_context.rs:160`).
                //
                // A Rust `onmousedown` is allowed here where a Rust `onscroll`
                // is not: the rule in `src/viewport.rs` is about events that
                // fire per FRAME, because each costs a synchronous XHR. This is
                // one press, after which the drag is AppKit's own.
                //
                // A SIBLING of the toggle rather than the bar itself, so that
                // pressing the toggle cannot also start a drag — the reference
                // needs a `no-drag` class for exactly this and we get it from
                // the box tree instead.
                div {
                    class: "window-drag",
                    onmousedown: move |_| {
                        // Dropped rather than handled: the only `Err` tao
                        // returns is "this platform cannot start a window
                        // drag", and the response to that is the one already
                        // taken — the native frame is still on there, so the
                        // window has a titlebar of its own to drag by.
                        let _ = dioxus::desktop::window().drag_window();
                    },
                }

                // ONE GREEN DOT PER WINDOW, and now structurally rather than
                // by a media query.
                //
                // `TopBar { conn: true }` is a fact about a SCREEN, so at
                // three columns two screens each painted one and
                // `assets/desktop.css` hid the detail's above 902px. The
                // consequence was a badge that jumped columns when the window
                // was dragged across that width — the state of the connection
                // moving because the layout reflowed. In the bar it belongs to
                // the window, there is exactly one of it, and it never moves.
                // The reference puts its `EnvironmentBadge` in the same place
                // for the same reason (`BaseChat.tsx`).
                //
                // Rendered directly rather than passed through a prop:
                // `ConnBadge` reads `ctx.conn` and nothing else, so the shell
                // can ask for it as it stands.
                crate::views::ConnBadge {}
            }

            div { class: "shell-body",

            // NOT `.drawer.open`. Three reasons, all load-bearing: `.drawer`
            // is `position: absolute` with `translateX(-100%)` and would have
            // to be fought rather than reused; `src/domdump.rs` files any dump
            // containing `.drawer.open` under a `-drawer` suffix, so every
            // desktop capture would be named as if a panel were over it; and
            // nothing has to move out of `assets/main.css`, because
            // `.drawer-brand`, `.drawer-nav`, `.drawer-item` and
            // `.drawer-group` are all independent of `.drawer` itself.
            //
            // Two elements where the first draft had one, and the inner one is
            // what carries the look. `.navpane` is the COLUMN — it is what
            // animates to zero width, and it clips — while `.navcard` is the
            // rounded, outlined panel inside it, at a width that does not
            // change while the column closes. One element cannot do both: a
            // panel that shrinks reflows its own destination labels on the way
            // out, so "Session History" rewraps to four lines and back again
            // over 200ms. Clipping a card that never moves is the same slide
            // with none of that, and it is the arrangement goose's own desktop
            // app uses for this panel, down to the 8px of padding.
            aside { class: "navpane",
                div { class: "navcard",
                    h2 { class: "drawer-brand", "goose" }
                    nav { class: "drawer-nav",
                        for group in Group::ALL {
                            {render_group(&ctx, group)}
                        }
                    }
                }
            }

            if let Some(root) = dest.root {
                section { class: "pane pane-list", {root(&ctx)} }
            }

            section { class: "pane pane-detail",
                if let Some(detail) = detail {
                    {detail.view}
                } else {
                    {empty_detail(dest)}
                }
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

    /// The nav COLUMN's width once it drops its labels.
    ///
    /// 72 and not 56, and the extra 16 is not slack: the nav is a card inside
    /// its column with `--shell-gap` of breathing room on each side, so the
    /// rail the icons actually sit in is 72 - 8 - 8 = 56. That is the number
    /// `assets/desktop.css` reaches by overriding `--nav-w`, and it is the
    /// same 56 the labels were dropped for.
    const RAIL: u32 = 72;

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

    /// Every class name the shell renders that only a stylesheet gives
    /// meaning to. Rust writes them, CSS is the only thing that reads them,
    /// and nothing in the compiler connects the two — so a rename on either
    /// side leaves a control that visibly does nothing, with no error
    /// anywhere. `.window-drag` is the worst of them: rename it and the
    /// window silently stops being draggable, because `src/main.rs` has taken
    /// the titlebar away and that strip is the only replacement.
    #[test]
    fn every_class_the_shell_renders_is_styled_somewhere() {
        let shell = include_str!("desktop.rs");
        let sheets = concat!(
            include_str!("../../assets/desktop.css"),
            include_str!("../../assets/platform/macos.css"),
        );
        // A selector, not a substring. `.contains(".window-drag")` is
        // satisfied by `.window-draggg`, so the first draft of this test
        // passed against a deliberately broken stylesheet — proved by
        // renaming the class and watching it go green. A class name ends
        // where a CSS identifier ends.
        let styled = |name: &str| {
            let needle = format!(".{name}");
            sheets.match_indices(&needle).any(|(at, _)| {
                sheets[at + needle.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !(c.is_alphanumeric() || c == '-' || c == '_'))
            })
        };

        for class in [
            "shell-chrome",
            "traffic-slot",
            "window-drag",
            "nav-toggle",
            "chrome-title",
            "chrome-heading",
            "chrome-sub",
            "navcard",
            "navpane",
        ] {
            assert!(
                shell.contains(&format!("class: \"{class}\"")),
                "src/shell/desktop.rs no longer renders .{class}"
            );
            assert!(
                styled(class),
                "nothing styles .{class}, which the shell still renders"
            );
        }
    }

    /// The window's bar takes the detail's title and its connection badge, and
    /// `assets/desktop.css` is what stops the pane below painting either of
    /// them again. That half is invisible to the compiler in BOTH directions:
    /// the sheet names classes Rust never writes (`.conn-badge` comes from
    /// `views::ConnBadge`, `.title` and `.titlegroup` from six hand-rolled
    /// headers and from `views::chrome::TopBar`), and the shell names an
    /// attribute the sheet has to agree about. Lose either and the window
    /// shows a chat's name twice, 500pt apart and at two different sizes, with
    /// nothing failing anywhere.
    ///
    /// A rule this cannot check and a reader should not assume: that the rule
    /// MATCHES. `.pane .topbar > .conn-badge` is a child combinator, so a
    /// header that wrapped its badge one element deeper would keep painting
    /// it and this would still pass. `docs/audit.js` on a captured desktop
    /// state is what sees that, and its `.shell-chrome` arm is what sees the
    /// band.
    #[test]
    fn the_pane_gives_up_what_the_window_bar_took() {
        let shell = include_str!("desktop.rs");
        let sheet = include_str!("../../assets/desktop.css");

        assert!(
            shell.contains("crate::views::ConnBadge {}"),
            "the window's bar no longer renders the connection badge, so \
             `.pane .topbar > .conn-badge` below now hides the only one there is"
        );
        for rule in [
            r#"[data-detail="open"] .pane-detail .topbar > .title"#,
            r#"[data-detail="open"] .pane-detail .topbar > .titlegroup"#,
            ".pane .topbar > .conn-badge",
        ] {
            assert!(
                sheet.contains(rule),
                "assets/desktop.css never mentions `{rule}`, so the pane paints \
                 a second copy of what `.shell-chrome` is already showing"
            );
        }
        assert!(
            shell.contains(r#""data-detail": if detail_open"#),
            "the sheet keys the rules above on [data-detail], which this file \
             is the only thing that sets"
        );
    }

    /// The chrome reservation is macOS's alone, because `src/main.rs` hides
    /// the titlebar there and nowhere else. Defaulted to zero in the shared
    /// desktop sheet and raised only by the platform sheet — otherwise a
    /// native-frame build gets its own titlebar AND a 52pt strip held empty
    /// for traffic lights it does not have.
    #[test]
    fn the_window_chrome_is_reserved_only_where_the_titlebar_is_hidden() {
        let desktop = include_str!("../../assets/desktop.css");
        let macos = include_str!("../../assets/platform/macos.css");
        assert!(
            desktop.contains("--traffic-w: 0px"),
            "assets/desktop.css must default the traffic-light slot to zero — \
             only the platform sheet may claim room for lights"
        );
        assert!(
            macos.contains("--traffic-w: 76px"),
            "assets/platform/macos.css is the only thing that may raise it"
        );
        assert!(
            macos.contains(r#".shell[data-fullscreen="true"]"#),
            "fullscreen hides the traffic lights, so it must drop the reservation \
             — and the attribute is on `.shell`, beside data-detail and data-nav"
        );
        // BOTH HALVES, because for the whole life of this feature only one of
        // them existed. The sheet had its rule and the attribute was set by a
        // JS heuristic that never once matched a real fullscreen window, so the
        // block above was dead and every check in the repo was green. A test
        // that asks only "does the sheet mention it" cannot tell that apart
        // from a working feature; this is the other half of the question.
        let shell = include_str!("desktop.rs");
        assert!(
            shell.contains(r#""data-fullscreen": if fullscreen()"#),
            "the shell must SET the attribute the sheet reads, in the render — \
             an attribute nothing writes is a stylesheet rule nothing reaches"
        );
        assert!(
            shell.contains("fn use_fullscreen()") && shell.contains(".fullscreen().is_some()"),
            "the flag must be read off the window; inferring it from geometry \
             is what shipped a feature that never engaged"
        );
    }

    /// The collapse is two decisions in two languages again: Rust sets
    /// `data-nav` and the sheet is the only thing that reads it. Nothing in
    /// the compiler connects them, so a rename on either side leaves a button
    /// that toggles an attribute nobody styles — a control that visibly does
    /// nothing, with no error anywhere.
    #[test]
    fn the_stylesheet_acts_on_the_attribute_the_shell_sets() {
        let sheet = include_str!("../../assets/desktop.css");
        for rule in [r#"[data-nav="closed"]"#, ".nav-toggle", ".navcard"] {
            assert!(
                sheet.contains(rule),
                "assets/desktop.css never mentions `{rule}`, so the nav's \
                 collapse control changes an attribute nothing styles"
            );
        }
    }

    /// A toggle is named for what it will DO, not for what it is. Getting
    /// this backwards is invisible to a sighted pointer user — the tooltip
    /// just reads oddly — and actively misleading to a screen reader, where
    /// this string is the button's whole identity.
    #[test]
    fn the_collapse_control_is_named_for_its_action() {
        assert_eq!(super::nav_toggle_label(true), "Hide sidebar");
        assert_eq!(super::nav_toggle_label(false), "Show sidebar");
    }

    /// The glyph the toggle asks for has to exist. `Icon` renders nothing at
    /// all for a name it does not know (`src/icons.rs`), so a typo here is an
    /// invisible button in an empty corner rather than a compile error — and
    /// with the nav shut that button is the only way back to the navigation.
    #[test]
    fn the_collapse_control_has_a_glyph() {
        assert!(crate::icons::path_for("sidebar").is_some());
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
