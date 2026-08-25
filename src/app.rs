use dioxus::prelude::*;

use crate::icons::Icon;
use crate::nav::{self, Destination, Group, DESTINATIONS};
use crate::state::{use_app_ctx_provider, AppCtx};
use crate::views;

#[component]
pub fn App() -> Element {
    let ctx = use_app_ctx_provider();

    // Which destination is on screen, what it renders and what its dump is
    // called are three questions with one answer, and that answer is a row of
    // `nav::DESTINATIONS` rather than three `match`es that have to agree.
    let dest = nav::current(&ctx);
    let body = (dest.view)(&ctx);

    crate::viewport::use_visual_viewport();
    crate::viewport::use_close_open_row();
    crate::viewport::use_pull_to_refresh();
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
            {body}
            Drawer {}
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

/// The navigation drawer.
///
/// It replaced a bottom tab bar, which spent 100px of every screen on two
/// destinations and left no room for a third. The destinations themselves
/// live in `nav::DESTINATIONS`; this walks them in group order, so a feature
/// appears in the drawer by adding a row to that table and touching nothing
/// here.
#[component]
fn Drawer() -> Element {
    let ctx = crate::state::use_app_ctx();
    let mut open = ctx.drawer_open;

    rsx! {
        div {
            class: if open() { "drawer-scrim open" } else { "drawer-scrim" },
            onclick: move |_| open.set(false),
        }
        aside { class: if open() { "drawer open" } else { "drawer" },
            h2 { class: "drawer-brand", "goose" }
            nav { class: "drawer-nav",
                for group in Group::ALL {
                    {render_group(&ctx, group)}
                }
            }
        }
    }
}

/// One labelled band of the drawer.
///
/// An empty group renders nothing at all, header included: until the features
/// land there is no Library to head, and a heading over a gap is the app
/// promising something it does not have.
fn render_group(ctx: &AppCtx, group: Group) -> Element {
    let items: Vec<&'static Destination> = DESTINATIONS
        .iter()
        .filter(|dest| dest.group == group)
        .collect();
    if items.is_empty() {
        return rsx! {};
    }
    let ctx = *ctx;

    rsx! {
        if let Some(header) = group.header() {
            div { class: "drawer-group", "{header}" }
        }
        for dest in items {
            button {
                key: "{dest.id}",
                class: if (dest.at_root)(&ctx) { "drawer-item active" } else { "drawer-item" },
                onclick: move |_| {
                    (dest.go)(&ctx);
                    let mut open = ctx.drawer_open;
                    open.set(false);
                },
                Icon { name: dest.icon }
                "{dest.label}"
            }
        }
    }
}
