use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;

use goose_acp_client::ConfigOption;

use crate::attach::AttachTarget;
use crate::icons::Icon;
use crate::markdown;
use crate::state::{
    answer_permission, new_session, send_prompt, show_toast, stop_turn, use_app_ctx, AppCtx,
    ChatItem, Screen, Usage,
};
use crate::views::attach::{attachment_list, AttachButton, AttachTray};
use crate::views::session_settings::{
    chip_effort, choice_label, mode_icon, ChoicePickerSheet, SessionSettingsSheet, SettingChoice,
    SettingRow,
};
use crate::views::{
    ConfirmDelete, MenuItem, OverflowButton, OverflowSheet, RenameSheet, ScrollToBottom,
};

/// The transcript's scroller, named so the pin and the scroll-to-bottom
/// button address the same element.
const SCROLL_ID: &str = "chat-scroll";

/// What this conversation is called, once.
///
/// Read by two things that are never on screen together: the header below,
/// and — on the desktop — the window's own bar, which takes the heading out of
/// the pane and paints it in `.shell-chrome` instead
/// (`src/shell/desktop/mod.rs`, `assets/desktop.css`).
pub(crate) fn crumb(ctx: &crate::state::AppCtx) -> crate::nav::Crumb {
    crate::nav::Crumb::plain(ctx.chat.read().title.clone())
}

