use dioxus::dioxus_core::spawn_forever;
use dioxus::document;
use dioxus::prelude::*;

use goose_acp_client::ConfigOption;

use crate::icons::Icon;
use crate::markdown;
use crate::state::{
    answer_permission, new_session, send_prompt, show_toast, stop_turn, use_app_ctx, ChatItem,
    Screen, Usage,
};
use crate::views::session_settings::{
    choice_label, mode_icon, ChoicePickerSheet, SessionSettingsSheet, SettingChoice, SettingRow,
};
use crate::views::{ConfirmDelete, MenuItem, OverflowButton, OverflowSheet};

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

    // Whatever the agent says it has, in the order it says it: provider,
    // mode, model and thinking effort today. Reading the list rather than
    // naming ids means a fifth option upstream appears without an app
    // change, and one goose stops sending disappears honestly.
    let config = (ctx.config_options)();
    let chip_label = config
        .iter()
        .find(|o| o.config_id == "model")
        .and_then(ConfigOption::current_label)
        .map_or_else(|| "Session".to_owned(), str::to_owned);
    let rows = goose_setting_rows(&config, usage);
    // A goose that stops offering a mode simply has no mode chip: this is
    // `None` and everything below skips it, rather than the chip appearing
    // with nothing behind it.
    let mode = config.iter().find(|o| is_mode_chip(o)).cloned();
    let mut sheet = use_signal(|| false);
    let mut mode_sheet = use_signal(|| false);
    let mut confirm_delete = use_signal(|| false);
    let mut menu = use_signal(|| false);

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
            div { class: "topbar-actions",
                button {
                    class: "icon-btn",
                    title: "New chat",
                    onclick: move |_| new_session(&ctx),
                    Icon { name: "plus" }
                }
                OverflowButton { onopen: move |()| menu.set(true) }
            }
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
                if !rows.is_empty() {
                    button {
                        class: "composer-chip action model",
                        title: "Session settings",
                        onclick: move |_| sheet.set(true),
                        span { class: "chip-label", "{chip_label}" }
                        Icon { name: "chevron-down" }
                    }
                }
                if let Some(mode) = mode.as_ref() {
                    button {
                        class: "composer-chip action mode",
                        title: "Mode",
                        onclick: move |_| mode_sheet.set(true),
                        Icon { name: mode_icon(mode.current_value.as_deref().unwrap_or_default()) }
                        span { class: "chip-label",
                            {mode.current_label().unwrap_or("Mode")}
                        }
                    }
                }
                if let Some((used, limit)) = usage {
                    span { class: "composer-chip", title: "Context used",
                        "{context_percent(used, limit)}"
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

        if sheet() {
            SessionSettingsSheet {
                backend: "goose",
                rows,
                onchoose: move |(config_id, value): (String, String)| {
                    crate::state::set_config_option(&ctx, &config_id, &value);
                },
                onclose: move |()| sheet.set(false),
            }
        }

        if let Some(mode) = mode.filter(|_| mode_sheet()) {
            ChoicePickerSheet {
                title: "Select mode",
                backend: "goose",
                choices: mode_choices(&mode),
                current: mode.current_value.clone(),
                // Unreachable while the chip only renders for an adjustable
                // option, and stated anyway: an empty picker with nothing in
                // it and nothing to say is the one outcome a reader cannot
                // act on.
                empty: "This agent offers no other mode.",
                onchoose: {
                    let config_id = mode.config_id.clone();
                    move |value: String| {
                        crate::state::set_config_option(&ctx, &config_id, &value);
                        mode_sheet.set(false);
                    }
                },
                onclose: move |()| mode_sheet.set(false),
            }
        }

        if menu() {
            OverflowSheet {
                items: vec![MenuItem { icon: "trash", label: "Delete chat", danger: true }],
                onpick: move |_| {
                    menu.set(false);
                    confirm_delete.set(true);
                },
                onclose: move |()| menu.set(false),
            }
        }

        if confirm_delete() {
            ConfirmDelete {
                title: "Delete this chat?",
                body: "The whole conversation goes from the goose server. \
                       This cannot be undone.",
                on_cancel: move |()| confirm_delete.set(false),
                on_confirm: move |()| {
                    confirm_delete.set(false);
                    let Some(session_id) = ctx.chat.peek().session_id.clone() else {
                        return;
                    };
                    spawn_forever(async move {
                        let Some(client) = ctx.client.peek().clone() else { return };
                        match client.session_delete(&session_id).await {
                            Ok(()) => {
                                let mut sessions = ctx.sessions;
                                sessions.write().retain(|s| s.session_id != session_id);
                                ctx.screen.clone().set(Screen::Sessions);
                            }
                            Err(e) => show_toast(&ctx, format!("Delete failed: {e}")),
                        }
                    });
                },
            }
        }
    }
}

