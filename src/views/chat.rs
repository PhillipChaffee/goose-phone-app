use dioxus::dioxus_core::spawn_forever;
use dioxus::document;
use dioxus::prelude::*;

use crate::markdown;
use crate::state::{answer_permission, send_prompt, stop_turn, use_app_ctx, ChatItem, Screen};

#[component]
pub fn ChatView() -> Element {
    let ctx = use_app_ctx();
    let chat = (ctx.chat)();
    let usage = (ctx.usage)();
    let mut draft = use_signal(String::new);

    // Keep the transcript pinned to the bottom as content streams in. The
    // chat signal is read INSIDE the effect so it re-runs on every change.
    use_effect(move || {
        let _ = ctx.chat.read().items.len();
        document::eval(
            "requestAnimationFrame(() => { \
               const el = document.getElementById('chat-scroll'); \
               if (el) el.scrollTop = el.scrollHeight; \
             });",
        );
    });

    let running = chat.running;
    let can_send = !running && !chat.loading;

    let mut submit = move || {
        let text = draft.peek().trim().to_string();
        if text.is_empty() {
            return;
        }
        // Only clear the draft once the message was actually accepted, so a
        // failed send (e.g. disconnected) doesn't eat the typed text.
        if send_prompt(ctx, text) {
            draft.set(String::new());
        }
    };

    rsx! {
        header { class: "topbar",
            button {
                class: "icon-btn",
                onclick: move |_| {
                    let mut screen = ctx.screen;
                    screen.set(Screen::Sessions);
                    spawn_forever(async move { crate::state::refresh_sessions(ctx, false).await });
                },
                "‹"
            }
            h1 { class: "title ellipsis", "{chat.title}" }
            if let Some((used, limit)) = usage {
                span { class: "usage", "{format_tokens(used)}/{format_tokens(limit)}" }
            }
        }

        main { class: "scroll chat", id: "chat-scroll",
            if chat.loading {
                p { class: "empty", "Loading history…" }
            }
            for (index, item) in chat.items.iter().enumerate() {
                {render_item(index, item)}
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
                placeholder: "Message goose…",
                value: "{draft}",
                rows: 1,
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
                    onclick: move |_| stop_turn(ctx),
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

/// Shared transcript renderer — the Code tab reuses it (views/code.rs).
pub(crate) fn render_item(index: usize, item: &ChatItem) -> Element {
    match item {
        ChatItem::User { text } => {
            let html = markdown::escape_text(text);
            rsx! {
                div { key: "{index}", class: "bubble user",
                    div { class: "bubble-text", dangerous_inner_html: "{html}" }
                }
            }
        }
        ChatItem::Assistant { text, .. } => {
            let html = markdown::to_html(text);
            rsx! {
                div { key: "{index}", class: "bubble assistant",
                    div { class: "md", dangerous_inner_html: "{html}" }
                }
            }
        }
        ChatItem::Thought { text, .. } => {
            let html = markdown::to_html(text);
            rsx! {
                details { key: "{index}", class: "thought",
                    summary { "Thinking" }
                    div { class: "md", dangerous_inner_html: "{html}" }
                }
            }
        }
        ChatItem::Tool {
            title,
            kind,
            status,
            output,
            ..
        } => {
            let icon = tool_icon(kind);
            let has_output = !output.is_empty();
            rsx! {
                div { key: "{index}", class: "tool status-{status}",
                    div { class: "tool-head",
                        span { class: "tool-icon", "{icon}" }
                        span { class: "tool-title", "{title}" }
                        span { class: "tool-status", "{status}" }
                    }
                    if has_output {
                        details { class: "tool-output",
                            summary { "Output" }
                            pre { "{output}" }
                        }
                    }
                }
            }
        }
    }
}

fn tool_icon(kind: &str) -> &'static str {
    match kind {
        "execute" => "❯_",
        "read" => "📄",
        "edit" => "✏️",
        "delete" => "🗑",
        "move" => "📦",
        "search" => "🔍",
        "fetch" => "🌐",
        "think" => "💭",
        _ => "🔧",
    }
}

fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Modal shown while the agent waits for a tool permission decision. Renders
/// the FRONT of the permission queue; answering reveals the next request.
#[component]
pub fn PermissionModal() -> Element {
    let ctx = use_app_ctx();
    let queue = (ctx.permission)();
    let Some(request) = queue.first().cloned() else {
        return rsx! {};
    };

    let title = request
        .tool_call
        .title
        .clone()
        .or_else(|| request.tool_call.tool_name().map(str::to_string))
        .unwrap_or_else(|| "Run a tool".to_string());
    let input = request
        .tool_call
        .raw_input
        .as_ref()
        .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
        .unwrap_or_default();

    // Show which session is asking when it isn't the one on screen.
    let current = ctx.chat.read().session_id.clone();
    let session_label = if current.as_deref() != Some(request.session_id.as_str()) {
        let sessions = ctx.sessions.read();
        let name = sessions
            .iter()
            .find(|s| s.session_id == request.session_id)
            .map(|s| s.display_title())
            .unwrap_or_else(|| request.session_id.clone());
        Some(format!("Session: {name}"))
    } else {
        None
    };
    let pending_more = queue.len().saturating_sub(1);

    rsx! {
        div { class: "modal-backdrop",
            div { class: "modal",
                h2 { "Permission request" }
                if let Some(label) = session_label {
                    p { class: "modal-session", "{label}" }
                }
                p { class: "modal-tool", "{title}" }
                if !input.is_empty() {
                    details { class: "tool-output",
                        summary { "Details" }
                        pre { "{input}" }
                    }
                }
                div { class: "modal-actions",
                    for option in request.options.iter() {
                        button {
                            key: "{option.option_id}",
                            class: if option.kind.as_deref().unwrap_or("").starts_with("allow") {
                                "btn primary"
                            } else {
                                "btn danger-outline"
                            },
                            onclick: {
                                let request_id = request.request_id.clone();
                                let option_id = option.option_id.clone();
                                move |_| {
                                    answer_permission(
                                        ctx,
                                        request_id.clone(),
                                        Some(option_id.clone()),
                                    );
                                }
                            },
                            {permission_label(option.name.as_deref(), &option.option_id)}
                        }
                    }
                }
                if pending_more > 0 {
                    p { class: "modal-pending", "+{pending_more} more waiting" }
                }
            }
        }
    }
}

fn permission_label(name: Option<&str>, option_id: &str) -> String {
    let raw = name.unwrap_or(option_id);
    match raw {
        "allow_once" => "Allow once".to_string(),
        "allow_always" => "Always allow".to_string(),
        "reject_once" => "Reject".to_string(),
        "reject_always" => "Always reject".to_string(),
        other => other.replace('_', " "),
    }
}
