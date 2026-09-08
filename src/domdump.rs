//! Dumping the live DOM, so the style gallery can be generated rather than
//! transcribed.
//!
//! `docs/style-gallery.html` used to be a hand-written copy of the markup the
//! views emit. It drifted — far enough that a whole review pass examined
//! states the app no longer produced, and `docs/audit.js`, which reads the
//! gallery, was reporting on markup that did not exist. A copy maintained by
//! hand will drift again.
//!
//! So the gallery is generated from this. Build a debug binary, drive the app
//! to a state, and `scripts/capture-gallery.py` reads the dump off the
//! console. Release builds contain none of it.

#![cfg(debug_assertions)]
#![allow(clippy::print_stdout, reason = "the console is this module's output")]

use dioxus::prelude::*;

/// Serialise `.app` whenever it settles in a new state.
///
/// Two things this has to get right, both learned the hard way:
///
/// 1500ms rather than a frame or two: `WebKit` applies `env(safe-area-inset-*)`
/// after first paint, and a dump taken earlier captures a shell positioned as
/// if the device had no notch.
///
/// And the screen is not the state. Keying only on which view is mounted meant
/// a drawer, a settings sheet, a swiped-open row and a confirm dialog were all
/// filed under the screen behind them, so the last dump won and the gallery
/// never contained any of them — which is how three branches' worth of new UI
/// came to sit outside everything `docs/audit.js` checks. The overlays are
/// therefore identified here, from the DOM, rather than by asking every view
/// to report itself to a debug harness.
const DUMP_JS: &str = r"
(() => {
  const app = document.querySelector('.app');
  if (!app || window.__dumpWired) return;
  window.__dumpWired = true;

  const suffix = () => {
    const on = [];
    if (document.querySelector('.drawer.open')) on.push('drawer');
    // These nest: a choice list lives inside a sheet, and a sheet is itself
    // a modal. Ask most-specific first, or every overlay reports as 'confirm'.
    if (document.querySelector('.modal.sheet.menu')) on.push('menu');
    else if (document.querySelector('.modal.sheet.picker')) on.push('picker');
    else if (document.querySelector('.modal.sheet.rename')) on.push('rename');
    else if (document.querySelector('.choice-list')) on.push('choices');
    else if (document.querySelector('.modal.sheet')) on.push('sheet');
    else if (document.querySelector('.modal-backdrop')) on.push('confirm');
    if (document.querySelector('.toast')) on.push('toast');
    // A row carrying a permission ask is a different card: a tinted panel,
    // two buttons and a dot on the tile. Filed under the plain list it would
    // simply replace it, and one of the two would go unaudited — the same
    // way the overlays above used to.
    if (document.querySelector('.session-ask')) on.push('ask');
    // AND NEITHER IS A COMPOSER THAT EXPANDS (#281). The desktop Code home's
    // composer gains three controls, a taller field and an attachment tray the
    // moment it takes focus — 776x126 to 776x158, measured — and that is the
    // same shape of state as the overlays above: filed under `code-list` it
    // would simply REPLACE the resting board, and one of the two would go
    // unaudited. Unlike them it carries no new class, so nothing else in the
    // repo could have noticed; `every_class_the_desktop_shell_renders_is_in_
    // the_captured_store` is a scan for names and this state is an attribute.
    if (document.querySelector('.home-compose[data-open=true]')) on.push('compose');
    const rows = document.querySelectorAll('.session-item');
    if ([...rows].some((r) => r.scrollLeft > 4)) on.push('swiped');
    return on.length ? '-' + on.join('-') : '';
  };

  let timer = 0;
  let last = '';
  const emit = () => {
    const el = document.querySelector('.app');
    if (!el) return;
    const html = el.outerHTML;
    const key = (window.__dumpKey || 'unknown') + suffix();
    const stamp = key + '@@SEP@@' + html;
    if (stamp === last) return;
    last = stamp;
    dioxus.send(key + '@@SEP@@' + html);
  };
  const settle = () => {
    clearTimeout(timer);
    timer = setTimeout(emit, 1500);
  };
  new MutationObserver(settle).observe(document.body, {
    subtree: true, childList: true, attributes: true, characterData: true,
  });
  // A swipe changes scrollLeft, which mutates nothing.
  document.addEventListener('scroll', settle, true);
  settle();
})();
";

/// Print the current screen's markup whenever the UI settles in a new state.
pub(crate) fn use_dom_dump(key: String) {
    // The effect has to read a reactive value to re-run; capturing the key by
    // move would fire it once and never again.
    let mut current = use_signal(|| key.clone());
    if *current.peek() != key {
        current.set(key);
    }
    use_effect(move || {
        let key = current();
        // The observer is installed once and reads the key from the window, so
        // a state that no Rust signal knows about — a swiped row — still gets
        // dumped under the right screen.
        let mut eval = document::eval(&format!("window.__dumpKey = {key:?};\n{DUMP_JS}"));
        spawn(async move {
            while let Ok(line) = eval.recv::<String>().await {
                if let Some((key, html)) = line.split_once("@@SEP@@") {
                    println!("@@DOM@@{key}@@{html}@@ENDDOM@@");
                }
            }
        });
    });
}
