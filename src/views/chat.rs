use dioxus::dioxus_core::spawn_forever;
use dioxus::document;
use dioxus::prelude::*;

use crate::icons::Icon;
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

    // The agent sends every session config option; the picker only offers the
    // model. The others (provider, mode, thinking effort) are deliberately
    // not surfaced yet — one control at a time.
    let config = (ctx.config_options)();
    let model_option = config.iter().find(|o| o.config_id == "model").cloned();
    let mut picker = use_signal(|| false);

    let mut submit = move || {
        let text = draft.peek().trim().to_string();
        if text.is_empty() {
            return;
        }
        // Only clear the draft once the message was actually accepted, so a
        // failed send (e.g. disconnected) doesn't eat the typed text.
        if send_prompt(&ctx, text) {
            draft.set(String::new());
        }
    };

    rsx! {
        header { class: "topbar",
            button {
                class: "icon-btn back",
                onclick: move |_| {
                    let mut screen = ctx.screen;
                    screen.set(Screen::Sessions);
                    spawn_forever(async move { crate::state::refresh_sessions(&ctx, false).await });
                },
                Icon { name: "chevron-left" }
            }
            h1 { class: "title ellipsis", "{chat.title}" }
            div { class: "topbar-actions" }
        }

        main { class: "scroll chat", id: "chat-scroll",
            if chat.loading {
                p { class: "empty", "Loading history…" }
            }
            {render_transcript(&chat.items, &chat.marks)}
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
            div { class: "composer-row",
                if let Some(model) = model_option.as_ref() {
                    button {
                        class: "composer-chip action",
                        onclick: move |_| picker.set(true),
                        {model.current_label().unwrap_or("Model").to_string()}
                        Icon { name: "chevron-down" }
                    }
                }
                if let Some((used, limit)) = usage {
                    span { class: "composer-chip",
                        "{format_tokens(used)}/{format_tokens(limit)}"
                    }
                }
                if running {
                    button {
                        class: "send stop",
                        title: "Stop",
                        onclick: move |_| stop_turn(&ctx),
                        Icon { name: "stop" }
                    }
                } else {
                    button {
                        class: "send",
                        title: "Send",
                        disabled: !can_send,
                        onclick: move |_| submit(),
                        Icon { name: "arrow-up" }
                    }
                }
            }
        }

        if picker() {
            if let Some(model) = model_option {
                div { class: "modal-backdrop", onclick: move |_| picker.set(false),
                    div { class: "modal sheet", onclick: move |e: Event<MouseData>| e.stop_propagation(),
                        h2 { "{model.name}" }
                        div { class: "choice-list",
                            for choice in model.options.iter() {
                                button {
                                    key: "{choice.value}",
                                    class: if model.current_value.as_deref() == Some(choice.value.as_str()) {
                                        "choice selected"
                                    } else {
                                        "choice"
                                    },
                                    onclick: {
                                        let value = choice.value.clone();
                                        move |_| {
                                            crate::state::set_config_option(&ctx, "model", &value);
                                            picker.set(false);
                                        }
                                    },
                                    span { class: "choice-name", "{choice.name}" }
                                    if model.current_value.as_deref() == Some(choice.value.as_str()) {
                                        Icon { name: "check" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Render a whole transcript, folding runs of tool calls into one line.
///
/// An agent that reads four files and runs a command produces five cards in
/// a row, and the reply they were in service of scrolls off the top. A run is
/// collapsed to a single summary the reader can open — unless something in it
/// failed or is still going, in which case it stays open, because that is
/// exactly when you want to see it.
pub(crate) fn render_transcript(items: &[ChatItem], marks: &[(usize, i64)]) -> Element {
    let mut out: Vec<Element> = Vec::new();
    let mut i = 0;
    while i < items.len() {
        for (at, secs) in marks.iter().filter(|(at, _)| *at == i) {
            let label = clock_label(*secs);
            out.push(rsx! {
                div { key: "mark-{at}", class: "timemark", span { "{label}" } }
            });
        }
        if !matches!(items[i], ChatItem::Tool { .. }) {
            out.push(render_item(i, &items[i]));
            i += 1;
            continue;
        }
        let start = i;
        while i < items.len() && matches!(items[i], ChatItem::Tool { .. }) {
            i += 1;
        }
        let run = &items[start..i];
        let unsettled = run.iter().any(|it| {
            matches!(it, ChatItem::Tool { status, .. }
                if !matches!(status.as_str(), "completed"))
        });
        if run.len() < 2 || unsettled {
            for (offset, item) in run.iter().enumerate() {
                out.push(render_item(start + offset, item));
            }
        } else {
            let summary = tool_run_summary(run);
            let cards = run
                .iter()
                .enumerate()
                .map(|(offset, item)| render_item(start + offset, item));
            out.push(rsx! {
                details { key: "run-{start}", class: "tool-run",
                    summary { "{summary}" }
                    {cards}
                }
            });
        }
    }
    rsx! { {out.into_iter()} }
}

/// Wall-clock "HH:MM" for a mark. Local time is not available without a
/// timezone database, so this is UTC — which is what the servers log in too,
/// and the mark exists to say "there was a gap here", not to be a clock.
fn clock_label(secs: i64) -> String {
    let mins = secs.div_euclid(60);
    format!(
        "{:02}:{:02}",
        mins.div_euclid(60).rem_euclid(24),
        mins.rem_euclid(60)
    )
}

/// "Used 4 tools" plus what they were, when they were all the same thing.
fn tool_run_summary(run: &[ChatItem]) -> String {
    let mut kinds: Vec<&str> = run
        .iter()
        .filter_map(|it| match it {
            ChatItem::Tool { kind, .. } => Some(kind.as_str()),
            _ => None,
        })
        .collect();
    kinds.sort_unstable();
    kinds.dedup();
    let n = run.len();
    match kinds.as_slice() {
        [one] => format!("Used {n} tools · {}", tool_kind_phrase(one, n)),
        _ => format!("Used {n} tools"),
    }
}

fn tool_kind_phrase(kind: &str, n: usize) -> String {
    let plural = if n == 1 { "" } else { "s" };
    match kind {
        "execute" => format!("ran {n} command{plural}"),
        "read" => format!("read {n} file{plural}"),
        "edit" => format!("edited {n} file{plural}"),
        "search" => format!("ran {n} search{}", if n == 1 { "" } else { "es" }),
        "fetch" => format!("fetched {n} URL{plural}"),
        other => format!("{other} x{n}"),
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
                        span { class: "tool-icon", Icon { name: "{icon}" } }
                        span { class: "tool-title", "{title}" }
                        span { class: "tool-status", "{tool_status_label(status)}" }
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

/// Human wording for a tool's status.
///
/// The two backends speak different vocabularies — ACP emits
/// `pending`/`in_progress`/`completed`/`failed`, `OpenCode` emits
/// `pending`/`running`/`completed`/`error` — and neither is UI copy. The
/// class name still carries the raw value, so the colour rules keep working
/// on both. Anything unrecognised is tidied rather than dropped: a backend
/// that grows a new state shows that state instead of nothing.
fn tool_status_label(status: &str) -> String {
    match status {
        "pending" => "Queued".to_owned(),
        "in_progress" | "running" => "Running".to_owned(),
        "completed" => "Done".to_owned(),
        "failed" | "error" => "Failed".to_owned(),
        other => {
            let mut words = other.replace('_', " ");
            if let Some(first) = words.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            words
        }
    }
}

/// Icon name for a tool kind. These were emoji; see `crate::icons` for why
/// they are not any more.
fn tool_icon(kind: &str) -> &'static str {
    match kind {
        "execute" => "terminal",
        "read" => "file",
        "edit" => "pencil",
        "delete" => "trash",
        "move" => "package",
        "search" => "search",
        "fetch" => "globe",
        "think" => "think",
        _ => "wrench",
    }
}

fn format_tokens(n: u64) -> String {
    // Scoped to this one cast, not to the whole function: anything else added
    // here should have to justify its own arithmetic.
    #[expect(
        clippy::cast_precision_loss,
        reason = "token counts are orders of magnitude below 2^53, where f64 \
                  stops representing integers exactly"
    )]
    let tokens = n as f64;
    if n >= 1_000_000 {
        format!("{:.1}M", tokens / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", tokens / 1_000.0)
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
    let session_label = if current.as_deref() == Some(request.session_id.as_str()) {
        None
    } else {
        let sessions = ctx.sessions.read();
        let name = sessions
            .iter()
            .find(|s| s.session_id == request.session_id)
            .map_or_else(
                || request.session_id.clone(),
                goose_acp_client::SessionInfo::display_title,
            );
        Some(format!("Session: {name}"))
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
                            class: permission_button_class(
                                option.kind.as_deref(),
                                option.name.as_deref(),
                                &option.option_id,
                            ),
                            onclick: {
                                let request_id = request.request_id.clone();
                                let option_id = option.option_id.clone();
                                move |_| {
                                    answer_permission(
                                        &ctx,
                                        &request_id,
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

/// Which button treatment a permission option gets.
///
/// Keyed off what the option *is*, never off where it appears in the list:
/// the backend decides that order, and on a real ACP server "always allow"
/// arrived first, so an order-dependent rule painted the broadest possible
/// grant as the solid default and the one-shot grant as the quiet secondary.
/// The safest allow is the default; a broader allow is available but never
/// the thing your thumb lands on.
fn permission_button_class(
    kind: Option<&str>,
    name: Option<&str>,
    option_id: &str,
) -> &'static str {
    let raw = name.or(kind).unwrap_or(option_id);
    if !raw.starts_with("allow") {
        return "btn danger-outline";
    }
    if raw.contains("always") {
        "btn secondary"
    } else {
        "btn primary"
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
