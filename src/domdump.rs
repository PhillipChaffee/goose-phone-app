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
//! to a state, and `scripts/capture-gallery.sh` reads the dump off the
//! console. Release builds contain none of it.

#![cfg(debug_assertions)]
#![allow(clippy::print_stdout, reason = "the console is this module's output")]

use dioxus::prelude::*;

/// Serialise `.app` after layout has settled.
///
/// 1500ms rather than a frame or two: `WebKit` applies `env(safe-area-inset-*)`
/// after first paint, and a dump taken earlier captures a shell positioned as
/// if the device had no notch.
const DUMP_JS: &str = r"
setTimeout(() => {
  const app = document.querySelector('.app');
  dioxus.send(app ? app.outerHTML : '');
}, 1500);
";

/// Print the current screen's markup whenever `key` changes.
pub(crate) fn use_dom_dump(key: String) {
    // The effect has to read a reactive value to re-run; capturing the key by
    // move would fire it once and never again.
    let mut current = use_signal(|| key.clone());
    if *current.peek() != key {
        current.set(key);
    }
    use_effect(move || {
        let key = current();
        let mut eval = document::eval(DUMP_JS);
        spawn(async move {
            if let Ok(html) = eval.recv::<String>().await {
                println!("@@DOM@@{key}@@{html}@@ENDDOM@@");
            }
        });
    });
}
