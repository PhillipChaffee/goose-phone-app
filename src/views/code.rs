//! Code tab views: the chat list (with lifecycle status), the new-session
//! form, the chat screen (cached-instant open, live streaming, diff, PR),
//! and the permission modal for code chats. Transcript items render through
//! the same `chat::render_item` the Home tab uses.

use dioxus::dioxus_core::spawn_forever;
use dioxus::document;
use dioxus::prelude::*;

use opencode_client::ModelInfo;

use crate::code::{
    answer_code_permission, delete_code_chat, ensure_code_models, is_free_model, load_code_diff,
    new_code_chat, open_chat_allows_free_models, open_code_chat, refresh_code_chats, request_pr,
    send_code_prompt, set_code_effort, set_code_model, start_code_poll, status_label,
    stop_code_turn, CodeScreen,
};
use crate::icons::Icon;
use crate::state::{relative_time_secs, use_app_ctx, AppCtx, ConnState};
use crate::views::chat::{format_tokens, render_transcript};
use crate::views::session_settings::{
    choice_label, SessionSettingsSheet, SettingChoice, SettingRow,
};

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
            button {
                class: "icon-btn menu",
                onclick: move |_| {
                    let mut open = ctx.drawer_open;
                    open.set(true);
                },
                Icon { name: "menu" }
            }
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
            div { class: "topbar-actions",
                button {
                    class: "icon-btn",
                    disabled: loading,
                    onclick: move |_| {
                        spawn_forever(async move { refresh_code_chats(&ctx).await });
                    },
                    if loading { "…" } else { Icon { name: "refresh" } }
                }
            }
        }
        main { class: "scroll has-fab",
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
                if chats.is_empty() && !loading {
                    p { class: "empty", "No code sessions yet — start one against a repo." }
                }

                ul { class: "session-list",
                    for meta in chats {
                        li {
                            key: "{meta.id}",
                            class: "session-item",
                            onclick: move |_| open_code_chat(&ctx, meta.clone()),
                            div { class: "session-tile", Icon { name: "code" } }
                            div {
                                class: "session-main",
                                div { class: "session-head",
                                    div { class: "session-title", "{meta.title}" }
                                    span { class: "session-age",
                                        {relative_time_secs(meta.last_active)}
                                    }
                                }
                                div { class: "session-meta",
                                    {
                                        let turn = running_chat.as_deref() == Some(meta.id.as_str())
                                            && running_turn;
                                        let (dot, label) = status_label(&meta, turn);
                                        rsx! {
                                            span { class: "chip",
                                                span { class: "{dot}" }
                                                "{label}"
                                            }
                                            span { "{meta.repo}" }
                                            if !meta.branch.is_empty() {
                                                span { "{meta.branch}" }
                                            }
                                        }
                                    }
                                }
                            }
                            if confirm_delete.read().as_deref() == Some(meta.id.as_str()) {
                                div { class: "confirm-row", onclick: move |e: Event<MouseData>| e.stop_propagation(),
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
                                        move |e: Event<MouseData>| {
                                            e.stop_propagation();
                                            confirm_delete.set(Some(id.clone()));
                                        }
                                    },
                                    Icon { name: "trash" }
                                }
                            }
                        }
                    }
                }
            }
        }

        if conn.is_connected() {
            button {
                class: "fab",
                onclick: move |_| {
                    let mut screen = ctx.code_screen;
                    screen.set(CodeScreen::New);
                },
                Icon { name: "plus" }
                "New session"
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
                class: "icon-btn back",
                onclick: move |_| {
                    let mut screen = ctx.code_screen;
                    screen.set(CodeScreen::List);
                },
                Icon { name: "chevron-left" }
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

    let models = (ctx.code_models)();
    let models_loading = (ctx.code_models_loading)();
    let mut sheet = use_signal(|| false);
    let chip_label = code_chip_label(chat.model.as_deref(), &models);
    let rows = code_setting_rows(&ctx, &models, models_loading);

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
                class: "icon-btn back",
                onclick: move |_| {
                    let mut screen = ctx.code_screen;
                    screen.set(CodeScreen::List);
                    spawn_forever(async move { refresh_code_chats(&ctx).await });
                },
                Icon { name: "chevron-left" }
            }
            div { class: "titlegroup",
                h1 { class: "title ellipsis", "{chat.title}" }
                if !chat.repo.is_empty() {
                    span { class: "subtitle ellipsis",
                        "{chat.repo}"
                        if !chat.branch.is_empty() {
                            " · {chat.branch}"
                        }
                    }
                }
            }
            div { class: "topbar-actions" }
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
            {render_transcript(&chat.items, &chat.marks)}
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
            div { class: "composer-row",
                button {
                    class: "composer-chip action",
                    title: "Session settings",
                    onclick: move |_| {
                        ensure_code_models(&ctx);
                        sheet.set(true);
                    },
                    "{chip_label}"
                    Icon { name: "chevron-down" }
                }
                button {
                    class: "composer-chip action",
                    title: "Show diff",
                    onclick: move |_| load_code_diff(&ctx),
                    Icon { name: "diff" }
                    "Diff"
                }
                button {
                    class: "composer-chip action",
                    title: "Push branch + open a PR",
                    disabled: !can_send,
                    onclick: move |_| request_pr(&ctx),
                    Icon { name: "pull-request" }
                    "PR"
                }
                if running {
                    button {
                        class: "send stop",
                        title: "Stop",
                        onclick: move |_| stop_code_turn(&ctx),
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

        if sheet() {
            SessionSettingsSheet {
                backend: "code agent",
                rows,
                onchoose: move |(id, value): (String, String)| match id.as_str() {
                    ROW_MODEL => set_code_model(&ctx, &value),
                    ROW_EFFORT => set_code_effort(
                        &ctx,
                        if value.is_empty() { None } else { Some(value.as_str()) },
                    ),
                    _ => {}
                },
                onclose: move |()| sheet.set(false),
            }
        }
    }
}

const ROW_MODEL: &str = "model";
const ROW_EFFORT: &str = "effort";

/// The chip's face: the model the next message will run on, by its catalogue
/// name once that has loaded and by its bare id before then.
fn code_chip_label(reference: Option<&str>, models: &[ModelInfo]) -> String {
    let Some(reference) = reference else {
        return "Model".to_owned();
    };
    models
        .iter()
        .find(|m| m.reference() == reference)
        .map_or_else(
            || reference.rsplit('/').next().unwrap_or(reference).to_owned(),
            |m| m.name.clone(),
        )
}

/// The code tab's rows: the two settings `OpenCode` really takes on a turn,
/// and the one number it only ever reports.
///
/// Model and thinking effort are both per-turn parameters of
/// `session/:id/prompt_async` (`model` and `variant`), which the server then
/// copies onto the session record — so "applies from your next message" is
/// literally the mechanism, not a hedge. Context length is not a parameter of
/// anything: it is catalogue metadata, and the one route that rewrites it
/// (`PATCH /config`) restarts the chat's server, killing the event stream the
/// app is reading. It is reported, not offered.
fn code_setting_rows(ctx: &AppCtx, models: &[ModelInfo], loading: bool) -> Vec<SettingRow> {
    let (current, effort) = {
        let chat = ctx.code_chat.peek();
        (chat.model.clone(), chat.effort.clone())
    };
    let allow_free = open_chat_allows_free_models(ctx);

    let offered: Vec<&ModelInfo> = models
        .iter()
        .filter(|m| allow_free || !is_free_model(&m.reference()))
        .collect();
    let withheld = models.len() - offered.len();
    let selected = current
        .as_deref()
        .and_then(|r| models.iter().find(|m| m.reference() == r));
    let unknown = || unknown_model_note(models, loading, current.is_some()).to_owned();

    let model_note = if withheld > 0 {
        // Say it plainly rather than letting models silently go missing: the
        // manager only checks this when a chat is created, and a per-turn
        // model would sail past that check through its transparent proxy.
        Some(format!(
            "{withheld} free models are hidden — they train on their input, and \
             this repo is not a public throwaway."
        ))
    } else if models.is_empty() {
        Some(unknown())
    } else {
        None
    };

    let mut rows = vec![SettingRow::select(
        ROW_MODEL,
        "Model",
        current.as_deref(),
        model_choices(&offered),
        model_note,
    )];

    // "Default" is a real value here, not a placeholder: OpenCode records the
    // literal string `default` on a session whose turn asked for no variant.
    let efforts = selected.map(ModelInfo::efforts).unwrap_or_default();
    let chosen = effort.as_deref().unwrap_or_default();
    let effort_value = if chosen.is_empty() {
        "Default".to_owned()
    } else {
        choice_label(chosen, chosen)
    };
    rows.push(if efforts.is_empty() {
        SettingRow::fact(
            ROW_EFFORT,
            "Thinking effort",
            effort_value,
            selected.map_or_else(unknown, |_| {
                "This model has no thinking-effort tiers.".to_owned()
            }),
        )
    } else {
        let mut choices = vec![SettingChoice::new("", "Default")];
        choices.extend(
            efforts
                .iter()
                .map(|e| SettingChoice::new(*e, choice_label(e, e))),
        );
        SettingRow::select(ROW_EFFORT, "Thinking effort", Some(chosen), choices, None)
    });

    rows.push(match selected.and_then(|m| m.limit.context_tokens()) {
        Some(limit) => SettingRow::fact(
            "context_length",
            "Context length",
            format!("{} tokens", format_tokens(limit)),
            "Declared by the model. A turn carries no context window.",
        ),
        None => SettingRow::fact("context_length", "Context length", "—", unknown()),
    });
    rows
}

/// Why a fact that comes off the model catalogue is unknown right now.
///
/// Shared by the effort row and the context-length row so the two can never
/// give different reasons for the same missing catalogue.
const fn unknown_model_note(
    models: &[ModelInfo],
    loading: bool,
    model_chosen: bool,
) -> &'static str {
    if models.is_empty() {
        if loading {
            "Available once the model list has loaded."
        } else {
            "The chat server did not offer a model list."
        }
    } else if model_chosen {
        "This model is not in the chat server's catalogue."
    } else {
        "Pick a model above and this follows from it."
    }
}

/// Catalogue entries as choices. A name shared by two providers is shown as
/// `provider/model` instead, so two rows are never indistinguishable.
fn model_choices(offered: &[&ModelInfo]) -> Vec<SettingChoice> {
    offered
        .iter()
        .map(|m| {
            let ambiguous = offered.iter().any(|o| o.name == m.name && o.id != m.id);
            let label = if ambiguous || m.name.is_empty() {
                m.reference()
            } else {
                m.name.clone()
            };
            SettingChoice::new(m.reference(), label)
        })
        .collect()
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
