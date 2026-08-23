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
