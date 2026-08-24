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
    answer_code_permission, delete_code_chat, expand_diff_gap, load_code_diff, mark_all_diff_seen,
    new_code_chat, open_code_chat, refresh_code_chats, request_pr, reveal_removed_lines,
    send_code_prompt, start_code_poll, status_label, stop_code_turn, toggle_diff_file,
    toggle_diff_seen, CodeScreen, DiffFile, DiffState,
};
use crate::diff::Block;
use crate::icons::Icon;
use crate::state::{relative_time_secs, use_app_ctx, AppCtx, ConnState};
use crate::views::chat::render_transcript;

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
                    title: "Review the session's changes",
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
                button {
                    class: "icon-btn",
                    title: "Re-fetch the diff",
                    disabled: diff.loading,
                    onclick: move |_| load_code_diff(&ctx),
                    Icon { name: "refresh" }
                }
            }
        }

        main { class: "scroll diff", id: "code-diff-scroll",
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

    let rows = diff_rows(&ctx, state, file);

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
                    {rows.into_iter()}
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
                "{rendered.dropped} more lines — too long to render in one screen."
            }
        });
    }
    rows
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