/// The mode selector, when picking between its values would change anything.
///
/// This is the one place the goose sheet stops being purely data-driven, and
/// it is deliberate: mode is a chip in the composer row with a picker of its
/// own, so it is taken out of the list rather than rendered twice. ACP gives
/// an agent two ways to say which option that is — the `mode` category, which
/// the spec defines for exactly this sort of placement decision, and the
/// `mode` id goose has always used — and either will do, so an agent that
/// sends only one of them is still understood.
///
/// An option with a single value stays in the sheet as a fact. A chip that
/// opens a one-row picker is a control that does nothing (design rule 11),
/// and the setting still exists, so it is reported rather than hidden.
fn is_mode_chip(option: &ConfigOption) -> bool {
    (option.category.as_deref() == Some("mode") || option.config_id == "mode")
        && option.is_adjustable()
}

/// The mode picker's rows, in the order the agent sent them.
///
/// Each carries the agent's own description — goose writes one per mode, and
/// they are what tells "Auto" from "Manual approval" without having to try
/// both — and an icon derived from the value, because neither ACP nor goose
/// has a field for one.
fn mode_choices(option: &ConfigOption) -> Vec<SettingChoice> {
    option
        .options
        .iter()
        .map(|c| {
            SettingChoice::new(&c.value, choice_label(&c.name, &c.value))
                .with_note(c.description.clone())
                .with_icon(mode_icon(&c.value))
        })
        .collect()
}

