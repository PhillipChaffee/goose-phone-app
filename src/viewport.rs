//! Keeping the app aligned with the *visual* viewport.
//!
//! When a field is focused, iOS does not shrink the layout viewport — it
//! offsets the visual one. Measured on iOS 26 with the keyboard accessory bar
//! open: `documentElement.clientHeight` stays 874, `visualViewport.height`
//! becomes 806, and `visualViewport.offsetTop` becomes 68. Everything
//! positioned against the layout viewport — which is all of this app's
//! floating chrome — therefore rides 68px up and the header leaves the screen.
//!
//! `interactive-widget=resizes-content` is the standards answer and is set in
//! the viewport meta, but iOS 26 `WKWebView` ignored it in testing; pinning
//! html/body did not help either, because nothing is scrolling in the first
//! place. So the offset is mirrored into two custom properties and the shell
//! follows it.
//!
//! Deliberately not a `transform`: that would make `.app` a backdrop root and
//! kill `backdrop-filter` on every piece of chrome inside it.

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;

const SYNC: &str = r"
(() => {
  const vv = window.visualViewport;
  if (!vv) return;
  const root = document.documentElement;
  let frame = 0;
  const sync = () => {
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      root.style.setProperty('--vv-top', vv.offsetTop + 'px');
      root.style.setProperty('--vv-height', vv.height + 'px');
      // The bottom inset exists to clear the home indicator. Once the
      // keyboard is over it, reserving it again just parks the composer 42px
      // above the keyboard on a strip of nothing. A shorter visual viewport
      // than layout viewport is the keyboard; '' drops the override and lets
      // env(safe-area-inset-bottom) come back.
      const covered = vv.height < root.clientHeight - 1;
      root.style.setProperty('--safe-bottom', covered ? '0px' : '');
    });
  };
  vv.addEventListener('resize', sync);
  vv.addEventListener('scroll', sync);
  sync();
})();
";

/// Mirror the visual viewport into `--vv-top` and `--vv-height`.
pub(crate) fn use_visual_viewport() {
    use_effect(move || {
        document::eval(SYNC);
    });
}

/// A tap on an open row closes its tray instead of opening the session.
///
/// That is what Mail and Messages do, and the alternative is worse than it
/// sounds: with a tray revealed, most of what your thumb can reach is still
/// the card, so the gesture that ought to dismiss the tray was navigating
/// away instead.
///
/// Done as one capture-phase listener rather than by tracking scroll offset
/// in Rust. The row is a scroll-snap scroller, so "open" is just a non-zero
/// `scrollLeft` — no state to keep, and nothing to keep in sync. It also
/// costs nothing per frame: the native renderer sends every listened-to event
/// through a synchronous XHR, so an `onscroll` handler would put a blocking
/// round-trip on each frame of a drag.
const CLOSE_OPEN_ROW: &str = r"
document.addEventListener('click', (e) => {
  const row = e.target.closest && e.target.closest('.session-item');
  if (!row || row.scrollLeft <= 4) return;
  // Not a tap on the tray itself — Delete has to keep working.
  if (e.target.closest('.session-actions')) return;
  e.stopPropagation();
  e.preventDefault();
  row.scrollTo({ left: 0, behavior: 'smooth' });
}, true);
";

/// Install the tap-to-close behaviour for swiped-open rows.
pub(crate) fn use_close_open_row() {
    use_effect(move || {
        document::eval(CLOSE_OPEN_ROW);
    });
}

