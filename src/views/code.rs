//! Code tab views: the chat list (with lifecycle status), the new-session
//! form, the chat screen (cached-instant open, live streaming, PR), the
//! review screen, and the permission modal for code chats. Transcript items
//! render through the same `chat::render_item` the Home tab uses.

use std::collections::HashMap;

use dioxus::dioxus_core::spawn_forever;
use dioxus::document;
use dioxus::prelude::*;
use opencode_client::FileStatus;

use crate::code::{
    answer_code_permission, ask_label, delete_code_chat, ensure_code_models, expand_diff_gap,
    is_free_model, load_code_diff, mark_all_diff_seen, new_code_chat, open_chat_allows_free_models,
    open_code_chat, refresh_code_chats, refresh_code_permissions, request_pr, reveal_removed_lines,
    send_code_prompt, set_code_effort, set_code_model, start_code_poll, status_label,
    stop_code_turn, toggle_diff_file, toggle_diff_seen, CodeScreen, DiffFile, DiffState,
};
use crate::diff::Block;
use crate::icons::Icon;
use crate::state::{relative_time_secs, use_app_ctx, AppCtx, ConnState};
use crate::views::chat::{format_tokens, render_transcript};
use crate::views::session_settings::{
    choice_label, SessionSettingsSheet, SettingChoice, SettingRow,
};
use crate::views::{ConfirmDelete, MenuItem, OverflowButton, OverflowSheet, SwipeDelete};
use opencode_client::{ChatMeta, CodePermission, ModelInfo};

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
    let asks = (ctx.code_permissions)();

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
        }
        main {
            class: "scroll has-fab",
            // Named, so the pull-to-refresh listener knows this list has
            // something to fetch and which fetch it is.
            "data-refresh": "code",
            "data-refreshing": "{loading}",
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
                    {
                        chats.iter().map(|meta| {
                            let turn = running_chat.as_deref() == Some(meta.id.as_str())
                                && running_turn;
                            render_code_row(&ctx, meta, turn, chat_ask(&asks, &meta.id), confirm_delete)
                        })
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

        if let Some(chat_id) = confirm_delete() {
            ConfirmDelete {
                title: "Delete this session?",
                body: "The chat and its workspace both go — any work on the \
                       branch that has not been pushed goes with them.",
                on_cancel: move |()| confirm_delete.set(None),
                on_confirm: move |()| {
                    confirm_delete.set(None);
                    delete_code_chat(&ctx, chat_id.clone());
                },
            }
        }
    }
}

/// The ask a chat's card should carry, and how many of its own are behind it.
///
/// Front of the queue plus a count, the same shape the permission modal has
/// always used: a card is not the place to work through a backlog, and the
/// count is what says there is one.
fn chat_ask(queue: &[(String, CodePermission)], chat_id: &str) -> Option<(CodePermission, usize)> {
    let mut mine = queue.iter().filter(|(cid, _)| cid == chat_id);
    let front = mine.next()?.1.clone();
    Some((front, mine.count()))
}

/// One chat's row: what it is, what its container is doing, and — when it is
/// parked on a permission — the ask itself.
fn render_code_row(
    ctx: &AppCtx,
    meta: &ChatMeta,
    running_turn: bool,
    ask: Option<(CodePermission, usize)>,
    mut confirm_delete: Signal<Option<String>>,
) -> Element {
    let ctx = *ctx;
    let meta = meta.clone();
    let id = meta.id.clone();
    let waiting = ask.is_some();
    let (dot, label) = status_label(&meta, running_turn, waiting);

    rsx! {
        li {
            key: "{meta.id}",
            class: "session-item",
            onclick: {
                let meta = meta.clone();
                move |_| open_code_chat(&ctx, meta.clone())
            },
            div { class: "session-swipe",
                div {
                    // Rule 8: state is a dot. On the tile rather than in the
                    // panel below it so a scroll down the list answers "which
                    // one wants me" without reading a word.
                    class: if waiting { "session-tile attention" } else { "session-tile" },
                    Icon { name: "code" }
                }
                div { class: "session-main",
                    div { class: "session-head",
                        div { class: "session-title", "{meta.title}" }
                        span { class: "session-age", {relative_time_secs(meta.last_active)} }
                    }
                    div { class: "session-meta",
                        span { class: "chip",
                            span { class: "{dot}" }
                            "{label}"
                        }
                        span { "{meta.repo}" }
                        if !meta.branch.is_empty() {
                            span { "{meta.branch}" }
                        }
                    }
                    if let Some((perm, more)) = ask {
                        {render_ask_panel(&ctx, &id, &perm, more)}
                    }
                }
            }
            SwipeDelete {
                on_delete: move |()| confirm_delete.set(Some(id.clone()))
            }
        }
    }
}

