use dioxus::prelude::*;

use crate::code::CodeScreen;
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
        },
    };

    let toast = (ctx.toast)();
    // Two independent, backend-tagged queues; the goose modal wins ties.
    let goose_permission_open = !ctx.permission.read().is_empty();
    let code_permission_open = !ctx.code_permissions.read().is_empty();

    rsx! {
        document::Style { {MAIN_CSS} }
        document::Meta {
            name: "viewport",
            content: "width=device-width, initial-scale=1, maximum-scale=1, viewport-fit=cover",
        }
        div { class: "app",
            {body}
            nav { class: "tabbar",
                button {
                    class: if tab == Tab::Home { "tab active" } else { "tab" },
                    onclick: move |_| {
                        let mut t = ctx.tab;
                        t.set(Tab::Home);
                    },
                    span { class: "tab-icon", "⌂" }
                    span { "Home" }
                }
                button {
                    class: if tab == Tab::Code { "tab active" } else { "tab" },
                    onclick: move |_| {
                        let mut t = ctx.tab;
                        t.set(Tab::Code);
                    },
                    span { class: "tab-icon", "</>" }
                    span { "Code" }
                }
            }
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
