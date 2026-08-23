use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;

use crate::state::{
    new_session, open_session, refresh_sessions, short_timestamp, show_toast, use_app_ctx, Screen,
};
use crate::views::ConnBadge;

#[component]
pub fn SessionsView() -> Element {
    let ctx = use_app_ctx();
    let sessions = (ctx.sessions)();
    let loading = (ctx.sessions_loading)();
    let has_more = ctx.sessions_cursor.read().is_some();
    let mut confirm_delete = use_signal(|| None::<String>);

    rsx! {
        header { class: "topbar",
            h1 { class: "title", "Sessions" }
            ConnBadge {}
            button {
                class: "icon-btn",
                onclick: move |_| {
                    let mut screen = ctx.screen;
                    screen.set(Screen::Settings);
                },
                "⚙"
            }
        }
        main { class: "scroll",
            div { class: "btn-row list-actions",
                button {
                    class: "btn primary grow",
                    onclick: move |_| new_session(ctx),
                    "＋ New chat"
                }
                button {
                    class: "btn secondary",
                    disabled: loading,
                    onclick: move |_| {
                        spawn_forever(async move { refresh_sessions(ctx, false).await });
                    },
                    if loading { "…" } else { "↻" }
                }
            }

            if sessions.is_empty() && !loading {
                p { class: "empty", "No sessions yet — start a new chat." }
            }

            ul { class: "session-list",
                for info in sessions {
                    li {
                        key: "{info.session_id}",
                        class: "session-item",
                        div {
                            class: "session-main",
                            onclick: {
                                let info = info;
                                move |_| open_session(ctx, info.clone())
                            },
                            div { class: "session-title", "{info.display_title()}" }
                            if let Some(snippet) = info.last_message_snippet() {
                                div { class: "session-snippet", "{snippet}" }
                            }
                            div { class: "session-meta",
                                span { "{info.session_id}" }
                                if let Some(count) = info.message_count() {
                                    span { "· {count} msgs" }
                                }
                                if let Some(ts) = &info.updated_at {
                                    span { "· {short_timestamp(ts)}" }
                                }
                            }
                        }
                        if confirm_delete.read().as_deref() == Some(info.session_id.as_str()) {
                            div { class: "confirm-row",
                                span { "Delete?" }
                                button {
                                    class: "btn danger small",
                                    onclick: {
                                        let session_id = info.session_id.clone();
                                        move |_| {
                                            let session_id = session_id.clone();
                                            confirm_delete.set(None);
                                            spawn_forever(async move {
                                                let Some(client) = ctx.client.peek().clone() else { return };
                                                match client.session_delete(&session_id).await {
                                                    Ok(()) => {
                                                        let mut sessions = ctx.sessions;
                                                        sessions.write().retain(|s| s.session_id != session_id);
                                                    }
                                                    Err(e) => show_toast(&ctx, format!("Delete failed: {e}")),
                                                }
                                            });
                                        }
                                    },
                                    "Delete"
                                }
                                button {
                                    class: "btn secondary small",
                                    onclick: move |_| confirm_delete.set(None),
                                    "Cancel"
                                }
                            }
                        } else {
                            button {
                                class: "icon-btn trash",
                                onclick: {
                                    let session_id = info.session_id.clone();
                                    move |_| confirm_delete.set(Some(session_id.clone()))
                                },
                                "🗑"
                            }
                        }
                    }
                }
            }

            if has_more {
                div { class: "btn-row",
                    button {
                        class: "btn secondary grow",
                        disabled: loading,
                        onclick: move |_| {
                            spawn_forever(async move { refresh_sessions(ctx, true).await });
                        },
                        "Load more"
                    }
                }
            }
        }
    }
}