/// The ask, inset in the card that is blocked on it.
///
/// Two answers, not three. "Always allow" changes what the agent may do
/// unattended from here on, and that is a decision to take with the
/// conversation in front of you — tapping the panel's text opens the chat,
/// where the modal offers it along with the ask's details. What is here is
/// the pair you can answer from a list without reading anything else.
fn render_ask_panel(ctx: &AppCtx, chat_id: &str, perm: &CodePermission, more: usize) -> Element {
    let ctx = *ctx;
    let label = ask_label(perm);
    // Each answer takes its own copy: the row re-renders on every poll, and a
    // handler that borrowed the queue's entry would outlive it.
    let answer = |response: &'static str| {
        let (chat_id, perm) = (chat_id.to_owned(), perm.clone());
        move |e: Event<MouseData>| {
            // The whole row opens the chat (design rule 9), so a control
            // inside it has to say it was the target — otherwise approving
            // also navigates.
            e.stop_propagation();
            answer_code_permission(&ctx, chat_id.clone(), perm.clone(), response);
        }
    };

    rsx! {
        div { class: "session-ask",
            p { class: "session-ask-title", "Approve or deny {label}" }
            div { class: "session-ask-actions",
                button { class: "btn small danger-outline", onclick: answer("reject"), "Deny" }
                button { class: "btn small primary", onclick: answer("once"), "Approve" }
            }
            if more > 0 {
                p { class: "session-ask-more", "+{more} more waiting" }
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
        let (count, verb) = if withheld == 1 {
            ("1 free model".to_owned(), "is")
        } else {
            (format!("{withheld} free models"), "are")
        };
        Some(format!(
            "{count} {verb} hidden — they train on their input, and this repo \
             is not a public throwaway."
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

/// Catalogue entries as choices. A name shared by two providers is shown as
/// `provider/model` instead, so two rows are never indistinguishable.
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

fn model_choices(offered: &[&ModelInfo]) -> Vec<SettingChoice> {
    offered
        .iter()
        .map(|m| {
            // Compare the whole reference, not just the id: `anthropic` and
            // the `opencode` zen proxy both offer `claude-sonnet-4-5` under
            // the same display name, and `o.id != m.id` is false for that
            // pair — so the one case this test exists for was the one it
            // called unambiguous, and the picker drew two identical rows for
            // two different providers.
            let ambiguous = offered
                .iter()
                .any(|o| o.name == m.name && (o.provider_id != m.provider_id || o.id != m.id));
            let label = if ambiguous || m.name.is_empty() {
                m.reference()
            } else {
                m.name.clone()
            };
            SettingChoice::new(m.reference(), label)
        })
        .collect()
}

/// The ask for the chat you have open, over whatever screen you are on.
///
/// Scoped to that one chat since the manager's aggregate started filling the
/// queue for every running container: a modal is an interruption, and being
/// interrupted about a conversation you are not in — possibly several, in a
/// row — is what the card in the list exists to avoid. Those are answered
/// there; this stays what it was.
#[component]
pub fn CodePermissionModal() -> Element {
    let ctx = use_app_ctx();
    let queue = (ctx.code_permissions)();
    // peek, not read: the modal is remounted when the queue changes, and
    // subscribing to the open chat would rebuild it on every streamed token.
    let open = ctx.code_chat.peek().chat_id.clone();
    let mut mine = queue
        .iter()
        .filter(|(cid, _)| Some(cid.as_str()) == open.as_deref());
    let Some((chat_id, perm)) = mine.next().cloned() else {
        return rsx! {};
    };
    let pending_more = mine.count();

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
    let title = ask_label(&perm);

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
    let mut draft = ctx.code_draft;

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
    let mut chat_confirm_delete = use_signal(|| false);
    let mut menu = use_signal(|| false);
    let chip_label = code_chip_label(chat.model.as_deref(), &models);
    // None until the fetch on chat open lands, and None for a session that has
    // changed nothing — the chip says "Diff" alone rather than "+0 −0", which
    // would be a claim it cannot back before the diff has been read.
    let diff_totals = {
        let d = ctx.code_diff.read();
        (!d.files.is_empty()).then(|| d.totals())
    };

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
                    spawn_forever(async move {
                        refresh_code_chats(&ctx).await;
                        // The rows and the asks are one statement about the
                        // list. Refreshing half of it is how a chat that had
                        // gone quiet came back showing a fresh timestamp and
                        // no sign that it was blocked.
                        refresh_code_permissions(&ctx).await;
                    });
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
            div { class: "topbar-actions",
                button {
                    class: "icon-btn",
                    title: "New session",
                    onclick: move |_| {
                        let mut screen = ctx.code_screen;
                        screen.set(CodeScreen::New);
                    },
                    Icon { name: "plus" }
                }
                OverflowButton { onopen: move |()| menu.set(true) }
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
            {render_transcript(&chat.items, &chat.marks)}
            if running {
                div { class: "typing",
                    span { class: "dot-anim" }
                    span { class: "dot-anim" }
                    span { class: "dot-anim" }
                }
            }
        }

        // Above the composer, not inside it. Diff and PR are about the work
        // the session has produced; the composer is about the next thing you
        // say to it. Putting them in the chip row also put them in a fixed
        // budget of horizontal space they had to share with the model name.
        div { class: "action-row",
            button {
                class: "action-chip",
                title: "Review the session's changes",
                onclick: move |_| load_code_diff(&ctx),
                Icon { name: "diff" }
                "Diff"
                if let Some((added, removed)) = diff_totals {
                    span { class: "stat add", "+{added}" }
                    span { class: "stat del", "−{removed}" }
                }
            }
            button {
                class: "action-chip",
                title: "Push branch + open a PR",
                disabled: !can_send,
                onclick: move |_| request_pr(&ctx),
                Icon { name: "pull-request" }
                "PR"
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
                    span { class: "chip-label", "{chip_label}" }
                    Icon { name: "chevron-down" }
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
            // Built here rather than beside `chip_label`: this component
            // re-renders on every keystroke and every streamed part, and the
            // row list is n^2 in the catalogue with a String per model.
            SessionSettingsSheet {
                backend: "code agent",
                rows: code_setting_rows(&ctx, &models, models_loading),
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

        if menu() {
            OverflowSheet {
                items: vec![MenuItem { icon: "trash", label: "Delete session", danger: true }],
                onpick: move |_| {
                    menu.set(false);
                    chat_confirm_delete.set(true);
                },
                onclose: move |()| menu.set(false),
            }
        }

        if chat_confirm_delete() {
            ConfirmDelete {
                title: "Delete this session?",
                body: "The chat and its workspace both go — any work on the \
                       branch that has not been pushed goes with them.",
                on_cancel: move |()| chat_confirm_delete.set(false),
                on_confirm: move |()| {
                    chat_confirm_delete.set(false);
                    let Some(id) = ctx.code_chat.peek().chat_id.clone() else {
                        return;
                    };
                    ctx.code_screen.clone().set(CodeScreen::List);
                    delete_code_chat(&ctx, id);
                },
            }
        }
    }
}

/// The review screen: the session's cumulative diff, one collapsible card
/// per file.
///
/// **Unified, not split.** At 402px the two columns of a split view get
/// `(368 − 2×18 − 8) ÷ 2 = 162px`, about 22 monospace columns each.
/// `pub(crate) fn load_code_diff(` is 30 characters, so every real line wraps
/// on both sides — and to *different* heights on each side, which stops the
/// rows lining up. Lining the before and after up on one visual row is the
/// entire value of split view, so at this width it is not merely cramped, it
/// is self-defeating.
#[component]
pub fn CodeDiffView() -> Element {
    let ctx = use_app_ctx();
    let diff = (ctx.code_diff)();
    let wrap = (ctx.code_diff_wrap)();

    let total = diff.files.len();
    let reviewed = diff.reviewed();
    let (added, removed) = diff.totals();
    let subtitle = if total == 0 {
        ctx.code_chat.read().title.clone()
    } else if total == 1 {
        format!("1 file · +{added} −{removed}")
    } else {
        format!("{total} files · +{added} −{removed}")
    };
    let percent = (reviewed * 100).checked_div(total).unwrap_or(0);
    let show_empty = !diff.loading && diff.files.is_empty() && diff.error.is_none();
    let cards = diff
        .files
        .iter()
        .map(|file| render_diff_file(&ctx, &diff, file, wrap));

    rsx! {
        header { class: "topbar",
            button {
                class: "icon-btn back",
                onclick: move |_| {
                    let mut screen = ctx.code_screen;
                    screen.set(CodeScreen::Chat);
                },
                Icon { name: "chevron-left" }
            }
            div { class: "titlegroup",
                h1 { class: "title ellipsis", "Review" }
                span { class: "subtitle ellipsis", "{subtitle}" }
            }
            div { class: "topbar-actions",
                button {
                    class: "icon-btn",
                    title: if wrap { "Scroll long lines instead of wrapping" } else { "Wrap long lines" },
                    "aria-pressed": "{wrap}",
                    onclick: move |_| {
                        let mut w = ctx.code_diff_wrap;
                        let next = !*w.peek();
                        w.set(next);
                    },
                    Icon { name: "wrap-text" }
                }
            }
        }

        main {
            class: "scroll diff",
            id: "code-diff-scroll",
            "data-refresh": "diff",
            "data-refreshing": "{diff.loading}",
            if let Some(error) = diff.error.as_ref() {
                p { class: "error-box", "{error}" }
            }
            if diff.loading && diff.files.is_empty() {
                p { class: "empty", "Reading the working tree — waking the container if it was asleep…" }
            }
            if show_empty {
                p { class: "empty", "Nothing has changed on this branch yet." }
            }
            if total > 1 {
                div { class: "diff-progress",
                    span { class: "diff-progress-label", "{reviewed} of {total} files reviewed" }
                    if reviewed < total {
                        button {
                            class: "btn small secondary",
                            onclick: move |_| mark_all_diff_seen(&ctx),
                            "Mark all"
                        }
                    }
                }
                div { class: "diff-progress-track",
                    div { class: "diff-progress-fill", style: "width: {percent}%" }
                }
            }
            {cards}
        }
    }
}

/// One file's card: a head you can scan, and a body that folds away
/// independently of every other file's.
fn render_diff_file(ctx: &AppCtx, state: &DiffState, file: &DiffFile, wrap: bool) -> Element {
    let ctx = *ctx;
    let path = file.info.file.clone();
    let (dir, name) = crate::diff::split_path(&file.info.file);
    let (dir, name) = (dir.to_owned(), name.to_owned());
    let fingerprint = file.fingerprint;
    let seen = state.is_seen(file);
    let open = state.is_open(file);
    let binary = file.info.is_binary();
    let deleted = file.info.status == FileStatus::Deleted;

    // Only for a card that is actually showing them. Re-hunking every file on
    // every render meant a session touching twenty files paid for twenty
    // parses to display one — and <details open=false> does not render its
    // children, so the work was thrown away.
    let rows = if open {
        Some(diff_rows(&ctx, state, file))
    } else {
        None
    };

    rsx! {
        details {
            key: "{path}",
            class: if seen { "diff-file seen" } else { "diff-file" },
            open,
            summary {
                class: "diff-file-head",
                // The native <summary> toggle is suppressed so `open` stays
                // the app's to decide: marking a file reviewed folds it, and
                // a DOM that had toggled itself would not hear about it.
                onclick: {
                    let path = path.clone();
                    move |e: Event<MouseData>| {
                        e.prevent_default();
                        toggle_diff_file(&ctx, &path);
                    }
                },
                div { class: "diff-file-id",
                    div { class: "diff-path",
                        if !dir.is_empty() {
                            span { class: "diff-dir", "{dir}" }
                        }
                        span { class: "diff-name", "{name}" }
                    }
                    div { class: "diff-stat",
                        if binary {
                            span { class: "diff-badge", "binary" }
                        } else {
                            if deleted {
                                span { class: "diff-badge", "deleted" }
                            }
                            if file.info.status == FileStatus::Added {
                                span { class: "diff-badge", "added" }
                            }
                            span { class: "add", "+{file.info.additions}" }
                            span { class: "del", "−{file.info.deletions}" }
                        }
                    }
                }
                // A trailing control inside a row-sized target: it stops the
                // click reaching the summary, the same way the list's trash
                // button does (design rule 9).
                button {
                    class: "diff-seen",
                    "aria-pressed": "{seen}",
                    title: if seen { "Reviewed — tap to unmark" } else { "Mark reviewed" },
                    onclick: move |e: Event<MouseData>| {
                        e.stop_propagation();
                        e.prevent_default();
                        toggle_diff_seen(&ctx, &path, fingerprint);
                    },
                    Icon { name: "check" }
                }
            }
            // Rendered only when open: a closed card costs nothing, which is
            // what keeps a twenty-file diff cheap.
            if open {
                div { class: if wrap { "diff-body" } else { "diff-body nowrap" },
                    {rows.into_iter().flatten()}
                }
            }
        }
    }
}

/// The contents of one file's body: rows, collapsed bands, and the notes
/// that stand in for a body there is no point rendering.
fn diff_rows(ctx: &AppCtx, state: &DiffState, file: &DiffFile) -> Vec<Element> {
    let ctx = *ctx;
    let path = file.info.file.clone();
    let view = state.view.get(&path);
    let no_expansions = HashMap::new();
    let expanded = view.map_or(&no_expansions, |v| &v.expanded);

    let mut rows: Vec<Element> = Vec::new();
    if file.info.is_binary() {
        rows.push(rsx! {
            p { key: "binary-{path}", class: "diff-note", "Binary file — not shown." }
        });
        return rows;
    }
    // Nobody reviews a deletion line by line, and its patch is one `-` row
    // per line of the file that used to be there.
    if file.info.status == FileStatus::Deleted && !view.is_some_and(|v| v.show_removed) {
        rows.push(rsx! {
            p { key: "deleted-{path}", class: "diff-note",
                "File deleted · {file.info.deletions} lines removed"
            }
        });
        rows.push(rsx! {
            button {
                key: "reveal-{path}",
                class: "diff-skip",
                onclick: {
                    let path = path.clone();
                    move |_| reveal_removed_lines(&ctx, &path)
                },
                span { class: "diff-skip-label", "Show removed lines" }
            }
        });
        return rows;
    }
    if file.lines.is_empty() {
        rows.push(rsx! {
            p { key: "empty-{path}", class: "diff-note", "No line changes — file metadata only." }
        });
        return rows;
    }

    let rendered = crate::diff::blocks(&file.lines, &file.gaps, expanded);
    for block in &rendered.blocks {
        match *block {
            Block::Rows { start, end } => {
                for (offset, line) in file.lines[start..end].iter().enumerate() {
                    let index = start + offset;
                    rows.push(rsx! {
                        div { key: "l{index}", class: "{line.row_class()}",
                            span { class: "diff-sign", "{line.sign()}" }
                            span { class: "diff-code", "{line.text}" }
                        }
                    });
                    if line.no_newline {
                        rows.push(rsx! {
                            p { key: "n{index}", class: "diff-note", "No newline at end of file" }
                        });
                    }
                }
            }
            Block::Gap { key, hidden, at } => rows.push(rsx! {
                button {
                    key: "g{key}",
                    class: "diff-skip",
                    onclick: {
                        let path = path.clone();
                        move |_| expand_diff_gap(&ctx, &path, key, hidden)
                    },
                    span { class: "diff-skip-label", "⋯ {hidden} unchanged lines" }
                    span { class: "diff-skip-at", "{at}" }
                }
            }),
        }
    }
    if rendered.dropped > 0 {
        rows.push(rsx! {
            p { key: "capped-{path}", class: "diff-note",
                if rendered.dropped_changes > 0 {
                    "{rendered.dropped} more lines, {rendered.dropped_changes} of them changes — too long to render in one screen."
                } else {
                    "{rendered.dropped} more unchanged lines — too long to render in one screen."
                }
            }
        });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::{chat_ask, code_chip_label, model_choices, CodePermission, ModelInfo};

    fn queued(entries: &[(&str, &str)]) -> Vec<(String, CodePermission)> {
        entries
            .iter()
            .map(|(chat, id)| {
                (
                    (*chat).to_owned(),
                    CodePermission {
                        id: (*id).to_owned(),
                        ..CodePermission::default()
                    },
                )
            })
            .collect()
    }

    /// A card shows the front of its own queue and counts the rest of its
    /// own — not the rest of the list's, which is what a global count would
    /// be now that every running chat's asks share one queue.
    #[test]
    fn a_card_counts_only_its_own_backlog() {
        let queue = queued(&[
            ("chat_a", "per_1"),
            ("chat_b", "per_2"),
            ("chat_a", "per_3"),
            ("chat_a", "per_4"),
        ]);
        assert_eq!(
            chat_ask(&queue, "chat_a").map(|(front, more)| (front.id, more)),
            Some(("per_1".to_owned(), 2)),
            "the front of this chat's queue, in arrival order, and its own count"
        );
        assert_eq!(chat_ask(&queue, "chat_b").map(|(_, n)| n), Some(0));
        assert!(chat_ask(&queue, "chat_c").is_none());
    }

    fn model(provider: &str, id: &str, name: &str) -> ModelInfo {
        ModelInfo {
            id: id.to_owned(),
            provider_id: provider.to_owned(),
            name: name.to_owned(),
            ..ModelInfo::default()
        }
    }

    /// The same display name from two providers is the case the label test
    /// exists for: one is the vendor direct, the other a proxy, and which one
    /// runs decides who sees the code.
    #[test]
    fn two_providers_offering_one_name_get_told_apart() {
        let a = model("anthropic", "claude-sonnet-4-5", "Claude Sonnet 4.5");
        let b = model("opencode", "claude-sonnet-4-5", "Claude Sonnet 4.5");
        let choices = model_choices(&[&a, &b]);
        let labels: Vec<&str> = choices.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(
            labels,
            ["anthropic/claude-sonnet-4-5", "opencode/claude-sonnet-4-5"],
            "identical names must fall back to the full reference"
        );
    }

    /// Two different models that merely share a name are also ambiguous.
    #[test]
    fn one_name_on_two_ids_is_ambiguous_too() {
        let a = model("opencode", "sonnet-4-5", "Claude Sonnet 4.5");
        let b = model("opencode", "sonnet-4-5-thinking", "Claude Sonnet 4.5");
        let labels: Vec<String> = model_choices(&[&a, &b])
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(
            labels,
            ["opencode/sonnet-4-5", "opencode/sonnet-4-5-thinking"]
        );
    }

    /// Distinct names stay readable — the fallback is for collisions only.
    #[test]
    fn distinct_names_are_left_alone() {
        let a = model("anthropic", "claude-opus-4-1", "Claude Opus 4.1");
        let b = model("opencode", "claude-sonnet-4-5", "Claude Sonnet 4.5");
        let labels: Vec<String> = model_choices(&[&a, &b])
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(labels, ["Claude Opus 4.1", "Claude Sonnet 4.5"]);
    }

    /// The chip names the model before the catalogue has loaded, and after a
    /// model the catalogue does not list.
    #[test]
    fn the_chip_falls_back_to_the_bare_id() {
        assert_eq!(code_chip_label(None, &[]), "Model");
        assert_eq!(
            code_chip_label(Some("opencode/deepseek-v4-flash"), &[]),
            "deepseek-v4-flash"
        );
        let known = model("opencode", "deepseek-v4-flash", "DeepSeek V4 Flash");
        assert_eq!(
            code_chip_label(Some("opencode/deepseek-v4-flash"), &[known]),
            "DeepSeek V4 Flash"
        );
    }
}
