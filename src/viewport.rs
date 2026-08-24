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
