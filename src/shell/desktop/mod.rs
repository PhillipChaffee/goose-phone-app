//! The desktop shell: a pinned nav, a list, and what the list opens.
//!
//! Three columns — nav, list, detail — and `assets/desktop/` reflows them
//! to two and then to one as the window narrows. NONE of that is decided here.
//! The breakpoints are `@media` rules in a stylesheet a phone binary does not
//! contain, so pane count costs zero Rust and nothing in this app listens to a
//! resize. What this file decides is what goes IN the columns, which is a
//! question about the destination rather than about the window.
//!
//! What it does not have, on purpose: no swipe tray (the rows carry their
//! actions inline, `src/views/chrome.rs`), no pull-to-refresh (a mount
//! re-fetch and ⌘R below), no hamburger (the phone's is hidden by
//! `assets/desktop/`; the nav's own toggle is below), and no drag-resize on
//! the nav.
//!
//! The nav DOES collapse, and that is the one thing about it the window gets a
//! say in. It starts open every launch and nothing persists the choice: the
//! default is the feature, and a nav that remembered being shut would open one
//! day into a window with no navigation in it and no explanation.

use dioxus::prelude::*;

mod home;
mod inspector;
mod sidebar;

use std::fmt::Write as _;

use crate::icons::Icon;
use crate::nav::{self, Destination, Plane};
use crate::shell::render_destination;
use crate::state::{AppCtx, ConnState};

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
    // LOAD-BEARING, not defensive. `INSP_KEY` below is this chord with
    // Option added, so without this line the one press would toggle both
    // panels and the reader would never see the inspector move on its own.
    if (e.altKey) return;
    e.preventDefault();
    dioxus.send('toggle');
  });
})();
";

/// Cmd-Option-slash shows and hides the inspector. Ctrl+Alt+/ elsewhere.
///
/// The nav's chord with Option added, because the two panels are the same
/// gesture on opposite sides of the window and goose's own
/// `CommandOrControl+/` is already spent on the left one. `NAV_KEY`'s
/// `altKey` guard is what keeps the two apart, and its comment now says so.
///
/// JS rather than a Rust `onkeydown`, for `REFRESH_KEY`'s reason. BOTH
/// characters are matched because on macOS Option+/ produces a division sign,
/// so the event's `key` is not a slash at all once Option is down.
const INSP_KEY: &str = r"
(() => {
  if (window.__inspKeyWired) return;
  window.__inspKeyWired = true;
  document.addEventListener('keydown', (e) => {
    if (e.key !== '/' && e.key !== '÷') return;
    if (!e.metaKey && !e.ctrlKey) return;
    if (!e.altKey) return;
    e.preventDefault();
    dioxus.send('toggle-insp');
  });
})();
";

/// Wire the inspector chord to its own open signal.
fn use_insp_key(mut open: Signal<bool>) {
    use_effect(move || {
        let mut eval = document::eval(INSP_KEY);
        spawn(async move {
            while eval.recv::<String>().await.is_ok() {
                let now = *open.peek();
                open.set(!now);
            }
        });
    });
}

/// Wire the nav chord to the nav's own open signal.
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
/// reserves a 34pt band with a 76pt indent for the traffic lights. Fullscreen
/// takes those lights away — `AppKit` hides them and re-draws them in an overlay
/// on hover near the top edge — so the indent becomes 76pt of nothing at the
/// top left of a window someone has just asked to be as large as possible.
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
/// What the New button says, per half.
///
/// The two halves keep their own vocabulary all the way down — a goose thread
/// is a chat, an `OpenCode` job is a working tree with a branch under it — and
/// the button that makes one is the last place to blur that. Data rather than
/// a branch inside the component, following [`nav_toggle_label`] below and
/// `Plane::label`: a rule taken as a value is a rule a test can hold.
const fn new_label(plane: Plane) -> &'static str {
    match plane {
        Plane::Chat => "New chat",
        Plane::Code => "New code session",
    }
}

/// What the inspector control says it will do.
///
/// "Inspector" and deliberately NOT "details": `nav::Destination`'s `Detail`
/// is already this shell's word for the CONTENT column and its `Crumb` is what
/// the band's title is drawn from, so "Show details" would name two different
/// columns in one window.
const fn insp_toggle_label(open: bool) -> &'static str {
    if open {
        "Hide inspector"
    } else {
        "Show inspector"
    }
}

const fn nav_toggle_label(open: bool) -> &'static str {
    if open {
        "Hide sidebar"
    } else {
        "Show sidebar"
    }
}

/// THE CONNECTION THAT BELONGS TO THIS HALF.
///
/// `home::Home` already switches on exactly this and the band did not, so the
/// band spent the whole of the code plane naming the chat server.
/// `docs/gallery-states.json`'s `desktop-code-list` is the proof: a green dot
/// and "goose-mock 1.47.0" over a pane whose every row comes from the other
/// wire. It is the worst kind of wrong a status indicator can be — confidently
/// green about a thing nobody asked.
///
/// One function, because the pill, the band's counts and the plane switch's
/// counts all have to answer the same question and three copies of a `match`
/// is three chances for two of them to disagree.
pub(crate) fn conn_of(ctx: &AppCtx, plane: Plane) -> ConnState {
    match plane {
        Plane::Chat => (ctx.conn)(),
        Plane::Code => (ctx.code_conn)(),
    }
}

/// The server this half is configured against, host and port only.
fn plane_host(ctx: &AppCtx, plane: Plane) -> Option<String> {
    let settings = ctx.settings.peek();
    home::host_of(match plane {
        Plane::Chat => &settings.server_url,
        Plane::Code => &settings.code_server_url,
    })
}

/// The connection, as the mockups draw it: a dot, a word, and the host in mono.
///
/// NOT `views::ConnBadge`, and `assets/shared.css` is not edited for it: that
/// component is the phone's and reads `ctx.conn` outright, which is right on a
/// phone where there is one connection on screen at a time. It keeps the
/// `conn-badge` class, so `.pane .topbar > .conn-badge` goes on hiding every
/// pane's copy and `docs/audit.js` goes on finding this one.
#[component]
pub(crate) fn PlaneConn(plane: Plane) -> Element {
    let ctx = crate::state::use_app_ctx();
    let (class, label) = match conn_of(&ctx, plane) {
        ConnState::Disconnected => ("dot off", "offline".to_owned()),
        ConnState::Connecting => ("dot busy", "connecting\u{2026}".to_owned()),
        ConnState::Connected { agent } => ("dot on", agent),
        ConnState::Failed(_) => ("dot err", "error".to_owned()),
    };
    rsx! {
        span { class: "conn-badge",
            span { class: "{class}" }
            span { class: "conn-label", "{label}" }
            if let Some(host) = plane_host(&ctx, plane) {
                code { class: "conn-host", "{host}" }
            }
        }
    }
}

/// What the window's bar says it has open: a parent, a leaf, and a qualifier.
///
/// The mockups' crumb is `goose server / **All conversations**` followed by a
/// line of counts. The band rendered only the middle of that, and only when
/// something was open — six of thirteen captured states had a plane badge, a
/// comment node, and a drag strip.
pub(crate) struct CrumbParts {
    /// `None` on a destination's own root, where the parent and the leaf would
    /// be the same word.
    pub parent: Option<&'static str>,
    pub leaf: String,
    /// Beside the leaf: the crumb's own subtitle where there is one, the
    /// half's counts on its home screen and behind a shut sidebar, nothing
    /// otherwise.
    pub after: Option<String>,
}

/// What the plane's home screen is called — the mockups' own leaf words, in
/// the vocabulary each half already uses everywhere else.
const fn home_leaf(plane: Plane) -> &'static str {
    match plane {
        Plane::Chat => "All conversations",
        Plane::Code => "Working trees",
    }
}

/// WHAT THE HALF'S HOME IS INSIDE — the thing you are connected to, not the
/// row in the nav table that got you there.
///
/// The home arm used to answer `dest.label`, which is the badge's own word one
/// character apart: the Code band read `CODE  Code / Working trees`, saying
/// "Code" twice, 10px from itself, once as a chip and once as a path segment.
/// Chat read `CHAT  Chats / All conversations`, a near-repeat rather than a
/// path. The detail arm has guarded against exactly this since the
/// `Settings / Settings` stutter — `(crumb.title != dest.label)` — and the home
/// arm had no equivalent guard against the badge beside it.
///
/// The mockups' own words, and they are deliberate rather than incidental:
/// 10-home-chat is `<span>goose server</span> / <b>All conversations</b>` and
/// 11-home-code is `<span>code plane</span> / <b>Working trees</b>`. Neither
/// names the destination; both name what the half IS. 11 says "code plane"
/// rather than "Code" for this issue's reason in the mockup's own hand — the
/// badge beside it already says Code.
///
/// NOT A CLAIM ABOUT STATE, which is what keeps it inside `CLAUDE.md`'s rule.
/// It is a static description of the half, not a fact read off a wire: the
/// connection's own name, version and host are the pill's, three inches to the
/// right, and they stay there. Saying "goose server" over a dead socket is
/// saying what this screen lists, not that anything answered.
///
/// Data rather than a branch inside the component, following [`home_leaf`]
/// above and [`new_label`]: a rule taken as a value is a rule a test can hold.
const fn home_parent(plane: Plane) -> &'static str {
    match plane {
        Plane::Chat => "goose server",
        Plane::Code => "code plane",
    }
}

/// A free function beside the table rather than rsx inside `AppShell`, for
/// `sidebar::chat_rows`' reason: `AppShell` calls `dioxus::desktop::window()`
/// and cannot be mounted in a test.
///
/// TOTAL, and that is a change rather than a tidy-up. The band used to render
/// a title only when something was OPEN, so six of thirteen captured states
/// put a plane badge, a comment node and a drag strip in the window's bar and
/// nothing else. Every screen has a name; the band says it now.
///
/// The consequence is that `.chrome-title` is always present, so
/// `assets/desktop/`'s `:has(.chrome-title)` suppression of the pane's own
/// heading became unconditional — which is what the mockups draw, where no
/// screen has a pane header at all.
pub(crate) fn crumb_parts(
    dest: &'static Destination,
    plane: Plane,
    on_home: bool,
    nav_open: bool,
    crumb: Option<crate::nav::Crumb>,
    after: Option<String>,
) -> CrumbParts {
    if let Some(crumb) = crumb {
        return CrumbParts {
            // NOT WHEN THEY ARE THE SAME WORD. Settings is its own detail and
            // its crumb's title is "Settings", so a parent taken blindly from
            // the table made the band read "Settings / Settings" — which is a
            // stutter rather than a path, the same defect the third arm below
            // avoids by answering `None`.
            parent: (crumb.title != dest.label).then_some(dest.label),
            leaf: crumb.title,
            // AND THE COUNTS TAKE THE SLOT WHEN THE OPEN THING HAS NOTHING TO
            // SAY AND THE SIDEBAR IS SHUT.
            //
            // The half's counts otherwise live in two places, and shutting the
            // sidebar takes both away: `.plane-seg-count` is inside the column
            // that just closed, and the band only ever passed `after` on the
            // home screen. So with a chat open and the sidebar collapsed —
            // which is the arrangement the mockups' 30-collapse-left and
            // 32-collapse-both are drawn for, and the arrangement `docs/audit.js`
            // walks in two of its four shell cells — nothing on screen said how
            // many conversations the half had or that any were waiting on you.
            // The string was already computed by `band_after`; only the slot was
            // contested.
            //
            // ONLY WHEN THE CRUMB HAS NO SUBTITLE OF ITS OWN, and that is a
            // priority rather than a hedge. `.chrome-sub` says what qualifies
            // the thing that is open; where the open thing qualifies itself —
            // "paused", "Global", "developer" — that is the more specific
            // answer and it keeps the slot. Where it says nothing, the half's
            // standing counts are the next most useful qualifier and they are
            // otherwise nowhere.
            //
            // WHAT THIS IS NOT: the mockups put a second element in the
            // collapsed band — `.backb`, a glyph, a chevron, the word
            // "Sessions" and a bold count — beside the crumb rather than in it.
            // That is new markup and a new class, which
            // `every_class_the_desktop_shell_renders_is_in_the_captured_store`
            // refuses until `docs/gallery-states.json` is retaken by an
            // operator. This is the same fact in the slot that already exists;
            // the second element is what to build when the store is retaken.
            after: crumb.subtitle.or(if nav_open { None } else { after }),
        };
    }
    if on_home {
        return CrumbParts {
            parent: Some(home_parent(plane)),
            leaf: home_leaf(plane).to_owned(),
            after,
        };
    }
    // A destination's own root — the Recipes grid, the Skills grid. Naming it
    // twice either side of a slash is not a path, it is a stutter.
    CrumbParts {
        parent: None,
        leaf: dest.label.to_owned(),
        after: None,
    }
}

/// The half's standing counts, or `None` when the half is not connected.
///
/// The gate is `home::standing`'s rule in its own words — "saying 12
/// conversations over a dead socket is the kind of confident wrongness that
/// makes a reader stop trusting the whole screen" — and it matters more here
/// than there. On the desktop the code half is not connected until something
/// mounts `views::code::CodeSessionsView`, and the band is on screen the
/// entire time.
pub(crate) fn band_after(ctx: &AppCtx, plane: Plane) -> Option<String> {
    if !conn_of(ctx, plane).is_connected() {
        return None;
    }
    match plane {
        Plane::Chat => {
            let total = (ctx.sessions)().len();
            let streaming = (ctx.running_sessions)().len();
            let waiting: std::collections::HashSet<String> = (ctx.permission)()
                .iter()
                .map(|p| p.session_id.clone())
                .collect();
            let mut out = if total == 1 {
                "1 conversation".to_owned()
            } else {
                format!("{total} conversations")
            };
            if streaming > 0 {
                let _ = write!(out, " \u{b7} {streaming} streaming");
            }
            // Said either way round, because "nothing waiting on you" is the
            // answer to the question the reader is actually asking and an
            // absent clause is not an answer.
            if waiting.is_empty() {
                out.push_str(" \u{b7} nothing waiting on you");
            } else {
                let _ = write!(out, " \u{b7} {} waiting on you", waiting.len());
            }
            Some(out)
        }
        Plane::Code => {
            let chats = (ctx.code_chats)();
            let running = chats.iter().filter(|c| c.is_running()).count();
            let repos = {
                let mut names: Vec<&str> = chats
                    .iter()
                    .map(|c| c.repo.as_str())
                    .filter(|r| !r.trim().is_empty())
                    .collect();
                names.sort_unstable();
                names.dedup();
                names.len()
            };
            let waiting: std::collections::HashSet<String> = (ctx.code_permissions)()
                .iter()
                .map(|(chat, _)| chat.clone())
                .collect();
            let trees = if chats.len() == 1 {
                "1 working tree".to_owned()
            } else {
                format!("{} working trees", chats.len())
            };
            let mut out = format!("{trees} across {repos} repos");
            if !waiting.is_empty() {
                let _ = write!(out, " \u{b7} {} waiting on you", waiting.len());
            }
            if running > 0 {
                let _ = write!(out, " \u{b7} {running} running");
            }
            Some(out)
        }
    }
}

