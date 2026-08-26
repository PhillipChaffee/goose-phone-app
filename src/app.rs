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
    // shells arrange the same destination differently; the dump key is not,
    // because it is the same key either way.
    let dest = nav::current(&ctx);

    // The three hooks that are true of both shells. The two gesture ones —
    // tap-to-close a swiped row, and pull-to-refresh — moved into
    // `shell::mobile`, so a desktop binary does not contain them at all.
    crate::viewport::use_visual_viewport();
    crate::viewport::use_transcript_bottom();
    crate::viewport::use_file_picker();
    #[cfg(debug_assertions)]
    crate::domdump::use_dom_dump((dest.key)(&ctx).unwrap_or(dest.id).to_owned());

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
        // After the design system, not before it: the Dynamic Type opt-in is
        // a rule on `html` and main.css has one of its own, so cascade order
        // is what decides. Empty on every platform but iOS, where it is the
        // one declaration that makes `rem` mean the reader's chosen text size.
        document::Style { {crate::css::PLATFORM} }
        // Last, so it is the last word — the same cascade reason PLATFORM
        // comes after STYLES. Empty on iOS and Android; on a desktop build it
        // is the pinned nav, the panes and the width breakpoints that reflow
        // them. See `crate::css::SHELL`.
        document::Style { {crate::css::SHELL} }
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
