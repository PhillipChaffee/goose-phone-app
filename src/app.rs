use dioxus::prelude::*;

use crate::nav;
use crate::shell::AppShell;
use crate::state::use_app_ctx_provider;
use crate::views;

#[component]
pub fn App() -> Element {
    let ctx = use_app_ctx_provider();

    // Which destination is on screen, what it renders and what its dump is
    // called are three questions with one answer, and that answer is a row of
    // `nav::DESTINATIONS` rather than three `match`es that have to agree.
    // What is rendered from it is the shell's business now, because the two
    // shells arrange the same destination differently; the NAME of the dump
    // still is not — it is the same destination on both — but which shell drew
    // it is, and that is what `shell::DUMP_PREFIX` adds below.
    let dest = nav::current(&ctx);

    // The three hooks that are true of both shells. The two gesture ones —
    // tap-to-close a swiped row, and pull-to-refresh — moved into
    // `shell::mobile`, so a desktop binary does not contain them at all.
    crate::viewport::use_visual_viewport();
    crate::viewport::use_transcript_bottom();
    crate::viewport::use_file_picker();
    // One store, two shells. The prefix is `""` in a phone binary, so every
    // key already in `docs/gallery-states.json` is byte-identical to what it
    // was; it is `"desktop-"` here, so a desktop `chats` no longer overwrites
    // the phone's. See `shell::DUMP_PREFIX`.
    #[cfg(debug_assertions)]
    crate::domdump::use_dom_dump(format!(
        "{}{}",
        crate::shell::DUMP_PREFIX,
        (dest.key)(&ctx).unwrap_or(dest.id)
    ));

    let toast = (ctx.toast)();
    // Two independent, backend-tagged queues; the goose modal wins ties.
    //
    // The code queue now holds asks for chats you are not in — the manager's
    // aggregate puts them there so the list can show them — and those are
    // answered on their card, not by a modal thrown over whatever you were
    // doing. Only the ask belonging to the chat on screen is worth
    // interrupting for, which includes not interrupting the list with the
    // chat you were reading a moment ago (`open_chat_has_ask`).
    let goose_permission_open = !ctx.permission.read().is_empty();
    let code_permission_open = crate::code::open_chat_has_ask(&ctx);

    rsx! {
        document::Style { {crate::css::STYLES} }
        // After the design system: the shell is the pinned nav, the panes and
        // the width breakpoints that reflow them. Empty on iOS and Android.
        // See `crate::css::SHELL`.
        document::Style { {crate::css::SHELL} }
        // LAST, so the platform genuinely has the last word.
        //
        // It used to sit between the two, and that was wrong in a way only the
        // second platform sheet could expose. iOS did not care — `SHELL` is
        // empty in a phone binary, so there was nothing after it to lose to.
        // macOS did: `assets/desktop.css` defaults `--chrome-h`/`--traffic-w`
        // to zero on `.app > .shell` and `assets/platform/macos.css` raises
        // them on the same selector, so at equal specificity the later sheet
        // won and the reservation was always zero — measured, the nav toggle
        // was painted on top of the close button at the rail width. A platform
        // rule that a shared sheet can silently outrank is not a platform rule.
        document::Style { {crate::css::PLATFORM} }
        document::Meta {
            name: "viewport",
            // interactive-widget=resizes-content: when the keyboard opens, shrink
            // the layout viewport instead of scrolling the visual one. Without
            // it iOS slides the whole page up to reveal the focused field,
            // which carries the floating header off the top of the screen.
            content: "width=device-width, initial-scale=1, maximum-scale=1, \
                      viewport-fit=cover, interactive-widget=resizes-content",
        }
        div { class: "app",
            AppShell {}
            if goose_permission_open {
                views::chat::PermissionModal {}
            } else if code_permission_open {
                views::code::CodePermissionModal {}
            }
            if let Some(message) = toast {
                div { class: "toast", "{message}" }
            }
        }
    }
}
