use dioxus::prelude::*;

use crate::code::CodeScreen;
use crate::icons::Icon;
use crate::state::{use_app_ctx_provider, Screen, Tab};
use crate::views;

// Embedded at compile time so styling works identically under `cargo run`,
// `dx serve`, and mobile bundles.
const MAIN_CSS: &str = include_str!("../assets/main.css");

#[component]
pub fn App() -> Element {
    let ctx = use_app_ctx_provider();

    // Each tab renders its own screen stack, so switching tabs preserves
    // where you were on the other one (issue #2, A3).
    let tab = (ctx.tab)();
    let body = match tab {
        Tab::Home => match (ctx.screen)() {
            Screen::Settings => rsx! { views::settings::SettingsView {} },
            Screen::Sessions => rsx! { views::sessions::SessionsView {} },
            Screen::Chat => rsx! { views::chat::ChatView {} },
        },
        Tab::Code => match (ctx.code_screen)() {
            CodeScreen::List => rsx! { views::code::CodeSessionsView {} },
            CodeScreen::New => rsx! { views::code::CodeNewView {} },
            CodeScreen::Chat => rsx! { views::code::CodeChatView {} },
            CodeScreen::Diff => rsx! { views::code::CodeDiffView {} },
        },
    };

    crate::viewport::use_visual_viewport();
    crate::viewport::use_close_open_row();
    crate::viewport::use_pull_to_refresh();
    crate::viewport::use_transcript_bottom();
    #[cfg(debug_assertions)]
    crate::domdump::use_dom_dump(
        match tab {
            Tab::Home => match (ctx.screen)() {
                Screen::Settings => "settings",
                Screen::Sessions => "chats",
                Screen::Chat => "chat",
            },
            Tab::Code => match (ctx.code_screen)() {
                CodeScreen::List => "code-list",
                CodeScreen::New => "code-new",
                CodeScreen::Chat => "code-chat",
                CodeScreen::Diff => "code-diff",
            },
        }
        .to_owned(),
    );

    let toast = (ctx.toast)();
    // Two independent, backend-tagged queues; the goose modal wins ties.
    let goose_permission_open = !ctx.permission.read().is_empty();
    let code_permission_open = !ctx.code_permissions.read().is_empty();

    rsx! {
        document::Style { {MAIN_CSS} }
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
/// destinations and left no room for a third. Destinations live here; the
/// screen underneath keeps its own back stack, so opening the drawer and
/// coming back leaves you where you were.
#[component]
fn Drawer() -> Element {
    let ctx = crate::state::use_app_ctx();
    let mut open = ctx.drawer_open;
    let tab = (ctx.tab)();
    let on_settings = tab == Tab::Home && (ctx.screen)() == Screen::Settings;

    // A destination is "here" only when its own stack is at its root: from a
    // chat, Chats is somewhere to go back to, not where you are.
    let chats_here = tab == Tab::Home && (ctx.screen)() == Screen::Sessions;
    let code_here = tab == Tab::Code;

    rsx! {
        div {
            class: if open() { "drawer-scrim open" } else { "drawer-scrim" },
            onclick: move |_| open.set(false),
        }
        aside { class: if open() { "drawer open" } else { "drawer" },
            h2 { class: "drawer-brand", "goose" }
            nav { class: "drawer-nav",
                button {
                    class: if chats_here { "drawer-item active" } else { "drawer-item" },
                    onclick: move |_| navigate(&ctx, Destination::Chats),
                    Icon { name: "message" }
                    "Chats"
                }
                button {
                    class: if code_here { "drawer-item active" } else { "drawer-item" },
                    onclick: move |_| navigate(&ctx, Destination::Code),
                    Icon { name: "code" }
                    "Code"
                }
                button {
                    class: if on_settings { "drawer-item active" } else { "drawer-item" },
                    onclick: move |_| navigate(&ctx, Destination::Settings),
                    Icon { name: "gear" }
                    "Settings"
                }
            }
        }
    }
}

/// Go to a destination and close the drawer behind you.
fn navigate(ctx: &crate::state::AppCtx, to: Destination) {
    let (mut tab, mut screen, mut open) = (ctx.tab, ctx.screen, ctx.drawer_open);
    match to {
        Destination::Chats => {
            tab.set(Tab::Home);
            screen.set(Screen::Sessions);
        }
        Destination::Code => tab.set(Tab::Code),
        Destination::Settings => {
            tab.set(Tab::Home);
            screen.set(Screen::Settings);
        }
    }
    open.set(false);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Destination {
    Chats,
    Code,
    Settings,
}
