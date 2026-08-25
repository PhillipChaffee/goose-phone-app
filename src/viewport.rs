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
/// What it must not swallow is a button. The rule is about the *card* — the
/// part of the row whose only job is to open the session — and a control
/// inside the row is not that. Delete has always been exempt; so is answering
/// a permission ask, which otherwise looked broken on any row you happened to
/// have swiped.
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
  // Not a tap on a control in the row — Delete, Approve and Deny all have to
  // keep working while the row is open.
  if (e.target.closest('.session-actions, .session-ask button')) return;
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

/// Whether the reader is still at the bottom of the open transcript.
///
/// Two things follow from the answer: whether new content pins the transcript
/// to its bottom, and whether the scroll-to-bottom button is on screen. Both
/// are the same question, so they are one piece of state rather than two that
/// can disagree.
///
/// It is answered in JS for the reason written above `CLOSE_OPEN_ROW`: an
/// `onscroll` handler in Rust would put a blocking round trip on every frame
/// of every scroll. Rust owns the button and its tap; this owns whether the
/// button is there to be tapped.
///
/// Three pieces of script share two globals. `window.__transcript` names the
/// scroller the answer is about and is claimed by [`pin_transcript`] when a
/// chat screen mounts, so this listener matches nothing until a transcript
/// exists and a second transcript replaces the first rather than both being
/// watched. `window.__atBottom` is the answer: written here as the reader
/// scrolls, and overridden by the other two only where the reader's position
/// is not in question — a transcript too short to scroll, one that has just
/// replaced another, or a tap on the button. It is deliberately read as
/// `!== false`, because before this listener has run there is nothing to have
/// scrolled away from and the pin must not wait on it.
///
/// Scrolling is not the only thing that moves the bottom of a transcript
/// away from the reader, which is why there is a `ResizeObserver` here as
/// well as a scroll listener. The scroller losing *height* at a fixed
/// `scrollTop` has exactly the same effect and fires no scroll event at all:
/// the keyboard shrinks `.app` to `--vv-height` — 68px for the accessory bar
/// alone, far more once the keys are up — and a draft growing to several
/// lines takes the same room out of the transcript from the other end.
const TRANSCRIPT_BOTTOM: &str = r"
(() => {
  if (window.__tbWired) return;
  window.__tbWired = true;
  // Slack, so the button cannot flicker. Content appended to a transcript
  // that was already at its bottom leaves it a few pixels short until the
  // next frame pins it again, and an exact test would flash the button on
  // for that frame of every streamed part.
  const NEAR = 48;
  const settle = (el) => {
    const away = el.scrollHeight - el.scrollTop - el.clientHeight > NEAR;
    window.__atBottom = !away;
    document.body.classList.toggle('away-from-bottom', away);
  };
  document.addEventListener('scroll', (e) => {
    if (e.target !== window.__transcript) return;
    settle(e.target);
  }, { capture: true, passive: true });
  // A shorter transcript, same scrollTop. Being at the bottom is a place
  // rather than an offset, so a reader who was there is taken there again —
  // that is what every native chat does when the keyboard covers the last
  // message, and it beats the other reading of the same event, which is to
  // tell them they have left a place they never moved from. A reader who was
  // somewhere else is left where they are and only the answer is recomputed,
  // since the distance to the bottom has just changed under them in whichever
  // direction the box moved.
  const observer = new ResizeObserver(() => {
    const el = window.__transcript;
    if (!el) return;
    if (window.__atBottom === false) settle(el);
    else el.scrollTop = el.scrollHeight;
  });
  window.__tbWatch = (el) => {
    if (window.__tbWatched === el) return;
    window.__tbWatched = el;
    observer.disconnect();
    observer.observe(el);
  };
  // The pin claims the transcript from an effect, and whether that effect
  // has already run by the time this one does is not something to depend on.
  if (window.__transcript) window.__tbWatch(window.__transcript);
})();
";

/// Wire the transcript-bottom listener. Once, for the whole app.
pub(crate) fn use_transcript_bottom() {
    use_effect(move || {
        document::eval(TRANSCRIPT_BOTTOM);
    });
}

/// Keep `id`'s transcript pinned to its bottom as it grows — unless the reader
/// has scrolled up, in which case leave them where they put themselves.
///
/// It used to pin unconditionally, which meant a streaming turn dragged you
/// back to the bottom on every part and reading back during a turn was
/// impossible.
///
/// Everything happens inside the frame callback, the element lookup included:
/// the eval is dispatched from an effect and can reach the webview before it
/// has applied the mutations that render the transcript.
pub(crate) fn pin_transcript(id: &str) {
    document::eval(&pin_script(id));
}

fn pin_script(id: &str) -> String {
    format!(
        "requestAnimationFrame(() => {{ \
           const el = document.getElementById('{id}'); \
           if (!el) return; \
           const fresh = window.__transcript !== el; \
           window.__transcript = el; \
           /* Watched for height as well as listened to for scrolling: a \
              transcript that loses height under a reader who was at its \
              bottom fires no scroll event, and the answer would go stale \
              with the button hidden. */ \
           if (window.__tbWatch) window.__tbWatch(el); \
           /* A transcript with nothing to scroll is at its bottom by \
              definition, and a transcript that has just replaced another has \
              not been read back from. Either way the answer is stale, and \
              nothing else will correct it: the listener only hears about \
              scrolling, and a screen with none never scrolls. That is the \
              state 'New chat' leaves behind when it empties the transcript \
              under a reader who had scrolled up. */ \
           if (fresh || el.scrollHeight - el.clientHeight < 8) {{ \
             window.__atBottom = true; \
             document.body.classList.remove('away-from-bottom'); \
           }} \
           if (window.__atBottom === false) return; \
           el.scrollTop = el.scrollHeight; \
         }});"
    )
}