/// Pull a list down to refresh it.
///
/// The gesture lives here rather than on the elements for the reason given
/// above `CLOSE_OPEN_ROW`: a Rust `ontouchmove` costs a blocking round trip
/// per frame of every drag. JS owns the whole gesture and Rust is told once,
/// when it has actually been completed.
///
/// Two things this has to avoid fighting:
///
/// The rows swipe. A `.session-item` is a horizontal scroll-snap scroller
/// inside the vertical one, so a drag that starts on a row is ambiguous until
/// it has moved far enough to have a direction. The gesture is abandoned
/// unless the movement is decisively vertical.
///
/// And the spinner does not move the content. Translating the list would
/// double up with iOS's own elastic overscroll and leave the rows somewhere
/// the swipe handler does not expect them; instead the indicator animates down
/// from behind the chrome on its own, which also works for a list too short to
/// rubber-band at all.
const PULL_TO_REFRESH: &str = r#"
(() => {
  if (window.__ptrWired) return;
  window.__ptrWired = true;

  const THRESHOLD = 64;   // how far to pull before it arms
  const MAX = 96;         // how far the indicator can travel
  const SLOP = 8;         // movement before a direction is credited

  let el = null, startY = 0, startX = 0, pull = 0, armed = false, decided = false;

  const spinner = document.createElement('div');
  spinner.className = 'ptr';
  spinner.innerHTML =
    '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"'
    + ' stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">'
    + '<path d="M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6"></path></svg>';
  document.body.appendChild(spinner);

  const place = (y, on) => {
    spinner.style.setProperty('--ptr-y', y + 'px');
    spinner.classList.toggle('on', on);
  };

  const reset = () => {
    el = null; pull = 0; armed = false; decided = false;
    spinner.classList.remove('armed');
    place(0, false);
  };

  document.addEventListener('touchstart', (e) => {
    if (spinner.classList.contains('run') || e.touches.length !== 1) return;
    const scroller = e.target.closest && e.target.closest('.scroll');
    // Only a list refreshes. A transcript is pinned to its bottom and a
    // settings form has nothing to fetch.
    if (!scroller || !scroller.dataset.refresh) return;
    if (scroller.scrollTop > 0) return;
    el = scroller;
    startY = e.touches[0].clientY;
    startX = e.touches[0].clientX;
  }, { passive: true });

  document.addEventListener('touchmove', (e) => {
    if (!el || e.touches.length !== 1) return;
    const dy = e.touches[0].clientY - startY;
    const dx = e.touches[0].clientX - startX;
    if (!decided) {
      if (Math.abs(dy) < SLOP && Math.abs(dx) < SLOP) return;
      // Sideways, or upwards: that is a swipe or a scroll, not a pull.
      if (dy <= 0 || Math.abs(dx) > Math.abs(dy)) { el = null; return; }
      decided = true;
    }
    // Resisted, so it slows as it goes — the pull should feel like it is
    // costing something rather than tracking the finger one to one.
    pull = Math.min(MAX, dy * 0.5);
    armed = pull >= THRESHOLD;
    spinner.classList.toggle('armed', armed);
    place(pull, true);
  }, { passive: true });

  const release = () => {
    if (!el) return reset();
    if (!armed) return reset();
    const scroller = el;
    reset();
    spinner.classList.add('run');
    place(THRESHOLD, true);
    dioxus.send(scroller.dataset.refresh);
    // The app clears data-refreshing when the fetch settles. The timeout is
    // the backstop for a fetch that never does, so the spinner cannot be left
    // turning forever over a list that has given up.
    const started = Date.now();
    const watch = setInterval(() => {
      const busy = scroller.isConnected && scroller.dataset.refreshing === 'true';
      if (busy && Date.now() - started < 20000) return;
      clearInterval(watch);
      spinner.classList.remove('run');
      place(0, false);
    }, 120);
  };

  document.addEventListener('touchend', release, { passive: true });
  document.addEventListener('touchcancel', release, { passive: true });
})();
"#;

/// Wire the composer's attach button to iOS's file sheet.
///
/// Installed once at the app root rather than per composer, for the reason
/// given above `CLOSE_OPEN_ROW`: the gesture has to be owned by JavaScript.
/// The script itself, and why the reading and resizing happen there too, is
/// in `crate::attach`.
pub(crate) fn use_file_picker() {
    let ctx = crate::state::use_app_ctx();
    use_effect(move || {
        let mut eval = document::eval(&crate::attach::picker_js());
        spawn(async move {
            while let Ok(payload) = eval.recv::<String>().await {
                crate::attach::receive(&ctx, &payload);
            }
        });
    });
}

/// Wire pull-to-refresh, dispatching on whichever list was pulled.
///
/// The scroller names its own refresh in `data-refresh`, so a list that has
/// nothing to fetch simply does not set it and the gesture never starts.
pub(crate) fn use_pull_to_refresh() {
    let ctx = crate::state::use_app_ctx();
    use_effect(move || {
        let mut eval = document::eval(PULL_TO_REFRESH);
        spawn(async move {
            while let Ok(which) = eval.recv::<String>().await {
                match which.as_str() {
                    "chats" => {
                        spawn_forever(
                            async move { crate::state::refresh_sessions(&ctx, false).await },
                        );
                    }
                    "code" => {
                        spawn_forever(async move { crate::code::refresh_code_chats(&ctx).await });
                    }
                    "diff" => crate::code::load_code_diff(&ctx),
                    _ => {}
                }
            }
        });
    });
}