#[component]
pub fn ChatView() -> Element {
    let ctx = use_app_ctx();
    let chat = (ctx.chat)();
    let usage = (ctx.usage)();
    // Not a `use_signal`: the draft outlives this screen (see
    // `AppCtx::chat_draft`).
    let mut draft = ctx.chat_draft;

    // Keep the transcript pinned to the bottom as content streams in. The
    // chat signal is read INSIDE the effect so it re-runs on every change.
    use_effect(move || {
        let _ = ctx.chat.read().items.len();
        crate::viewport::pin_transcript(SCROLL_ID);
    });

    let running = chat.running;
    let can_send = !running && !chat.loading;
    // Which conversation the composer's picks belong to. Passed down rather
    // than read off the context inside the two components, so neither of them
    // subscribes to a signal that changes on every streamed token.
    let conversation = chat.session_id.clone().unwrap_or_default();

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
    // The effort rides on the chip after the model, so the setting is visible
    // without opening the sheet. Only when it is a choice: goose ships
    // `thinking_effort` as a lone `off` whenever the session's model cannot
    // reason, and "Claude Sonnet 5 Off" reads as something switched off rather
    // than as something the model never had. `is_adjustable` is already the
    // question "would choosing change anything", and this is the same one.
    let effort = config
        .iter()
        .find(|o| o.config_id == "thinking_effort")
        .filter(|o| o.is_adjustable())
        .and_then(|o| chip_effort(o.current_value.as_deref()));
    // Only a session that exists can be renamed, and a chat that has not been
    // opened yet has no title worth correcting either.
    let named = chat.session_id.is_some().then_some(chat.title.as_str());
    let rows = goose_setting_rows(&config, usage, named);
    // A goose that stops offering a mode simply has no mode chip: this is
    // `None` and everything below skips it, rather than the chip appearing
    // with nothing behind it.
    let mode = config.iter().find(|o| is_mode_chip(o)).cloned();
    let mut sheet = use_signal(|| false);
    let mut mode_sheet = use_signal(|| false);
    let mut confirm_delete = use_signal(|| false);
    let mut rename = use_signal(|| false);
    let mut menu = use_signal(|| false);

    let mut submit = move || {
        let text = draft.peek().trim().to_string();
        // A message can be attachments alone — a photo with nothing to say
        // about it is still a message.
        let files = ctx.attachments.peek().clone();
        if text.is_empty() && files.is_empty() {
            return;
        }
        // Cleared only once the message is on its way, so a send that never
        // starts — disconnected, no session — leaves the typed text and the
        // picked files where they were. A send that starts and then fails on
        // the wire is `send_prompt`'s to put right: it answers long after
        // this returns, and it hands the files back to the tray itself.
        if send_prompt(&ctx, text, &files) {
            draft.set(String::new());
            // Your own message always takes you back to the bottom, whatever
            // you had scrolled up to read. Without this the transcript stays
            // where it was and the message you just sent is off screen.
            crate::viewport::scroll_to_bottom(SCROLL_ID);
            ctx.attachments.clone().set(Vec::new());
        }
    };

    let heading = crumb(&ctx).title;

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
            // The same expression the window's bar reads, so the two names
            // cannot drift; the substituted value is byte-identical to the
            // `{chat.title}` it replaces, which is what keeps the phone's
            // captured markup unchanged.
            h1 { class: "title ellipsis", "{heading}" }
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

        main { class: "scroll chat", id: SCROLL_ID,
            if chat.loading {
                p { class: "empty", "Loading history…" }
            }
            {render_transcript(&chat.items, &chat.marks)}
            {render_lost_asks(&ctx, chat.session_id.as_deref())}
            if running {
                div { class: "typing",
                    span { class: "dot-anim" }
                    span { class: "dot-anim" }
                    span { class: "dot-anim" }
                }
            }
        }

        ScrollToBottom { scroller: SCROLL_ID }

        footer { class: "composer",
            AttachTray { target: AttachTarget::Goose, conversation: conversation.clone() }
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
                // This box is one line and never more (`.chip-row` in
                // main.css). The send button is outside it, which is what
                // keeps it pinned to the trailing edge whatever the chips
                // inside do — and what it used to be outside a *wrapping* box
                // for. The wrap is gone: a composer that grows a row under
                // your thumb is worse than a model name you can tap to read in
                // full.
                div { class: "chip-row",
                    AttachButton { target: AttachTarget::Goose, conversation }
                    if !rows.is_empty() {
                        button {
                            class: "composer-chip action model",
                            title: "Session settings",
                            onclick: move |_| sheet.set(true),
                            span { class: "chip-label",
                                span { class: "chip-model", "{chip_label}" }
                                if let Some(effort) = effort {
                                    span { class: "chip-effort", "{effort}" }
                                }
                            }
                            Icon { name: "chevron-down" }
                        }
                    }
                    if let Some(mode) = mode.as_ref() {
                        button {
                            class: "composer-chip action mode",
                            title: "Mode",
                            onclick: move |_| mode_sheet.set(true),
                            Icon {
                                name: mode_icon(mode.current_value.as_deref().unwrap_or_default()),
                            }
                            // The one place in this app a chip can end up
                            // naming its own control, and it is a fallback
                            // rather than a state the app can produce: goose
                            // ships `currentValue` inside the `configOptions`
                            // of `session/new`, so `current_label` answers on
                            // every build that has one. Hiding the chip
                            // instead would hide the picker with it — the mode
                            // is filtered out of the settings sheet
                            // (`goose_setting_rows`) precisely because this
                            // chip is where it lives.
                            span { class: "chip-label",
                                {mode.current_label().unwrap_or("Mode")}
                            }
                        }
                    }
                    if let Some(percent) = crowding(usage) {
                        span { class: "composer-chip warn", title: "Context used",
                            "{percent}%"
                        }
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
                onaction: move |_| {
                    // The only action row this sheet has. It swaps one sheet
                    // for another rather than nesting them: the rename field
                    // is a screen's worth of keyboard, and the settings sheet
                    // underneath it would be a backdrop nobody can reach.
                    sheet.set(false);
                    rename.set(true);
                },
                onclose: move |()| sheet.set(false),
            }
        }

        if rename() {
            RenameSheet {
                heading: "Rename chat",
                value: chat.title.clone(),
                on_cancel: move |()| rename.set(false),
                on_save: move |title: String| {
                    rename.set(false);
                    let Some(session_id) = ctx.chat.peek().session_id.clone() else {
                        return;
                    };
                    spawn_forever(async move {
                        crate::state::rename_session(&ctx, &session_id, &title).await;
                    });
                },
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

/// The goose tab's rows: the session's name, every option the agent offers
/// apart from mode, and the one fact about it that is worth stating and
/// cannot be changed.
///
/// The title leads because it is the only row that is about *this* session
/// rather than about how the agent will answer in it — and because goose named
/// it from the first message, which is a guess made before the conversation
/// happened. The place you notice the guess was wrong is here, reading it.
///
/// Context length is that fact. `session/set_config_option` routes exactly
/// four ids — provider, mode, model, `thinking_effort` — and rejects anything
/// else; a context window reaches the client only as read-only information
/// about a model. The number is already flowing in on every `usage_update`,
/// so the sheet reports it rather than pretending to a control — and reports
/// it as unknown before the first turn has produced one, because a row that
/// appears partway through a conversation reads as the app having found
/// something rather than as the agent having said it.
fn goose_setting_rows(
    config: &[ConfigOption],
    usage: Option<Usage>,
    title: Option<&str>,
) -> Vec<SettingRow> {
    // A session whose config has not arrived, that has not run a turn and
    // that has no name yet has nothing to report. Returning no rows is what
    // keeps the chip off the composer entirely, rather than offering a sheet
    // holding one dash.
    if config.is_empty() && usage.is_none() && title.is_none() {
        return Vec::new();
    }
    let mut rows: Vec<SettingRow> = title
        .map(|title| SettingRow::action("title", "Title", title))
        .into_iter()
        .collect();
    rows.extend(config.iter().filter(|o| !is_mode_chip(o)).map(|option| {
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
    }));
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

/// The tail of the transcript: asks this chat lost, and what that cost.
///
/// Rendered FROM THE JOURNAL, not pushed into `chat.items`, and that is not a
/// stylistic choice. `reload_chat` clears `items` and rebuilds them from
/// `session/load`, so a stored transcript item would be wiped by the very
/// reconnect that reveals the loss. Derived and appended after the items loop,
/// it is also immune to `ChatState::marks` and `CodeChatState::part_index`,
/// both of which hold indices into that vector.
///
/// It says what was measured rather than what was believed. There is no
/// declined tool in the transcript and no "you declined this" note — the round
/// was discarded whole (`docs/permission-durability.md` section 0) — so the
/// card points at the prompt, which survived, and at asking again, which is
/// the only thing the reader can actually do.
fn render_lost_asks(ctx: &AppCtx, session_id: Option<&str>) -> Element {
    let Some(session_id) = session_id else {
        return rsx! {};
    };
    // `losses_in` hands back owned pairs, which is what lets the read guard
    // end on this line: holding it across the rsx! below would put a read
    // borrow of the journal underneath a button whose handler writes it.
    let lost = crate::ask_journal::losses_in(&ctx.lost_asks.read(), session_id);
    let ctx = *ctx;
    rsx! {
        for (id, sentence) in lost {
            div { key: "{id}", class: "error-box warn",
                div { class: "lost-ask",
                    "{sentence}"
                    button {
                        class: "btn small secondary",
                        onclick: move |_| crate::state::dismiss_lost_ask(&ctx, &id),
                        "Dismiss"
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
        ChatItem::User { text, attachments } => {
            let html = markdown::escape_text(text);
            rsx! {
                div { key: "{index}", class: "bubble user",
                    if !attachments.is_empty() {
                        {attachment_list(attachments)}
                    }
                    // An empty text node still draws a line box, which is a
                    // blank strip under a photo sent with nothing to say.
                    if !text.is_empty() {
                        div { class: "bubble-text", dangerous_inner_html: "{html}" }
                    }
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
/// The context readout, but only once it is worth the room it costs.
///
/// It used to be there always. Four chips do not fit a 360pt composer — a
/// model name, its effort tier, a mode and this came to 306px of a 306px row,
/// and the model name was rendered in under six of them. Something had to
/// leave, and this is the one whose absence costs least: the window is stated
/// in full in the settings sheet, and "12% used" is not a fact anyone acts on.
/// Near the end of the window it becomes the most useful thing in the row, so
/// that is when it appears — which is also what the reference app does, as a
/// notice rather than a permanent readout.
///
/// Returns `None` below the threshold, so the chip does not render at all
/// rather than rendering empty.
fn crowding(usage: Option<Usage>) -> Option<u128> {
    const SPEAK_UP_AT: u128 = 75;
    let (used, limit) = usage?;
    if limit == 0 {
        return None;
    }
    let percent = (u128::from(used) * 100 / u128::from(limit)).min(100);
    (percent >= SPEAK_UP_AT).then_some(percent)
}

/// A token count, as short as it can be said without losing anything.
///
/// The decimal earns its place under ten thousand, where a tenth of a
/// thousand is a tenth of what is on screen. Above that it is noise that
/// costs room: "128.0k/200.0k" measured 106px of the 306px the goose
/// composer's chip row has at 360pt, which is what left the model name with
/// nothing to give when the effort tier arrived beside it. "128k/200k" says
/// the same thing in 86.
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
    } else if n >= 10_000 {
        format!("{:.0}k", tokens / 1_000.0)
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
    use super::{
        crowding, format_tokens, goose_setting_rows, is_mode_chip, mode_choices, ConfigOption,
    };
    use goose_acp_client::ConfigChoice;

    /// The readout costs a chip in a row that has none to spare, so it only
    /// appears when it is the most useful thing there — and a turn that
    /// overran its own window reads as full rather than as 103%.
    #[test]
    fn the_context_chip_speaks_up_only_near_the_end_of_the_window() {
        assert_eq!(crowding(None), None);
        assert_eq!(crowding(Some((128_000, 200_000))), None); // 64%, not yet
        assert_eq!(crowding(Some((150_000, 200_000))), Some(75));
        assert_eq!(crowding(Some((190_000, 200_000))), Some(95));
        // Overran, and clamped rather than reading 103%.
        assert_eq!(crowding(Some((206_000, 200_000))), Some(100));
        // A server that has not said what the window is says nothing here.
        assert_eq!(crowding(Some((1, 0))), None);
        // No overflow on a window the size of a whole model's training set.
        assert_eq!(crowding(Some((u64::MAX, u64::MAX))), Some(100));
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
        goose_setting_rows(config, None, None)
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
        let rows = goose_setting_rows(&config, None, None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].value, "—");
        assert_eq!(
            goose_setting_rows(&config, Some((1_000, 200_000)), None)[1].value,
            "200k tokens"
        );
    }

    /// Nothing to report is nothing to open: with no config, no turn yet and
    /// no session to name, the sheet has no rows and the composer has no chip.
    #[test]
    fn a_session_with_nothing_to_report_has_no_rows() {
        assert!(goose_setting_rows(&[], None, None).is_empty());
        assert_eq!(
            goose_setting_rows(&[], Some((1_000, 200_000)), None).len(),
            1
        );
    }

    /// A name is something to report. The title leads, and it is the one row
    /// that hands itself back to the caller instead of drilling into values.
    #[test]
    fn a_named_session_leads_with_the_row_that_renames_it() {
        let config = [option("model", Some("model"), &["opus", "sonnet"])];
        let rows = goose_setting_rows(&config, None, Some("Deploy the thing"));
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["title", "model", "context_length"]
        );
        assert_eq!(rows[0].value, "Deploy the thing");
        assert!(rows[0].action);
        // A chat that has not been opened has no title, and no row for one.
        assert!(!row_ids(&config).contains(&"title".to_owned()));
        // The name alone is enough to open the sheet on a session that has
        // told the app nothing else yet.
        assert_eq!(
            goose_setting_rows(&[], None, Some("Standup")).len(),
            2,
            "the title and the unknown context length"
        );
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

    /// A count keeps its decimal only where the decimal is a real difference.
    /// Above ten thousand it is a hundred tokens, and it is spent on the one
    /// row in the app that has no width to spare.
    #[test]
    fn a_token_count_carries_a_decimal_only_while_it_says_something() {
        assert_eq!(format_tokens(0), "0");
        assert_eq!(format_tokens(842), "842");
        assert_eq!(format_tokens(1_200), "1.2k");
        assert_eq!(format_tokens(9_400), "9.4k");
        assert_eq!(format_tokens(10_000), "10k");
        assert_eq!(format_tokens(128_400), "128k");
        assert_eq!(format_tokens(200_000), "200k");
        assert_eq!(format_tokens(1_500_000), "1.5M");
    }
}