/// Take `id`'s transcript back to its bottom, whatever the reader did — the
/// scroll-to-bottom button's tap.
///
/// A jump rather than a smooth scroll, for the same reason the pin above is
/// one: a transcript is thousands of pixels tall, an animation over that is a
/// blur, and it would fire a scroll event per frame — each of which would put
/// the button back on screen until the animation caught up with it.
pub(crate) fn scroll_to_bottom(id: &str) {
    document::eval(&jump_script(id));
}

fn jump_script(id: &str) -> String {
    // An IIFE, not a bare statement: a top-level `const` is a global lexical
    // binding that outlives the script, so the second tap would throw
    // "Identifier has already been declared" and do nothing at all.
    format!(
        "(() => {{ \
           const el = document.getElementById('{id}'); \
           if (!el) return; \
           window.__atBottom = true; \
           document.body.classList.remove('away-from-bottom'); \
           el.scrollTop = el.scrollHeight; \
         }})();"
    )
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
                        spawn_forever(async move {
                            crate::code::refresh_code_chats(&ctx).await;
                            // The asks are part of what this list says, so a
                            // pull refreshes them too rather than leaving the
                            // cards up to ten seconds behind the rows.
                            crate::code::refresh_code_permissions(&ctx).await;
                        });
                    }
                    "diff" => crate::code::load_code_diff(&ctx),
                    "pulls" => crate::code::refresh_pulls(&ctx),
                    "skills" => crate::skills::refresh(&ctx),
                    _ => {}
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::{jump_script, pin_script, TRANSCRIPT_BOTTOM};

    /// Both scripts are `format!` over a wall of escaped braces, and getting
    /// one wrong fails silently on a device: the JS either does not parse or
    /// looks up an element called `{id}`, and the transcript simply stops
    /// following the agent with nothing to say why.
    #[test]
    fn each_script_addresses_the_scroller_it_was_given() {
        for script in [pin_script("chat-scroll"), jump_script("chat-scroll")] {
            assert!(script.contains("getElementById('chat-scroll')"), "{script}");
            assert!(!script.contains("{{") && !script.contains("}}"), "{script}");
        }
        assert!(jump_script("code-chat-scroll").contains("'code-chat-scroll'"));
    }

    /// Every transcript is watched for height, not only listened to for
    /// scrolling. The keyboard shrinking the shell moves the bottom of a
    /// transcript away from a reader who has not scrolled at all, and a
    /// scroller that nothing observes answers "still at the bottom" for as
    /// long as the keyboard is up — with the button that would take them back
    /// down hidden, because that is the same answer.
    ///
    /// `docs/measure-scroll-bottom.js` drives these scripts in a browser and
    /// is where the behaviour itself is checked; this is only the wiring, and
    /// the wiring is the half that fails silently.
    #[test]
    fn a_transcript_is_watched_for_height_as_well_as_for_scrolling() {
        assert!(
            TRANSCRIPT_BOTTOM.contains("ResizeObserver"),
            "{TRANSCRIPT_BOTTOM}"
        );
        assert!(
            TRANSCRIPT_BOTTOM.contains("window.__tbWatch = "),
            "{TRANSCRIPT_BOTTOM}"
        );
        let script = pin_script("chat-scroll");
        assert!(script.contains("window.__tbWatch(el)"), "{script}");
    }

    /// A scroller names its own refresh in `data-refresh` and the JS sends
    /// that string back verbatim, so the name is a contract between a view
    /// and the `match` in `use_pull_to_refresh` — and nothing else enforces
    /// it. A view whose name has no arm still gets the whole gesture: the
    /// spinner arms, drops, and the list is never re-fetched. No compiler
    /// error, no clippy warning, because the fallthrough arm is legitimately
    /// there for the scrollers that set no name at all.
    ///
    /// This is how the skills list arrived: `crate::skills::refresh` was
    /// written for the pull and then had no caller, because the gesture and
    /// the view were authored on branches that had never met.
    #[test]
    fn every_scroller_that_names_a_refresh_has_an_arm_that_answers_to_it() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/views");
        let read = std::fs::read_dir(&dir);
        assert!(read.is_ok(), "cannot read {}", dir.display());

        // The arms live in this file, so it is its own fixture — matching on
        // the source beats a hand-kept list that would go stale exactly when
        // someone adds a destination and forgets this test exists.
        let dispatch = include_str!("viewport.rs");
        let mut found = 0;
        for entry in read.into_iter().flatten().flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap_or_default();
            for tail in source.split("\"data-refresh\": \"").skip(1) {
                let Some(name) = tail.split('"').next() else {
                    continue;
                };
                found += 1;
                assert!(
                    dispatch.contains(&format!("\"{name}\" =>")),
                    "{} emits data-refresh=\"{name}\" but use_pull_to_refresh \
                     has no arm for it, so pulling that list does nothing",
                    path.display(),
                );
            }
        }
        // A scan that silently matches nothing would pass forever.
        assert!(found >= 5, "only found {found} data-refresh scrollers");
    }
}
