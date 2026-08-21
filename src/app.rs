use dioxus::prelude::*;

use crate::state::{use_app_ctx_provider, Screen};
use crate::views;

const MAIN_CSS: Asset = asset!("/assets/main.css");

#[component]
pub fn App() -> Element {
    let ctx = use_app_ctx_provider();

    let screen = (ctx.screen)();
    let body = match screen {
        Screen::Settings => rsx! { views::settings::SettingsView {} },
        Screen::Sessions => rsx! { views::sessions::SessionsView {} },
        Screen::Chat => rsx! { views::chat::ChatView {} },
    };

    let toast = (ctx.toast)();
    let permission_open = ctx.permission.read().is_some();

    rsx! {
        document::Link { rel: "stylesheet", href: MAIN_CSS }
        document::Meta {
            name: "viewport",
            content: "width=device-width, initial-scale=1, maximum-scale=1, viewport-fit=cover",
        }
        div { class: "app",
            {body}
            if permission_open {
                views::chat::PermissionModal {}
            }
            if let Some(message) = toast {
                div { class: "toast", "{message}" }
            }
        }
    }
}
