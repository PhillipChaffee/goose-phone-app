//! Code tab views: the chat list (with lifecycle status), the new-session
//! form, the chat screen (cached-instant open, live streaming, diff, PR),
//! and the permission modal for code chats. Transcript items render through
//! the same `chat::render_item` the Home tab uses.

use dioxus::dioxus_core::spawn_forever;
use dioxus::document;
use dioxus::prelude::*;

use crate::code::{
    answer_code_permission, delete_code_chat, load_code_diff, new_code_chat, open_code_chat,
    refresh_code_chats, request_pr, send_code_prompt, start_code_poll, status_label,
    stop_code_turn, CodeScreen,
};
use crate::state::{use_app_ctx, ConnState, Screen, Tab};
use crate::views::chat::render_item;

#[component]
pub fn CodeSessionsView() -> Element {
    let ctx = use_app_ctx();
    let chats = (ctx.code_chats)();
    let loading = (ctx.code_chats_loading)();
    let conn = (ctx.code_conn)();
    let mut confirm_delete = use_signal(|| None::<String>);

    // First visit: connect with saved settings and start the list poll.
    use_hook(|| {
        if ctx.code_client.peek().is_none()
            && !ctx.settings.peek().code_server_url.trim().is_empty()
        {
            spawn_forever(async move {
                if crate::code::code_connect(&ctx).await {
                    start_code_poll(&ctx);
                }
            });
        }
    });

    let running_chat = ctx.code_chat.read().chat_id.clone();
    let running_turn = ctx.code_chat.read().running;

    rsx! {
        header { class: "topbar",
            h1 { class: "title", "Code" }
            span { class: "conn-badge",
                match &conn {
                    ConnState::Connected { agent } => rsx! {
                        span { class: "dot on" }
                        span { class: "conn-label", "{agent}" }
                    },
                    ConnState::Connecting => rsx! {
                        span { class: "dot busy" }
                        span { class: "conn-label", "connecting…" }
                    },
                    ConnState::Failed(_) => rsx! {
                        span { class: "dot err" }
                        span { class: "conn-label", "error" }
                    },
                    ConnState::Disconnected => rsx! {
                        span { class: "dot off" }
                        span { class: "conn-label", "offline" }
                    },
                }
            }
            button {
                class: "icon-btn",
                onclick: move |_| {
                    let mut tab = ctx.tab;
                    let mut screen = ctx.screen;
                    tab.set(Tab::Home);
                    screen.set(Screen::Settings);
                },
                "⚙"
            }
        }
        main { class: "scroll",
            if let ConnState::Failed(error) = &conn {
                p { class: "error-box", "{error}" }
                div { class: "btn-row",
                    button {
                        class: "btn primary grow",
                        onclick: move |_| {
                            spawn_forever(async move {
                                if crate::code::code_connect(&ctx).await {
                                    start_code_poll(&ctx);
                                }
                            });
                        },
                        "Retry"
                    }
                }
            }
            if matches!(conn, ConnState::Disconnected) {
                p { class: "empty",
                    "Set the code server URL and password in Settings, then come back."
                }
            }

            if conn.is_connected() {
                div { class: "btn-row list-actions",
                    button {
                        class: "btn primary grow",
                        onclick: move |_| {
                            let mut screen = ctx.code_screen;
                            screen.set(CodeScreen::New);
                        },
                        "＋ New session"
                    }
                    button {
                        class: "btn secondary",
                        disabled: loading,
                        onclick: move |_| {
                            spawn_forever(async move { refresh_code_chats(&ctx).await });
                        },
                        if loading { "…" } else { "↻" }
                    }
                }

                if chats.is_empty() && !loading {
                    p { class: "empty", "No code sessions yet — start one against a repo." }
                }

                ul { class: "session-list",
                    for meta in chats {
                        li {
                            key: "{meta.id}",
                            class: "session-item",
                            div {
                                class: "session-main",
                                onclick: move |_| open_code_chat(&ctx, meta.clone()),
                                div { class: "session-title",
                                    {
                                        let turn = running_chat.as_deref() == Some(meta.id.as_str())
                                            && running_turn;
                                        let (dot, label) = status_label(&meta, turn);
                                        rsx! {
                                            span { class: "{dot}" }
                                            span { " {meta.title} " }
                                            span { class: "chip", "{label}" }
                                        }
                                    }
                                }
                                div { class: "session-meta",
                                    span { "{meta.repo}" }
                                    if !meta.branch.is_empty() {
                                        span { "· {meta.branch}" }
                                    }
                                }
                            }
                            if confirm_delete.read().as_deref() == Some(meta.id.as_str()) {
                                div { class: "confirm-row",
                                    span { "Delete chat + workspace?" }
                                    button {
                                        class: "btn danger small",
                                        onclick: {
                                            let id = meta.id.clone();
                                            move |_| {
                                                confirm_delete.set(None);
                                                delete_code_chat(&ctx, id.clone());
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
                                        let id = meta.id.clone();
                                        move |_| confirm_delete.set(Some(id.clone()))
                                    },
                                    "🗑"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn CodeNewView() -> Element {
    let ctx = use_app_ctx();
    let repos = (ctx.code_repos)();
    let mut repo = use_signal(String::new);
    let mut task = use_signal(String::new);
    let mut model = use_signal(String::new);

    // Default the picker to the first allowlisted repo.
    if repo.peek().is_empty() {
        if let Some(first) = repos.first() {
            repo.set(first.name.clone());
        }
    }

    rsx! {
        header { class: "topbar",
            button {
                class: "icon-btn",
                onclick: move |_| {
                    let mut screen = ctx.code_screen;
                    screen.set(CodeScreen::List);
                },
                "‹"
            }
            h1 { class: "title", "New code session" }
        }
        main { class: "scroll settings",
            section { class: "card",
                label { class: "field-label", "Repository" }
                select {
                    class: "field",
                    value: "{repo}",
                    onchange: move |e| repo.set(e.value()),
                    for r in repos.iter() {
                        option { key: "{r.name}", value: "{r.name}", "{r.name}" }
                    }
                }
                p { class: "hint",
                    "Repos come from the brain's allowlist (/data/code-agents/repos.json)."
                }

                label { class: "field-label", "Task" }
                textarea {
                    class: "field",
                    rows: 4,
                    placeholder: "What should the agent do?",
                    value: "{task}",
                    oninput: move |e| task.set(e.value()),
                }

                label { class: "field-label", "Model (optional)" }
                input {
                    class: "field",
                    r#type: "text",
                    placeholder: "opencode/deepseek-v4-flash (default)",
                    autocapitalize: "off",
                    autocomplete: "off",
                    spellcheck: "false",
                    value: "{model}",
                    oninput: move |e| model.set(e.value()),
                }
                p { class: "hint",
                    "Any provider/model from the OpenCode catalog. Free models are "
                    "refused for private repos (privacy hard rule 1)."
                }
            }
            div { class: "btn-row",
                button {
                    class: "btn primary grow",
                    disabled: repo.read().is_empty() || task.read().trim().is_empty(),
                    onclick: move |_| {
                        let m = model.peek().trim().to_string();
                        new_code_chat(
                            &ctx,
                            repo.peek().clone(),
                            task.peek().trim().to_string(),
                            if m.is_empty() { None } else { Some(m) },
                        );
                    },
                    "Start session"
                }
            }
        }
    }
}

#[component]
pub fn CodeChatView() -> Element {
    let ctx = use_app_ctx();
    let chat = (ctx.code_chat)();
    let mut draft = use_signal(String::new);

    use_effect(move || {
        let _ = ctx.code_chat.read().items.len();
        document::eval(
            "requestAnimationFrame(() => { \
               const el = document.getElementById('code-chat-scroll'); \
               if (el) el.scrollTop = el.scrollHeight; \
             });",
        );
    });

    let running = chat.running;
    // Cached transcript is read-only until the server is authoritative (A5).
    let can_send = !running && !chat.waking && !chat.loading;

    let mut submit = move || {
        let text = draft.peek().trim().to_string();
        if text.is_empty() {
            return;
        }
        if send_code_prompt(&ctx, text) {
            draft.set(String::new());
        }
    };

    rsx! {
        header { class: "topbar",
            button {
                class: "icon-btn",
                onclick: move |_| {
                    let mut screen = ctx.code_screen;
                    screen.set(CodeScreen::List);
                    spawn_forever(async move { refresh_code_chats(&ctx).await });
                },
                "‹"
            }
            h1 { class: "title ellipsis", "{chat.title}" }
            button {
                class: "icon-btn",
                title: "Show diff",
                onclick: move |_| load_code_diff(&ctx),
                "±"
            }
            button {
                class: "icon-btn",
                title: "Push branch + open a PR",
                disabled: !can_send,
                onclick: move |_| request_pr(&ctx),
                "⇪ PR"
            }
        }

        main { class: "scroll chat", id: "code-chat-scroll",
            if chat.waking {
                div { class: "banner",
                    "Waking the container — showing the cached transcript…"
                }
            }
            if chat.loading && chat.items.is_empty() {
                p { class: "empty", "Loading history…" }
            }
            for (index, item) in chat.items.iter().enumerate() {
                {render_item(index, item)}
            }
            if let Some(diff) = &chat.diff {
                div { class: "diff-panel",
                    div { class: "tool-head",
                        span { class: "tool-title", "Session diff" }
                        button {
                            class: "icon-btn",
                            onclick: move |_| {
                                let mut c = ctx.code_chat;
                                c.write().diff = None;
                            },
                            "✕"
                        }
                    }
                    pre { "{diff}" }
                }
            }
            if running {
                div { class: "typing",
                    span { class: "dot-anim" }
                    span { class: "dot-anim" }
                    span { class: "dot-anim" }
                }
            }
        }

        footer { class: "composer",
            textarea {
                class: "input",
                placeholder: if chat.waking { "Waking…" } else { "Message the code agent…" },
                value: "{draft}",
                rows: 1,
                disabled: chat.waking,
                oninput: move |e| draft.set(e.value()),
                onkeydown: move |e| {
                    if e.key() == Key::Enter && !e.modifiers().contains(Modifiers::SHIFT) {
                        e.prevent_default();
                        if can_send {
                            submit();
                        }
                    }
                },
            }
            if running {
                button {
                    class: "btn danger",
                    onclick: move |_| stop_code_turn(&ctx),
                    "Stop"
                }
            } else {
                button {
                    class: "btn primary",
                    disabled: !can_send,
                    onclick: move |_| submit(),
                    "Send"
                }
            }
        }
    }
}

/// Modal for the front of the code-permission queue. Backend-tagged by
/// construction: it answers over the code client only, so a goose ask and a
/// code ask can never be confused (issue #2, A6).
#[component]
pub fn CodePermissionModal() -> Element {
    let ctx = use_app_ctx();
    let queue = (ctx.code_permissions)();
    let Some((chat_id, perm)) = queue.first().cloned() else {
        return rsx! {};
    };

    let detail = if perm.metadata.is_null() {
        String::new()
    } else {
        serde_json::to_string_pretty(&perm.metadata).unwrap_or_default()
    };
    let chat_label = {
        let chats = ctx.code_chats.read();
        chats
            .iter()
            .find(|c| c.id == chat_id)
            .map_or_else(|| chat_id.clone(), |c| c.title.clone())
    };
    let pending_more = queue.len().saturating_sub(1);
    let title = if perm.title.is_empty() {
        perm.kind.clone()
    } else {
        perm.title.clone()
    };

    rsx! {
        div { class: "modal-backdrop",
            div { class: "modal",
                h2 { "Code agent asks" }
                p { class: "modal-session", "Session: {chat_label}" }
                p { class: "modal-tool", "{title}" }
                if !detail.is_empty() {
                    details { class: "tool-output",
                        summary { "Details" }
                        pre { "{detail}" }
                    }
                }
                div { class: "modal-actions",
                    button {
                        class: "btn primary",
                        onclick: {
                            let chat_id = chat_id.clone();
                            let perm = perm.clone();
                            move |_| {
                                answer_code_permission(&ctx, chat_id.clone(), perm.clone(), "once");
                            }
                        },
                        "Allow once"
                    }
                    button {
                        class: "btn primary",
                        onclick: {
                            let chat_id = chat_id.clone();
                            let perm = perm.clone();
                            move |_| {
                                answer_code_permission(&ctx, chat_id.clone(), perm.clone(), "always");
                            }
                        },
                        "Always allow"
                    }
                    button {
                        class: "btn danger-outline",
                        onclick: move |_| {
                            answer_code_permission(&ctx, chat_id.clone(), perm.clone(), "reject");
                        },
                        "Reject"
                    }
                }
                if pending_more > 0 {
                    p { class: "modal-pending", "+{pending_more} more waiting" }
                }
            }
        }
    }
}