/// The goose tab's rows: every option the agent offers apart from mode, plus
/// the one fact about the session that is worth stating and cannot be
/// changed.
///
/// Context length is that fact. `session/set_config_option` routes exactly
/// four ids — provider, mode, model, `thinking_effort` — and rejects anything
/// else; a context window reaches the client only as read-only information
/// about a model. The number is already flowing in on every `usage_update`,
/// so the sheet reports it rather than pretending to a control — and reports
/// it as unknown before the first turn has produced one, because a row that
/// appears partway through a conversation reads as the app having found
/// something rather than as the agent having said it.
fn goose_setting_rows(config: &[ConfigOption], usage: Option<Usage>) -> Vec<SettingRow> {
    // A session whose config has not arrived and that has not run a turn has
    // nothing to report. Returning no rows is what keeps the chip off the
    // composer entirely, rather than offering a sheet holding one dash.
    if config.is_empty() && usage.is_none() {
        return Vec::new();
    }
    let mut rows: Vec<SettingRow> = config
        .iter()
        .filter(|option| !is_mode_chip(option))
        .map(|option| {
            SettingRow::select(
                &option.config_id,
                &option.name,
                option.current_value.as_deref(),
                option
                    .options
                    .iter()
                    .map(|c| {
                        SettingChoice::new(&c.value, choice_label(&c.name, &c.value))
                            .with_note(c.description.clone())
                    })
                    .collect(),
                option.description.clone(),
            )
        })
        .collect();
    rows.push(match usage {
        Some((_, limit)) => SettingRow::fact(
            "context_length",
            "Context length",
            format!("{} tokens", format_tokens(limit)),
            "Fixed by the model. Nothing a message carries changes it.",
        ),
        None => SettingRow::fact(
            "context_length",
            "Context length",
            "—",
            "The agent reports this with the first turn of the session.",
        ),
    });
    rows
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

/// How full the context window is, as the chip states it.
///
/// It used to read `128.0k/200.0k`, which is 106px of a 306px composer row —
/// and with a mode chip in that row as well, a phone at 360pt had 48px left
/// for two chip labels, so even `GPT-5.2` and `Auto` came out ellipsised.
/// The two numbers were never the question anyway: "how much room is left" is,
/// and one percentage answers it in a third of the width. The window itself is
/// still stated in full, as the Context length row of the settings sheet.
///
/// A limit of zero is a server that has not said, and there is no fraction of
/// nothing; a count above it is clamped, because a turn that overran its own
/// window should read as full rather than as 103%. Widened to `u128` first,
/// so scaling by 100 cannot wrap and cannot saturate — saturating would turn
/// a nearly-full window into "1%", which is the opposite of the truth.
fn context_percent(used: u64, limit: u64) -> String {
    if limit == 0 {
        return "—".to_owned();
    }
    let percent = u128::from(used) * 100 / u128::from(limit);
    format!("{}%", percent.min(100))
}

pub(crate) fn format_tokens(n: u64) -> String {
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

#[cfg(test)]
mod tests {
    use super::{context_percent, goose_setting_rows, is_mode_chip, mode_choices, ConfigOption};
    use goose_acp_client::ConfigChoice;

    /// The chip reports a fraction of a window, so it needs a window; and a
    /// turn that overran one should read as full rather than as 103%.
    #[test]
    fn context_percent_needs_a_window_and_stops_at_full() {
        assert_eq!(context_percent(128_000, 200_000), "64%");
        assert_eq!(context_percent(0, 200_000), "0%");
        assert_eq!(context_percent(206_000, 200_000), "100%");
        assert_eq!(context_percent(1, 0), "—");
        // No overflow on a window the size of a whole model's training set.
        assert_eq!(context_percent(u64::MAX, u64::MAX), "100%");
    }

    fn choice(value: &str, name: &str, description: Option<&str>) -> ConfigChoice {
        ConfigChoice {
            value: value.to_owned(),
            name: name.to_owned(),
            description: description.map(str::to_owned),
        }
    }

    fn option(config_id: &str, category: Option<&str>, values: &[&str]) -> ConfigOption {
        ConfigOption {
            config_id: config_id.to_owned(),
            name: config_id.to_owned(),
            description: None,
            category: category.map(str::to_owned),
            kind: Some("select".to_owned()),
            current_value: values.first().map(|v| (*v).to_owned()),
            options: values.iter().map(|v| choice(v, v, None)).collect(),
        }
    }

    fn row_ids(config: &[ConfigOption]) -> Vec<String> {
        goose_setting_rows(config, None)
            .into_iter()
            .map(|r| r.id)
            .collect()
    }

    /// Mode has a chip and a picker of its own, so the sheet must not carry
    /// it as well — and everything else keeps the agent's own order.
    #[test]
    fn mode_leaves_the_sheet_it_has_a_chip_in() {
        let config = [
            option("provider", None, &["anthropic", "openai"]),
            option("mode", Some("mode"), &["auto", "approve"]),
            option("model", Some("model"), &["opus", "sonnet"]),
            option("thinking_effort", Some("thought_level"), &["off", "high"]),
        ];
        assert_eq!(
            row_ids(&config),
            ["provider", "model", "thinking_effort", "context_length"]
        );
    }

    /// The agent may say which option is the mode with the spec's category
    /// rather than the id goose happens to use, and either has to be enough.
    #[test]
    fn the_mode_is_found_by_category_as_well_as_by_id() {
        assert!(is_mode_chip(&option(
            "session_mode",
            Some("mode"),
            &["auto", "approve"]
        )));
        assert!(is_mode_chip(&option("mode", None, &["auto", "approve"])));
        assert!(!is_mode_chip(&option("model", Some("model"), &["a", "b"])));
    }

    /// One value is not a choice, so it earns no chip — and it stays in the
    /// sheet as a fact rather than leaving the app altogether.
    #[test]
    fn a_mode_with_one_value_stays_in_the_sheet() {
        let config = [option("mode", Some("mode"), &["auto"])];
        assert!(!is_mode_chip(&config[0]));
        assert_eq!(row_ids(&config), ["mode", "context_length"]);
    }

    /// A goose that sends no mode at all has nothing to put on a chip, and
    /// nothing else about the sheet changes.
    #[test]
    fn a_goose_with_no_mode_option_has_nothing_to_pick() {
        let config = [option("model", Some("model"), &["opus", "sonnet"])];
        assert!(!config.iter().any(is_mode_chip));
        assert_eq!(row_ids(&config), ["model", "context_length"]);
    }

    /// Context length is a row before the first turn as well as after it: one
    /// that appears partway through a conversation reads as the app having
    /// found something rather than the agent having said it.
    #[test]
    fn context_length_is_stated_before_the_agent_has_reported_one() {
        let config = [option("model", Some("model"), &["opus", "sonnet"])];
        let rows = goose_setting_rows(&config, None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].value, "—");
        assert_eq!(
            goose_setting_rows(&config, Some((1_000, 200_000)))[1].value,
            "200.0k tokens"
        );
    }

    /// Nothing to report is nothing to open: with no config and no turn yet,
    /// the sheet has no rows and the composer has no chip.
    #[test]
    fn a_session_with_nothing_to_report_has_no_rows() {
        assert!(goose_setting_rows(&[], None).is_empty());
        assert_eq!(goose_setting_rows(&[], Some((1_000, 200_000))).len(), 1);
    }

    /// The picker's rows carry the agent's own words, and a mark each.
    #[test]
    fn mode_choices_keep_the_agents_descriptions() {
        let mut mode = option("mode", Some("mode"), &["auto", "approve"]);
        mode.options = vec![
            choice("auto", "Auto", Some("Run tools without asking.")),
            choice("approve", "Manual approval", None),
        ];
        let choices = mode_choices(&mode);
        assert_eq!(choices[0].label, "Auto");
        assert_eq!(
            choices[0].note.as_deref(),
            Some("Run tools without asking.")
        );
        assert_eq!(choices[0].icon.as_deref(), Some("bolt"));
        assert_eq!(choices[1].label, "Manual approval");
        assert_eq!(choices[1].note, None);
        assert_eq!(choices[1].icon.as_deref(), Some("shield-check"));
    }
}
