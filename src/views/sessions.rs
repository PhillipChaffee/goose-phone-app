use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;

use crate::icons::Icon;
use crate::state::{
    new_session, open_session, refresh_sessions, relative_time, rfc3339_to_epoch, show_toast,
    use_app_ctx,
};
use crate::views::chrome::{ListRow, RowAction, TopBar};
use crate::views::ConfirmDelete;

#[component]
pub fn SessionsView() -> Element {
    let ctx = use_app_ctx();
    let sessions = (ctx.sessions)();
    let loading = (ctx.sessions_loading)();
    let has_more = ctx.sessions_next.read().is_some();
    let mut confirm_delete = use_signal(|| None::<String>);

    rsx! {
        TopBar { title: "Chats", conn: true }
        main {
            class: "scroll has-fab",
            // Named, so the pull-to-refresh listener knows this list has
            // something to fetch and which fetch it is.
            "data-refresh": "chats",
            "data-refreshing": "{loading}",
            if sessions.is_empty() && !loading {
                p { class: "empty", "No sessions yet — start a new chat." }
            }

            ul { class: "session-list",
                for info in sessions {
                    ListRow {
                        key: "{info.session_id}",
                        icon: "message",
                        title: info.display_title(),
                        trailing: info.updated_at.as_deref()
                            .and_then(rfc3339_to_epoch)
                            .map(relative_time),
                        actions: vec![RowAction::delete(EventHandler::new({
                            let session_id = info.session_id.clone();
                            move |()| confirm_delete.set(Some(session_id.clone()))
                        }))],
                        on_open: EventHandler::new({
                            let info = info.clone();
                            move |()| open_session(&ctx, info.clone())
                        }),
                        div { class: "session-meta",
                            if let Some(count) = info.message_count() {
                                span { "{count} msgs" }
                            }
                            span { "{info.session_id}" }
                        }
                        if let Some(snippet) = info.last_message_snippet() {
                            div { class: "session-quote", "{snippet}" }
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
                            spawn_forever(async move { refresh_sessions(&ctx, true).await });
                        },
                        "Load more"
                    }
                }
            }
        }

        button {
            class: "fab",
            onclick: move |_| new_session(&ctx),
            Icon { name: "plus" }
            "New chat"
        }

        if let Some(session_id) = confirm_delete() {
            ConfirmDelete {
                title: "Delete this chat?",
                body: "The whole conversation goes from the goose server. \
                       This cannot be undone.",
                on_cancel: move |()| confirm_delete.set(None),
                on_confirm: move |()| {
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
                },
            }
        }
    }
}