/// How much is in a half, or `None` when the half has not answered.
///
/// The gate is not decoration. The desktop never mounts
/// `views::code::CodeSessionsView` while it is on the Code half's home — the
/// pane renders `home::Home` instead — so `ctx.code_chats` is empty until
/// something connects. An ungated count paints a confident `0` beside "Code"
/// for a server nobody has asked.
pub(crate) fn seg_count(ctx: &AppCtx, plane: Plane) -> Option<usize> {
    conn_of(ctx, plane).is_connected().then(|| match plane {
        Plane::Chat => (ctx.sessions)().len(),
        Plane::Code => (ctx.code_chats)().len(),
    })
}

/// The half's standing fact, under the list.
///
/// The mockups' footer is an avatar, a name and a line, and only the line has
/// a source: `Settings` is `server_url` / `secret_key` / `working_dir` /
/// `code_server_url` / `code_password`, and neither wire carries a user.
///
/// It says what the band says on the home screen, from the same expression,
/// and that is the point rather than a duplication — the band's counts are
/// replaced by the open thing's name the moment you open one, and this is the
/// number that stays put.
fn standing_line(ctx: &AppCtx, plane: Plane) -> Option<String> {
    let n = seg_count(ctx, plane)?;
    Some(match plane {
        Plane::Chat if n == 1 => "1 conversation kept".to_owned(),
        Plane::Chat => format!("{n} conversations kept"),
        Plane::Code if n == 1 => "1 working tree".to_owned(),
        Plane::Code => format!("{n} working trees"),
    })
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
    use_insp_key(ctx.inspector_open);
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
    // and `assets/desktop/` takes the pane's own copy of it back out.
    let detail = (dest.detail)(&ctx);
    let crumb = detail.as_ref().map(|detail| detail.crumb.clone());

    // WHICH HALF THE SIDEBAR IS SHOWING, derived rather than held.
    //
    // The destination already knows — `nav::Destination::plane` is a fact
    // about the row — so holding the plane as state beside it would be two
    // sources for one thing, and the failure mode is a switch that says Chat
    // while a code session fills the pane. Every route into a plane goes
    // through a destination: the switch below, the ⌘K palette later, and a
    // deep link if one ever arrives.
    //
    // The signal is not that second source. It is memory for the ONE row that
    // belongs to neither half — Settings — so that leaving Code for Settings
    // and coming back does not silently land on Chat. The peek-then-set is
    // `use_arrival`'s pattern below, and for its reason: `set` inside a render
    // is only safe because the value is compared first.
    let mut remembered = use_signal(|| Plane::Chat);
    let plane = match dest.plane {
        Some(plane) => {
            if *remembered.peek() != plane {
                remembered.set(plane);
            }
            plane
        }
        None => remembered(),
    };

    // WHAT THE CONTENT COLUMN IS SHOWING, as one fact rather than two.
    //
    // The pane and the New button both turn on it, and they used to be able to
    // disagree: a button conditioned on `detail.is_none()` alone would come
    // back on the Recipes grid, where there is no composer to be a second door
    // onto. Computed once, read twice.
    //
    // "Home" is the plane's own opening destination with nothing pushed on it.
    // Settings is deliberately not home for either half — it belongs to
    // neither plane, its detail is unconditional, and it takes the first arm.
    let on_home = detail.is_none() && dest.id == nav::primary(plane).id;
    // WHAT THE INSPECTOR IS INSPECTING, and it is not `on_home` inverted:
    // Settings, Recipes and Skills all produce a `detail` while belonging to
    // NEITHER plane, and each plane's own signals (`ctx.chat`,
    // `ctx.code_chat`) keep their last value after you leave — so an inspector
    // keyed on `detail` alone would confidently describe a conversation nobody
    // is looking at.
    let on_subject = dest.plane == Some(plane) && detail.is_some();
    let insp_open = ctx.inspector_open;

    // The half's library, computed once: the disclosure needs to know whether
    // it has anything before it decides to exist, and then needs the rows.
    let library = nav::library(plane);

    // Shut every launch, and nothing persists the choice — the nav toggle's
    // rule (see the module comment) for the same reason one level down. The
    // sidebar's body is the session list; a library that remembered being open
    // would push it below the fold in a window that never asked.
    let mut library_open = use_signal(|| false);

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
            "data-insp": if insp_open() { "open" } else { "closed" },

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
                // WHICH HALF THE WINDOW IS IN, in the window's own bar.
                //
                // The switch says it too, and that is not a duplication: the
                // switch can be off screen. It is inside the sidebar, and the
                // sidebar collapses — by the toggle, by the chord, and on its
                // own below the two-column width. The band is the one strip
                // that is present at every width and in every state, which is
                // the same argument that put the connection badge here.
                //
                // AND IT SAYS IT WITH THE GLYPH TOO, which is #130's other
                // half. This shipped as a bare word in all 13 captured
                // `desktop-` states while the switch six lines of markup away
                // drew `Icon { name: half.icon() }` beside its own — so the
                // one strip that is always on screen was the one place the
                // half was named in words only, and the mockups put the mark
                // in all three places they name it. No new class: `Icon`
                // renders `class="icon"`, which
                // `every_class_the_desktop_shell_renders_is_in_the_captured_store`
                // already has.
                span { class: "plane-badge",
                    Icon { name: plane.icon() }
                    "{plane.label()}"
                }

                {
                    let crumb = crumb_parts(
                        dest,
                        plane,
                        on_home,
                        nav_open(),
                        crumb,
                        band_after(&ctx, plane),
                    );
                    rsx! {
                    div { class: "chrome-title",
                        if let Some(parent) = crumb.parent {
                            span { class: "chrome-parent", "{parent}" }
                            span { class: "chrome-sep", "/" }
                        }
                        h1 { class: "chrome-heading", "{crumb.leaf}" }
                        if let Some(after) = crumb.after {
                            span { class: "chrome-sub", "{after}" }
                        }
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
                // `assets/desktop/` hid the detail's above 902px. The
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
                PlaneConn { plane }

                button {
                    class: "insp-toggle",
                    title: insp_toggle_label(insp_open()),
                    "aria-label": insp_toggle_label(insp_open()),
                    "aria-expanded": if insp_open() { "true" } else { "false" },
                    onclick: move |_| {
                        let now = *insp_open.peek();
                        insp_open.clone().set(!now);
                    },
                    Icon { name: "inspector" }
                }
            }

            div { class: "shell-body",

            // NOT `.drawer.open`. Three reasons, all load-bearing: `.drawer`
            // is `position: absolute` with `translateX(-100%)` and would have
            // to be fought rather than reused; `src/domdump.rs` files any dump
            // containing `.drawer.open` under a `-drawer` suffix, so every
            // desktop capture would be named as if a panel were over it; and
            // nothing has to move out of `assets/shared.css`, because
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
                    // NO WORDMARK. `h2.drawer-brand` was the first child here
                    // and it was the largest type in the column — 22px on a
                    // 32px line with 24 below it, 56px of a 252px-wide panel
                    // spent saying the name of the app the reader just
                    // launched. It is the phone's, where the drawer is a
                    // panel that slides over the page and has to announce
                    // whose panel it is; a window's title is the window's.
                    //
                    // It also appears in none of the eight E-option mockups:
                    // every one of them starts `.side` with the plane switch.
                    // The first thing in this column is now the choice the
                    // whole shell is organised around.
                    //
                    // THE PLANE SWITCH, and the top-level shape of this shell.
                    //
                    // Chat is goose's own things, where nothing touches a repo;
                    // Code is the OpenCode plane and its working trees. The two
                    // halves are separate all the way down — own list, own
                    // library, own vocabulary — so this is the only control in
                    // the app that crosses between them.
                    //
                    // `radiogroup` and not `tablist`, and the distinction is
                    // the design's rather than the markup's: there are no tabs
                    // anywhere in this shell, and a screen reader told these
                    // were tabs would announce a tab panel that does not exist.
                    // Two mutually exclusive modes IS a radio group, and it is
                    // what the pattern is for.
                    div { class: "plane-switch", role: "radiogroup", "aria-label": "Half",
                        for half in Plane::ALL {
                            button {
                                key: "{half.label()}",
                                class: if half == plane { "plane-seg active" } else { "plane-seg" },
                                role: "radio",
                                "aria-checked": if half == plane { "true" } else { "false" },
                                // Go to the half's own opening destination
                                // rather than setting a mode beside the
                                // navigation: the plane is READ back off
                                // whatever is on screen (see `plane` above), so
                                // a switch that only set a signal would say
                                // Code while a chat was open.
                                onclick: move |_| (nav::primary(half).go)(&ctx),
                                Icon { name: half.icon() }
                                "{half.label()}"
                                // HOW MUCH IS IN THE OTHER HALF, which is the
                                // only thing this control can say about the
                                // side you are not looking at.
                                if let Some(n) = seg_count(&ctx, half) {
                                    span { class: "plane-seg-count", "{n}" }
                                }
                            }
                        }
                    }

                    // NEW, and only while something is open.
                    //
                    // The owner's rule, and the composer is the reason: "I
                    // don't think we need a new chat button when the big chat
                    // box is visible in the middle." On the plane's own home
                    // screen the composer IS the new-session affordance, so a
                    // button here would be a second door onto the same room.
                    // It comes back the moment the main column is showing
                    // something else, because then there is no composer to
                    // press.
                    //
                    // It falls out of `on_home` rather than being its own
                    // condition, so the button and the pane cannot disagree
                    // about which screen is up.
                    //
                    // `go_root` AND NOT `go`, and the difference is the whole
                    // of #197. `go` means "restore this destination's stack
                    // where you left it" (`src/nav.rs`), which for Code is one
                    // `tab.set(Tab::Code)` — and the condition above means this
                    // button is only ever on screen where the tab is ALREADY
                    // `Tab::Code`. So the press wrote `Tab::Code` over
                    // `Tab::Code` and the conversation the reader was trying to
                    // leave re-rendered: a control labelled "New code session"
                    // that did nothing from the chat, the review screen and the
                    // pull list alike, and from Settings reopened an old
                    // session. The Chat half worked by accident — `CHATS.go`
                    // has to name a screen because Chats and Settings share
                    // `ctx.screen`, and the screen it names is the root.
                    if !on_home {
                        button {
                            class: "nav-new",
                            title: new_label(plane),
                            onclick: move |_| (nav::primary(plane).go_root)(&ctx),
                            Icon { name: "plus" }
                            "{new_label(plane)}"
                        }
                    }

                    // THE LIBRARY, behind one row.
                    //
                    // What the half has SAVED, as against what it is doing —
                    // recipes, skills, schedules, extensions on the chat side;
                    // nothing yet on the code side, because the gateway has no
                    // commands, skills or MCP endpoints to list. An empty
                    // library renders no row at all rather than a row that
                    // expands onto nothing.
                    //
                    // A disclosure and not the flat list this shell had until
                    // now, because the sidebar's body is the session list
                    // beneath it: setup you visit weekly must not push aside
                    // the thing you scan every minute. Shut by default for the
                    // same reason.
                    if !library.is_empty() {
                        button {
                            class: if library_open() { "nav-library open" } else { "nav-library" },
                            "aria-expanded": if library_open() { "true" } else { "false" },
                            onclick: move |_| {
                                let now = *library_open.peek();
                                library_open.set(!now);
                            },
                            Icon { name: "chevron-right" }
                            span { class: "nav-library-icon", Icon { name: "grid" } }
                            span { class: "nav-library-label", "Library" }
                            span { class: "nav-library-count", "{library.len()}" }
                        }
                        if library_open() {
                            nav { class: "drawer-nav",
                                for dest in library.iter().copied() {
                                    {render_destination(&ctx, dest)}
                                }
                            }
                        }
                    }

                    // THE PLANE'S OWN LIST, and the sidebar's body.
                    //
                    // Its own component rather than more rsx here, because
                    // `AppShell` cannot be mounted in a test — it calls
                    // `dioxus::desktop::window()`, which panics without an
                    // event loop — and `SidebarList` reads `AppCtx` and
                    // nothing else. See `sidebar.rs` for why it is not the
                    // list VIEW that already exists.
                    sidebar::SidebarList { plane }

                    // Below the fold, and outside the scroller: what belongs to
                    // neither half. Settings configures BOTH servers, so filing
                    // it under a plane would hide the code gateway's fields
                    // behind the chat half.
                    div { class: "nav-footer",
                        // The one line of the mockups' `.ident` that has a
                        // source. See `standing_line`.
                        if let Some(line) = standing_line(&ctx, plane) {
                            p { class: "nav-standing", "{line}" }
                        }
                        for dest in nav::plane_free() {
                            {render_destination(&ctx, dest)}
                        }
                    }
                }
            }

            // ONE CONTENT COLUMN, where there were two.
            //
            // The list moved into the sidebar, so the pane that used to hold
            // it is gone and with it `data-detail` — the attribute existed
            // only so the sheet could tell which of two columns had something
            // in it, and one column does not need telling.
            //
            // Three arms, and the order is the rule: whatever the destination
            // has pushed, else the plane's HOME when nothing is pushed and you
            // are on its primary, else the destination's own root — the
            // Recipes grid, the Skills grid, screens the sidebar does not
            // carry.
            //
            // The home arm was held back until the sidebar could take the
            // controls the primary's root was carrying. It is not a cosmetic
            // ordering: an earlier draft put a "Nothing open" card here while
            // rename, delete and search still lived only in that root, and
            // took all three off the desktop without a word. `sidebar.rs` now
            // has them, which is what makes this switch safe —
            // `the_home_screen_does_not_cost_the_controls_it_replaced` is the
            // check that keeps it that way.
            //
            // `empty_detail` stays for the one destination that has no root:
            // Settings' detail is unconditional so it never reaches that arm,
            // and a `root: None` row rendering nothing would be a blank pane.
            section { class: "pane pane-main",
                if let Some(detail) = detail {
                    {detail.view}
                } else if on_home {
                    home::Home { plane }
                } else if let Some(root) = dest.root {
                    {root(&ctx)}
                } else {
                    {empty_detail(dest)}
                }
            }

            // THE THIRD COLUMN. Rendered unconditionally and hidden by the
            // sheet, not by Rust: `data-insp` on `.shell` above is the one
            // fact, and `assets/desktop/` decides both whether the column
            // has a width and whether the window is wide enough for it. That
            // is this shell's standing rule — width decides only how many
            // columns, entirely inside the stylesheet, so nothing in Rust ever
            // listens to a DOM resize.
            inspector::Inspector { plane, on_subject }

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
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use dioxus::document::{Document, Eval, EvalError, Evaluator};
    use dioxus::prelude::*;

    use super::MIN_INNER;
    use super::{band_after, conn_of, crumb_parts, seg_count, standing_line};
    use crate::nav::Plane;
    use crate::nav::{Destination, DESTINATIONS};
    use crate::state::{AppCtx, ConnState};

    // The layout's numbers. None of them appears in the binary — the
    // columns are drawn by `assets/desktop/` and nothing else, which is
    // exactly what keeps every resize listener out of this app. They live
    // here because being checkable is the only job they have.
    /// The sidebar's width, and the mockups' number — read out of their CSS
    /// (`grid-template-columns: 268px minmax(0,1fr) 344px`) rather than off
    /// the picture.
    ///
    /// It grew from 212 when the plane's list moved into it. That was not a
    /// preference: measured in Chromium against the real sheets, the old list
    /// view's title box renders 0px wide at 212 and 25px at 270, and reaches
    /// its former parity only near 390. 268 is the design's answer, and it is
    /// wide enough for the compact row `sidebar.rs` renders instead. Still
    /// inside goose's own 160..400 band for the same panel
    /// (`ui/desktop/src/components/Layout/constants.ts`).
    const NAV: u32 = 268;

    /// The narrowest a content column is allowed to get.
    ///
    /// The app's own floor, not a guess: `docs/design.md` records that "320pt is a
    /// defensive width rather than a device; the narrowest this app really meets
    /// is 360", and `node docs/measure-composer.js 360` is a gate. Below it the
    /// composer's chip row is documented to fail.
    const CONTENT_MIN: u32 = 360;

    /// Where the sidebar stops reserving a column and starts floating over
    /// one.
    ///
    /// The sum of the two above, and the only breakpoint this shell has left.
    /// There used to be two — a three-column sum and a two-column one — because
    /// there used to be three columns; the list moved into the sidebar and took
    /// the middle one with it.
    ///
    /// Below this the sidebar becomes an overlay rather than a 72px rail. The
    /// rail is gone with the same restructure: it worked while the sidebar
    /// held seven destination icons and cannot survive it holding a list,
    /// because 14px of content box is not a session title. `MIN_INNER` is
    /// unchanged at 480 — a content column with an overlay available is a
    /// usable window, so nothing forces the floor up.
    pub(crate) const OVERLAY: u32 = NAV + CONTENT_MIN;

    /// The window minimum and the stylesheet's breakpoint are one decision
    /// held in two languages, and only one of them can be compiled. Raise the
    /// floor above the breakpoint and the overlay tier becomes unreachable —
    /// designed, styled, and impossible to see; lower it past the measure and
    /// the window opens onto a content column narrower than anything this app
    /// is gated at.
    #[test]
    fn the_window_floor_lands_inside_the_narrowest_tier() {
        let (width, height) = MIN_INNER;
        assert!(
            width < f64::from(OVERLAY),
            "a {width}pt floor is at or above the {OVERLAY}pt breakpoint, so \
             the overlay tier can never be reached"
        );
        // The whole window is the content column below the breakpoint — the
        // sidebar floats over it rather than taking a share — so the floor has
        // only to clear the measure itself. That is the change the overlay
        // bought: under the old rail the sidebar kept 72pt at every width, and
        // this assertion had to add them.
        assert!(
            width >= f64::from(CONTENT_MIN),
            "a {width}pt floor leaves the one content column narrower than \
             the {CONTENT_MIN}pt this app is measured at"
        );
        assert!(height > 0.0);
    }

    /// THE HOME SCREEN MUST NOT COST THE CONTROLS IT REPLACED.
    ///
    /// The pane used to render the plane's primary root — the full list view,
    /// which carried rename, delete and search. Now it renders `home::Home`
    /// instead, and those three exist on the desktop only because
    /// `sidebar.rs` took them first.
    ///
    /// That ordering was not free. An earlier draft put a "Nothing open" card
    /// in this arm while the controls still lived only in the root, and took
    /// all three off the desktop silently — measured on the captured markup at
    /// the time, `desktop-chats` held three `nav-row`s, zero `session-item`s
    /// and zero `row-action`s. A restructure may move a control; it may not
    /// drop one in passing.
    ///
    /// So this is a COUPLING check across two files, which is the only shape
    /// that can state the rule: the moment the pane grows a home arm, the
    /// sidebar owes the controls. Reading both sources rather than rendering,
    /// because `AppShell` cannot be mounted (it calls
    /// `dioxus::desktop::window()`) and the two halves are in different
    /// components — `sidebar.rs`'s own tests assert the controls RENDER; this
    /// asserts they are required to.
    ///
    /// Shown to fail: delete the search field from `sidebar.rs` and this goes
    /// red naming it, while every test in that file still passes except its
    /// own.
    #[test]
    fn the_home_screen_does_not_cost_the_controls_it_replaced() {
        let code = shell_code();
        let pane = block(
            &code,
            "section { class: \"pane pane-main\",",
            "fn empty_detail",
        );
        assert!(
            pane.contains("home::Home"),
            "the content pane no longer renders a home screen, so this check \
             is asserting a coupling that has stopped existing"
        );

        let sidebar =
            crate::selfscan::code_of("src/shell/desktop/sidebar.rs", include_str!("sidebar.rs"));
        for (needle, what) in [
            ("nav-row-actions", "rename and delete"),
            ("SearchField", "the search field"),
        ] {
            assert!(
                sidebar.contains(needle),
                "the pane renders a home screen instead of the plane's list, \
                 and the sidebar does not carry {what} — so it is reachable \
                 nowhere on the desktop. That is the regression the home arm \
                 was held back for."
            );
        }
    }

    /// EVERY TOKEN THE DESKTOP SHEET SPENDS IS ONE THAT EXISTS.
    ///
    /// `var(--nope)` is not an error. It resolves to nothing, the declaration
    /// is dropped, and the element inherits — so a mistyped token is a rule
    /// that silently does not apply, which is the same failure mode as the
    /// stray brace above and just as invisible.
    ///
    /// Found by writing four of them at once. The home screen was drafted
    /// against `--text-3xl`, `--lh-3xl` and `--lh-2xl`, none of which exist:
    /// `assets/shared.css`'s scale stops at `--text-2xl` and its own comment
    /// explains why there is no `--lh-lg`. The greeting would have rendered at
    /// whatever it inherited, on every window, and nothing would have said so.
    ///
    /// Definitions are collected from BOTH sheets because the desktop declares
    /// its own (`--nav-w`, `--shell-gap`, `--chrome-h`, `--nav-fill`) and
    /// inherits the rest. A `var()` with a fallback is skipped: that is the
    /// one form where an undefined name is deliberate.
    #[test]
    fn the_desktop_sheet_spends_no_token_that_does_not_exist() {
        let main = include_str!("../../../assets/shared.css");
        let desktop = crate::css::SHELL;
        let macos = include_str!("../../../assets/platform/macos.css");

        let defined: std::collections::HashSet<&str> = [main, desktop, macos]
            .iter()
            .flat_map(|sheet| {
                sheet.match_indices("--").filter_map(|(at, _)| {
                    let rest = &sheet[at..];
                    let end = rest.find(':')?;
                    let name = &rest[..end];
                    // A definition is `--x:`; a use is `var(--x)`. Only the
                    // former has a colon before any bracket or space.
                    if name.contains([' ', ')', '(', ',', ';', '\n']) {
                        return None;
                    }
                    Some(name)
                })
            })
            .collect();
        assert!(
            defined.contains("--text-2xl"),
            "the definition scan found nothing recognisable, so the check \
             below would pass over any name at all"
        );

        let mut missing: Vec<&str> = Vec::new();
        for (at, _) in desktop.match_indices("var(--") {
            let rest = &desktop[at + 4..];
            let Some(end) = rest.find([')', ',']) else {
                continue;
            };
            // A comma means a fallback was given, which is the one form where
            // an undefined name is a deliberate choice.
            if rest.as_bytes().get(end) == Some(&b',') {
                continue;
            }
            let name = rest[..end].trim();
            if !defined.contains(name) && !missing.contains(&name) {
                missing.push(name);
            }
        }
        assert!(
            missing.is_empty(),
            "assets/desktop/ spends {} token(s) nothing defines: {}. \
             `var()` on an undefined name resolves to nothing and the whole \
             declaration is dropped, so those rules are silently not applying",
            missing.len(),
            missing.join(", ")
        );
    }

    /// THE SHEET IS BALANCED, which nothing else in this toolchain checks.
    ///
    /// A stray `}` in CSS is not a parse error a browser reports — it silently
    /// ends the enclosing block and everything after it is reinterpreted. That
    /// happened here: deleting the three-column media block left one closing
    /// brace behind, and the rules below it stopped applying. The visible
    /// symptom was the pane's own heading painted ON TOP of the screen's first
    /// element, because `.topbar { position: static }` was one of the rules
    /// that stopped winning and the phone's floating `position: absolute` took
    /// over.
    ///
    /// Nothing caught it. `cargo` does not read CSS; `docs/audit.js` renders
    /// the sheet and measures boxes, and a shell whose header floats is a
    /// layout it will happily measure. It was found by eye, in a screenshot,
    /// which is the slowest possible route to a one-character defect.
    ///
    /// Comments are stripped first because they contain braces — this file's
    /// own prose quotes rules — and a counter that read them would answer
    /// about the wrong thing.
    ///
    /// PART BY PART, not over the concatenation. `crate::css::SHELL` is fifteen
    /// files joined, so an unbalanced one reported against the whole string
    /// names 165k characters and a line number that belongs to no file on disk.
    /// Walking [`crate::css::SHELL_PARTS`] costs nothing and names the file —
    /// and it is strictly stronger, because a `}` too many in one region and a
    /// `{` too many in the next cancel out in the concatenation and are two
    /// failures here.
    #[test]
    fn the_stylesheet_closes_every_block_it_opens() {
        let sheets = crate::css::SHELL_PARTS
            .iter()
            .map(|&(name, raw)| (name, raw))
            .chain(std::iter::once((
                "assets/platform/macos.css",
                include_str!("../../../assets/platform/macos.css"),
            )));
        for (name, raw) in sheets {
            let mut code = String::with_capacity(raw.len());
            let mut rest = raw;
            while let Some((before, after)) = rest.split_once("/*") {
                code.push_str(before);
                rest = after.split_once("*/").map_or("", |(_, tail)| tail);
            }
            code.push_str(rest);

            let mut depth: i32 = 0;
            let mut line = 1;
            for ch in code.chars() {
                match ch {
                    '\n' => line += 1,
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        assert!(
                            depth >= 0,
                            "{name} closes a block it never opened, at about line \
                             {line} of the comment-stripped sheet — every rule \
                             after it is being read at the wrong nesting level"
                        );
                    }
                    _ => {}
                }
            }
            assert_eq!(
                depth, 0,
                "{name} leaves {depth} block(s) open at the end of the file, so \
                 the rules inside them apply under a selector or a media query \
                 nobody wrote"
            );
        }
    }

    /// Where the sidebar stops taking a column is decided entirely inside
    /// `assets/desktop/`, which is exactly what keeps every resize listener
    /// out of this app — and it is also what puts the number out of the
    /// compiler's reach. This is the only thing that notices when the
    /// arithmetic above and the sheet stop agreeing.
    ///
    /// One breakpoint where there were two. The three-column sum went with the
    /// list column; the sheet is asserted to hold neither of the old ones, so
    /// a media query left behind by a half-finished revert is a failure rather
    /// than dead CSS nobody reads.
    #[test]
    fn the_stylesheet_breaks_where_the_arithmetic_says() {
        // Comments stripped first. This file's own prose explains which tiers
        // were deleted and why, so it names them — and a `contains` over the
        // whole sheet would read that explanation as the thing it forbids.
        // `crate::selfscan::code_of` drops comment lines from Rust for exactly
        // this reason; CSS has one comment form and it is easier.
        let raw = crate::css::SHELL;
        let mut sheet = String::with_capacity(raw.len());
        let mut rest = raw;
        while let Some((before, after)) = rest.split_once("/*") {
            sheet.push_str(before);
            rest = after.split_once("*/").map_or("", |(_, tail)| tail);
        }
        sheet.push_str(rest);
        assert!(
            sheet.contains(".navpane"),
            "the comment stripper ate the rules as well as the prose"
        );
        let rule = format!("@media (max-width: {}px)", OVERLAY - 1);
        assert!(
            sheet.contains(&rule),
            "assets/desktop/ has no `{rule}`, so the sidebar does not start \
             floating where src/shell/desktop/mod.rs says it does"
        );
        for stale in ["max-width: 901px", "max-width: 571px"] {
            assert!(
                !sheet.contains(stale),
                "assets/desktop/ still has a `{stale}` query — that is a \
                 tier from the three-column layout, and the columns it reflowed \
                 no longer exist"
            );
        }
    }

    // ---- reading the shell's source, without reading these tests ---------
    //
    // Four of the assertions below are of the form "the shell still writes the
    // thing the stylesheet reads", which is a question no compiler in this
    // build can answer: Rust writes an attribute or a class, CSS is the only
    // thing that reads it, and nothing connects them. They were written as
    // `include_str!` of the shell's own file and a `contains` — and the file that pulls
    // in is THIS one, test module and all, so each needle was supplied by the
    // assertion asking for it and the suite could not fail. Measured: with
    // `use_fullscreen`, its call site and the `data-fullscreen` attribute all
    // deleted, `cargo test --package goose-mobile` reported 231 passed, 0
    // failed; with the band's `ConnBadge` and the `data-detail` attribute
    // deleted, the ten tests in this module were ten passes.
    //
    // `crate::selfscan::code_of` is the repair and its module comment carries
    // the whole argument, including the two stronger mechanisms that were
    // tried and rejected. What is left here is the two slices, which ask a
    // sharper question than the old scans did: not whether the file mentions
    // the attribute but which ELEMENT carries it.
    //
    // That is not a hypothetical distinction. `data-fullscreen` has already
    // moved element once in this feature's short life — it was on `.app`,
    // written by JS, with `assets/platform/macos.css` selecting
    // `.app[data-fullscreen="true"] > .shell` to match, and both halves came
    // across to `.shell` together in ece4857. A move is exactly what a
    // file-wide `contains` cannot see: it goes on finding the string wherever
    // it lands, so a half-finished move — render across, sheet not, or the
    // reverse — reads as a working feature. Nothing else in the repo asks
    // either, `docs/audit.js` least of all: the audit sets `data-nav` and
    // `data-fullscreen` on `.shell` itself before it measures, so it would
    // report a clean grid against a shell that writes neither.

    /// `src/shell/desktop/mod.rs` with this test module, and every comment, taken
    /// out of it.
    fn shell_code() -> String {
        crate::selfscan::code_of("src/shell/desktop/mod.rs", include_str!("mod.rs"))
    }

    /// The attributes on the `.shell` div — the element `assets/desktop/`
    /// and `assets/platform/macos.css` key every state rule they have off.
    fn shell_attributes() -> String {
        let code = shell_code();
        block(&code, "class: \"shell\",", "header {").to_owned()
    }

    /// What the window's own bar renders. The other half of the same
    /// question: `.shell-chrome` is where the connection badge has to be,
    /// because `assets/desktop/` hides every pane's copy of it
    /// unconditionally.
    fn chrome_band() -> String {
        let code = shell_code();
        block(
            &code,
            "header { class: \"shell-chrome\",",
            "div { class: \"shell-body\",",
        )
        .to_owned()
    }

    /// What the sidebar renders: everything inside `.navcard`.
    ///
    /// The third slice, and the same question the other two ask — not whether
    /// the file mentions a class but which ELEMENT carries it. The switch,
    /// the destination list and the footer are three siblings whose ORDER is
    /// the layout (`.drawer-nav` is the only one of the three that scrolls, so
    /// a switch that drifted inside it would scroll away), and a file-wide
    /// `contains` cannot tell any of them apart.
    ///
    /// It ends at `section { class: "pane`, and the choice of terminator is
    /// the whole care here — a care that has since been collected, which is
    /// why this reads in the past tense. It used to end at
    /// `if let Some(root) = dest.root` — the line that guarded the list column
    /// — and the three-column restructure (#40) deleted exactly that line:
    /// the list moved into the sidebar and the content area stopped being
    /// conditional. Ending there would have taken three tests red at once with
    /// a message about a slice that ran to the end of the file, naming no
    /// feature and pointing at nothing.
    ///
    /// `section { class: "pane` survived it, because it is a prefix of what
    /// followed the sidebar before (`pane pane-list`) AND of what follows it
    /// now (`pane pane-main`, the only pane left — `.pane-list` and
    /// `.pane-detail` were both deleted with the middle column). It is also
    /// not `chrome_band`'s terminator, which `div { class: "shell-body",`
    /// already is — two slices sharing an end anchor is two slices that
    /// cannot both be right once anything moves between them.
    fn nav_card() -> String {
        let code = shell_code();
        block(
            &code,
            "div { class: \"navcard\",",
            "section { class: \"pane",
        )
        .to_owned()
    }

    /// The sidebar slice is BOUNDED, which is the half `block`'s own asserts
    /// cannot check.
    ///
    /// `block` proves both anchors were found, so it catches a slice that is
    /// empty and a slice that runs to the end of the file. It cannot catch a
    /// slice that merely grew — and a `nav_card()` that quietly swallowed the
    /// panes would make every assertion above it vacuously true, which is the
    /// same failure `crate::selfscan` exists to prevent, one level along. The
    /// three sidebar tests would then go on passing with the sidebar deleted,
    /// as long as the strings appeared anywhere in the render.
    ///
    /// So this names things that live AFTER the sidebar and requires them to
    /// be outside it. It is the check that let the terminator be changed
    /// through the restructure — and that will let it be changed again —
    /// without anyone having to re-derive by hand what the slice now covers.
    ///
    /// `pane-detail` in the list below is a class this shell no longer emits
    /// anywhere: the restructure replaced `.pane-list`/`.pane-detail` with the
    /// single `.pane-main`. Its assertion is therefore vacuous today and is
    /// kept as a name that must not come back INSIDE the sidebar rather than
    /// as a live boundary check — the live ones are `pane-empty`,
    /// `shell-chrome` and `conn-badge`, all three of which the shell renders.
    #[test]
    fn the_sidebar_slice_stops_at_the_sidebar() {
        let card = nav_card();
        for inside in ["plane-switch", "plane-seg", "nav-footer"] {
            assert!(
                card.contains(inside),
                "the sidebar slice no longer covers `{inside}`, so the tests \
                 that assert on it are asking about the wrong region"
            );
        }
        for outside in ["pane-detail", "pane-empty", "shell-chrome", "conn-badge"] {
            assert!(
                !card.contains(outside),
                "the sidebar slice has widened far enough to include \
                 `{outside}`, which is not in the sidebar — every assertion \
                 made against this slice is now weaker than it reads"
            );
        }
    }

    /// The sidebar writes the three class names `assets/desktop/` styles,
    /// and the sheet still styles all three.
    ///
    /// Two halves of one decision in two languages, which is the class of rule
    /// `crate::selfscan` exists for: Rust emits a class, CSS is the only thing
    /// that reads it, and a rename on either side leaves a control that is
    /// visibly unstyled with nothing in the compiler to say so. The switch is
    /// the worst case in this shell — unstyled, `.plane-seg` is two bare
    /// buttons with no track, which reads as a layout bug rather than as a
    /// missing rule, and it is the only route between the app's two halves.
    ///
    /// Shown to fail, both ways: renaming `plane-switch` to `plane-tabs` in
    /// the rsx fails on the first assertion; deleting the `.plane-seg.active`
    /// block from the sheet still passes (the base rule remains) but deleting
    /// every `.plane-seg` rule fails on the second.
    #[test]
    fn the_sidebar_writes_the_classes_the_sheet_styles() {
        let card = nav_card();
        let sheet = crate::css::SHELL;
        for class in [
            "plane-switch",
            "plane-seg",
            "nav-footer",
            "nav-new",
            "nav-library",
        ] {
            assert!(
                card.contains(&format!("\"{class}")),
                "the sidebar no longer renders `{class}`, which assets/desktop/ \
                 still has rules for"
            );
            assert!(
                sheet.contains(&format!(".{class}")),
                "assets/desktop/ has no rule for `.{class}`, which the sidebar \
                 renders — so the control ships unstyled"
            );
        }
    }

    /// Every half is reachable from the switch, and the one row that belongs
    /// to neither half is reachable from the footer.
    ///
    /// Both are spelled as a loop over the table rather than as two literal
    /// buttons, and this is what holds that: a third plane, or a second
    /// plane-free destination, then arrives as a row in `src/nav.rs` and
    /// appears here on its own. Written out because the failure is silent —
    /// a hard-coded pair goes on rendering two perfectly good segments while
    /// the third half of the app has no way in.
    ///
    /// Shown to fail: replace the `for half in Plane::ALL` loop with two
    /// literal `Plane::Chat` / `Plane::Code` buttons and the first assertion
    /// goes; drop the footer's loop and the second does.
    #[test]
    fn the_sidebar_is_generated_from_the_table_and_not_from_a_list() {
        let card = nav_card();
        assert!(
            card.contains("Plane::ALL"),
            "the plane switch no longer iterates `Plane::ALL`, so a half added \
             to the table would have no control that reaches it"
        );
        assert!(
            card.contains("nav::plane_free()"),
            "the sidebar footer no longer iterates `nav::plane_free()`, so \
             Settings — the one destination in neither half — has no way in"
        );
    }

    /// Pressing a segment NAVIGATES; it does not set a mode beside the
    /// navigation.
    ///
    /// The distinction is the whole reason `plane` is derived from
    /// `dest.plane` rather than held in a signal, and getting it wrong is not
    /// a visible bug on the day it is written: a switch that only set a local
    /// signal would look right on every click and then say "Code" over a chat
    /// the moment anything else navigated — ⌘R, the palette, an arrival
    /// effect. Two sources for one fact, and the render is the one that loses.
    ///
    /// A source scan because there is nothing else to ask: the alternative is
    /// mounting `AppShell`, which needs a `DesktopContext` and an `AppCtx`
    /// with a live storage provider under it, and `crate::selfscan`'s module
    /// comment records why faking those renders a component that is not the
    /// one that ships.
    ///
    /// Shown to fail: replace the `onclick` with `plane_signal.set(half)` and
    /// this goes red while every other test in the file stays green.
    #[test]
    fn the_switch_navigates_rather_than_setting_a_mode() {
        let card = nav_card();
        assert!(
            card.contains("(nav::primary(half).go)(&ctx)"),
            "a plane segment no longer navigates to that half's own \
             destination, so the switch and the pane can disagree about which \
             half the window is in"
        );
    }

    /// New MAKES ONE; it does not restore the one you had.
    ///
    /// The sidebar's two navigating controls press the two halves of
    /// `src/nav.rs`'s pair and it matters which is which. The switch presses
    /// `go`, because "show me the other half" has nothing to reset. This
    /// presses `go_root`, because a control called "New code session" that
    /// lands you back in an old one is worse than one that is missing.
    ///
    /// It shipped pressing `go`, and #197 is what that cost: `CODE.go` is one
    /// `tab.set(Tab::Code)`, the button renders only when `on_home` is false,
    /// and on the Code half that means the tab is already `Tab::Code` — so
    /// every press from a code chat, the review screen or the pull list wrote
    /// `Tab::Code` over `Tab::Code` and re-rendered the same screen. The Chat
    /// half worked by accident, because `CHATS.go` shares `ctx.screen` with
    /// Settings and so has to name a root on the way in.
    ///
    /// A source scan for `the_switch_navigates_rather_than_setting_a_mode`'s
    /// reason, written out there: `AppShell` calls
    /// `dioxus::desktop::window()` and cannot be mounted. What `go_root` DOES
    /// is held on the table itself, by `opening_a_destination_lands_on_its_own_root`
    /// in `src/nav.rs`, from a pushed screen; this is what says the button
    /// reaches for it.
    ///
    /// Shown to fail: put `.go` back in that `onclick` and this goes red while
    /// every other test in both files stays green — which is exactly the state
    /// the shell shipped in.
    #[test]
    fn the_new_button_resets_the_half_rather_than_restoring_it() {
        let card = nav_card();
        assert!(
            card.contains("(nav::primary(plane).go_root)(&ctx)"),
            "the sidebar's New button no longer resets the half to its own \
             root, so on the Code plane — where it only ever renders with the \
             tab already `Tab::Code` — it writes the tab it is already on and \
             does nothing at all"
        );
        assert!(
            !card.contains("(nav::primary(plane).go)(&ctx)"),
            "something in the sidebar still navigates with `go` where the \
             plane is already the one on screen, which is the shape of #197"
        );
    }

    /// One rsx block: what lies between the thing that opens it and the thing
    /// that opens whatever comes next.
    ///
    /// Deliberately crude — it counts no braces, and does not need to. Both
    /// call sites bound a run of attributes or of children by the element that
    /// follows it, and rustfmt keeps each of those on a line of its own. The
    /// failure modes of a crude slice are a slice that is empty and a slice
    /// that runs to the end of the file, and both of them would make every
    /// assertion below vacuous rather than wrong, so both ends assert.
    fn block<'a>(code: &'a str, opens: &str, ends_before: &str) -> &'a str {
        let after = code.split_once(opens).map(|(_, rest)| rest);
        assert!(
            after.is_some(),
            "src/shell/desktop/mod.rs no longer contains `{opens}`, so the block \
             this test reads is not there to be read"
        );
        let after = after.unwrap_or_default();
        let body = after.split_once(ends_before).map(|(body, _)| body);
        assert!(
            body.is_some(),
            "`{opens}` is no longer followed by `{ends_before}`, so this slice \
             would run to the end of the file and stop saying anything about \
             where anything is"
        );
        body.unwrap_or_default()
    }

    /// Every class name the shell renders that only a stylesheet gives
    /// meaning to. Rust writes them, CSS is the only thing that reads them,
    /// and nothing in the compiler connects the two — so a rename on either
    /// side leaves a control that visibly does nothing, with no error
    /// anywhere. `.window-drag` is the worst of them: rename it and the
    /// window silently stops being draggable, because `src/main.rs` has taken
    /// the titlebar away and that strip is the only replacement.
    ///
    /// The one scan in this module that could already fail, and it was
    /// checked rather than assumed: `class: "traffic-slotz"` in the render
    /// fails it with "src/shell/desktop/mod.rs no longer renders .traffic-slot",
    /// because the needle is `class: "…"` and the list below holds the bare
    /// name. It reads `shell_code()` all the same, so that this stays true of
    /// the next name somebody adds to that list.
    #[test]
    fn every_class_the_shell_renders_is_styled_somewhere() {
        let shell = shell_code();
        // `format!` and not `concat!`: `concat!` takes literals only, and the
        // desktop sheet is now fifteen files joined by a `concat!` of its own
        // in `src/css.rs`, which is a const rather than a literal.
        let sheets = format!(
            "{}{}",
            crate::css::SHELL,
            include_str!("../../../assets/platform/macos.css"),
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
                "src/shell/desktop/mod.rs no longer renders .{class}"
            );
            assert!(
                styled(class),
                "nothing styles .{class}, which the shell still renders"
            );
        }
    }

    // ---- is the store still a description of this app? -------------------
    //
    // `docs/gallery-states.json` is 69 states of real markup, and everything
    // visual this repo checks reads it: `docs/audit.js` measures those states
    // and `docs/style-gallery.html` renders them. A capture REPLACES it, so
    // between captures it is a photograph — and the one thing a photograph
    // cannot show is that the thing it is of has changed. Markup lands, the
    // store keeps saying what the app said on the day it was driven, and the
    // audit goes on reporting Clean over it. That is not hypothetical: it has
    // already produced a Clean audit over a sidebar the app no longer
    // rendered.
    //
    // ONE DIRECTION IS DECIDABLE AND THE OTHER IS NOT, which is the whole
    // shape of this check. `src/selfscan.rs` rejects captured markup as
    // evidence and is right to: "the shell has stopped rendering this" cannot
    // be read off a capture, because a stale answer is still an answer. The
    // question here is the reverse one — the shell renders `.tree-branch`, is
    // there any captured state that contains it — and that one the source
    // settles, because the source is what ships. Source is authoritative for
    // what the app emits; the store is then measured against it, never the
    // other way round.

    /// Every class name the four files of the desktop shell render, mapped to
    /// the one that renders it.
    ///
    /// Through [`crate::selfscan::code_of`] for the reason its module comment
    /// gives at length: `include_str!` of a file pulls in that file's own test
    /// module, and a scan that reads its own assertions is a scan that cannot
    /// fail. The allowlist below is written in this same file, so without the
    /// cut every name on it would be its own evidence.
    fn rendered_classes() -> std::collections::BTreeMap<String, &'static str> {
        let mut out = std::collections::BTreeMap::new();
        for (name, source) in [
            ("mod.rs", include_str!("mod.rs")),
            ("home.rs", include_str!("home.rs")),
            ("inspector.rs", include_str!("inspector.rs")),
            ("sidebar.rs", include_str!("sidebar.rs")),
        ] {
            let code = crate::selfscan::code_of(name, source);
            for start in code.match_indices("class:").map(|(at, m)| at + m.len()) {
                let value = attribute_value(&code[start..]);
                // Odd `split('"')` fields are the string literals. No class
                // value in this tree contains an escaped quote, which is the
                // one thing that would fool both this and the scan below.
                for literal in value.split('"').skip(1).step_by(2) {
                    for token in literal.split_whitespace() {
                        // `class: "{class}"` and `class: "insp-step-dot
                        // {step.dot}"` interpolate: the name is decided at run
                        // time, and asking the store about the literal
                        // `{step.dot}` would be asking about a string no
                        // browser ever sees.
                        if token.contains('{') || token.contains('}') {
                            continue;
                        }
                        out.entry(token.to_owned()).or_insert(name);
                    }
                }
            }
        }
        out
    }

    /// The source of one `rsx!` attribute value: from `rest` to the comma that
    /// ends it.
    ///
    /// Not `split(',')`, because a dozen of the class attributes in this shell
    /// are `class: if row.selected { "nav-row on" } else { "nav-row" },` and a
    /// naive split takes the first branch and loses the second — along with
    /// `on`, `active`, `off`, `seen` and every other state modifier, which are
    /// exactly the names a capture is most likely never to have driven. Depth
    /// counts both braces and parens so `class: value_class(fact.mono,
    /// fact.accent),` is one value and not two.
    fn attribute_value(rest: &str) -> &str {
        let mut depth = 0_i32;
        let mut quoted = false;
        for (at, c) in rest.char_indices() {
            if quoted {
                quoted = c != '"';
                continue;
            }
            match c {
                '"' => quoted = true,
                '{' | '(' => depth += 1,
                '}' | ')' => {
                    depth -= 1;
                    // Out of the element this attribute is on: the value was
                    // the last one and had no trailing comma.
                    if depth < 0 {
                        return &rest[..at];
                    }
                }
                ',' if depth == 0 => return &rest[..at],
                _ => {}
            }
        }
        rest
    }

    /// Every class name any captured DESKTOP state contains.
    ///
    /// The desktop's half of the store alone. A phone state is markup from the
    /// other shell, and counting it would let `.warn` be "covered" by a phone
    /// banner while no desktop frame the audit measures has ever contained
    /// one — which is a state this app does not have, answering a question
    /// about a state it does.
    fn captured_classes() -> std::collections::BTreeSet<String> {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/gallery-states.json");
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let states: std::collections::BTreeMap<String, String> =
            serde_json::from_str(&raw).unwrap_or_default();
        assert!(!states.is_empty(), "cannot read {}", path.display());

        let mut out = std::collections::BTreeSet::new();
        let mut seen = 0_usize;
        for markup in states
            .iter()
            .filter(|(key, _)| key.starts_with(crate::shell::DUMP_PREFIX_DESKTOP))
            .map(|(_, markup)| markup)
        {
            seen += 1;
            for tail in markup.split("class=\"").skip(1) {
                let value = tail.split('"').next().unwrap_or_default();
                out.extend(value.split_whitespace().map(str::to_owned));
            }
        }
        assert!(
            seen > 0,
            "docs/gallery-states.json holds no `{}` state at all, so every \
             assertion below would be a claim about an empty set",
            crate::shell::DUMP_PREFIX_DESKTOP
        );
        out
    }

    /// THE GAP, NAMED. Classes the desktop shell renders that no captured
    /// state contains — so the audit has never measured one, and the gallery
    /// has never shown one.
    ///
    /// THIS LIST MAY ONLY SHRINK. It is not a set of exemptions: it is the
    /// unmeasured surface of this shell, written down at the size it was on
    /// the day the check landed, and the test below fails if an entry stops
    /// being needed — either because a capture reached it or because the shell
    /// stopped rendering it. Adding to it is how the gate this whole block
    /// exists to build gets given away one name at a time; the answer to a new
    /// name is a capture that drives the screen it is on, not a line here.
    ///
    /// What is left on it is four names, and all four are unreachable rather
    /// than merely undriven — which is the distinction that matters, because
    /// the other forty-nine went away by being driven. The capture that
    /// removed them ran both fakes, connected both halves, opened a code
    /// session, a review, a pull request and a goose turn that called two
    /// tools, and parked a permission on the board so the tiles could be seen
    /// counting one. What no drive can reach:
    ///
    /// `pane-empty` and its two children are `empty_detail`, and the arm that
    /// calls it is dead. It runs for a destination with `root: None` and no
    /// detail; `src/nav.rs` has exactly one `root: None` row — Settings — and
    /// Settings' `detail` is unconditional, so the arm is never taken. The
    /// three names are the gate's own evidence for that, which is why they
    /// stay rather than being deleted with the function.
    ///
    /// `insp-empty` is the inspector saying it has nothing, and
    /// `inspector::plane_facts` returns empty only when the plane's server URL
    /// is *unset* — not when it is set and unreachable, which is what a killed
    /// fake gives you. A `dev_seed!` build has both URLs compiled in, so the
    /// only route to it is a first launch with an untouched Settings screen,
    /// and that is a launch whose every other state is emptier than the one
    /// already in the store. It went uncaptured because the state that used to
    /// carry it — `desktop-code-list` against a gateway that was not connected
    /// — is precisely the defect #140 asked to fix. One class' worth of
    /// coverage was the price of the whole Code board, and it is written here
    /// rather than argued again.
    const UNCAPTURED: &[&str] = &[
        // The inspector with no server URL to describe. See above: a seeded
        // build cannot reach it, and the state that used to was the bug.
        "insp-empty",
        // The pane with nothing in it. `empty_detail`'s arm is unreachable:
        // Settings is the only `root: None` destination and its detail is
        // unconditional.
        "pane-empty",
        "pane-empty-hint",
        "pane-empty-line",
    ];

    /// The store still describes the app — or says exactly where it does not.
    ///
    /// REPRODUCED, because a check that cannot fail is worse than none and
    /// this repo has shipped two of those. Both directions were measured on
    /// this tree.
    ///
    /// ADDITIVE, which is the campaign's standing rule for markup and so the
    /// staleness this will actually meet: one `span { class: "chrome-nonce" }`
    /// added to the window's bar and nothing else touched. This fails with
    /// ".chrome-nonce (mod.rs)" — and `node docs/audit.js both` on the very
    /// same tree reports **Clean**, because the audit reads the store and the
    /// stylesheets and no Rust at all. There is nothing else in the repo that
    /// can see an element the app gained and the store never did.
    ///
    /// A RENAME, the other shape: `class="navcard"` renamed across all
    /// thirteen desktop states, which is what a store one generation behind a
    /// renamed class looks like. This fails with ".navcard (mod.rs)". The
    /// audit reports 14,728 findings on that one — but that is the sidebar's
    /// layout collapsing without its panel rule, not the audit noticing a
    /// stale name, and it is a number a purely cosmetic class would not have
    /// produced. The audit measures what it is given; only this can ask
    /// whether it was given the right thing.
    ///
    /// It fails in the other direction too, which is what makes [`UNCAPTURED`]
    /// a ledger rather than a suppression: add a name to that list which is
    /// already in the store and the second assertion names it back.
    #[test]
    fn every_class_the_desktop_shell_renders_is_in_the_captured_store() {
        let rendered = rendered_classes();
        // The floor, `src/shell/mod.rs`'s habit: say out loud that the scan
        // found something. A scan that matched nothing would pass forever.
        assert!(
            rendered.len() > 120,
            "the class scan found only {} names across the four files of the \
             desktop shell, which is fewer than the shell has — has `class:` \
             stopped being how an attribute is written?",
            rendered.len()
        );
        let captured = captured_classes();

        let missing: Vec<String> = rendered
            .iter()
            .filter(|(class, _)| !captured.contains(class.as_str()))
            .filter(|(class, _)| !UNCAPTURED.contains(&class.as_str()))
            .map(|(class, file)| format!(".{class} ({file})"))
            .collect();
        assert!(
            missing.is_empty(),
            "the desktop shell renders {} class(es) that no captured state in \
             docs/gallery-states.json contains: {}. The store is a photograph \
             of the app and this is the app having changed since it was taken \
             — re-capture (`scripts/capture-gallery.py --only {} <log>` drives \
             the desktop half alone), because until then every audit and every \
             gallery frame is measuring markup that no longer ships.",
            missing.len(),
            missing.join(", "),
            crate::shell::DUMP_PREFIX_DESKTOP
        );

        let landed: Vec<&&str> = UNCAPTURED
            .iter()
            .filter(|class| captured.contains(**class))
            .collect();
        assert!(
            landed.is_empty(),
            "UNCAPTURED names {landed:?}, which the store now contains. The \
             list may only shrink: delete them from it, so that the next class \
             to go uncaptured is a failure and not a line in a list that has \
             stopped being true."
        );

        let gone: Vec<&&str> = UNCAPTURED
            .iter()
            .filter(|class| !rendered.contains_key(**class))
            .collect();
        assert!(
            gone.is_empty(),
            "UNCAPTURED names {gone:?}, which the desktop shell no longer \
             renders. Same rule: delete them. An entry that names nothing is \
             an entry nobody can check."
        );
    }

    /// The window's bar takes the detail's title and its connection badge, and
    /// `assets/desktop/` is what stops the pane below painting either of
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
    ///
    /// REPRODUCED, because as shipped it could not fail. Both shell-side
    /// assertions read `include_str!` of this very file, which includes this
    /// module, so the two needles below were supplied by the two lines
    /// containing them: delete `crate::views::ConnBadge {}` from the band and
    /// `"data-detail"` from the `.shell` div and all ten tests in this module
    /// still passed. Against `chrome_band()` and `shell_attributes()` the same
    /// two deletions fail here, one message each — "the window's bar no longer
    /// renders the connection badge" and "the `.shell` div no longer sets
    /// data-detail" — and putting them back is green again.
    #[test]
    fn the_pane_gives_up_what_the_window_bar_took() {
        let sheet = crate::css::SHELL;

        assert!(
            chrome_band().contains("PlaneConn { plane }"),
            "the window's bar no longer renders the connection badge, so \
             `.pane .topbar > .conn-badge` below now hides the only one there is"
        );
        for rule in [
            ":has(.chrome-title) .pane-main .topbar > .title",
            ":has(.chrome-title) .pane-main .topbar > .titlegroup",
            ".pane .topbar > .conn-badge",
        ] {
            assert!(
                sheet.contains(rule),
                "assets/desktop/ never mentions `{rule}`, so the pane paints \
                 a second copy of what `.shell-chrome` is already showing"
            );
        }

        // The other half of the same decision, and the reason this can be
        // asked of the markup at all now. The sheet suppresses the pane's
        // heading when `.chrome-title` is present, so the band has to be the
        // thing that renders `.chrome-title` — and it has to render it exactly
        // when there is a crumb to put in it. An `if let` that stopped being
        // conditional would hide every pane heading behind an empty bar.
        //
        // This replaces an assertion on `data-detail`. The attribute is gone:
        // it existed to tell the sheet which of two columns held content, and
        // there is one column now. `:has()` asks the markup directly, so the
        // bar's title and the pane's suppression can no longer drift apart —
        // which is the failure the attribute made possible and this test was
        // written for.
        let band = chrome_band();
        assert!(
            band.contains("crumb_parts(") && band.contains("band_after(&ctx, plane)"),
            "the window's bar no longer renders `.chrome-title` conditionally, \
             so `:has(.chrome-title)` either never matches — and every screen \
             paints two titles — or always does, and the screens the bar does \
             not name paint none"
        );
        // AND IT IS TOLD WHETHER THE SIDEBAR IS SHUT, which is the whole of
        // the half's counts reaching a collapsed window: `crumb_parts` gives
        // the sub slot to `band_after` only when the column that otherwise
        // carries the count is closed. Passing a constant here would be a
        // silent no-op — the string would still be computed, the slot would
        // still be filled on the home screen, and the one arrangement this
        // exists for would go on saying nothing.
        assert!(
            band.contains("nav_open(),"),
            "the band no longer passes the sidebar's state to `crumb_parts`, \
             so a shut sidebar takes the half's counts off screen with it and \
             nothing puts them back"
        );
    }

    /// THE BADGE WEARS THE HALF'S GLYPH, and for thirteen captured states it
    /// wore nothing — #130.
    ///
    /// `docs/gallery-states.json` has `<span class="plane-badge">Chat</span>`
    /// with zero child nodes in every `desktop-` key, against the mockups'
    /// `<span class="plane-badge"><svg …/> Chat</span>`. The switch six lines
    /// of markup away has drawn `Icon { name: half.icon() }` beside its own
    /// label since it was written, so the one strip that is on screen at every
    /// width and in every state was the only place this app named a half in
    /// words alone.
    ///
    /// NOTHING ELSE IN THE REPO CAN SEE IT, which is why this is here rather
    /// than left to the audit. `Icon` renders `class="icon"`, a class the
    /// store already holds, so
    /// `every_class_the_desktop_shell_renders_is_in_the_captured_store` is
    /// silent; `docs/audit.js` renders the STORE, and the store is the
    /// photograph that predates the glyph — it would report Clean over a badge
    /// with no mark in it and over the two declarations below as no-ops. That
    /// was measured, and it is what stopped the last attempt at this issue.
    ///
    /// BOTH DECLARATIONS, because each alone is a defect the other hides.
    /// Without `.plane-badge > .icon` the mark is `1em` of the badge's own
    /// font-size — 10px, the floor rung the LABEL is set in, against the
    /// mockups' 13px svg. Without the `gap` the two flex items touch: measured
    /// at 1440x860 against the real sheet list, the chip goes 48.22x19 →
    /// 66.22x19 with the glyph, a 13x13 mark at 8px in from its leading edge.
    ///
    /// REPRODUCED: drop the `Icon` line and the first assertion fails; drop
    /// either declaration from `assets/desktop/50-band.css` and the matching
    /// one does.
    #[test]
    fn the_bands_badge_wears_its_halfs_glyph() {
        let band = chrome_band();
        // Bounded to the span, not asked of the band: `Icon { … }` appears
        // three times in this bar and a file-wide `contains` would be answered
        // by the nav toggle's.
        let badge = block(&band, "span { class: \"plane-badge\",", "crumb_parts(");
        assert!(
            badge.contains("Icon { name: plane.icon() }"),
            "the window's bar names its half in words alone, so the one strip \
             that is present at every width says nothing a reader can \
             recognise without reading it: {badge}"
        );

        // COMMENTS OUT FIRST, and this is not tidiness — it is the difference
        // between a check and a tautology. Both declarations are ARGUED FOR in
        // prose beside themselves, and one of those comments quotes the
        // mockups' own `.plane-badge{…;gap:5px}` verbatim, so a `contains`
        // over the raw sheet is answered by the paragraph explaining the rule
        // rather than by the rule. Measured: delete both declarations and this
        // test still passed until this line went in. `crate::css` keeps the
        // stripper for exactly this class of mistake.
        let sheet = crate::css::without_comments(crate::css::SHELL);
        assert!(
            sheet.contains(".plane-badge > .icon"),
            "assets/desktop/ never sizes the badge's glyph, so it draws at the \
             10px floor its label is set in rather than the mockups' 13px"
        );
        let rule = sheet
            .split_once("\n.plane-badge {")
            .and_then(|(_, rest)| rest.split_once("\n}"))
            .map_or("", |(body, _)| body);
        assert!(
            rule.contains("gap:"),
            "`.plane-badge` is a flex box with two items in it and no gap, so \
             the branch mark touches the C of CODE: {rule}"
        );
    }

    /// THE BAND REPORTS THE HALF IT IS OVER, and it did not.
    ///
    /// `docs/gallery-states.json`'s `desktop-code-list` was captured showing a
    /// green dot and "goose-mock 1.47.0" — the CHAT server's agent string —
    /// across the top of a pane whose every row comes from the other wire.
    /// `views::ConnBadge` reads `ctx.conn` outright, which is right on a phone
    /// and wrong in a window with a plane switch in it.
    ///
    /// REPRODUCED: point `conn_of`'s `Code` arm at `(ctx.conn)()` — the state
    /// this shipped in — and this fails on the second assertion.
    #[test]
    fn the_band_reports_the_half_it_is_over() {
        let (chat, code) = crate::testkit::with_ctx(
            |ctx| {
                let mut conn = ctx.conn;
                conn.set(ConnState::Connected {
                    agent: "goose 1.47.0".to_owned(),
                });
                // The code gateway is not answering, which is the ordinary
                // case on the desktop until something dials it.
                let mut code = ctx.code_conn;
                code.set(ConnState::Disconnected);
            },
            |ctx| (conn_of(ctx, Plane::Chat), conn_of(ctx, Plane::Code)),
        );
        assert!(
            chat.is_connected(),
            "the chat half's own connection was lost"
        );
        assert!(
            !code.is_connected(),
            "the band reported the chat server's connection over the code \
             plane — a green dot for a socket nobody has opened, which is the \
             worst thing a status indicator can be"
        );
    }

    /// A COUNT IS ONLY OFFERED BY A HALF THAT ANSWERED.
    ///
    /// `home::standing`'s rule — "saying 12 conversations over a dead socket
    /// is the kind of confident wrongness that makes a reader stop trusting
    /// the whole screen" — applied to the band and the switch. It matters more
    /// here: the code half is not connected until something dials it, and the
    /// band is on screen the entire time, so an ungated count paints a
    /// confident `0` beside "Code" at every launch.
    ///
    /// REPRODUCED: drop the `is_connected()` gate from either function and the
    /// matching assertion fails.
    #[test]
    fn a_count_is_only_offered_by_a_half_that_answered() {
        fn seeded(ctx: &AppCtx) {
            let mut sessions = ctx.sessions;
            sessions.set(vec![session("s1"), session("s2")]);
        }
        fn seeded_and_up(ctx: &AppCtx) {
            seeded(ctx);
            let mut conn = ctx.conn;
            conn.set(ConnState::Connected {
                agent: "goose".to_owned(),
            });
        }
        let offline = crate::testkit::with_ctx(seeded, |ctx| {
            (
                seg_count(ctx, Plane::Chat),
                band_after(ctx, Plane::Chat),
                standing_line(ctx, Plane::Chat),
            )
        });
        assert_eq!(offline.0, None, "a count over a socket nobody opened");
        assert_eq!(offline.1, None);
        assert_eq!(offline.2, None);

        let online = crate::testkit::with_ctx(seeded_and_up, |ctx| {
            (
                seg_count(ctx, Plane::Chat),
                band_after(ctx, Plane::Chat),
                standing_line(ctx, Plane::Chat),
            )
        });
        assert_eq!(online.0, Some(2));
        let after = online.1.unwrap_or_default();
        assert!(after.starts_with("2 conversations"), "{after}");
        assert!(
            after.contains("nothing waiting on you"),
            "the band should answer the question rather than leave the clause \
             out when the answer is none: {after}"
        );
        assert_eq!(online.2.as_deref(), Some("2 conversations kept"));
    }

    /// THE CRUMB NAMES THE THING INSIDE THE THING IT IS IN, and never itself
    /// twice.
    ///
    /// Three arms and each is a different shape. An open detail is
    /// `parent / leaf`; a home screen is `parent / the half's own word`; a
    /// destination's own root is the leaf alone, because "Recipes / Recipes"
    /// is a stutter rather than a path. The fourth case is the one this
    /// shipped wrong: Settings is reachable as a detail AND is its own
    /// destination, so its crumb title and its label are the same string.
    ///
    /// REPRODUCED: write the first arm as an unconditional
    /// `parent: Some(dest.label)` — the state this shipped in — and the
    /// Settings assertion fails with "Settings / Settings".
    #[test]
    fn the_crumb_names_the_thing_inside_the_thing_it_is_in() {
        let chats = crate::nav::DESTINATIONS
            .iter()
            .find(|d| d.id == "chats")
            .unwrap_or(&crate::nav::DESTINATIONS[0]);

        let open = crumb_parts(
            chats,
            Plane::Chat,
            false,
            true,
            Some(crate::nav::Crumb {
                title: "A conversation".to_owned(),
                subtitle: None,
            }),
            None,
        );
        assert_eq!(open.parent, Some("Chats"));
        assert_eq!(open.leaf, "A conversation");

        let home = crumb_parts(
            chats,
            Plane::Chat,
            true,
            true,
            None,
            Some("2 threads".to_owned()),
        );
        assert_eq!(home.leaf, "All conversations");
        assert_eq!(home.after.as_deref(), Some("2 threads"));

        let root = crumb_parts(chats, Plane::Chat, false, true, None, None);
        assert_eq!(
            root.parent, None,
            "a destination's own root named itself either side of a slash"
        );
        assert_eq!(root.leaf, "Chats");

        let itself = crumb_parts(
            chats,
            Plane::Chat,
            false,
            true,
            Some(crate::nav::Crumb {
                title: "Chats".to_owned(),
                subtitle: None,
            }),
            None,
        );
        assert_eq!(
            itself.parent, None,
            "the band read `Chats / Chats`, which is a stutter and not a path"
        );
    }

    /// A HOME SCREEN'S CRUMB NAMES THE HALF, NOT THE ROW THAT GOT YOU THERE.
    ///
    /// The home arm answered `dest.label`, which is the plane badge's own word
    /// one character apart, so the Code band read `CODE  Code / Working trees`
    /// — the same word twice, 10px from itself. The detail arm has guarded
    /// against precisely this since the `Settings / Settings` stutter and the
    /// home arm had no equivalent guard against the badge.
    ///
    /// Held against the badge rather than against a literal, so the day
    /// `Plane::label` changes this fails instead of quietly re-creating the
    /// repeat it exists to remove.
    ///
    /// REPRODUCED: put `Some(dest.label)` back in the home arm and the second
    /// assertion of each pair fails naming the pair.
    #[test]
    fn a_home_crumb_does_not_repeat_the_plane_badge_beside_it() {
        for plane in Plane::ALL {
            let dest = crate::nav::primary(plane);
            let home = crumb_parts(dest, plane, true, true, None, None);
            let parent = home.parent.unwrap_or_default();
            assert!(
                !parent.is_empty(),
                "{plane:?}'s home crumb has no parent at all, so the band shows \
                 a leaf with nothing to place it in"
            );
            let badge = plane.label();
            assert!(
                !parent.eq_ignore_ascii_case(badge)
                    && !parent.eq_ignore_ascii_case(&format!("{badge}s")),
                "{plane:?}'s home band reads `{badge}  {parent} / {}` — the \
                 badge's own word, repeated as the crumb's parent 10px away. \
                 The parent names what the half IS (the mockups' \"goose \
                 server\" and \"code plane\"); the badge already names which \
                 half it is",
                home.leaf
            );
        }
    }

    /// WITH THE SIDEBAR SHUT, THE HALF'S COUNTS COME BACK INTO THE BAND.
    ///
    /// They otherwise live in two places and shutting the column takes both:
    /// `.plane-seg-count` is inside the sidebar, and the band passed `after`
    /// on the home screen only. So a chat open behind a collapsed sidebar —
    /// the arrangement mockups 30 and 32 are drawn for, and two of the four
    /// shell cells `docs/audit.js` walks — said nothing at all about how much
    /// the half held or whether any of it was waiting on you.
    ///
    /// AND THE OPEN THING'S OWN SUBTITLE STILL WINS, which is the half of this
    /// that is a priority rather than a feature: "paused" qualifies the
    /// schedule you opened, and the half's counts do not.
    ///
    /// REPRODUCED, both ways: drop the `nav_open` term and the first assertion
    /// fails (the counts never arrive); take `crumb.subtitle.or(...)` to
    /// `Some(counts)` unconditionally and the third fails, because the
    /// schedule's own "paused" has been replaced by a count.
    #[test]
    fn a_shut_sidebar_puts_the_halfs_counts_back_in_the_band() {
        let chats = crate::nav::primary(Plane::Chat);
        let counts = || Some("12 conversations \u{b7} nothing waiting on you".to_owned());
        let bare = || {
            Some(crate::nav::Crumb {
                title: "A conversation".to_owned(),
                subtitle: None,
            })
        };

        let shut = crumb_parts(chats, Plane::Chat, false, false, bare(), counts());
        assert_eq!(
            shut.after,
            counts(),
            "with the sidebar shut and a chat open, the band says nothing about \
             the half — and `.plane-seg-count`, the only other place that says \
             it, is inside the column that just closed"
        );

        let open = crumb_parts(chats, Plane::Chat, false, true, bare(), counts());
        assert_eq!(
            open.after, None,
            "with the sidebar OPEN the counts are already on screen in the \
             plane switch, so the band saying them again is one fact in two \
             places three inches apart"
        );

        let titled = crumb_parts(
            chats,
            Plane::Chat,
            false,
            false,
            Some(crate::nav::Crumb {
                title: "Nightly digest".to_owned(),
                subtitle: Some("paused".to_owned()),
            }),
            counts(),
        );
        assert_eq!(
            titled.after.as_deref(),
            Some("paused"),
            "the open thing's own subtitle was replaced by the half's counts. \
             `.chrome-sub` says what qualifies what is open; where the open \
             thing qualifies itself that is the more specific answer and it \
             keeps the slot"
        );
    }

    /// The two halves say different things, in their own vocabulary.
    #[test]
    fn each_half_counts_in_its_own_words() {
        let code = crate::testkit::with_ctx(
            |ctx| {
                let mut conn = ctx.code_conn;
                conn.set(ConnState::Connected {
                    agent: "opencode".to_owned(),
                });
                let mut chats = ctx.code_chats;
                chats.set(vec![tree("c1", "one"), tree("c2", "two")]);
            },
            |ctx| band_after(ctx, Plane::Code).unwrap_or_default(),
        );
        assert!(code.contains("2 working trees"), "{code}");
        assert!(code.contains("2 repos"), "{code}");
        assert!(
            !code.contains("conversation"),
            "the code half borrowed the chat half's noun: {code}"
        );
    }

    fn session(id: &str) -> goose_acp_client::SessionInfo {
        goose_acp_client::SessionInfo {
            session_id: id.to_owned(),
            cwd: None,
            title: Some(id.to_owned()),
            updated_at: None,
            meta: None,
        }
    }

    fn tree(id: &str, repo: &str) -> opencode_client::ChatMeta {
        opencode_client::ChatMeta {
            id: id.to_owned(),
            repo: repo.to_owned(),
            title: id.to_owned(),
            branch: String::new(),
            base: String::new(),
            status: "stopped".to_owned(),
            model: None,
            last_active: 0.0,
        }
    }

    /// THE WINDOW HAS MORE THAN ONE SURFACE IN IT.
    ///
    /// This is the defect the surface ramp was written for, and it is worth
    /// stating plainly because it is invisible to every other check in this
    /// file: the sheet parsed, balanced its braces, spent no token that did
    /// not exist and cleared every contrast floor `docs/audit.js` measures —
    /// while painting the window's bar, the sidebar and the content column
    /// the SAME colour. Nothing in a stylesheet is wrong about that. It is
    /// only wrong about the design, and the design's whole first move is four
    /// rungs (`--win / --panel / --canvas / --surface`, `10-home-chat.html`).
    ///
    /// So the check is on the VALUES and on their being different from each
    /// other, not on any rule spending them. Light is exempt by construction
    /// and says so on `--surface-panel`: it separates its columns with fills
    /// that shared.css already provides, and the 1.3% wash that makes a card in
    /// dark makes an invisible one on white.
    ///
    /// REPRODUCED four ways, one per rung: set any one of the four dark
    /// values equal to any other — the state this shipped in, where panel and
    /// canvas were both `--bg-primary` — and this fails naming the pair. Put
    /// it back and it is green. It does NOT fail for a merely small step: two
    /// rungs one hex apart are two rungs, and telling "too close" from
    /// "deliberately subtle" is what the audit's contrast sweep is for.
    #[test]
    fn the_shell_paints_more_than_one_surface() {
        let sheet = crate::css::SHELL;
        let dark = block(sheet, ":root[data-theme=\"dark\"] .app > .shell {", "\n}");

        let rungs = [
            "--surface-panel",
            "--surface-canvas",
            "--surface-card",
            "--surface-raise",
        ];
        let mut painted: Vec<(&str, &str)> = Vec::new();
        for rung in rungs {
            let decl = dark.split(&format!("{rung}:")).nth(1).unwrap_or_default();
            assert!(
                !decl.is_empty(),
                "the dark shell block gives no value for `{rung}`, so it falls back to \
                 the light declaration — which is shared.css's own two backgrounds, and \
                 the window goes back to one fill from edge to edge"
            );
            painted.push((rung, decl.split(';').next().unwrap_or_default().trim()));
        }

        for (i, (rung, value)) in painted.iter().enumerate() {
            for (other, other_value) in &painted[i + 1..] {
                assert_ne!(
                    value, other_value,
                    "`{rung}` and `{other}` are both {value} in dark, so two of the four \
                     planes this shell is built out of are one plane. That is the state \
                     the desktop shipped in — the band, the sidebar and the content \
                     column were all #22252a — and it is why the columns did not read \
                     as columns"
                );
            }
        }
    }

    /// AND THE TWO THAT FRAME CONTENT DO NOT PAINT THE RUNG CONTENT PAINTS.
    ///
    /// The rung values being distinct is half of it; the other half is that
    /// the chrome and the canvas actually spend different ones. Both of these
    /// lines were missing rather than wrong — `.shell-chrome` declared no
    /// background at all and inherited the page, and `.pane` named
    /// `--bg-primary`, which the dark block now remaps to the canvas anyway.
    /// The second is the subtler failure of the two: it would keep working
    /// until someone remapped `--bg-primary` for a nested view and silently
    /// took the content column with it.
    ///
    /// REPRODUCED: delete either `background` declaration, or point `.pane`
    /// back at `--bg-primary`, and this fails naming the element.
    #[test]
    fn the_chrome_and_the_canvas_are_told_apart_by_name() {
        let sheet = crate::css::SHELL;
        for (rule, rung) in [
            (".shell-chrome {", "--surface-panel"),
            (".pane {", "--surface-canvas"),
        ] {
            let body = block(sheet, rule, "\n}");
            assert!(
                body.contains(&format!("background: var({rung})")),
                "`{rule}` does not paint `{rung}`. The rung it should be naming is the \
                 one thing that tells this element apart from the column beside it, and \
                 an inherited or aliased background puts them back on the same plane"
            );
        }
    }

    /// THE SIDEBAR DOES NOT SPEND ITS WIDTH ON THE NAME OF THE APP.
    ///
    /// `h2.drawer-brand` was the first child of `.navcard`: 22px on a 32px
    /// line with 24 below it, which is 56px of a 252px-wide panel and the
    /// largest type in the column. It belongs to the phone, where the drawer
    /// slides over the page and has to say whose panel it is. A window's name
    /// is the window's, and none of the eight E-option mockups draws one —
    /// every `.side` in them opens with the plane switch.
    ///
    /// Asserted on `nav_card()` rather than on the file, because the question
    /// is which ELEMENT is first in that column and a file-wide `contains`
    /// cannot see order. `assets/shared.css` keeps the rule; the phone still
    /// renders it.
    ///
    /// REPRODUCED: put the `h2` back and this fails; take it out and it is
    /// green. The `starts_with` half fails on its own if the switch is merely
    /// displaced — inserting any element above it is the failure this guards,
    /// and a wordmark is only the one it shipped with.
    #[test]
    fn the_sidebar_opens_with_the_choice_and_not_with_the_wordmark() {
        let card = nav_card();
        assert!(
            !card.contains("drawer-brand"),
            "the sidebar renders `.drawer-brand` again — 56px of a 252px column \
             spent saying the name of the app the reader has already launched"
        );
        let first = card
            .split("class: \"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .unwrap_or_default();
        assert_eq!(
            first, "plane-switch",
            "the first thing in the sidebar is `.{first}`, not the plane switch. \
             This column is organised around one choice and that choice is what \
             should be at the top of it"
        );
    }

    /// The chrome reservation is macOS's alone, because `src/main.rs` hides
    /// the titlebar there and nowhere else. Defaulted to zero in the shared
    /// desktop sheet and raised only by the platform sheet — otherwise a
    /// native-frame build gets its own titlebar AND a 76pt slot held empty
    /// for traffic lights it does not have.
    ///
    /// REPRODUCED, and it is the reason `shell_code()` exists. Delete
    /// `use_fullscreen`, its call site and the attribute — the whole feature,
    /// leaving nothing but the doc comment — and the version of this test that
    /// read the shell's own source passed, along with the other 230:
    /// both of its needles were on the two assertion lines below. Reading the
    /// `.shell` div's own attributes instead, that same deletion fails here
    /// with "the `.shell` div must SET the attribute the sheet reads"
    /// (measured: 234 passed, 1 failed); restoring the feature is green again.
    #[test]
    fn the_window_chrome_is_reserved_only_where_the_titlebar_is_hidden() {
        let desktop = crate::css::SHELL;
        let macos = include_str!("../../../assets/platform/macos.css");
        assert!(
            desktop.contains("--traffic-w: 0px"),
            "assets/desktop/ must default the traffic-light slot to zero — \
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
        //
        // ON THE `.shell` DIV, and not merely somewhere in the file, because
        // the assertion above names an element too. This attribute was on
        // `.app` until ece4857 and the sheet's selector was
        // `.app[data-fullscreen="true"] > .shell` to match it; the two halves
        // moved together, and the only thing that says they still agree is
        // that both of these read the same element. Reproduced rather than
        // reasoned: put the attribute back on `.shell-chrome`, leaving the
        // string in the file and the sheet untouched, and this fails.
        assert!(
            shell_attributes().contains(r#""data-fullscreen": if fullscreen()"#),
            "the `.shell` div must SET the attribute the sheet reads, in the \
             render — an attribute nothing writes is a stylesheet rule nothing \
             reaches"
        );
        let shell = shell_code();
        assert!(
            shell.contains("fn use_fullscreen()") && shell.contains(".fullscreen().is_some()"),
            "the flag must be read off the window; inferring it from geometry \
             is what shipped a feature that never engaged"
        );
    }

    /// THE BAND'S HEIGHT AND THE ROW'S OFFSET ARE ONE NUMBER TWICE, and until
    /// this test nothing in the repo held them together.
    ///
    /// `--chrome-pad` in the fullscreen block is not a taste, it is
    /// `(--chrome-h - 32) / 2` — the room left over after the 32pt control the
    /// band's whole content is. Change one and the other is silently wrong,
    /// and it is silent in the worst way: the arithmetic still parses, the
    /// sheet still applies, and the fault only appears in a `fullscreen` frame,
    /// which is the one shell state a human never renders by accident.
    ///
    /// MEASURED, on this tree. Taking `--chrome-h` from 52 to 34 and leaving
    /// the fullscreen pad at its old `(52 - 32) / 2 = 10px` makes
    /// `node docs/audit.js both` report **572 findings — 468 FULLSCREEN and 104
    /// SQUARE** — a 32pt control starting at y=10 in a 33px content box, i.e.
    /// nine points past the bottom of the band that contains it. At 1px the
    /// same run is Clean. So the audit CAN see it; what the audit cannot do is
    /// tell anyone which of the two numbers was the one that moved, and it only
    /// runs where a browser does.
    ///
    /// The 52 -> 34 move also invalidated four comments that quote the band's
    /// height as prose and two that quote a y-offset derived from it, in three
    /// files and in Rust. Nothing in the tree can check prose; this at least
    /// makes the number itself impossible to move quietly, which is the hook a
    /// reader needs to go and look for the sentences.
    ///
    /// Shown to fail: set the fullscreen pad back to 10px and this reports the
    /// value it found beside the value the height implies.
    #[test]
    fn the_fullscreen_pad_is_derived_from_the_band_height() {
        // COMMENTS FIRST, and that is not tidiness. Both blocks below argue
        // their number in prose that names the property, so a plain `find`
        // reads the argument rather than the declaration — measured while
        // writing this: it returned four lines of the comment above
        // `--chrome-h`. Only a real declaration has the colon attached.
        let macos = include_str!("../../../assets/platform/macos.css");
        let mut bare = String::with_capacity(macos.len());
        let mut rest = macos;
        while let Some(open) = rest.find("/*") {
            bare.push_str(&rest[..open]);
            let Some(close) = rest[open..].find("*/") else {
                break;
            };
            rest = &rest[open + close + 2..];
        }
        bare.push_str(rest);
        let value = |after: &str, prop: &str| -> Option<String> {
            let at = bare.find(after)?;
            let tail = &bare[at..];
            let start = tail.find(prop)? + prop.len();
            let end = tail[start..].find(';')? + start;
            Some(tail[start..end].trim().to_owned())
        };
        let height = value(".app > .shell {", "--chrome-h:");
        assert_eq!(
            height.as_deref(),
            Some("34px"),
            "assets/platform/macos.css gives the band's height as {height:?}. \
             It is 34 by derivation, not by taste: the row is a 32pt control \
             pinned to the traffic lights' own y, so every point past 34 falls \
             BELOW the row rather than around it. If it is meant to move, move \
             the fullscreen pad below with it and re-read the four comments \
             that quote the number as prose."
        );
        let pad = value(r#"[data-fullscreen="true"] {"#, "--chrome-pad:");
        assert_eq!(
            pad.as_deref(),
            Some("1px"),
            "the fullscreen block gives `--chrome-pad` as {pad:?}, and the band \
             above it is {height:?}. The pad is (--chrome-h - 32) / 2 — the room \
             left after the 32pt control the band holds — and a pad that no \
             longer matches the height puts that control outside its own band in \
             every fullscreen frame. Measured on this tree: 34px against a 10px \
             pad is 572 findings from `node docs/audit.js both`, 468 of them \
             FULLSCREEN."
        );
    }

    /// The collapse is two decisions in two languages again: Rust sets
    /// `data-nav` and the sheet is the only thing that reads it. Nothing in
    /// the compiler connects them, so a rename on either side leaves a button
    /// that toggles an attribute nobody styles — a control that visibly does
    /// nothing, with no error anywhere.
    ///
    /// EITHER SIDE, which this test's name has always claimed and only half of
    /// it did. It read the stylesheet and stopped there: delete
    /// `"data-nav": if nav_open()` from the render — the toggle still on
    /// screen, still flipping a signal, `[data-nav="closed"]` now matching
    /// nothing ever — and `cargo test --package goose-mobile` reported 231
    /// passed, 0 failed. Nothing else in the repo covered it either.
    /// `docs/audit.js` walks a closed nav, but it sets `data-nav` on `.shell`
    /// itself from `DESKTOP_SHELL`, so the audit measures a collapsed column
    /// whether or not the app can ever produce one. With the Rust half below,
    /// that deletion fails here with "the `.shell` div no longer sets
    /// data-nav" (measured: 234 passed, 1 failed); restored, it is green.
    #[test]
    fn the_stylesheet_acts_on_the_attribute_the_shell_sets() {
        let sheet = crate::css::SHELL;
        for rule in [r#"[data-nav="closed"]"#, ".nav-toggle", ".navcard"] {
            assert!(
                sheet.contains(rule),
                "assets/desktop/ never mentions `{rule}`, so the nav's \
                 collapse control changes an attribute nothing styles"
            );
        }
        assert!(
            shell_attributes().contains(r#""data-nav": if nav_open()"#),
            "the `.shell` div no longer sets data-nav, so the rules above \
             match nothing: the toggle flips a signal, the sheet is never \
             reached, and the nav cannot be collapsed at all"
        );
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
        let dispatch = include_str!("../../viewport.rs");
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

    // ---- the third column with nothing in it ----------------------------

    thread_local! {
        /// Which destination [`EmptyColumn`] is standing in for.
        ///
        /// A thread-local rather than a prop because `empty_detail` takes a
        /// `&'static Destination` and a Dioxus component's props must be
        /// `PartialEq`; the table's rows have `fn` fields, which cannot be
        /// compared.
        static NOTHING_OPEN: RefCell<Option<&'static Destination>> = const {
            RefCell::new(None)
        };
    }

    /// The detail column, on its own, with nothing selected in it.
    #[component]
    fn EmptyColumn() -> Element {
        let dest = NOTHING_OPEN
            .with(|slot| *slot.borrow())
            .expect("the test names the destination before it mounts this");
        super::empty_detail(dest)
    }

    fn empty_column(dest: &'static Destination) -> String {
        NOTHING_OPEN.with(|slot| *slot.borrow_mut() = Some(dest));
        let mut dom = VirtualDom::new(EmptyColumn);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// The column a three-column window opens on says what to do about it, in
    /// the words of the list beside it.
    ///
    /// This is the very first thing the desktop shell shows: launch lands on a
    /// destination with nothing selected, so an empty third column is the
    /// state, not an edge of it. Two ways it fails silently, both covered
    /// here. The sentence is built from `dest.label`, so a destination whose
    /// wording drifted would point at a list that is not the one on screen —
    /// "Pick something from Chats" beside the recipes. And the glyph is
    /// `dest.icon`, which `Icon` renders as *nothing at all* when it does not
    /// know the name (`src/icons.rs`), so a typo is a blank column rather than
    /// a compile error.
    #[test]
    fn the_empty_column_names_the_list_it_wants_you_to_pick_from() {
        let mut checked = 0;
        for dest in DESTINATIONS {
            // Settings has no list, so its detail is unconditional and this
            // column is never the one it draws.
            if dest.root.is_none() {
                continue;
            }
            let html = empty_column(dest);
            assert!(
                html.contains("Nothing open"),
                "the empty detail column for {} says nothing at all: {html}",
                dest.id
            );
            assert!(
                html.contains(&format!(
                    "Pick something from {} to see it here.",
                    dest.label
                )),
                "the empty column beside the {} list does not name that list, \
                 so it tells the reader to pick from somewhere they are not: \
                 {html}",
                dest.label
            );
            let glyph = crate::icons::path_for(dest.icon).unwrap_or_default();
            assert!(
                !glyph.is_empty() && html.contains(glyph),
                "nothing drew {}'s `{}` glyph, so the empty column is a \
                 sentence over a blank square: {html}",
                dest.id,
                dest.icon
            );
            checked += 1;
        }
        assert!(
            checked >= 6,
            "only {checked} destinations have a list to be empty beside — this \
             walked the table and found almost nothing in it"
        );
    }

    // ---- the keyboard chords, as strings and as wiring -------------------
    //
    // The three constants are JavaScript, so no compiler in this build reads a
    // character of them and nothing else in the repo does either: they are
    // installed by `document::eval` at run time, in a web view no test has.
    // What CAN be held is the shape of each one, and each rule below is a line
    // the doc comment above the constant calls load-bearing.

    /// A chord installs its listener once per window, whatever Dioxus does
    /// with the effect that installs it.
    ///
    /// `use_effect` re-runs whenever a signal read inside it changes and every
    /// run re-evaluates the script, so a listener with no guard is added again
    /// on each pass. The consequence is not a slow key: `REFRESH_KEY` sends
    /// one message per listener, so the fifth ⌘R of a session would fire five
    /// fetches of every list on screen, and Escape would click a dialog's
    /// Cancel and then whatever took its place.
    #[test]
    fn each_chord_wires_its_listener_once_per_window() {
        for (chord, script, flag) in [
            ("⌘R", super::REFRESH_KEY, "__refreshKeyWired"),
            ("Escape", super::DISMISS_KEY, "__dismissKeyWired"),
            ("⌘/", super::NAV_KEY, "__navKeyWired"),
        ] {
            assert!(
                script.contains(&format!("if (window.{flag}) return;")),
                "the {chord} listener has no re-entry guard, so every re-render \
                 of the shell adds another copy of it"
            );
            assert!(
                script.contains(&format!("window.{flag} = true;")),
                "the {chord} listener never claims its guard, so the guard \
                 above is a flag nothing ever sets"
            );
        }
    }

    /// ⌘R swallows the whole modifier+`r` family and only then decides whether
    /// it is the chord it wants.
    ///
    /// The order is the rule. ⌘R and ⌘⇧R are both the web view's own reload,
    /// which throws the entire app away — connection, drafts and all — to
    /// re-fetch a list. Narrowing first and preventing second means a stray
    /// Shift reloads the app; that is a lost conversation, not a missed
    /// refresh.
    #[test]
    fn the_refresh_chord_swallows_the_reload_before_it_narrows() {
        let js = super::REFRESH_KEY;
        let prevented = js
            .find("e.preventDefault();")
            .expect("⌘R no longer calls preventDefault, so the web view reloads the whole app");
        let narrowed = js.find("if (e.altKey || e.shiftKey) return;").expect(
            "the ⌘R listener no longer guards the other modifiers, so ⌥⌘R and \
             ⇧⌘R now refresh as well",
        );
        assert!(
            prevented < narrowed,
            "⌘R narrows to the plain chord before it takes preventDefault, so \
             ⇧⌘R reaches the web view's own reload and throws the app away"
        );
        assert!(
            js.contains("e.key.toLowerCase() !== 'r'"),
            "the key compare is case-sensitive again, so Caps Lock or a held \
             Shift makes ⌘R do nothing at all"
        );
        assert!(
            js.contains("const names = new Set();"),
            "the names are no longer deduplicated: the Scheduler's list and \
             its job detail both answer to `scheduler`, so one press would \
             fetch it twice"
        );
    }

    /// Escape presses the control that already means cancel — and only inside
    /// a Confirm.
    ///
    /// `p.modal-body` is the discriminator and it has to be consulted BEFORE a
    /// button is chosen. `views/chat.rs`'s `permission_button_class` paints
    /// "Always allow" as `.btn.secondary`, so an Escape that reached for the
    /// secondary button in any dialog would answer a permission request with
    /// the broadest grant this app can give — from a key the reader pressed to
    /// get out of the way.
    #[test]
    fn escape_reaches_for_a_cancel_only_inside_a_confirm() {
        let js = super::DISMISS_KEY;
        let discriminator = js
            .find(".modal-body")
            .expect("Escape no longer tells a Confirm from a permission prompt");
        let cancel = js
            .find(".modal-actions > .btn.secondary")
            .expect("Escape no longer clicks the dialog's own Cancel");
        assert!(
            discriminator < cancel,
            "Escape picks the secondary button before it checks that the \
             dialog is a Confirm — in a permission prompt that button is \
             `Always allow`"
        );
        assert!(
            js.contains("if (cancel) cancel.click(); else back.click();"),
            "the fallback is gone: a sheet that is not a Confirm has only its \
             backdrop to dismiss it, and without this Escape does nothing to \
             the rename sheet, the overflow menu or the pickers"
        );
        assert!(
            !js.contains("dioxus.send"),
            "Escape now reports back to Rust, which would mean a registry of \
             open dialogs for the shell to close — state every sheet in this \
             app deliberately keeps in its own view"
        );
    }

    /// ⌘/ is matched on the CHARACTER, so it is the same chord on every
    /// keyboard.
    ///
    /// On a US layout the slash is `Slash`, on AZERTY it is Shift+colon, on a
    /// German layout Shift+7. Reading `e.code` would leave most of Europe with
    /// a nav they cannot bring back, and a shift guard would do the same to
    /// all of them at once.
    #[test]
    fn the_nav_chord_matches_the_character_and_not_the_key_position() {
        let js = super::NAV_KEY;
        assert!(
            js.contains("if (e.key !== '/') return;"),
            "⌘/ no longer matches on the character it is named for"
        );
        assert!(
            !js.contains("e.code"),
            "⌘/ reads the physical key, so on any layout that reaches `/` with \
             a modifier the nav has no chord at all"
        );
        assert!(
            !js.contains("shiftKey"),
            "⌘/ now refuses a held Shift, which is how most of Europe types a \
             slash in the first place"
        );
    }

    // ---- the chords' Rust halves, driven through a fake web view ---------
    //
    // The strings above are half of each chord; the other half is what Rust
    // does when the page sends one back, and until this existed nothing
    // reached it. `document::eval` resolves an `Rc<dyn Document>` out of the
    // context and falls back to a no-op that answers every `recv` with
    // `Unsupported` (`dioxus-document-0.7.10/src/document.rs:120`), so under a
    // bare `VirtualDom` these loops exit before their first turn.
    //
    // Providing a `Document` is NOT the faked `DesktopContext` that
    // `src/selfscan.rs` rejects. That one would render a different component
    // than the one that ships; this is the seam Dioxus itself defines for a
    // renderer, so the code under test is exactly the code that runs on the
    // desktop — only the web view on the far end of the channel is ours.

    thread_local! {
        /// Every script the shell asked the page to run.
        static SCRIPTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        /// What the page has to say back, in order. Loaded before the mount,
        /// so the first poll of the hook's `recv` already has it.
        static INBOX: RefCell<VecDeque<serde_json::Value>> = const {
            RefCell::new(VecDeque::new())
        };
    }

    /// One `document::eval` handle: it delivers [`INBOX`] and then ends.
    ///
    /// Ending matters. `Finished` is what breaks the hook's `while ... .await`
    /// loop, so the spawned task retires instead of parking forever and the
    /// settle loop below has something to settle to.
    ///
    /// `send` and `poll_join` are the trait's, not this shell's: every chord
    /// here is one-way, page to Rust, so nothing ever calls them and they show
    /// up as the only uncovered lines in this module that are not behind a
    /// window. Written as the honest answers rather than as a `todo!()`, which
    /// would make a future two-way chord fail as a panic in the harness
    /// instead of as a message the test can read.
    struct Wire;

    impl Evaluator for Wire {
        fn send(&self, _data: serde_json::Value) -> Result<(), EvalError> {
            Ok(())
        }

        fn poll_recv(&mut self, _: &mut Context<'_>) -> Poll<Result<serde_json::Value, EvalError>> {
            Poll::Ready(
                INBOX
                    .with(|queue| queue.borrow_mut().pop_front())
                    .ok_or(EvalError::Finished),
            )
        }

        fn poll_join(&mut self, _: &mut Context<'_>) -> Poll<Result<serde_json::Value, EvalError>> {
            Poll::Ready(Err(EvalError::Finished))
        }
    }

    /// The web view, minus the web view.
    struct FakeWebView {
        /// The `Eval`s handed out are `GenerationalBox`es owned by this, so it
        /// has to outlive them — a dropped owner turns every `recv` into
        /// `Finished` before the first message is read.
        owner: Owner,
    }

    impl Document for FakeWebView {
        fn eval(&self, js: String) -> Eval {
            SCRIPTS.with(|log| log.borrow_mut().push(js));
            let wire: Box<dyn Evaluator> = Box::new(Wire);
            Eval::new(self.owner.insert(wire))
        }
    }

    /// Put a page under the component that is about to talk to one.
    fn fake_web_view() {
        let page: Rc<dyn Document> = Rc::new(FakeWebView {
            owner: Owner::default(),
        });
        provide_context(page);
    }

    /// Load what the page will send, and forget what the last test heard.
    fn page_sends(messages: &[&str]) {
        SCRIPTS.with(|log| log.borrow_mut().clear());
        INBOX.with(|queue| {
            *queue.borrow_mut() = messages
                .iter()
                .map(|m| serde_json::Value::String((*m).to_owned()))
                .collect();
        });
    }

    fn installed(needle: &str) -> bool {
        SCRIPTS.with(|log| log.borrow().iter().any(|js| js.contains(needle)))
    }

    /// One mounted app at a time.
    ///
    /// The tests below are the only ones in this module that mount an
    /// `AppCtx` and run the virtual DOM's task queue, and the ask journal's
    /// `use_synced_storage` keys its sender by storage key in a process-wide
    /// `static` — so two mounts rendering at once feed each other through it
    /// and never settle. `src/views/chat.rs` carries the measurement.
    ///
    /// A poisoned lock is taken anyway: a test that panicked while holding it
    /// has already reported the thing it exists to report, and taking the rest
    /// of the module down behind it would only hide which one broke.
    fn alone() -> std::sync::MutexGuard<'static, ()> {
        static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
        ONE_AT_A_TIME
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The nav's own signal, wired to ⌘/ exactly as `AppShell` wires it.
    #[component]
    fn NavKeyProbe() -> Element {
        fake_web_view();
        let open = use_signal(|| true);
        super::use_nav_key(open);
        rsx! { p { class: "probe-nav", "{open}" } }
    }

    fn press_nav_chord(times: usize) -> String {
        page_sends(&vec!["toggle"; times]);
        let mut dom = VirtualDom::new(NavKeyProbe);
        dom.rebuild_in_place();
        settle(&mut dom);
        dioxus_ssr::render(&dom)
    }

    /// ⌘/ hides the nav, and pressing it again brings it back.
    ///
    /// The nav has no other keyboard route and — once it is shut — its only
    /// pointer route is one unlabelled glyph in the corner of the window. A
    /// chord that fired once and stuck, or that toggled a signal nobody reads,
    /// looks identical from outside: the sidebar simply never moves.
    ///
    /// Both halves of the wiring are under this. The script has to reach the
    /// page (a hook that stopped calling `document::eval` would install
    /// nothing at all) and the message has to come back to a `set` on the
    /// nav's own signal.
    #[test]
    fn the_nav_chord_shuts_the_sidebar_and_opens_it_again() {
        let opened = press_nav_chord(0);
        assert!(
            opened.contains(">true<"),
            "the nav does not start open, so the window launches with no \
             navigation in it: {opened}"
        );
        assert!(
            installed("__navKeyWired"),
            "the shell never asked the page to listen for ⌘/, so the chord is \
             a constant nothing installs"
        );

        let once = press_nav_chord(1);
        assert!(
            once.contains(">false<"),
            "⌘/ arrived and the nav stayed open — the chord is wired to \
             nothing: {once}"
        );

        let twice = press_nav_chord(2);
        assert!(
            twice.contains(">true<"),
            "⌘/ shuts the nav and then cannot open it, which leaves one \
             unlabelled glyph as the only way back to the navigation: {twice}"
        );
    }

    /// ⌘R's dispatch, with the toast the shell would show for it.
    #[component]
    fn RefreshKeyProbe() -> Element {
        fake_web_view();
        let ctx = crate::state::use_app_ctx();
        super::use_refresh_key();
        let toast = (ctx.toast)().unwrap_or_default();
        rsx! { p { class: "probe-toast", "{toast}" } }
    }

    /// ⌘R sends the name the page names, and Rust acts on that name.
    ///
    /// The name is the whole contract between the two halves: JS collects
    /// every `data-refresh` on screen and sends the string, and
    /// `viewport::refresh_named` is a `match` on it whose fallthrough arm is
    /// legitimately silent. So a dispatch that dropped the string, or handed
    /// on a different one, refreshes nothing and says nothing — the reader
    /// presses ⌘R and the list simply stays as old as the window.
    ///
    /// `diff` is the name asserted on because its arm answers without a
    /// server: `code::load_code_diff` on a chat with no session says so in a
    /// toast, which is a sentence this test can read back out of the markup.
    #[test]
    fn the_refresh_chord_acts_on_the_name_the_page_sent() {
        let _alone = alone();

        page_sends(&["diff"]);
        let html = crate::testkit::render_settled(|_| {}, || rsx! { RefreshKeyProbe {} });
        assert!(
            installed("__refreshKeyWired"),
            "the shell never asked the page to listen for ⌘R"
        );
        assert!(
            html.contains("No changes yet"),
            "the page sent `diff` and nothing answered to it, so ⌘R reaches \
             `refresh_named` with the wrong name or with none: {html}"
        );

        // And a name with no arm is silent rather than wrong: the scrollers
        // that set no `data-refresh` are the majority.
        page_sends(&["nothing-answers-to-this"]);
        let quiet = crate::testkit::render_settled(|_| {}, || rsx! { RefreshKeyProbe {} });
        assert!(
            !quiet.contains("No changes yet"),
            "an unrecognised refresh name reached the `diff` arm: {quiet}"
        );
    }

    // ---- arriving somewhere, with a context under it ---------------------

    /// The Scheduler's row, which is the destination this pair navigates to.
    ///
    /// Named by id off the real table rather than built here: a fabricated
    /// destination would let this go green over a row `refresh_named` has no
    /// arm for.
    fn scheduler_row() -> &'static Destination {
        DESTINATIONS
            .iter()
            .find(|dest| dest.id == "scheduler")
            .expect("the destination table still has a Scheduler in it")
    }

    /// `AppShell`'s arrival effect, over the one destination whose refresh
    /// claims something synchronously.
    #[component]
    fn ArrivalRefreshProbe() -> Element {
        let ctx = crate::state::use_app_ctx();
        super::use_arrival_refresh(scheduler_row());
        let claimed = (ctx.scheduler.history_of)().unwrap_or_default();
        rsx! { p { class: "probe-history", "{claimed}" } }
    }

    /// A job is open, so `scheduler::pull_refresh` has a history to claim —
    /// which is the one thing a refresh does without a server to answer it.
    fn a_job_open_and_connected(ctx: &AppCtx) {
        let mut conn = ctx.conn;
        conn.set(ConnState::Connected {
            agent: "goose".to_owned(),
        });
        let mut open = ctx.scheduler.open;
        open.set(Some("nightly".to_owned()));
    }

    /// The same window, still offline — which is where every launch starts.
    fn a_job_open_and_offline(ctx: &AppCtx) {
        let mut open = ctx.scheduler.open;
        open.set(Some("nightly".to_owned()));
    }

    /// Arriving at a destination re-fetches its list, which is the desktop's
    /// whole answer to refresh: there is no pull gesture here and no button in
    /// the bar.
    ///
    /// The assertion is on the Scheduler's history slot because
    /// `scheduler::pull_refresh` claims it in the same beat as the call, so it
    /// is the one effect of a refresh that is visible without a server. Cut
    /// the `refresh_named` out of the arrival hook and the slot stays empty:
    /// the lists would then only ever load the first time a screen mounted,
    /// and a window left open overnight would show yesterday's schedule with
    /// nothing to say so.
    #[test]
    fn arriving_at_a_destination_refreshes_it() {
        let _alone = alone();
        let html = crate::testkit::render_settled(
            a_job_open_and_connected,
            || rsx! { ArrivalRefreshProbe {} },
        );
        assert!(
            html.contains("nightly"),
            "arriving at the Scheduler refreshed nothing, so the desktop's \
             only automatic re-fetch is dead: {html}"
        );
    }

    /// And it does NOT fetch while the app is disconnected.
    ///
    /// Every launch starts here — `state.rs` opens on Settings with no
    /// connection — and the effect re-runs when the connection arrives under a
    /// screen that is already up, which is the arrival that matters. Firing
    /// regardless would throw a request at every destination change while the
    /// tailnet is down, and each one would land in the failure path of a
    /// screen that already says it is offline.
    #[test]
    fn arriving_while_disconnected_asks_for_nothing() {
        let _alone = alone();
        let html = crate::testkit::render_settled(
            a_job_open_and_offline,
            || rsx! { ArrivalRefreshProbe {} },
        );
        assert!(
            !html.contains("nightly"),
            "a disconnected window still fetched on arrival, so the gate on \
             the connection is gone: {html}"
        );
    }

    /// Escape's hook, which installs a script and listens for nothing back.
    #[component]
    fn DismissKeyProbe() -> Element {
        fake_web_view();
        super::use_dismiss_key();
        rsx! { p { class: "probe-dismiss", "wired" } }
    }

    /// Escape is wired, and wired to the page alone.
    ///
    /// The hook is three lines and all three are load-bearing: no
    /// `document::eval` at all and Escape does nothing anywhere in the app —
    /// which is exactly the state this feature was added to fix, with "Delete
    /// this chat?" up and no keyboard way out of it. `use_dismiss_key` is also
    /// the one chord that never reports back, so this checks that the script
    /// it installs is the one that answers in the page.
    #[test]
    fn the_shell_wires_escape_to_the_page_and_asks_for_nothing_back() {
        page_sends(&[]);
        let mut dom = VirtualDom::new(DismissKeyProbe);
        dom.rebuild_in_place();
        settle(&mut dom);

        assert!(
            installed("__dismissKeyWired"),
            "the shell never asked the page to listen for Escape, so a dialog \
             can only be dismissed with the pointer"
        );
        assert!(
            installed(".modal-backdrop"),
            "the script the shell installed for Escape is not the one that \
             looks for an open dialog"
        );
    }
}
