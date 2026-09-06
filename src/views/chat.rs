use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;

use goose_acp_client::{ConfigOption, ToolCallContent};

use crate::attach::AttachTarget;
use crate::diff::{Block, DiffLine, LineKind};
use crate::icons::Icon;
use crate::markdown;
use crate::state::{
    answer_permission, new_session, send_prompt, show_toast, stop_turn, use_app_ctx, AppCtx,
    ChatItem, Screen, Usage,
};
use crate::views::attach::{attachment_list, AttachButton, AttachTray};
use crate::views::session_settings::{
    chip_effort, mode_icon, option_choices, ChoicePickerSheet, SessionSettingsSheet, SettingChoice,
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
/// (`src/shell/desktop/mod.rs`, `assets/desktop/`).
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
                // shared.css). The send button is outside it, which is what
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
/// The label and the note come from [`option_choices`], which every screen
/// that renders a goose option shares; the icon is added here, because it is
/// the one thing about a mode that is not true of every option.
fn mode_choices(option: &ConfigOption) -> Vec<SettingChoice> {
    option_choices(option)
        .into_iter()
        .map(|choice| {
            let icon = mode_icon(&choice.value);
            choice.with_icon(icon)
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
///
/// **Confirmed rather than assumed, at #205.** The number is the server's own
/// — it is the `limit` off `usage_update` — so the standing rule that nothing
/// may display a number no server sends is met by reporting it, and would be
/// broken by hiding it. Making it a control is not an app change at all: it
/// starts upstream with a fifth id on `session/set_config_option`, and until
/// that exists a chevron here could only open a list whose every entry the
/// agent answers `-32602` to. The row stays a fact.
///
/// What that decision leaves behind is [`GOOSE_CONTEXT_ID`], which is the
/// price of the sheet being open to an option this app has never heard of.
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
            option_choices(option),
            option.description.clone(),
        )
    }));
    // THE AGENT'S OWN ROW WINS, and this guard is why the sheet can stay open
    // to an id nobody here has heard of.
    //
    // Every option the agent sends is rendered above, by id, whatever it is —
    // that is the design, and a fifth option upstream reaches the sheet with no
    // app change. `context_length` is the one id where that meets a row this
    // file appends on its own, and appending it anyway would put TWO rows
    // keyed `context_length` in the list: `render_row` builds its Dioxus key
    // from `row.id`, so it would be a duplicate key as well as a duplicate row,
    // and Dioxus reuses nodes by key.
    //
    // Skipping ours is the right way round rather than merely the easy one.
    // goose sending this id at all would mean it had grown the route the
    // paragraph above says does not exist, and the agent's version carries its
    // own choices and its own description — a real control, where ours is a
    // fact whose whole content is "you cannot change this here".
    if !config.iter().any(|o| o.config_id == GOOSE_CONTEXT_ID) {
        rows.push(match usage {
            Some((_, limit)) => SettingRow::fact(
                GOOSE_CONTEXT_ID,
                "Context length",
                format!("{} tokens", format_tokens(limit)),
                "Fixed by the model. Nothing a message carries changes it.",
            ),
            None => SettingRow::fact(
                GOOSE_CONTEXT_ID,
                "Context length",
                "—",
                "The agent reports this with the first turn of the session.",
            ),
        });
    }
    rows
}

/// The id of the context-length row, named because two places now have to
/// agree about it: the row this file appends, and the guard that stands down
/// when goose sends an option of its own under the same name.
const GOOSE_CONTEXT_ID: &str = "context_length";

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
/// WHO IS SPEAKING, and it is desktop-only by a const rather than a `cfg`.
///
/// The mockups head every turn with an attribution row — `YOU`, and `goose` in
/// the accent — and the phone has never had one, because on a phone the user's
/// turn is a right-aligned bubble and the alignment IS the attribution. The
/// desktop de-inverts that bubble into a left-aligned card (see
/// `assets/desktop/`), so alignment stops saying anything and something has
/// to.
///
/// `Shell::CURRENT` is a `const`, so the phone binary compiles this branch out
/// and its markup is byte-identical — which is the promise this whole
/// restructure is under. It is also why this is not `::before` content, which
/// was the first design: `docs/audit.js`'s contrast walk keys on real text
/// nodes (`childNodes` of type 3), so a name painted by a pseudo-element is a
/// name no gate can measure. `src/views/` keeps its zero `cfg(target_os)`.
const fn attributed() -> bool {
    matches!(crate::shell::Shell::CURRENT, crate::shell::Shell::Desktop)
}

/// Which side of the conversation an item belongs to.
const fn speaker(item: &ChatItem) -> &'static str {
    match item {
        ChatItem::User { .. } => "you",
        _ => "goose",
    }
}

pub(crate) fn render_transcript(items: &[ChatItem], marks: &[(usize, i64)]) -> Element {
    let mut out: Vec<Element> = Vec::new();
    let mut said: Option<&'static str> = None;
    let mut i = 0;
    while i < items.len() {
        // At every CHANGE of speaker, not at every item: a turn is a run of
        // items from one side, and heading each of six tool cards with the
        // same word would be six times the noise for none of the information.
        if attributed() {
            let who = speaker(&items[i]);
            if said != Some(who) {
                said = Some(who);
                out.push(rsx! {
                    div { key: "who-{i}", class: "who-line",
                        span { class: if who == "you" { "who-name" } else { "who-name goose" },
                            "{who}"
                        }
                    }
                });
            }
        }
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
        [one] => tool_kind_phrase(one, n).map_or_else(
            || format!("Used {n} tools"),
            |phrase| format!("Used {n} tools · {phrase}"),
        ),
        _ => format!("Used {n} tools"),
    }
}

/// What a run of `n` tools of one kind DID, or `None` when the kind names no
/// tool.
///
/// `None` IS AN ANSWER, the same way it is in [`tool_kind_word`], and the
/// caller already knows what to do with it: "Used 2 tools" is the sentence a
/// mixed run gets, and a run whose one kind is not a name is in exactly that
/// position — it has a count and nothing else that is true.
///
/// THE KIND THAT IS NOT A NAME IS `other`, and it arrives by two routes.
/// `src/state.rs` manufactures it — `call.kind.clone().unwrap_or_else(|| "other"
/// .to_string())` — when the wire carries no kind at all, which is what a real
/// goose does for an `extensionmanager` call; and base ACP's own `ToolKind`
/// enum has `other` as its catch-all, so a server can send the string too. Both
/// routes mean "none of the above". Neither is a tool, and the fallback arm
/// below printed whichever one arrived as though it were: **"Used 2 tools ·
/// other x2"**, which is the app quoting its own placeholder back at the
/// reader in a line every other kind answers in the app's voice ("read 3
/// files", "ran 2 searches"). The empty string is the same claim written a
/// third way — a transcript cached by an older build could hold one — and gets
/// the same answer.
///
/// UNREACHABLE FROM THE WHOLE LOCAL STACK, which is why a screenshot was the
/// first sighting: `crates/mock-goose-server` sends `"kind":"execute"` on every
/// call it emits and `src/code.rs` hard-codes the same, so nothing here can
/// produce a kind-less tool call. The tests below are the only thing that can.
///
/// A KIND THIS APP HAS NEVER HEARD OF IS STILL REPORTED, because a goose that
/// grows a tool should not make the fold lie about it — but it is reported as a
/// word and not as a wire enum. `switch_mode` is a real `ToolKind` with no
/// phrase of its own, and `switch_mode x2` reads as debug output for the reason
/// [`tool_status_label`] gives about `IN_PROGRESS`; underscores come out there
/// and they come out here.
fn tool_kind_phrase(kind: &str, n: usize) -> Option<String> {
    let plural = if n == 1 { "" } else { "s" };
    Some(match kind {
        "execute" => format!("ran {n} command{plural}"),
        "read" => format!("read {n} file{plural}"),
        "edit" => format!("edited {n} file{plural}"),
        "search" => format!("ran {n} search{}", if n == 1 { "" } else { "es" }),
        "fetch" => format!("fetched {n} URL{plural}"),
        "other" | "" => return None,
        other => format!("{} x{n}", other.replace('_', " ")),
    })
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
            contents,
            ..
        } => {
            let icon = tool_icon(kind);
            // Desktop-only, the same way the speaker attribution above is — so
            // the phone's tool card is byte-identical to what it was.
            let word = tool_kind_word(crate::shell::Shell::CURRENT, kind);
            let edits = tool_edits(contents, title);
            let counts = edit_counts(&edits);
            // THE FLAT `<pre>` GOES AWAY EXACTLY WHEN THE CARD HAS ALREADY
            // DRAWN EVERYTHING, and not one case earlier. `output` for an edit
            // is `content_text`'s `[diff: src/scheduler.rs]` followed by the
            // NEW side — the same bytes the slab above it is now showing in
            // colour, and showing them twice is the transcript arguing with
            // itself. Three conditions, each of which is a way for the card to
            // be an incomplete account of the result: no edit was drawn, some
            // content entry was not a diff (a text block, a terminal), or a
            // row budget cut one of them short. In all three the disclosure is
            // the fallback and stays.
            let drawn_it_all = !edits.is_empty()
                && edits.len() == contents.len()
                && edits.iter().all(|edit| edit.cut == 0);
            let has_output = !output.is_empty() && !drawn_it_all;
            rsx! {
                div { key: "{index}", class: "tool status-{status}",
                    div { class: "tool-head",
                        // One leading mark, never two and never none. The word
                        // when there is one, the phone's glyph when there is
                        // not — see `tool_kind_word`.
                        if let Some(word) = word {
                            span { class: "tool-kind", "{word}" }
                        } else {
                            span { class: "tool-icon", Icon { name: "{icon}" } }
                        }
                        span { class: "tool-title", "{title}" }
                        // `.diff-stat` and its two children are the Diff
                        // screen's own head counts, unchanged: `flex-shrink:
                        // 0` so a number never ellipsises (`+12…` is a number
                        // that lies), `--text-success` / `--text-danger` at
                        // weight 600, and the same U+2212 minus the review
                        // screen sets. It sits between the title and the
                        // status because the title is the only shrinkable item
                        // in this row and both of the others are counts of
                        // what the tool did.
                        if let Some((add, del)) = counts {
                            span { class: "diff-stat",
                                span { class: "add", "+{add}" }
                                span { class: "del", "\u{2212}{del}" }
                            }
                        }
                        span { class: "tool-status", "{tool_status_label(status)}" }
                    }
                    if !edits.is_empty() {
                        div { class: "diff-body", {edit_rows(&edits).into_iter()} }
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

/// The most source rows one edit card draws.
///
/// The Diff screen's own ceiling is [`crate::diff::RENDER_CAP`] at 800, and it
/// is a ceiling for a different object: that screen IS the diff, and 800 rows
/// there is a long scroll on a page whose only job is to be scrolled. This is a
/// card in a conversation, between the thing that asked for the edit and the
/// thing the agent said afterwards, and a 200-row card is a transcript nobody
/// can get past. 60 clears an ordinary edit whole — the collapse below has
/// already taken the unchanged runs out — and what it cuts, it says it cut.
const CARD_ROWS: usize = 60;

/// One file edit out of a tool call's structured contents, laid out.
struct Edit {
    /// The caption over the slab, when the card owes the reader one.
    note: Option<String>,
    /// Every row of the edit, in file order.
    lines: Vec<DiffLine>,
    /// The runs to draw and the bands that stand in for the rest, after
    /// [`CARD_ROWS`].
    blocks: Vec<Block>,
    /// Rows neither drawn nor stood in for by a band.
    cut: usize,
}

/// The file edits in a tool call's result, as rows to draw.
///
/// THE HEAD IS THE CARD'S HEAD AND THERE IS NOT A SECOND ONE. The mockups draw
/// an edit with a header carrying the path; this card already has a header
/// carrying the path, because `ChatItem::Tool.title` for a real goose edit is
/// `edit: src/scheduler.rs`. Repeating it would print the path twice on every
/// edit in the transcript. So the caption is conditional and the condition is
/// stated rather than guessed: it appears when the title does NOT already
/// contain the path, when one call edited more than one file, and always for
/// the two claims a title never makes — a file that did not exist before and a
/// file that does not exist after.
///
/// THOSE TWO ARE WHY [`goose_acp_client::FileDiff`] KEEPS ITS `Option`s. Its
/// own comment says it: `old_text: None` is a file that did not exist, which is
/// a different claim from `Some("")` — a file that existed and was empty — and
/// a renderer that wants to say "new file" has to be able to tell them apart.
/// This is that renderer, and the match below is the first thing in the
/// workspace that reads the distinction.
fn tool_edits(contents: &[ToolCallContent], title: &str) -> Vec<Edit> {
    let files: Vec<_> = contents
        .iter()
        .filter_map(|entry| match entry {
            ToolCallContent::Diff(file) => Some(file),
            _ => None,
        })
        .collect();
    let expanded = std::collections::HashMap::new();
    files
        .iter()
        .filter_map(|file| {
            // Neither half means nothing to draw at all — `{"type":"diff"}`
            // with only a path, which `content_text` renders as `[diff: file]`
            // and which the disclosure is still the right home for.
            let lines = crate::diff::between(
                file.old_text.as_deref().unwrap_or_default(),
                file.new_text.as_deref().unwrap_or_default(),
            );
            if lines.is_empty() {
                return None;
            }
            let path = file.display_path();
            let note = match (&file.old_text, &file.new_text) {
                (None, Some(_)) => Some(format!("New file {path}")),
                (Some(_), None) => Some(format!("Deleted {path}")),
                _ if files.len() > 1 || !title.contains(path) => Some(format!("Edited {path}")),
                _ => None,
            };

            // The Diff screen's collapse, with no expansion state: a
            // transcript card has no band to tap, so every gap stays shut and
            // says how much it is holding.
            let rendered = crate::diff::blocks(&lines, &crate::diff::gaps(&lines), &expanded);
            let mut blocks = Vec::new();
            let mut budget = CARD_ROWS;
            let mut covered = 0_usize;
            for block in rendered.blocks {
                if budget == 0 {
                    break;
                }
                match block {
                    Block::Rows { start, end } => {
                        let take = (end - start).min(budget);
                        blocks.push(Block::Rows {
                            start,
                            end: start + take,
                        });
                        budget -= take;
                        covered += take;
                    }
                    Block::Gap { hidden, .. } => {
                        blocks.push(block);
                        covered += hidden;
                    }
                }
            }
            let cut = lines.len() - covered;
            Some(Edit {
                note,
                lines,
                blocks,
                cut,
            })
        })
        .collect()
}

/// `(additions, deletions)` over every edit in one tool call, or `None` when
/// the call edited nothing.
///
/// Counted off the rows that were BUILT and not the rows that are DRAWN, which
/// is the only way the head can be true: [`CARD_ROWS`] cuts a long edit short
/// and a count taken after that cut would under-report the change it is
/// summarising. The line that says something was cut is the card's, not the
/// head's.
fn edit_counts(edits: &[Edit]) -> Option<(usize, usize)> {
    if edits.is_empty() {
        return None;
    }
    let count = |kind: LineKind| {
        edits
            .iter()
            .flat_map(|edit| edit.lines.iter())
            .filter(|line| line.kind == kind)
            .count()
    };
    Some((count(LineKind::Add), count(LineKind::Del)))
}

/// Every row of every edit on one card, in one slab.
///
/// ONE `.diff-body` FOR THE WHOLE CALL rather than one per file. Two slabs
/// abutting inside a card with no rule between them read as one slab with a
/// seam, and the caption each edit already carries is a better separator than
/// a border this file cannot add — `assets/shared.css` is the design system
/// both shells link and is not edited for a desktop want.
fn edit_rows(edits: &[Edit]) -> Vec<Element> {
    let mut rows: Vec<Element> = Vec::new();
    for (e, edit) in edits.iter().enumerate() {
        if let Some(note) = edit.note.clone() {
            rows.push(rsx! {
                p { key: "c{e}", class: "diff-note", "{note}" }
            });
        }
        for block in &edit.blocks {
            match *block {
                Block::Rows { start, end } => {
                    for (offset, line) in edit.lines[start..end].iter().enumerate() {
                        let index = start + offset;
                        rows.push(rsx! {
                            div { key: "l{e}-{index}", class: "{line.row_class()}",
                                span { class: "diff-sign", "{line.sign()}" }
                                span { class: "diff-code", "{line.text}" }
                            }
                        });
                    }
                }
                // A `<p>` and not the Diff screen's `<button>`: there is
                // nothing here to press. `.diff-skip` is a control that reveals
                // what it hides, and a control that does nothing is worse than
                // the sentence it would have been.
                // And the band's `at` — the new-side line number it resumes
                // at, which that screen right-aligns in `.diff-skip-at` — is
                // dropped rather than run into the sentence. It is an anchor
                // for a reader working through a file, and nobody works through
                // a file inside a chat bubble; set as prose it read
                // `27 unchanged lines · 1`, which is a number with nothing
                // saying what it counts.
                Block::Gap { key, hidden, .. } => rows.push(rsx! {
                    p { key: "g{e}-{key}", class: "diff-note", "⋯ {hidden} unchanged lines" }
                }),
            }
        }
        if edit.cut > 0 {
            rows.push(rsx! {
                p { key: "x{e}", class: "diff-note",
                    "⋯ {edit.cut} more lines — too long to render in a transcript card."
                }
            });
        }
    }
    rows
}

/// Human wording for a tool's status.
///
/// The two backends speak different vocabularies — ACP emits
/// `pending`/`in_progress`/`completed`/`failed`, `OpenCode` emits
/// `pending`/`running`/`completed`/`error` — and neither is UI copy. The
/// class name still carries the raw value, so the colour rules keep working
/// on both. Anything unrecognised is tidied rather than dropped: a backend
/// that grows a new state shows that state instead of nothing.
pub(crate) fn tool_status_label(status: &str) -> String {
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

/// THE KIND, AS A WORD — the desktop's leading token on a tool row, and
/// desktop-only for the same reason the speaker attribution above it is.
///
/// The mockups head every tool row with a fixed-width accent name — `read`,
/// `edit`, `shell` — and it is what makes a column of tool calls scannable:
/// a card reading "src/scheduler.rs · Done" does not say whether the file was
/// read or rewritten. The phone has a glyph in that slot and the desktop had
/// neither, because `assets/desktop/95-transcript.css` hid the glyph and the
/// rule meant to replace it read an attribute nothing emits. That sheet has
/// the rest of the story, including why the catalogued fix (`data-kind` on the
/// card, `content: attr()` on the head) could not have worked.
///
/// `None` IS A REAL ANSWER and not a fallback. `ChatItem::Tool.kind` is the
/// ACP `ToolCallUpdate.kind` or `OpenCode`'s equivalent, and both can send a
/// kind this app has no word for — `other`, `switch_mode`, the empty string
/// when the field is absent. Naming one of those in an accent column is worse
/// than the glyph: it is chrome saying nothing in the loudest colour on the
/// page, at whatever width the server chose. So the closed set gets a word and
/// everything else keeps the glyph [`tool_icon`] already picks, and the row
/// carries exactly one leading mark either way.
///
/// `shell` and not `execute`: the mockups' vocabulary for the same thing, and
/// the same translation [`tool_status_label`] already does for `in_progress`.
/// Backend enum names are not UI copy. The class still carries the raw status,
/// so nothing that keys on the wire's own words is affected.
///
/// THE SHELL ARRIVES AS A PARAMETER, which is `src/shell/mod.rs`'s own rule
/// for anything keyed off it: "the host runs the desktop arm, so every rule
/// keyed off the shell has to take it as a PARAMETER rather than read
/// `Shell::CURRENT` — otherwise the mobile assertions silently assert about
/// desktop and pass". [`attributed`] above predates that rule and is a `const`
/// the compiler folds; this is the newer shape, and it is the only way the
/// phone's arm of this decision is checkable at all from a `cargo test` that
/// builds the desktop one.
fn tool_kind_word(shell: crate::shell::Shell, kind: &str) -> Option<&'static str> {
    if matches!(shell, crate::shell::Shell::Mobile) {
        return None;
    }
    match kind {
        "execute" => Some("shell"),
        "read" => Some("read"),
        "edit" => Some("edit"),
        "delete" => Some("delete"),
        "move" => Some("move"),
        "search" => Some("search"),
        "fetch" => Some("fetch"),
        "think" => Some("think"),
        _ => None,
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
pub(crate) fn crowding(usage: Option<Usage>) -> Option<u128> {
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
        clock_label, crowding, format_tokens, goose_setting_rows, is_mode_chip, mode_choices,
        permission_button_class, permission_label, tool_icon, tool_kind_phrase, tool_kind_word,
        tool_run_summary, tool_status_label, ConfigOption,
    };
    use crate::ask_journal::{AskRecord, AskState, LostCause};
    use crate::shell::Shell;
    use crate::state::{ChatItem, ChatState};
    use crate::testkit::{render, render_seeded};
    use dioxus::prelude::*;
    use goose_acp_client::{
        ConfigChoice, FileDiff, PermissionOption, PermissionRequest, SessionInfo, ToolCallContent,
        ToolCallUpdate,
    };

    /// `n` numbered lines, as a file.
    ///
    /// Built with `push_str` rather than `map(format!).collect()` because
    /// clippy's `format_collect` is on: one `format!` allocation per line to
    /// build one string is the thing that lint exists to catch.
    fn numbered(word: &str, n: usize) -> String {
        let mut out = String::new();
        for i in 0..n {
            out.push_str(word);
            out.push(' ');
            out.push_str(&i.to_string());
            out.push('\n');
        }
        out
    }

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

    /// AND IT IS STATED ONCE. A goose that grows a `context_length` option of
    /// its own must not produce two rows under one id.
    ///
    /// The sheet renders every option the agent sends, by id, so a fifth one
    /// upstream arrives with no app change — and this is the id where that
    /// meets a row this file appends on its own. `render_row` keys a row by
    /// `row.id`, so the collision is a duplicate Dioxus key as well as a
    /// duplicate row, and Dioxus reuses nodes by key. The agent's version
    /// wins: it carries choices and a description, where ours is a fact whose
    /// content is that there is nothing to choose.
    ///
    /// Nothing in goose sends this today (#205: `session/set_config_option`
    /// routes four ids and rejects the rest), which is exactly why the hazard
    /// needs a test rather than a demonstration.
    #[test]
    fn a_context_length_the_agent_sends_replaces_the_one_this_sheet_appends() {
        let config = [
            option("model", Some("model"), &["opus", "sonnet"]),
            option("context_length", None, &["200k", "1M"]),
        ];
        assert_eq!(row_ids(&config), ["model", "context_length"]);
        // Not merely deduped — it is the AGENT's row that survived, with the
        // choices this app could not have invented.
        let rows = goose_setting_rows(&config, Some((1_000, 200_000)), None);
        assert_eq!(rows[1].value, "200k");
        assert_eq!(
            rows[1]
                .choices
                .iter()
                .map(|c| c.value.as_str())
                .collect::<Vec<_>>(),
            ["200k", "1M"],
            "the agent's own control was replaced by the app's fact, so an id \
             goose grew a route for reads as unchangeable"
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

    // ---------------------------------------------------------------- pure

    /// A folded run says what it folded away. The count alone ("Used 4
    /// tools") is the line the reader was already looking at four cards of;
    /// what makes the fold safe to leave closed is that it names the KIND, so
    /// a run of reads and a run of shell commands are not the same sentence.
    /// A mixed run honestly declines to name one.
    #[test]
    fn a_folded_run_names_what_the_tools_actually_did() {
        assert_eq!(
            tool_run_summary(&[
                tool("a", "Read main.rs", "read", "completed"),
                tool("b", "Read state.rs", "read", "completed"),
                tool("c", "Read chat.rs", "read", "completed"),
            ]),
            "Used 3 tools · read 3 files"
        );
        assert_eq!(
            tool_run_summary(&[
                tool("a", "Read main.rs", "read", "completed"),
                tool("b", "cargo test", "execute", "completed"),
            ]),
            "Used 2 tools",
            "two different kinds cannot be described as one, so the summary \
             must fall back to the count rather than name whichever kind \
             happened to sort first"
        );
        // Singulars, because "read 1 files" is the kind of thing that makes a
        // reader distrust everything else on the screen.
        assert_eq!(
            tool_run_summary(&[tool("a", "cargo test", "execute", "completed")]),
            "Used 1 tools · ran 1 command"
        );
        assert_eq!(
            tool_run_summary(&[tool("a", "grep", "search", "completed")]),
            "Used 1 tools · ran 1 search"
        );
        assert_eq!(
            tool_run_summary(&[
                tool("a", "grep", "search", "completed"),
                tool("b", "grep", "search", "completed"),
            ]),
            "Used 2 tools · ran 2 searches",
            "\"searchs\" is the plural this one gets wrong if the rule is a \
             bare 's'"
        );
        assert_eq!(
            tool_run_summary(&[
                tool("a", "edit", "edit", "completed"),
                tool("b", "edit", "edit", "completed"),
            ]),
            "Used 2 tools · edited 2 files"
        );
        assert_eq!(
            tool_run_summary(&[
                tool("a", "fetch", "fetch", "completed"),
                tool("b", "fetch", "fetch", "completed"),
            ]),
            "Used 2 tools · fetched 2 URLs"
        );
        // A kind this app has never heard of is still reported, because a
        // goose that grows a tool should not make the fold lie about it.
        assert_eq!(
            tool_run_summary(&[
                tool("a", "?", "screenshot", "completed"),
                tool("b", "?", "screenshot", "completed"),
            ]),
            "Used 2 tools · screenshot x2"
        );
    }

    /// THE APP'S OWN PLACEHOLDER IS NOT A TOOL NAME, and the fold used to
    /// print it as one: "Used 2 tools · other x2", seen against a real goose
    /// server and reproducible from nothing in this repo.
    ///
    /// `other` reaches here two ways and both mean "none of the above" —
    /// `src/state.rs` substitutes it when the wire carries no kind (which is
    /// what goose does for an `extensionmanager` call), and base ACP's own
    /// `ToolKind` has it as the catch-all, so a server can send the word. The
    /// summary line every other kind answers in the app's voice was answering
    /// this one by quoting the app back at itself.
    ///
    /// What it says instead is the sentence a MIXED run already gets, which is
    /// the same claim: there were two tool calls and nothing true to add.
    #[test]
    fn a_run_the_app_has_no_name_for_is_a_count_and_not_a_placeholder() {
        for nameless in ["other", ""] {
            assert_eq!(
                tool_run_summary(&[
                    tool("a", "extensionmanager", nameless, "completed"),
                    tool("b", "extensionmanager", nameless, "completed"),
                ]),
                "Used 2 tools",
                "the fold printed {nameless:?} as though it were a tool name, \
                 which is the app's own placeholder read back to the reader in \
                 the one line that is supposed to say what the tools did"
            );
        }
        assert_eq!(
            tool_kind_phrase("other", 2),
            None,
            "a kind that names no tool has no phrase, and the caller — not \
             this function — decides what a run with no phrase reads as"
        );
        // And the wire's underscored enum names are not UI copy either, the
        // same way `IN_PROGRESS` is not: `switch_mode` is a real ToolKind.
        assert_eq!(
            tool_run_summary(&[
                tool("a", "?", "switch_mode", "completed"),
                tool("b", "?", "switch_mode", "completed"),
            ]),
            "Used 2 tools · switch mode x2"
        );
    }

    /// The mark exists to say "there was a gap here", and it is UTC because
    /// there is no timezone database on the device. What it must never do is
    /// render an impossible clock: a mark past midnight, or one from before
    /// the epoch on a device with a wrong clock, still has to read as a time.
    #[test]
    fn a_time_mark_reads_as_a_clock_at_every_hour_of_the_day() {
        assert_eq!(clock_label(0), "00:00");
        assert_eq!(clock_label(3_661), "01:01");
        assert_eq!(clock_label(86_399), "23:59");
        // The next day, not "24:00".
        assert_eq!(clock_label(86_400), "00:00");
        assert_eq!(clock_label(1_800_000_000), "08:00");
        // A clock set before 1970 wraps rather than printing "-1:-1".
        assert_eq!(clock_label(-60), "23:59");
    }

    /// The two backends spell the same four states differently, and neither
    /// vocabulary is UI copy. If the mapping were dropped the transcript
    /// would show `in_progress` to the reader; if a state were missed it
    /// would show nothing at all, which is worse — a tool card with no status
    /// reads as one that never ran.
    #[test]
    fn both_backends_tool_states_are_said_in_the_same_english() {
        assert_eq!(tool_status_label("pending"), "Queued");
        assert_eq!(tool_status_label("in_progress"), "Running");
        assert_eq!(tool_status_label("running"), "Running");
        assert_eq!(tool_status_label("completed"), "Done");
        assert_eq!(tool_status_label("failed"), "Failed");
        assert_eq!(tool_status_label("error"), "Failed");
        // Unrecognised is tidied, not dropped.
        assert_eq!(tool_status_label("awaiting_input"), "Awaiting input");
        assert_eq!(
            tool_status_label(""),
            "",
            "an empty status must not panic on the first-character uppercase"
        );
    }

    /// Every tool kind either has a glyph that says what it did or falls back
    /// to the wrench. A missing arm would leave a wrench beside "Read
    /// main.rs", which is the whole reason these stopped being emoji.
    #[test]
    fn a_tool_card_gets_a_glyph_for_what_it_did() {
        assert_eq!(tool_icon("execute"), "terminal");
        assert_eq!(tool_icon("read"), "file");
        assert_eq!(tool_icon("edit"), "pencil");
        assert_eq!(tool_icon("delete"), "trash");
        assert_eq!(tool_icon("move"), "package");
        assert_eq!(tool_icon("search"), "search");
        assert_eq!(tool_icon("fetch"), "globe");
        assert_eq!(tool_icon("think"), "think");
        assert_eq!(tool_icon("screenshot"), "wrench");
        assert_eq!(tool_icon(""), "wrench");
    }

    /// The word beside the glyph's slot, and the two halves have to agree:
    /// every kind [`tool_icon`] knows a glyph for is a kind this names, and
    /// the fallback is `None` rather than a word — because `None` is what puts
    /// the glyph back.
    ///
    /// The one that would rot silently is the `execute`/`shell` pair. It is
    /// the only entry whose word is not its key, so an arm added later by
    /// copying its neighbour would give the mockups' shell row the backend's
    /// enum name and nothing would say so.
    #[test]
    fn a_tool_kind_has_a_word_only_when_the_word_says_something() {
        let desk = |kind| tool_kind_word(Shell::Desktop, kind);
        assert_eq!(desk("execute"), Some("shell"));
        assert_eq!(desk("read"), Some("read"));
        assert_eq!(desk("edit"), Some("edit"));
        assert_eq!(desk("delete"), Some("delete"));
        assert_eq!(desk("move"), Some("move"));
        assert_eq!(desk("search"), Some("search"));
        assert_eq!(desk("fetch"), Some("fetch"));
        assert_eq!(desk("think"), Some("think"));
        // A kind this app has no word for keeps the wrench rather than
        // shouting the wire's own vocabulary in the accent colour.
        assert_eq!(desk("screenshot"), None);
        assert_eq!(desk("switch_mode"), None);
        assert_eq!(desk("other"), None);
        assert_eq!(desk(""), None);

        // THE PHONE'S ARM, which is the branch a `cargo test` on this host can
        // reach no other way — `Shell::CURRENT` is `Desktop` here, and
        // `render_item` renders whichever mark this returns. `None` for every
        // kind is the whole statement that the phone's tool card did not
        // change: it keeps the glyph, in the slot it has always been in.
        for kind in [
            "execute",
            "read",
            "edit",
            "delete",
            "move",
            "search",
            "fetch",
            "think",
            "screenshot",
            "",
        ] {
            assert_eq!(
                tool_kind_word(Shell::Mobile, kind),
                None,
                "the phone's `{kind}` card gained the desktop's kind word, so \
                 a shared view has stopped rendering the same markup on the \
                 phone that it always did"
            );
        }

        // Both functions are total over the same set, which is the invariant
        // that keeps a row from carrying two leading marks or none.
        for kind in [
            "execute", "read", "edit", "delete", "move", "search", "fetch", "think",
        ] {
            assert_ne!(
                tool_icon(kind),
                "wrench",
                "`{kind}` has a word but falls through to the wrench, so a \
                 desktop row and a phone row disagree about what it did"
            );
        }
    }

    /// The measured regression this rule exists for: a real ACP server sent
    /// "always allow" FIRST, and a rule keyed off list position painted the
    /// broadest possible grant as the solid default under the reader's thumb.
    /// So the treatment is decided by what the option IS, and this pins that
    /// — swap the two entries below and nothing here may change.
    #[test]
    fn the_narrowest_allow_is_the_default_whatever_order_the_server_sent() {
        assert_eq!(
            permission_button_class(None, Some("allow_always"), "0"),
            "btn secondary",
            "a blanket grant arriving first must stay the quiet option"
        );
        assert_eq!(
            permission_button_class(None, Some("allow_once"), "1"),
            "btn primary",
            "the one-shot grant is the one your thumb should land on"
        );
        assert_eq!(
            permission_button_class(None, Some("reject_once"), "2"),
            "btn danger-outline"
        );
        assert_eq!(
            permission_button_class(None, Some("reject_always"), "3"),
            "btn danger-outline"
        );
        // `kind` when there is no name, and the raw id when there is neither
        // — a server that sends only an id still gets a treatment that
        // matches the meaning of that id.
        assert_eq!(
            permission_button_class(Some("allow_once"), None, "opt-7"),
            "btn primary"
        );
        assert_eq!(
            permission_button_class(None, None, "allow_always"),
            "btn secondary"
        );
        assert_eq!(
            permission_button_class(None, None, "cancel"),
            "btn danger-outline"
        );
        // Name wins over kind: the two disagree on a real server only when
        // the name is the more specific of the pair.
        assert_eq!(
            permission_button_class(Some("allow_always"), Some("allow_once"), "x"),
            "btn primary"
        );
        // Anything not an allow is treated as the destructive answer, which
        // is the safe direction for an option this app has never seen.
        assert_eq!(
            permission_button_class(None, Some("defer"), "x"),
            "btn danger-outline"
        );
    }

    /// A permission button is the one control in the app pressed under time
    /// pressure, so it may never read as a wire identifier. The four goose
    /// sends get real copy; anything else is at least de-underscored rather
    /// than shown raw.
    #[test]
    fn a_permission_button_never_reads_as_a_wire_identifier() {
        assert_eq!(permission_label(Some("allow_once"), "0"), "Allow once");
        assert_eq!(permission_label(Some("allow_always"), "0"), "Always allow");
        assert_eq!(permission_label(Some("reject_once"), "0"), "Reject");
        assert_eq!(
            permission_label(Some("reject_always"), "0"),
            "Always reject"
        );
        assert_eq!(
            permission_label(Some("allow_for_repo"), "0"),
            "allow for repo"
        );
        // No name at all: the id is what there is, and it is still tidied.
        assert_eq!(permission_label(None, "reject_once"), "Reject");
        assert_eq!(permission_label(None, "grant_scope"), "grant scope");
    }

    // ------------------------------------------------------- mounted views

    fn tool(id: &str, title: &str, kind: &str, status: &str) -> ChatItem {
        ChatItem::Tool {
            id: id.to_owned(),
            title: title.to_owned(),
            kind: kind.to_owned(),
            status: status.to_owned(),
            output: String::new(),
            contents: Vec::new(),
        }
    }

    fn chat() -> fn() -> Element {
        || rsx! { super::ChatView {} }
    }

    fn modal() -> fn() -> Element {
        || rsx! { super::PermissionModal {} }
    }

    pub(super) fn model_option() -> ConfigOption {
        ConfigOption {
            config_id: "model".to_owned(),
            name: "Model".to_owned(),
            description: None,
            category: Some("model".to_owned()),
            kind: Some("select".to_owned()),
            current_value: Some("sonnet".to_owned()),
            options: vec![
                choice("sonnet", "Claude Sonnet 5", None),
                choice("opus", "Claude Opus 5", None),
            ],
        }
    }

    pub(super) fn mode_option() -> ConfigOption {
        ConfigOption {
            config_id: "mode".to_owned(),
            name: "Mode".to_owned(),
            description: None,
            category: Some("mode".to_owned()),
            kind: Some("select".to_owned()),
            current_value: Some("auto".to_owned()),
            options: vec![
                choice("auto", "Auto", Some("Run tools without asking.")),
                choice("approve", "Manual approval", None),
            ],
        }
    }

    /// `values` short of two makes the option a fact rather than a control,
    /// which is the branch the effort chip is filtered on.
    fn effort_option(values: &[&str]) -> ConfigOption {
        let mut effort = option("thinking_effort", Some("thought_level"), values);
        effort.current_value = values.last().map(|v| (*v).to_owned());
        effort
    }

    /// The pane's own header and — on a desktop build — the window bar both
    /// name the open chat, and `crumb` is the single expression they share
    /// (`src/shell/desktop/mod.rs` reads it; `assets/desktop/` takes the
    /// heading back out of the pane). If they stopped agreeing the window
    /// would title itself one thing while the pane said another, with nothing
    /// on screen to say which was stale. The subtitle half matters too: the
    /// bar renders one when a crumb carries one, and a chat has no "where it
    /// lives" to offer, so a subtitle here would be a second line of chrome
    /// with nothing in it.
    #[test]
    fn the_window_bar_and_the_pane_take_the_chats_name_from_one_expression() {
        fn probe() -> Element {
            let ctx = crate::state::use_app_ctx();
            let crumb = super::crumb(&ctx);
            let subtitled = crumb.subtitle.is_some();
            rsx! {
                p { class: "probe-title", "{crumb.title}" }
                p { class: "probe-sub", "{subtitled}" }
            }
        }
        let seed: fn(&crate::state::AppCtx) = |ctx| {
            let mut chat = ctx.chat;
            chat.set(ChatState {
                session_id: Some("s-1".to_owned()),
                title: "Rotate the tailnet certificate".to_owned(),
                ..ChatState::default()
            });
        };
        let crumbed = render_seeded(seed, probe);
        assert!(
            crumbed.contains("<p class=\"probe-title\">Rotate the tailnet certificate</p>"),
            "the crumb the window bar reads is not the chat's title: {crumbed}"
        );
        assert!(
            crumbed.contains("<p class=\"probe-sub\">false</p>"),
            "a chat crumb grew a subtitle, which the window bar would paint \
             as a second line about nothing: {crumbed}"
        );

        let html = render_seeded(seed, chat());
        assert!(
            html.contains("<h1 class=\"title ellipsis\">Rotate the tailnet certificate</h1>"),
            "the pane's own header is not the crumb's title, so the window \
             and the pane have started naming the chat differently: {}",
            &html[..html.len().min(400)]
        );
    }

    /// History arrives over the wire, and a chat that shows an empty
    /// transcript while it is still coming reads as a chat that lost it. The
    /// send button is disabled for the same window, because a prompt sent
    /// into a session whose transcript is half-loaded lands in the middle of
    /// it.
    #[test]
    fn a_chat_still_fetching_its_history_says_so_and_refuses_to_send() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    loading: true,
                    ..ChatState::default()
                });
            },
            chat(),
        );
        assert!(
            html.contains("<p class=\"empty\">Loading history…</p>"),
            "a chat fetching its history shows nothing about it, so an empty \
             transcript is indistinguishable from a lost one: {html}"
        );
        assert!(
            html.contains("<button class=\"send\" title=\"Send\" disabled"),
            "the send button is live while the history is still arriving: {}",
            &html[html.find("composer-row").unwrap_or(0)..]
        );

        let idle = render(chat());
        assert!(
            !idle.contains("Loading history…"),
            "an idle chat claims to be loading"
        );
        assert!(
            idle.contains("<button class=\"send\" title=\"Send\">"),
            "an idle chat cannot be sent to at all: {idle}"
        );
    }

    /// While the agent is answering there is nothing to send and everything
    /// to stop. Losing the swap would leave a Send button that queues a
    /// second turn into a session already running one; losing the dots would
    /// leave a screen that looks finished while the answer is still coming.
    #[test]
    fn a_running_turn_shows_the_dots_and_offers_stop_in_place_of_send() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    running: true,
                    ..ChatState::default()
                });
            },
            chat(),
        );
        assert_eq!(
            html.matches("class=\"dot-anim\"").count(),
            3,
            "the typing indicator is not three dots, so a turn in flight \
             reads as a screen that has stopped: {html}"
        );
        assert!(
            html.contains("<button class=\"send stop\" title=\"Stop\">"),
            "a running turn offers no way to stop it: {html}"
        );
        assert!(
            !html.contains("title=\"Send\""),
            "Send is still on screen during a running turn, so a second \
             prompt can be queued into a session already answering: {html}"
        );

        let idle = render(chat());
        assert!(
            !idle.contains("dot-anim"),
            "an idle chat is drawing the typing indicator"
        );
    }

    /// Every shape a transcript can hold, rendered as its own card. The
    /// interesting part is not that four divs appear: it is that user text is
    /// ESCAPED and assistant text is not. Both go through
    /// `dangerous_inner_html`, so the day `escape_text` stops being applied
    /// to what the user typed, a pasted `<script>` is a script.
    #[test]
    fn each_kind_of_transcript_entry_renders_as_its_own_card() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    items: vec![
                        ChatItem::User {
                            text: "deploy <prod> & wait".to_owned(),
                            attachments: Vec::new(),
                        },
                        ChatItem::Assistant {
                            message_id: None,
                            text: "# Shipped\n\nIt is out.".to_owned(),
                        },
                        ChatItem::Thought {
                            message_id: None,
                            text: "Checking the *manifest*.".to_owned(),
                        },
                        ChatItem::Tool {
                            id: "t-1".to_owned(),
                            title: "shell · uname -a".to_owned(),
                            kind: "execute".to_owned(),
                            status: "completed".to_owned(),
                            output: "Darwin".to_owned(),
                            contents: Vec::new(),
                        },
                    ],
                    ..ChatState::default()
                });
            },
            chat(),
        );

        assert!(
            html.contains("<div class=\"bubble-text\">deploy &lt;prod&gt; &amp; wait</div>"),
            "what the user typed reached the DOM unescaped, so a pasted tag \
             is markup rather than text: {html}"
        );
        assert!(
            html.contains("<div class=\"md\"><h1>Shipped</h1>"),
            "the agent's markdown is not being rendered as markdown: {html}"
        );
        assert!(
            html.contains("<details class=\"thought\"><summary>Thinking</summary>")
                && html.contains("<em>manifest</em>"),
            "reasoning is not in a collapsed Thinking block: {html}"
        );
        assert!(
            html.contains("<div class=\"tool status-completed\">"),
            "the tool card lost the raw status the colour rules key off: {html}"
        );
        assert!(
            html.contains("<span class=\"tool-title\">shell · uname -a</span>")
                && html.contains("<span class=\"tool-status\">Done</span>"),
            "the tool card does not name the tool and its state: {html}"
        );
        assert!(
            html.contains("<summary>Output</summary><pre>Darwin</pre>"),
            "a tool with output offers no way to read it: {html}"
        );
    }

    /// `crates/mock-goose-server`'s own edit, end to end: the shape a real
    /// goose puts on the wire for a `developer__text_editor` call, through
    /// `ToolCallUpdate::contents` and `ChatItem::Tool.contents`, to the card.
    ///
    /// WHAT THIS USED TO RENDER is the reason it is worth a test rather than a
    /// screenshot: `content_text` flattens a diff to `[diff: src/scheduler.rs]`
    /// followed by the NEW side only, so an edit was a closed disclosure whose
    /// contents were the file as it now stands — no deletions, no counts, and
    /// nothing to say which of those lines the agent had touched.
    #[test]
    fn a_file_edit_is_a_diff_card_and_not_a_pre_full_of_new_text() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    items: vec![ChatItem::Tool {
                        id: "t-2".to_owned(),
                        title: "edit: src/scheduler.rs".to_owned(),
                        kind: "edit".to_owned(),
                        status: "completed".to_owned(),
                        output: "[diff: src/scheduler.rs]\nfn tick() {\n    sleep(2);\n}"
                            .to_owned(),
                        contents: vec![ToolCallContent::Diff(FileDiff {
                            path: Some("src/scheduler.rs".to_owned()),
                            old_text: Some("fn tick() {\n    sleep(1);\n}\n".to_owned()),
                            new_text: Some(
                                "fn tick() {\n    sleep(2);\n    log(\"tick\");\n}\n".to_owned(),
                            ),
                        })],
                    }],
                    ..ChatState::default()
                });
            },
            chat(),
        );

        assert!(
            html.contains("<div class=\"diff-body\">"),
            "an edit did not draw a slab: {html}"
        );
        assert!(
            html.contains(
                "diff-line del\"><span class=\"diff-sign\">-</span>\
                           <span class=\"diff-code\">    sleep(1);</span>"
            ),
            "the line the edit REPLACED is not on screen, which is the half \
             #191 went and fetched: {html}"
        );
        assert!(
            html.contains(
                "diff-line add\"><span class=\"diff-sign\">+</span>\
                           <span class=\"diff-code\">    sleep(2);</span>"
            ),
            "the line the edit wrote is not marked as an addition: {html}"
        );
        assert!(
            html.contains("diff-line ctx\""),
            "the unchanged lines around the change are gone, so the edit has \
             no context at all: {html}"
        );
        assert!(
            html.contains(
                "<span class=\"diff-stat\"><span class=\"add\">+2</span>\
                           <span class=\"del\">\u{2212}1</span></span>"
            ),
            "the head does not carry the edit's counts: {html}"
        );
        // The title already names the file, so the card does not name it
        // twice.
        assert!(
            !html.contains("Edited src/scheduler.rs"),
            "the path is printed twice on a card whose title already has it: \
             {html}"
        );
        assert!(
            !html.contains("<summary>Output</summary>"),
            "the flat `<pre>` survived beside the slab, so the same edit is on \
             screen twice: {html}"
        );
    }

    /// The two claims a tool title never makes, and the one case where the
    /// path has to be repeated: a call that edited more than one file.
    ///
    /// `FileDiff` keeps `old_text` and `new_text` as `Option`s precisely so a
    /// renderer can tell "did not exist" from "existed and was empty", and its
    /// own comment says a renderer that wants to say "new file" needs that.
    /// This is the only reader of that distinction in the workspace.
    #[test]
    fn a_new_file_and_a_deleted_one_say_which_they_are() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    items: vec![ChatItem::Tool {
                        id: "t-3".to_owned(),
                        title: "edit".to_owned(),
                        kind: "edit".to_owned(),
                        status: "completed".to_owned(),
                        output: "[diff: src/new.rs]".to_owned(),
                        contents: vec![
                            ToolCallContent::Diff(FileDiff {
                                path: Some("src/new.rs".to_owned()),
                                old_text: None,
                                new_text: Some("fn main() {}\n".to_owned()),
                            }),
                            ToolCallContent::Diff(FileDiff {
                                path: Some("src/old.rs".to_owned()),
                                old_text: Some("gone\n".to_owned()),
                                new_text: None,
                            }),
                            // Neither half: nothing to draw, and the card must
                            // not manufacture a row for it.
                            ToolCallContent::Diff(FileDiff::default()),
                        ],
                    }],
                    ..ChatState::default()
                });
            },
            chat(),
        );

        assert!(
            html.contains("<p class=\"diff-note\">New file src/new.rs</p>"),
            "a file that did not exist before is captioned as an ordinary \
             edit: {html}"
        );
        assert!(
            html.contains("<p class=\"diff-note\">Deleted src/old.rs</p>"),
            "a file that does not exist after is captioned as an ordinary \
             edit: {html}"
        );
        assert!(
            html.contains("<span class=\"add\">+1</span>")
                && html.contains("<span class=\"del\">\u{2212}1</span>"),
            "the head's counts do not add up across the call's two files: \
             {html}"
        );
        assert!(
            html.contains("<summary>Output</summary>"),
            "one content entry drew nothing, so the flat output is the only \
             account of it and has to stay: {html}"
        );
    }

    /// The card is a summary and the disclosure is the fallback, so the two
    /// are coupled: an edit the row budget cut short keeps its `<pre>`, and so
    /// does a call whose result was not all diffs.
    #[test]
    fn an_edit_the_card_had_to_cut_keeps_the_flat_output_beside_it() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                // Every line different, so there is nothing to collapse and
                // the budget is what stops it.
                let old = numbered("old", 200);
                let new = numbered("new", 200);
                let mut chat_state = ChatState {
                    session_id: Some("s-1".to_owned()),
                    ..ChatState::default()
                };
                chat_state.items.push(ChatItem::Tool {
                    id: "t-4".to_owned(),
                    title: "edit: src/big.rs".to_owned(),
                    kind: "edit".to_owned(),
                    status: "completed".to_owned(),
                    output: "[diff: src/big.rs]".to_owned(),
                    contents: vec![ToolCallContent::Diff(FileDiff {
                        path: Some("src/big.rs".to_owned()),
                        old_text: Some(old),
                        new_text: Some(new),
                    })],
                });
                chat.set(chat_state);
            },
            chat(),
        );

        assert!(
            html.contains("more lines — too long to render in a transcript card."),
            "the card silently stopped drawing rather than saying it had: \
             {html}"
        );
        assert!(
            html.contains("<summary>Output</summary>"),
            "the card is an incomplete account of the edit and the disclosure \
             that would complete it was dropped anyway: {html}"
        );
        assert!(
            html.contains("<span class=\"add\">+200</span>"),
            "the head's count was taken after the cut, so it under-reports \
             the edit it is summarising: {html}"
        );
    }

    /// A long unchanged run inside an edit collapses into the same band the
    /// Diff screen draws — as prose rather than as a button, because there is
    /// nothing in a transcript to press.
    #[test]
    fn an_edit_inside_a_long_file_collapses_what_did_not_change() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                let old = numbered("line", 60);
                let new = old.replace("line 30\n", "LINE 30\n");
                let mut chat_state = ChatState {
                    session_id: Some("s-1".to_owned()),
                    ..ChatState::default()
                };
                chat_state.items.push(ChatItem::Tool {
                    id: "t-5".to_owned(),
                    title: "edit: src/long.rs".to_owned(),
                    kind: "edit".to_owned(),
                    status: "completed".to_owned(),
                    output: "[diff: src/long.rs]".to_owned(),
                    contents: vec![ToolCallContent::Diff(FileDiff {
                        path: Some("src/long.rs".to_owned()),
                        old_text: Some(old),
                        new_text: Some(new),
                    })],
                });
                chat.set(chat_state);
            },
            chat(),
        );

        assert!(
            html.contains("<p class=\"diff-note\">⋯ 27 unchanged lines</p>"),
            "a 60-line file drew all 60 rows into a transcript card: {html}"
        );
        assert!(
            !html.contains("<button class=\"diff-skip\""),
            "the band is a control that reveals nothing: {html}"
        );
        assert!(
            !html.contains("<summary>Output</summary>"),
            "the whole edit is drawn, so the flat `<pre>` is the same bytes \
             twice: {html}"
        );
    }

    /// A photo sent with nothing to say is a real message, and an empty text
    /// node under it still draws a line box — a blank strip below the
    /// picture. So the text div is omitted rather than rendered empty.
    #[test]
    fn a_photo_sent_without_words_draws_no_empty_strip_under_it() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    items: vec![ChatItem::User {
                        text: String::new(),
                        attachments: vec![crate::attach::Attachment {
                            name: "receipt.png".to_owned(),
                            mime: "image/png".to_owned(),
                            size: 2_048,
                            thumb: String::new(),
                        }],
                    }],
                    ..ChatState::default()
                });
            },
            chat(),
        );
        assert!(
            html.contains("<span class=\"attach-name\">receipt.png</span>"),
            "the attachment sent with the message is not in the transcript: {html}"
        );
        assert!(
            !html.contains("bubble-text"),
            "a message with no words rendered an empty text node, which draws \
             a blank strip under the photo: {html}"
        );
    }

    /// THE TRANSCRIPT'S TOP FADE AND ITS TOP PADDING ARE ONE NUMBER.
    ///
    /// `assets/desktop/95-transcript.css` dissolves this scroller under the
    /// window band with a mask, and the mask is positioned on the BOX rather
    /// than on the content — so the only thing that keeps the ramp off the
    /// first attribution row at rest is the scroller padding above it being at
    /// least as long as the ramp. Held apart in three declarations, because
    /// `WKWebView` needs the `-webkit-` copy of the mask. Change one and the
    /// sheet still parses, the layout is still legal, and the first `YOU` in
    /// every conversation is painted at partial alpha for good; `docs/audit.js`
    /// cannot see it, because its contrast walk reads `color` and `opacity`
    /// and a mask sets neither.
    ///
    /// `30-sidebar-list.css` has the same pair at the other end of its list
    /// and `src/css.rs` pins it there. This one is pinned HERE because this
    /// file is where the transcript is built and the pair is the transcript's;
    /// the mockup it comes from pads 20 and ramps 26, so copying it faithfully
    /// is the failure, not the fix.
    #[test]
    fn the_transcripts_fade_lands_on_the_padding_it_is_measured_from() {
        const FILE: &str = "95-transcript.css";
        // The leading newline anchors this to the start of a rule, so it
        // cannot match `.pane-main .scroll.chat > .who-line` or a mention of
        // the selector inside a comment.
        const SELECTOR: &str = "\n.pane-main .scroll.chat {";

        let sheet = crate::css::SHELL_PARTS
            .iter()
            .find(|&&(name, _)| name == FILE)
            .map(|&(_, body)| body)
            .unwrap_or_default();
        assert!(
            !sheet.is_empty(),
            "no `{FILE}` in `SHELL_PARTS` — the transcript's region file has \
             been renamed or split, and this test now checks nothing"
        );

        let block = sheet
            .find(SELECTOR)
            .and_then(|at| sheet.get(at + SELECTOR.len()..))
            .and_then(|rest| rest.split('}').next())
            .unwrap_or_default();
        assert!(
            !block.is_empty(),
            "no `{SELECTOR}` rule in {FILE}: the transcript scroller is styled \
             somewhere else now, or under another name"
        );

        let padding = block
            .find("padding-top:")
            .and_then(|at| block.get(at + "padding-top:".len()..))
            .and_then(|rest| rest.split(';').next())
            .unwrap_or_default()
            .trim();
        let fades: Vec<&str> = block
            .match_indices("#000 ")
            .filter_map(|(at, needle)| {
                block
                    .get(at + needle.len()..)
                    .and_then(|rest| rest.split(')').next())
            })
            .collect();

        assert_eq!(
            fades.len(),
            2,
            "`.pane-main .scroll.chat` should carry the fade twice — \
             `mask-image` and the `-webkit-` copy WKWebView needs — and it \
             carries {}. One of the two is gone, so the transcript fades on \
             one engine only.",
            fades.len()
        );
        for fade in &fades {
            assert_eq!(
                *fade, padding,
                "the ramp is {fade} and the scroller pads {padding}. They are \
                 one number: the gradient is a LENGTH precisely so that at \
                 scroll-top it lands on padding rather than on the first \
                 speaker's name, and a ramp that outruns the padding fades \
                 that name permanently with nothing to report it."
            );
        }
    }

    /// EXACTLY ONE LEADING MARK ON A TOOL ROW, whichever kind arrives.
    ///
    /// The desktop's card opens with the mockups' accent word and the phone's
    /// with a glyph, and the failure this pins is the one that shipped: the
    /// sheet hid the glyph and the rule meant to replace it produced nothing,
    /// so the row began with 38px of empty accent column. Two marks is the
    /// other side of the same fault and is just as invisible — both are
    /// `<span>`s in a flex row that would simply lay out.
    ///
    /// It runs on the desktop shell, which is what `cargo test` builds here;
    /// the `Shell::Mobile` half of
    /// `a_tool_kind_has_a_word_only_when_the_word_says_something` is the other
    /// side, and asserts the branch a phone binary takes.
    #[test]
    fn a_tool_row_carries_a_kind_word_or_a_glyph_and_never_both() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    items: vec![
                        tool("a", "cargo test", "execute", "running"),
                        tool("b", "screenshot", "screenshot", "running"),
                    ],
                    ..ChatState::default()
                });
            },
            chat(),
        );
        assert!(
            html.contains("<span class=\"tool-kind\">shell</span>"),
            "the desktop tool card has no kind token, so a run of tool calls \
             has nothing to scan down and the 38px accent column the sheet \
             reserves is empty: {html}"
        );
        assert_eq!(
            html.matches("class=\"tool-kind\"").count(),
            1,
            "a kind with no word of its own was given one anyway, which puts \
             the wire's enum name in the loudest colour on the page: {html}"
        );
        assert_eq!(
            html.matches("class=\"tool-icon\"").count(),
            1,
            "the row that has no word did not fall back to the glyph, so the \
             desktop says less about that tool than the phone does: {html}"
        );
        // And the two never land on the same card.
        assert!(
            !html.contains("class=\"tool-kind\">shell</span><span class=\"tool-icon\""),
            "one card carries both marks: {html}"
        );
    }

    /// Four cards in a row push the reply they were in service of off the
    /// top of the screen, so a settled run collapses to one line the reader
    /// can open. The cards must still be inside it — a fold that dropped
    /// them would be a transcript that quietly lost four tool calls.
    #[test]
    fn a_run_of_finished_tools_folds_into_one_openable_line() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    items: vec![
                        tool("a", "Read main.rs", "read", "completed"),
                        tool("b", "Read state.rs", "read", "completed"),
                        tool("c", "Read chat.rs", "read", "completed"),
                    ],
                    ..ChatState::default()
                });
            },
            chat(),
        );
        assert!(
            html.contains(
                "<details class=\"tool-run\"><summary>Used 3 tools · read 3 files</summary>"
            ),
            "a settled run of three tools did not fold, so the reply they \
             served is three cards further up the screen: {html}"
        );
        assert_eq!(
            html.matches("class=\"tool status-completed\"").count(),
            3,
            "the fold swallowed the cards it was supposed to be hiding \
             behind a summary: {html}"
        );
    }

    /// The one time you want a run open is the time something in it went
    /// wrong or is still going — which is exactly when folding it would hide
    /// the only thing on screen worth reading.
    #[test]
    fn a_run_holding_a_failure_stays_open() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    items: vec![
                        tool("a", "Read main.rs", "read", "completed"),
                        tool("b", "Read gone.rs", "read", "failed"),
                        tool("c", "Read chat.rs", "read", "completed"),
                    ],
                    ..ChatState::default()
                });
            },
            chat(),
        );
        assert!(
            !html.contains("tool-run"),
            "a run with a failure in it folded away, hiding the failure: {html}"
        );
        assert!(
            html.contains("<div class=\"tool status-failed\">")
                && html.contains("<span class=\"tool-status\">Failed</span>"),
            "the failed call is not on screen as a failure: {html}"
        );
        assert_eq!(
            html.matches("class=\"tool status-").count(),
            3,
            "all three cards should be open on their own: {html}"
        );
    }

    /// Folding one card gains nothing and costs a click, so a lone tool call
    /// — and a run interrupted by a message — is rendered flat.
    #[test]
    fn a_single_tool_call_is_not_worth_folding() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    items: vec![
                        tool("a", "Read main.rs", "read", "completed"),
                        ChatItem::Assistant {
                            message_id: None,
                            text: "Reading.".to_owned(),
                        },
                        tool("b", "Read state.rs", "read", "completed"),
                    ],
                    ..ChatState::default()
                });
            },
            chat(),
        );
        assert!(
            !html.contains("tool-run"),
            "two tool calls with a message between them were folded as one \
             run, which reorders the conversation: {html}"
        );
        assert_eq!(html.matches("class=\"tool status-completed\"").count(), 2);
        assert!(html.contains("<p>Reading.</p>"));
    }

    /// A gap in a conversation is a fact about it — the difference between
    /// "the agent answered that" and "you came back the next morning". The
    /// mark is keyed to the index of the item AFTER the pause, so a mark that
    /// stopped being placed would silently join two days into one thread.
    #[test]
    fn a_pause_in_the_conversation_is_marked_with_the_time_it_resumed() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    items: vec![
                        ChatItem::Assistant {
                            message_id: None,
                            text: "Before.".to_owned(),
                        },
                        ChatItem::Assistant {
                            message_id: None,
                            text: "After.".to_owned(),
                        },
                    ],
                    marks: vec![(1, 3_661)],
                    ..ChatState::default()
                });
            },
            chat(),
        );
        assert!(
            html.contains("<div class=\"timemark\"><span>01:01</span></div>"),
            "the pause between the two messages is unmarked: {html}"
        );
        let mark = html.find("timemark").unwrap_or(0);
        let before = html.find("<p>Before.</p>").unwrap_or(0);
        let after = html.find("<p>After.</p>").unwrap_or(0);
        assert!(
            before < mark && mark < after,
            "the mark is not between the messages it separates, so it dates \
             the wrong side of the pause: {html}"
        );
    }

    /// The composer chip is the only place the model and its effort tier are
    /// visible without opening a sheet, and both come off whatever the agent
    /// happens to offer rather than off ids this app hard-codes. The mode is
    /// a chip of its own beside it.
    #[test]
    fn the_composer_names_the_model_its_effort_and_the_mode() {
        let html = render_seeded(
            |ctx| {
                let mut config = ctx.config_options;
                config.set(vec![
                    model_option(),
                    mode_option(),
                    effort_option(&["off", "high"]),
                ]);
            },
            chat(),
        );
        assert!(
            html.contains("<span class=\"chip-model\">Claude Sonnet 5</span>"),
            "the chip shows the raw model id, or the word Session, instead of \
             the label the agent sent: {html}"
        );
        assert!(
            html.contains("<span class=\"chip-effort\">High</span>"),
            "the thinking effort is not on the chip, so the one setting that \
             changes what the next message costs is invisible: {html}"
        );
        assert!(
            html.contains("class=\"composer-chip action mode\"")
                && html.contains("<span class=\"chip-label\">Auto</span>"),
            "the mode chip is missing or unnamed, and mode is filtered out of \
             the settings sheet precisely because this chip is where it \
             lives: {html}"
        );
    }

    /// goose ships `thinking_effort` as a lone `off` whenever the session's
    /// model cannot reason. "Claude Sonnet 5 Off" reads as something switched
    /// off rather than as something the model never had, so a one-value
    /// effort earns no chip — while the model name it rides on stays.
    #[test]
    fn an_effort_the_model_never_had_stays_off_the_chip() {
        let html = render_seeded(
            |ctx| {
                let mut config = ctx.config_options;
                config.set(vec![model_option(), effort_option(&["off"])]);
            },
            chat(),
        );
        assert!(
            html.contains("<span class=\"chip-model\">Claude Sonnet 5</span>"),
            "the chip vanished entirely rather than dropping the effort: {html}"
        );
        assert!(
            !html.contains("chip-effort"),
            "a model that cannot reason is labelled \"Off\", which reads as a \
             setting somebody turned down: {html}"
        );
        assert!(
            !html.contains("class=\"composer-chip action mode\""),
            "a goose that sent no mode grew a mode chip with nothing behind \
             it: {html}"
        );
    }

    /// The percentage costs a chip in a row that has none to spare, so it
    /// only appears once it is the most useful thing there. A chip that was
    /// always on screen is what left the model name rendered in six pixels.
    #[test]
    fn the_context_readout_reaches_the_composer_only_when_the_window_is_nearly_full() {
        let crowded = render_seeded(
            |ctx| {
                let mut usage = ctx.usage;
                usage.set(Some((190_000, 200_000)));
            },
            chat(),
        );
        assert!(
            crowded
                .contains("<span class=\"composer-chip warn\" title=\"Context used\">95%</span>"),
            "a window 95% full says nothing about it: {crowded}"
        );

        let roomy = render_seeded(
            |ctx| {
                let mut usage = ctx.usage;
                usage.set(Some((20_000, 200_000)));
            },
            chat(),
        );
        assert!(
            !roomy.contains("composer-chip warn"),
            "the context chip is spending a slot in the composer row on \"10% \
             used\", which is not a fact anyone acts on: {roomy}"
        );
        assert!(
            roomy.contains("composer-chip action model"),
            "the usage that reached the context length row did not open the \
             settings chip: {roomy}"
        );
    }

    /// Nothing to report is nothing to open. A chat with no config, no turn
    /// and no name would otherwise offer a sheet holding one dash.
    #[test]
    fn a_chat_with_nothing_to_report_offers_no_settings_chip() {
        let html = render(chat());
        assert!(
            !html.contains("composer-chip action model"),
            "the settings chip is on a composer whose sheet has no rows: {html}"
        );
        assert!(
            html.contains("class=\"composer-chip action attach\""),
            "the composer lost its attach button, so this test is passing on \
             an empty render: {html}"
        );
    }

    /// The measured case `docs/permission-durability.md` section 0 is about:
    /// an ask the app answered into a dead socket, whose whole round goose
    /// discarded. There is no declined tool in the transcript and nothing at
    /// all to see, so the loss has to be narrated or it is silent — and the
    /// narration is rendered from the journal rather than pushed into
    /// `items`, because the reconnect that reveals the loss also rebuilds
    /// `items` from `session/load`.
    #[test]
    fn an_ask_lost_with_the_connection_is_narrated_in_the_chat_it_belonged_to() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    title: "Rotate the certificate".to_owned(),
                    ..ChatState::default()
                });
                let mut journal = ctx.lost_asks;
                journal.set(vec![
                    lost("s-1", "shell · uname -a"),
                    lost("s-2", "git push"),
                ]);
            },
            chat(),
        );
        assert!(
            html.contains(
                "shell · uname -a was waiting on your answer when the \
                 connection dropped."
            ),
            "the lost ask is not narrated, so a discarded round looks exactly \
             like an agent that stopped talking: {html}"
        );
        assert!(
            html.contains("Ask again to retry."),
            "the card does not say the one thing the reader can do about it: {html}"
        );
        assert!(
            html.contains("<div class=\"error-box warn\"><div class=\"lost-ask\">")
                && html.contains(">Dismiss</button>"),
            "the narration is not a dismissable warning card: {html}"
        );
        assert!(
            !html.contains("git push"),
            "another chat's lost ask is being narrated in this one: {html}"
        );
    }

    /// A chat that has never been opened has no session to attribute a loss
    /// to, and the journal is keyed by session. Narrating anything here would
    /// mean showing one chat's loss on a blank New Chat screen.
    #[test]
    fn a_chat_with_no_session_narrates_nobodys_loss() {
        let html = render_seeded(
            |ctx| {
                let mut journal = ctx.lost_asks;
                journal.set(vec![lost("s-1", "shell · uname -a")]);
            },
            chat(),
        );
        assert!(
            !html.contains("shell · uname -a"),
            "a chat with no session of its own is narrating a loss from \
             another one: {html}"
        );
        assert!(
            !html.contains("lost-ask"),
            "an empty new chat is rendering a loss card: {html}"
        );
    }

    /// The modal renders the FRONT of the queue and nothing else. With an
    /// empty queue it must put no backdrop on screen at all — a
    /// `.modal-backdrop` with nothing in it is a transparent sheet over the
    /// whole app that swallows every tap.
    #[test]
    fn an_empty_permission_queue_puts_no_backdrop_over_the_app() {
        let html = render(modal());
        assert_eq!(
            html, "",
            "the permission modal rendered chrome with no request behind it, \
             which is an invisible sheet over the entire app"
        );
    }

    /// What the reader has to decide on, under time pressure: which tool,
    /// with what arguments, and how many more are behind it. The button
    /// treatments are the other half — the narrow allow is the solid one even
    /// though the server sent the blanket grant first.
    #[test]
    fn the_permission_modal_names_the_tool_its_input_and_the_queue_behind_it() {
        let html = render_seeded(
            |ctx| {
                let mut queue = ctx.permission;
                let mut first = permission_ask("s-1", Some("shell · uname -a"));
                first.tool_call.raw_input = Some(serde_json::json!({ "command": "uname -a" }));
                queue.set(vec![
                    first,
                    permission_ask("s-1", Some("write")),
                    permission_ask("s-1", Some("read")),
                ]);
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    ..ChatState::default()
                });
            },
            modal(),
        );
        assert!(
            html.contains("<h2>Permission request</h2>")
                && html.contains("<p class=\"modal-tool\">shell · uname -a</p>"),
            "the modal does not say which tool is asking: {html}"
        );
        assert!(
            html.contains("<summary>Details</summary><pre>") && html.contains("uname -a"),
            "the arguments the reader is approving are not on screen: {html}"
        );
        assert!(
            html.contains("<button class=\"btn secondary\">Always allow</button>"),
            "the blanket grant is not the quiet option, even though the \
             server sent it first: {html}"
        );
        assert!(
            html.contains("<button class=\"btn primary\">Allow once</button>"),
            "the one-shot grant is not the solid default: {html}"
        );
        assert!(
            html.contains("<button class=\"btn danger-outline\">Reject</button>"),
            "the refusal is not styled as one: {html}"
        );
        assert!(
            html.contains("<p class=\"modal-pending\">+2 more waiting</p>"),
            "the two asks behind this one are invisible, so answering looks \
             like the end of it: {html}"
        );
        assert!(
            !html.contains("modal-session"),
            "the modal is naming the session it is already on top of: {html}"
        );
    }

    /// An ask can arrive for a session that is not the one on screen, and
    /// answering the wrong agent's shell command is the mistake this line
    /// exists to prevent. The name comes from the chats list when it is
    /// there and falls back to the id when it is not — an unnamed session is
    /// still better identified by its id than by nothing.
    #[test]
    fn an_ask_from_another_session_says_which_one() {
        let named = render_seeded(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![SessionInfo {
                    session_id: "s-2".to_owned(),
                    cwd: None,
                    title: Some("Nightly deploy".to_owned()),
                    updated_at: None,
                    meta: None,
                }]);
                let mut queue = ctx.permission;
                queue.set(vec![permission_ask("s-2", Some("shell · rm -rf /"))]);
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    ..ChatState::default()
                });
            },
            modal(),
        );
        assert!(
            named.contains("<p class=\"modal-session\">Session: Nightly deploy</p>"),
            "an ask from a session other than the open one does not say so, \
             so the reader approves a command they think belongs to the chat \
             in front of them: {named}"
        );

        let unnamed = render_seeded(
            |ctx| {
                let mut queue = ctx.permission;
                queue.set(vec![permission_ask("s-2", Some("shell · rm -rf /"))]);
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    ..ChatState::default()
                });
            },
            modal(),
        );
        assert!(
            unnamed.contains("<p class=\"modal-session\">Session: s-2</p>"),
            "a session the chats list has never seen is identified by nothing \
             at all: {unnamed}"
        );
    }

    /// ACP lets a tool call arrive with no title. The fallback chain is
    /// goose's own tool name, then a generic sentence — because "Permission
    /// request" over a blank line is a dialog asking the reader to approve
    /// they-know-not-what.
    #[test]
    fn an_untitled_tool_call_is_still_named_in_the_modal() {
        let from_meta = render_seeded(
            |ctx| {
                let mut queue = ctx.permission;
                let mut request = permission_ask("s-1", None);
                request.tool_call.meta = Some(serde_json::json!({
                    "goose": { "toolCall": { "toolName": "developer__shell" } }
                }));
                queue.set(vec![request]);
            },
            modal(),
        );
        assert!(
            from_meta.contains("<p class=\"modal-tool\">developer__shell</p>"),
            "a titleless call did not fall back to the tool goose named: {from_meta}"
        );

        let nameless = render_seeded(
            |ctx| {
                let mut queue = ctx.permission;
                queue.set(vec![permission_ask("s-1", None)]);
            },
            modal(),
        );
        assert!(
            nameless.contains("<p class=\"modal-tool\">Run a tool</p>"),
            "a call with neither title nor tool name left the modal asking \
             about a blank line: {nameless}"
        );
        assert!(
            !nameless.contains("tool-output"),
            "a call with no arguments offered an empty Details block: {nameless}"
        );
    }

    pub(super) fn lost(session_id: &str, title: &str) -> AskRecord {
        let mut record = AskRecord::open(
            session_id.to_owned(),
            "Rotate the certificate".to_owned(),
            format!("call-{title}"),
            title.to_owned(),
            1_800_000_000,
        );
        record.state = AskState::Lost {
            at: 1_800_000_060,
            cause: LostCause::Connection,
        };
        record
    }

    pub(super) fn permission_ask(session_id: &str, title: Option<&str>) -> PermissionRequest {
        PermissionRequest {
            request_id: serde_json::Value::from(7),
            session_id: session_id.to_owned(),
            tool_call: ToolCallUpdate {
                tool_call_id: "call-1".to_owned(),
                title: title.map(str::to_owned),
                ..ToolCallUpdate::default()
            },
            // Deliberately in the order a real ACP server sent them, with the
            // broadest grant first.
            options: vec![
                permission_choice("0", "allow_always"),
                permission_choice("1", "allow_once"),
                permission_choice("2", "reject_once"),
            ],
        }
    }

    fn permission_choice(option_id: &str, name: &str) -> PermissionOption {
        PermissionOption {
            option_id: option_id.to_owned(),
            name: Some(name.to_owned()),
            kind: None,
        }
    }
}

/// Pressing things, because a chat screen is mostly buttons.
///
/// `crate::testkit` mounts a view and reads its markup back, which is enough
/// for everything the state decides and nothing the reader decides. Half of
/// this file is event handlers — the settings sheet, the mode picker, the
/// overflow menu, Send, Enter-to-send, answering a permission ask — and every
/// one of them is a closure that a render never runs. A suite that stopped at
/// markup would report those lines as covered by nothing, which is exactly
/// what they were.
///
/// So: dispatch a real event into the `VirtualDom` and render again. Two
/// things have to be solved to do that without a window, and both are solved
/// by borrowing what the framework already does rather than by inventing
/// something.
///
/// WHICH ELEMENT. `Runtime::handle_event` takes an `ElementId`, and nothing
/// in the rendered HTML is one. `dioxus_ssr::pre_render` does write a
/// `data-node-hydration` attribute naming each hydratable element and the
/// events it carries — but those numbers are the SSR renderer's own counter,
/// not `ElementId`s. Pairing the two is exactly the job of hydration, so
/// [`hydration_ids`] is `dioxus-web`'s `rehydrate` walk
/// (`dioxus-web-0.7.10/src/hydration/hydrate.rs`) over public `dioxus-core`
/// API: the k-th element that walk visits is the element the SSR renderer
/// numbered k.
///
/// The obvious cheaper pairing is WRONG, and was measured to be wrong before
/// this was written. `VirtualDom::rebuild_to_vec` also hands back one
/// `NewEventListener` per listener, and taking the k-th of those to be the
/// k-th listener in the markup put Send's press on the settings chip:
/// creation order is not document order, because `create` writes a template
/// root's dynamic ATTRIBUTES first and then instantiates its dynamic NODES in
/// reverse (`dioxus-core-0.7.10/src/diff/node.rs`, `load_placeholders`), and
/// the composer's chip and its send button are both dynamic nodes. A press
/// landing on the wrong control is the worst failure available here: it is
/// silent, and the test that "passes" is asserting about a button nobody
/// pressed.
///
/// WHICH PAYLOAD. `dioxus-html` routes every listener through a process-global
/// `HtmlEventConverter` that a renderer installs at launch, and without one
/// the `.unwrap()` inside `ListenerCallback` panics.
/// `SerializedHtmlEventConverter` is the converter `dioxus-desktop` itself
/// installs, so a press here is converted by the code the shipped app uses.
///
/// It is `pub(crate)` because it is the only thing in the suite that can press
/// a button, and this is not the only file made of them: `views/scheduler.rs`
/// hangs eleven handlers off rows, sheets and confirms, and
/// `views/chrome.rs`'s `SearchField` is a debounce that only a keystroke
/// starts. Duplicating the hydration walk per file would be duplicating the
/// one piece of this harness that was measured WRONG the obvious way — so
/// there is one of it, and [`alone`] is shared with it, because the storage
/// hazard the lock exists for is process-wide and does not care which file a
/// mount was written in.
#[cfg(test)]
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test scaffolding: a press that cannot find its button has \
              nothing to assert, so failing loudly there IS the check"
)]
pub(crate) mod pressing {
    use std::any::Any;
    use std::cell::RefCell;
    use std::rc::Rc;

    use dioxus::dioxus_core::{
        DynamicNode, ElementId, NoOpMutations, ScopeState, TemplateAttribute, TemplateNode, VNode,
    };
    use dioxus::html::{
        Code, Key, Location, Modifiers, PlatformEventData, SerializedFormData,
        SerializedHtmlEventConverter, SerializedKeyboardData, SerializedMouseData,
    };
    use dioxus::prelude::*;
    use goose_acp_client::{AcpClient, AcpEvent, ConnectConfig};
    use serde_json::{json, Value};

    use super::tests::{lost, mode_option, model_option, permission_ask};
    use crate::scheduler::tests::{ok, rpc_error, serve, Reply, Script, Server};
    use crate::state::{AppCtx, ChatItem, ChatState, Screen};

    /// The two process-wide things a press needs, set up exactly once, and
    /// the runtime handed back because that is the half a press has to enter.
    ///
    /// Both were per-mount first, and both had to stop being: `cargo test`
    /// runs these on every core at once, and an event converter reinstalled
    /// under a reader — or a `tokio` runtime built and dropped around every
    /// press — wedged the whole binary for minutes while every one of these
    /// tests still passed when run on its own. Order- and
    /// parallelism-dependent failure is the worst property a merge gate can
    /// have.
    ///
    /// The runtime is entered and never driven. `show_toast` arms a
    /// `tokio::time::sleep` to take the toast away again and polling that
    /// without a reactor panics; registered-and-never-fired is the right
    /// answer for a test that renders one frame and reads it.
    fn install_once() -> &'static tokio::runtime::Runtime {
        static TIMERS: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        TIMERS.get_or_init(|| {
            dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
            // `enable_all` rather than `enable_time`: the timer is what a
            // toast needs, and the IO driver is what [`wire`]'s socket needs.
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread tokio runtime for the toast timer")
        })
    }

    /// A mounted view with a way in.
    pub(crate) struct Pressable {
        dom: VirtualDom,
        /// The render with `data-node-hydration` left in, and the elements it
        /// numbers, in the same order. Recomputed after every event, because
        /// an event that opens a sheet creates elements.
        hydrated: String,
        ids: Vec<ElementId>,
    }

    #[derive(Clone, Copy)]
    struct Mount {
        seed: fn(&AppCtx),
        view: fn() -> Element,
    }

    /// Never memoise: a harness renders once and is dropped, and comparing
    /// `fn` pointers is meaningless (see `crate::testkit`).
    impl PartialEq for Mount {
        fn eq(&self, _other: &Self) -> bool {
            false
        }
    }

    #[expect(
        non_snake_case,
        reason = "a Dioxus component is named like a component, not like a fn"
    )]
    fn Harness(props: Mount) -> Element {
        let ctx = crate::state::use_app_ctx_provider();
        use_hook(|| (props.seed)(&ctx));
        (props.view)()
    }

    /// The element behind each `data-node-hydration` number, in that order.
    ///
    /// `dioxus-web`'s `rehydrate`, minus the parts about suspense and
    /// `onmounted` that only a real renderer needs. It is the framework's own
    /// definition of the pairing rather than a guess at it, and it walks the
    /// live `VirtualDom` rather than a log of edits — so it can be redone
    /// after an event has changed the tree.
    fn hydration_ids(dom: &VirtualDom) -> Vec<ElementId> {
        let mut ids = Vec::new();
        walk_scope(dom, dom.base_scope(), &mut ids);
        ids
    }

    fn walk_scope(dom: &VirtualDom, scope: &ScopeState, ids: &mut Vec<ElementId>) {
        walk_vnode(dom, scope.root_node(), ids);
    }

    fn walk_vnode(dom: &VirtualDom, vnode: &VNode, ids: &mut Vec<ElementId>) {
        for (index, root) in vnode.template.roots.iter().enumerate() {
            walk_template_node(dom, vnode, root, ids, vnode.mounted_root(index, dom));
        }
    }

    fn walk_template_node(
        dom: &VirtualDom,
        vnode: &VNode,
        node: &TemplateNode,
        ids: &mut Vec<ElementId>,
        root_id: Option<ElementId>,
    ) {
        match node {
            TemplateNode::Element {
                children, attrs, ..
            } => {
                // An element is numbered when it is a template root or
                // carries a dynamic attribute, which is exactly when the SSR
                // renderer writes `data-node-hydration` on it.
                let mut mounted = root_id;
                for attr in *attrs {
                    if let TemplateAttribute::Dynamic { id } = attr {
                        if let Some(id) = vnode.mounted_dynamic_attribute(*id, dom) {
                            mounted = Some(id);
                        }
                    }
                }
                if let Some(id) = mounted {
                    ids.push(id);
                }
                for child in *children {
                    walk_template_node(dom, vnode, child, ids, None);
                }
            }
            TemplateNode::Dynamic { id } => {
                walk_dynamic_node(dom, vnode, &vnode.dynamic_nodes[*id], *id, ids);
            }
            TemplateNode::Text { .. } => {
                if let Some(id) = root_id {
                    ids.push(id);
                }
            }
        }
    }

    fn walk_dynamic_node(
        dom: &VirtualDom,
        vnode: &VNode,
        dynamic: &DynamicNode,
        index: usize,
        ids: &mut Vec<ElementId>,
    ) {
        match dynamic {
            DynamicNode::Text(_) | DynamicNode::Placeholder(_) => {
                if let Some(id) = vnode.mounted_dynamic_node(index, dom) {
                    ids.push(id);
                }
            }
            DynamicNode::Component(component) => {
                if let Some(scope) = component.mounted_scope(index, vnode, dom) {
                    walk_scope(dom, scope, ids);
                }
            }
            DynamicNode::Fragment(fragment) => {
                for node in fragment {
                    walk_vnode(dom, node, ids);
                }
            }
        }
    }

    impl Pressable {
        pub(crate) fn mount(seed: fn(&AppCtx), view: fn() -> Element) -> Self {
            // The same one owner as `crate::testkit`: `set_directory` writes a
            // process-wide `OnceLock` and unwraps, so the second caller in a
            // test binary panics.
            let _ = crate::testkit::storage_dir();
            let _ = install_once();
            let mut dom = VirtualDom::new_with_props(Harness, Mount { seed, view });
            dom.rebuild_in_place();
            let mut screen = Self {
                dom,
                hydrated: String::new(),
                ids: Vec::new(),
            };
            screen.reread();
            screen
        }

        fn reread(&mut self) {
            self.hydrated = dioxus_ssr::pre_render(&self.dom);
            self.ids = hydration_ids(&self.dom);
        }

        /// What the screen says right now.
        pub(crate) fn markup(&self) -> String {
            dioxus_ssr::render(&self.dom)
        }

        /// Let the tasks a press started run, and re-read the screen.
        ///
        /// Most handlers in this app finish inside the dispatch: they write a
        /// signal and return. The exception is anything that waits — the
        /// debounce in `views::chrome::SearchField` sleeps 250 ms before it
        /// calls anything at all — and for those the press alone proves
        /// nothing, because the observable half has not happened yet.
        ///
        /// Wall-clock rather than `tokio::time::pause`, deliberately: the
        /// timer lives on a task Dioxus owns and polls from its own executor,
        /// so nothing here is in a position to hand tokio the idle it needs
        /// before it will auto-advance. The budget is 20 slices of 25 ms
        /// against a 250 ms debounce, and a slice with work in it returns at
        /// once — so a settled screen costs nothing and an unsettled one
        /// cannot hang the suite.
        pub(crate) fn settle(&mut self) {
            let dom = &mut self.dom;
            install_once().block_on(async {
                for _ in 0..20 {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_millis(25),
                        dom.wait_for_work(),
                    )
                    .await;
                    dom.render_immediate_to_vec();
                }
            });
            self.reread();
        }

        /// The `ElementId` of the first element whose opening tag contains
        /// `needle` and which carries an `event` listener.
        fn locate(&self, event: &str, needle: &str) -> ElementId {
            const MARK: &str = " data-node-hydration=\"";
            let mut at = 0;
            while let Some(rel) = self.hydrated[at..].find(MARK) {
                let start = at + rel;
                let value = start + MARK.len();
                let end = value
                    + self.hydrated[value..]
                        .find('"')
                        .expect("an unterminated data-node-hydration attribute");
                let tag_start = self.hydrated[..start].rfind('<').unwrap_or(0);
                let tag = &self.hydrated[tag_start..start];
                let mut parts = self.hydrated[value..end].split(',');
                let number: usize = parts
                    .next()
                    .and_then(|n| n.parse().ok())
                    .expect("a data-node-hydration number that is not a number");
                if parts.any(|l| l.split(':').next() == Some(event)) && tag.contains(needle) {
                    return *self.ids.get(number).expect(
                        "the markup numbers an element the hydration walk never \
                         reached, so the two are out of step and a press would \
                         land somewhere else entirely",
                    );
                }
                at = end;
            }
            panic!(
                "nothing matching {needle:?} carries an {event} listener:\n{}",
                self.hydrated
            )
        }

        fn dispatch(&mut self, event: &str, needle: &str, data: Box<dyn Any>) {
            let id = self.locate(event, needle);
            let payload: Rc<dyn Any> = Rc::new(PlatformEventData::new(data));
            {
                let _timers = install_once().enter();
                self.dom
                    .runtime()
                    .handle_event(event, Event::new(payload, true), id);
                self.dom.render_immediate(&mut NoOpMutations);
            }
            self.reread();
        }

        /// Tap the first control whose opening tag contains `needle`.
        pub(crate) fn press(&mut self, needle: &str) {
            self.dispatch("click", needle, Box::new(SerializedMouseData::default()));
        }

        /// Type into the first field whose opening tag contains `needle`,
        /// exactly as a `WebView` reports it: the field's whole new value.
        pub(crate) fn type_into(&mut self, needle: &str, value: &str) {
            self.dispatch(
                "input",
                needle,
                Box::new(SerializedFormData::new(value.to_owned(), Vec::new())),
            );
        }

        /// Press a key in the first field whose opening tag contains `needle`.
        ///
        /// The logical key and the physical one are both given because a
        /// browser sends both; this app reads only the logical one, and a
        /// harness that quietly reported every keystroke as physically Enter
        /// would be lying to the next handler that looks at `code`.
        fn key(&mut self, needle: &str, key: &Key, code: Code, modifiers: Modifiers) {
            self.dispatch(
                "keydown",
                needle,
                Box::new(SerializedKeyboardData::new(
                    key.clone(),
                    code,
                    Location::Standard,
                    false,
                    modifiers,
                    false,
                )),
            );
        }
    }

    /// The chat, plus the two pieces of state a press changes that the chat
    /// itself never puts on screen.
    fn chat_and_state() -> Element {
        let ctx = crate::state::use_app_ctx();
        let toast = (ctx.toast)().unwrap_or_default();
        let left = (ctx.screen)() == Screen::Sessions;
        rsx! {
            p { class: "probe-toast", "{toast}" }
            p { class: "probe-left", "{left}" }
            super::ChatView {}
        }
    }

    fn chat() -> fn() -> Element {
        || rsx! { super::ChatView {} }
    }

    /// These tests run one at a time, and the lock is not decoration.
    ///
    /// Rendering after an event is the only thing in this file that runs the
    /// `VirtualDom`'s task queue, and the ask journal's `use_synced_storage`
    /// puts two tasks on that queue talking to a watch channel that
    /// `dioxus-sdk-storage` keys by storage key in a `static` — process-wide,
    /// so every mount in the binary is on the same one. Two mounts rendering
    /// at once feed each other through it and never settle: measured at 7
    /// wedged runs in 40 of this module, each one a single thread spinning
    /// inside `watch::Receiver::changed`, and 0 in 40 with
    /// `--test-threads=1`. Every one of these tests passes when run alone,
    /// which is precisely what makes that shape of failure worth locking out
    /// rather than living with.
    ///
    /// `PoisonError::into_inner` because a test that fails while holding this
    /// has already reported the thing it exists to report, and taking the
    /// rest of the module down behind it would only hide which one broke.
    pub(crate) fn alone() -> std::sync::MutexGuard<'static, ()> {
        static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
        ONE_AT_A_TIME
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A conversation that exists, so the composer and the transcript are on
    /// their live arms rather than their empty ones.
    fn open_chat(ctx: &AppCtx) {
        let mut chat = ctx.chat;
        chat.set(ChatState {
            session_id: Some("s-1".to_owned()),
            title: "Rotate the certificate".to_owned(),
            ..ChatState::default()
        });
    }

    // ---------------------------------------------------- with a real socket
    //
    // Three of this screen's presses do nothing observable without one. Send
    // returns before it clears the composer unless `send_prompt` got as far as
    // a client, and Delete's whole outcome — the chat leaving, the row going
    // from the list, the sentence when goose refuses — is on the far side of
    // an `await` on one. `AcpClient` has no constructor but `connect`, so the
    // answer is the same as `src/scheduler.rs`'s: a scripted JSON-RPC server
    // on a loopback port over plain `ws://`.
    //
    // The client is parked in a `thread_local` rather than handed to the
    // seed, because a seed is a plain `fn` pointer — `Mount` is `Copy` and has
    // to be — and a `fn` cannot carry one.

    thread_local! {
        /// The live client a seed installs, and the event stream that keeps it
        /// alive: the client's actor gives up the socket the moment the
        /// receiver is dropped.
        static WIRED: RefCell<Option<(AcpClient, tokio::sync::mpsc::Receiver<AcpEvent>)>> =
            const { RefCell::new(None) };
    }

    /// Stand a goose up that answers `script`, and connect to it.
    ///
    /// The returned [`Server`] is the log of what the screen actually asked
    /// for; holding it also keeps the listener's port claimed.
    fn wire(script: Script) -> Server {
        let runtime = install_once();
        let server = serve(runtime, script);
        let cfg = ConnectConfig {
            base_url: server.base_url.clone(),
            secret: String::new(),
            fingerprint: None,
        };
        let (client, events, _info) = runtime
            .block_on(AcpClient::connect(&cfg))
            .expect("the mock server accepted the handshake");
        WIRED.with(|slot| *slot.borrow_mut() = Some((client, events)));
        server
    }

    fn install_wired_client(ctx: &AppCtx) {
        WIRED.with(|slot| {
            if let Some((client, _)) = slot.borrow().as_ref() {
                ctx.client.clone().set(Some(client.clone()));
            }
        });
    }

    fn open_chat_on_a_live_socket(ctx: &AppCtx) {
        open_chat(ctx);
        install_wired_client(ctx);
    }

    /// The same, with the chat also present in the list it was opened from —
    /// which is the copy a delete has to remove.
    fn a_listed_chat_on_a_live_socket(ctx: &AppCtx) {
        open_chat_on_a_live_socket(ctx);
        let mut sessions = ctx.sessions;
        sessions.set(vec![
            goose_acp_client::SessionInfo {
                session_id: "s-1".to_owned(),
                cwd: None,
                title: Some("Rotate the certificate".to_owned()),
                updated_at: None,
                meta: None,
            },
            goose_acp_client::SessionInfo {
                session_id: "s-2".to_owned(),
                cwd: None,
                title: Some("Nightly deploy".to_owned()),
                updated_at: None,
                meta: None,
            },
        ]);
    }

    /// The chat, plus the two things a delete changes that it never draws.
    fn chat_and_sessions() -> Element {
        let ctx = crate::state::use_app_ctx();
        let toast = (ctx.toast)().unwrap_or_default();
        let left = (ctx.screen)() == Screen::Sessions;
        let listed: Vec<String> = (ctx.sessions)()
            .iter()
            .map(|info| info.session_id.clone())
            .collect();
        rsx! {
            p { class: "probe-toast", "{toast}" }
            p { class: "probe-left", "{left}" }
            p { class: "probe-list", "{listed.join(\",\")}" }
            super::ChatView {}
        }
    }

    /// A goose that takes whatever it is sent.
    fn agreeable(_method: &str, _params: &Value) -> Reply {
        ok(json!({}))
    }

    /// The composer is cleared only once the message is on its way — and this
    /// is the "on its way" half, which nothing could reach before there was a
    /// socket to be on the way over.
    ///
    /// Three things happen together or the screen lies about what it did: the
    /// text leaves the box, the message joins the transcript, and the turn
    /// starts. Drop the clear and the next Enter sends the message twice; drop
    /// the push and the reader watches an agent answer something that is not
    /// on screen anywhere.
    #[test]
    fn a_send_that_leaves_the_phone_clears_the_composer_and_posts_the_message() {
        let _alone = alone();
        let server = wire(agreeable);
        let mut screen = Pressable::mount(open_chat_on_a_live_socket, chat_and_sessions);
        screen.type_into(r#"class="input""#, "roll the certificate");

        screen.press(r#"title="Send""#);
        let html = screen.markup();
        assert!(
            html.contains(r#"value="""#),
            "the composer kept the message it had just sent, so the next Enter \
             sends it a second time: {html}"
        );
        assert!(
            html.contains(r#"<div class="bubble-text">roll the certificate</div>"#),
            "the sent message is not in the transcript, so the reader watches \
             an agent answer something that is nowhere on screen: {html}"
        );
        assert!(
            html.contains(r#"<button class="send stop" title="Stop">"#),
            "the turn is not showing as running, so the composer will take a \
             second prompt into a session already answering: {html}"
        );
        assert!(
            html.contains(r#"<p class="probe-toast"></p>"#),
            "a send that reached goose reported a failure: {html}"
        );

        screen.settle();
        let sent: Vec<String> = server
            .log()
            .iter()
            .map(|(method, _)| method.clone())
            .filter(|method| method != "initialize")
            .collect();
        assert_eq!(
            sent,
            ["session/prompt"],
            "the press cleared the composer without putting a prompt on the \
             wire, which loses the message outright"
        );
    }

    /// The delete goose agreed to: the chat leaves, and its row leaves with
    /// it.
    ///
    /// The row is dropped locally rather than by re-listing, so it has to be
    /// dropped on the `Ok` arm and nowhere else. Leave it and the list you
    /// land on still offers the conversation that has just been unlinked —
    /// one tap from a `session/load` that cannot answer.
    #[test]
    fn a_delete_goose_took_leaves_the_chat_and_takes_its_row_with_it() {
        let _alone = alone();
        let server = wire(agreeable);
        let mut screen = Pressable::mount(a_listed_chat_on_a_live_socket, chat_and_sessions);
        assert!(screen
            .markup()
            .contains(r#"<p class="probe-list">s-1,s-2</p>"#));

        screen.press(r#"title="More""#);
        screen.press(r#"class="setting-row danger""#);
        screen.press(r#"class="btn danger""#);
        screen.settle();

        let html = screen.markup();
        assert!(
            html.contains(r#"<p class="probe-left">true</p>"#),
            "the deleted chat is still on screen, showing a conversation that \
             is no longer on the server: {html}"
        );
        assert!(
            html.contains(r#"<p class="probe-list">s-2</p>"#),
            "the chats list still offers the conversation that was just \
             unlinked, which is one tap from a load that cannot answer: {html}"
        );
        assert_eq!(
            server
                .log()
                .last()
                .map(|(method, params)| (method.clone(), params.clone())),
            Some(("session/delete".to_owned(), json!({ "sessionId": "s-1" }))),
            "the confirmation deleted a different session than the one it was \
             asked about"
        );
    }

    /// The delete goose refused: nothing moves, and the reason is said.
    ///
    /// This is the arm that must not be optimistic. Walking back to the list
    /// and dropping the row on a refusal reports a deletion that did not
    /// happen — and the chat is still there, so the next list brings it
    /// straight back.
    #[test]
    fn a_delete_goose_refused_keeps_the_chat_and_says_why() {
        fn refuses(method: &str, params: &Value) -> Reply {
            if method == "session/delete" {
                return rpc_error(-32602, "no session with that id");
            }
            agreeable(method, params)
        }
        let _alone = alone();
        let _server = wire(refuses);
        let mut screen = Pressable::mount(a_listed_chat_on_a_live_socket, chat_and_sessions);
        screen.press(r#"title="More""#);
        screen.press(r#"class="setting-row danger""#);
        screen.press(r#"class="btn danger""#);
        screen.settle();

        let html = screen.markup();
        assert!(
            html.contains(r#"<p class="probe-toast">Delete failed: no session with that id</p>"#),
            "a delete goose refused said nothing, so the chat looks deleted \
             and is not: {html}"
        );
        assert!(
            html.contains(r#"<p class="probe-left">false</p>"#),
            "a refused delete walked back to the chats list, which reports a \
             deletion that never happened: {html}"
        );
        assert!(
            html.contains(r#"<p class="probe-list">s-1,s-2</p>"#),
            "a refused delete dropped the row anyway — and the next list \
             brings it back, which reads as an undo: {html}"
        );
    }

    /// The chip summarises the sheet, and the sheet is where the summary can
    /// be changed. If the press stopped opening it, the model, the provider
    /// and the session's own name would all become unreachable from the
    /// composer — there is no other way in.
    #[test]
    fn the_composers_chip_opens_the_sheet_it_summarises() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                open_chat(ctx);
                let mut config = ctx.config_options;
                config.set(vec![model_option(), mode_option()]);
            },
            chat(),
        );
        assert!(
            !screen.markup().contains("Session settings</h2>"),
            "the settings sheet is open before anything was pressed"
        );

        screen.press("title=\"Session settings\"");
        let html = screen.markup();
        assert!(
            html.contains("<h2>Session settings</h2>"),
            "pressing the composer's chip did not open the sheet, so nothing \
             it summarises can be changed: {html}"
        );
        assert!(
            html.contains("goose · applies from your next message"),
            "the sheet does not say which agent it is about: {html}"
        );
        assert!(
            html.contains("Rotate the certificate"),
            "the sheet's first row should be the session's own name, which is \
             the only row about this chat rather than about the agent: {html}"
        );
        assert!(
            html.contains("Claude Sonnet 5"),
            "the model the chip named is not a row in the sheet it opened: {html}"
        );
        assert!(
            !html.contains(">Mode<"),
            "mode is in the sheet as well as on its own chip, so the same \
             setting is offered twice: {html}"
        );
    }

    /// Mode is the setting you change mid-conversation, so it is the one with
    /// a chip of its own — and it is filtered out of the settings sheet
    /// precisely because this picker is where it lives. A press that stopped
    /// opening it would strand the setting with no way to reach it at all.
    #[test]
    fn the_mode_chip_opens_a_picker_of_the_agents_own_modes() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                open_chat(ctx);
                let mut config = ctx.config_options;
                config.set(vec![model_option(), mode_option()]);
            },
            chat(),
        );
        screen.press("title=\"Mode\"");
        let html = screen.markup();
        assert!(
            html.contains("<h2>Select mode</h2>"),
            "the mode chip opened no picker: {html}"
        );
        assert!(
            html.contains("Manual approval") && html.contains("Run tools without asking."),
            "the picker does not carry the agent's own names and descriptions \
             for its modes, which is what tells them apart: {html}"
        );
        assert!(
            !html.contains("This agent offers no other mode."),
            "a picker with two modes in it is claiming to have none: {html}"
        );
    }

    /// Delete lives behind the overflow because it is the one irreversible
    /// thing on this screen. It is also the only item there, so a press that
    /// stopped opening the menu would take deleting a chat out of the app.
    #[test]
    fn the_overflow_is_the_only_way_to_delete_a_chat() {
        let _alone = alone();
        let mut screen = Pressable::mount(open_chat, chat());
        assert!(
            !screen.markup().contains("Delete chat"),
            "the overflow menu is open before it was pressed"
        );

        screen.press("title=\"More\"");
        let html = screen.markup();
        assert!(
            html.contains("Delete chat"),
            "pressing the overflow opened no menu, so a chat cannot be \
             deleted from the app at all: {html}"
        );
        assert!(
            html.contains("danger"),
            "the one irreversible item in the menu is not marked as one: {html}"
        );
    }

    /// The composer is cleared only once the message is on its way. A send
    /// that never starts — no connection — has to leave the typed text where
    /// it was, or a dropped tailnet silently eats what you wrote.
    #[test]
    fn a_send_that_never_starts_keeps_what_you_typed() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                open_chat(ctx);
                let mut draft = ctx.chat_draft;
                draft.set("roll the certificate".to_owned());
            },
            chat_and_state,
        );
        screen.press("title=\"Send\"");
        let html = screen.markup();
        assert!(
            html.contains("value=\"roll the certificate\""),
            "a send that never left the phone cleared the composer, so the \
             message is gone and was never sent: {html}"
        );
        assert!(
            html.contains("<p class=\"probe-toast\">Not connected — reconnect in Settings</p>"),
            "pressing Send with no connection said nothing at all: {html}"
        );
    }

    /// An empty composer is not a message, and the guard is what stops Send
    /// from posting a blank turn into the transcript. Asserted through the
    /// toast: reaching `send_prompt` at all would report the connection.
    #[test]
    fn an_empty_composer_sends_nothing_at_all() {
        let _alone = alone();
        let mut screen = Pressable::mount(open_chat, chat_and_state);
        screen.press("title=\"Send\"");
        let html = screen.markup();
        assert!(
            html.contains("<p class=\"probe-toast\"></p>"),
            "Send with nothing typed and nothing attached went on to try to \
             send, which posts an empty turn: {html}"
        );
    }

    /// Enter sends and Shift+Enter does not, which is the difference between
    /// a paragraph break and a message posted mid-sentence. Both arms are
    /// checked in one test because the bug is always that one of them started
    /// behaving like the other.
    #[test]
    fn enter_sends_and_shift_enter_writes_a_new_line() {
        let _alone = alone();
        let seed: fn(&AppCtx) = |ctx| {
            open_chat(ctx);
            let mut draft = ctx.chat_draft;
            draft.set("roll the certificate".to_owned());
        };

        let mut plain = Pressable::mount(seed, chat_and_state);
        plain.key(
            "class=\"input\"",
            &Key::Enter,
            Code::Enter,
            Modifiers::empty(),
        );
        assert!(
            plain
                .markup()
                .contains("Not connected — reconnect in Settings"),
            "Enter did not try to send, so the only way to post a message is \
             the button: {}",
            plain.markup()
        );

        let mut shifted = Pressable::mount(seed, chat_and_state);
        shifted.key(
            "class=\"input\"",
            &Key::Enter,
            Code::Enter,
            Modifiers::SHIFT,
        );
        assert!(
            shifted.markup().contains("<p class=\"probe-toast\"></p>"),
            "Shift+Enter sent the message instead of breaking the line, so a \
             half-written thought is posted: {}",
            shifted.markup()
        );

        let mut other = Pressable::mount(seed, chat_and_state);
        other.key(
            "class=\"input\"",
            &Key::Escape,
            Code::Escape,
            Modifiers::empty(),
        );
        assert!(
            other.markup().contains("<p class=\"probe-toast\"></p>"),
            "a key that is not Enter sent the message: {}",
            other.markup()
        );
    }

    /// The draft is on the context rather than in the component because it
    /// has to outlive the screen — a recipe and a scheduled run both fill it
    /// in from elsewhere. This is the write half of that: what is typed goes
    /// somewhere that survives leaving the chat.
    #[test]
    fn what_is_typed_goes_into_the_draft_the_screen_does_not_own() {
        let _alone = alone();
        let mut screen = Pressable::mount(open_chat, chat_and_state);
        assert!(screen.markup().contains("value=\"\""));

        screen.type_into("class=\"input\"", "half a thought");
        assert!(
            screen.markup().contains("value=\"half a thought\""),
            "typing did not reach the draft, so the composer shows nothing \
             back and nothing survives navigating away: {}",
            screen.markup()
        );
    }

    /// The modal renders the FRONT of the queue, so answering has to reveal
    /// the next ask rather than closing the dialog on the ones behind it —
    /// each of which is a turn blocked on an answer.
    #[test]
    fn answering_the_front_of_the_queue_reveals_the_ask_behind_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut queue = ctx.permission;
                let mut first = permission_ask("s-1", Some("shell · uname -a"));
                first.request_id = serde_json::Value::from(1);
                let mut second = permission_ask("s-1", Some("shell · rm -rf /"));
                second.request_id = serde_json::Value::from(2);
                queue.set(vec![first, second]);
            },
            || rsx! { super::PermissionModal {} },
        );
        assert!(screen.markup().contains("+1 more waiting"));

        screen.press("class=\"btn primary\"");
        let html = screen.markup();
        assert!(
            html.contains("<p class=\"modal-tool\">shell · rm -rf /</p>"),
            "answering the first ask did not bring up the second, so a turn \
             is left blocked on an answer nobody can give: {html}"
        );
        assert!(
            !html.contains("modal-pending"),
            "the last ask in the queue still claims more are waiting: {html}"
        );
    }

    /// Dismissing is the only thing the reader can do about a lost ask, and
    /// it has to stick: the card is derived from the journal on every render,
    /// so an acknowledgement that did not reach the journal would put the
    /// same card straight back on screen.
    #[test]
    fn dismissing_a_lost_ask_takes_the_card_away_for_good() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                open_chat(ctx);
                let mut journal = ctx.lost_asks;
                journal.set(vec![lost("s-1", "shell · uname -a")]);
            },
            chat(),
        );
        assert!(screen.markup().contains("was waiting on your answer"));

        screen.press("class=\"btn small secondary\"");
        let html = screen.markup();
        assert!(
            !html.contains("was waiting on your answer"),
            "the dismissed loss came straight back, so the acknowledgement \
             never reached the journal the card is rendered from: {html}"
        );
    }

    /// The back arrow is the only way off this screen on a phone, and it also
    /// kicks off the refresh that puts the chat you were just in at the top
    /// of the list you land on.
    #[test]
    fn the_back_arrow_leaves_the_chat_for_the_chats_list() {
        let _alone = alone();
        let mut screen = Pressable::mount(open_chat, chat_and_state);
        assert!(screen
            .markup()
            .contains("<p class=\"probe-left\">false</p>"));

        screen.press("class=\"icon-btn back\"");
        assert!(
            screen.markup().contains("<p class=\"probe-left\">true</p>"),
            "the back arrow did not leave the chat, which is the only way off \
             this screen: {}",
            screen.markup()
        );
    }

    /// A new chat needs somewhere on the server to run, and the app cannot
    /// guess it. Saying so is the whole of the failure: without it the button
    /// does nothing and looks broken.
    #[test]
    fn a_new_chat_with_nowhere_to_run_says_what_is_missing() {
        let _alone = alone();
        let mut screen = Pressable::mount(open_chat, chat_and_state);
        screen.press("title=\"New chat\"");
        assert!(
            screen.markup().contains(
                "Set an absolute working directory (a path on the server) in \
                 Settings first"
            ),
            "New chat with no working directory set said nothing, so the \
             button reads as broken: {}",
            screen.markup()
        );
    }

    /// A folded run is only worth folding if it opens, and what it opens is
    /// the cards themselves — this is the one place the transcript hides
    /// something the reader may need.
    #[test]
    fn a_folded_run_keeps_the_cards_it_folded() {
        let _alone = alone();
        let screen = Pressable::mount(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    items: vec![
                        ChatItem::Tool {
                            id: "a".to_owned(),
                            title: "Read main.rs".to_owned(),
                            kind: "read".to_owned(),
                            status: "completed".to_owned(),
                            output: "fn main".to_owned(),
                            contents: Vec::new(),
                        },
                        ChatItem::Tool {
                            id: "b".to_owned(),
                            title: "Read state.rs".to_owned(),
                            kind: "read".to_owned(),
                            status: "completed".to_owned(),
                            output: String::new(),
                            contents: Vec::new(),
                        },
                    ],
                    ..ChatState::default()
                });
            },
            chat(),
        );
        let html = screen.markup();
        assert!(
            html.contains("Read main.rs") && html.contains("Read state.rs"),
            "the fold dropped the cards instead of hiding them: {html}"
        );
        assert!(
            html.contains("<pre>fn main</pre>"),
            "a folded tool's output is not inside the fold, so opening it \
             shows a card with nothing in it: {html}"
        );
    }

    /// Renaming swaps one sheet for another rather than nesting them: the
    /// rename field is a screen's worth of keyboard, and the settings sheet
    /// underneath it would be a backdrop nobody can reach. It is also the only
    /// route to renaming a chat from inside the chat.
    #[test]
    fn the_settings_sheet_hands_a_rename_over_rather_than_stacking_on_top_of_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                open_chat(ctx);
                let mut config = ctx.config_options;
                config.set(vec![model_option()]);
            },
            chat(),
        );
        screen.press("title=\"Session settings\"");
        // The title leads the sheet, so it is the first row in it.
        screen.press("class=\"setting-row\"");
        let html = screen.markup();
        assert!(
            html.contains("<h2>Rename chat</h2>"),
            "the sheet's Title row did not open the rename field, so a chat \
             cannot be renamed from inside itself: {html}"
        );
        assert!(
            !html.contains("<h2>Session settings</h2>"),
            "the rename sheet opened ON TOP of the settings sheet, leaving a \
             backdrop under the keyboard that nobody can reach: {html}"
        );
        assert!(
            html.contains("value=\"Rotate the certificate\""),
            "the field is not filled in with the name being corrected, so \
             every rename starts from an empty box: {html}"
        );

        screen.press("class=\"btn secondary\"");
        assert!(
            !screen.markup().contains("Rename chat"),
            "Cancel left the rename sheet on screen: {}",
            screen.markup()
        );
    }

    /// It saves on the button and never on a keystroke — the app's standing
    /// rule for anything that writes to the server. What has to be true after
    /// the press is that the sheet is gone: one that stayed open would read as
    /// a save that did not take.
    #[test]
    fn renaming_saves_on_the_button_and_puts_the_sheet_away() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                open_chat(ctx);
                let mut config = ctx.config_options;
                config.set(vec![model_option()]);
            },
            chat(),
        );
        screen.press("title=\"Session settings\"");
        screen.press("class=\"setting-row\"");
        screen.type_into("class=\"field\"", "Certificate rotation, take two");
        assert!(
            screen
                .markup()
                .contains("value=\"Certificate rotation, take two\""),
            "the rename field does not show what was typed into it: {}",
            screen.markup()
        );

        screen.press("class=\"btn primary\"");
        assert!(
            !screen.markup().contains("Rename chat"),
            "Save left the rename sheet open, which reads as a save that did \
             not take: {}",
            screen.markup()
        );
    }

    /// A settings row drills into the values it can take, and picking one
    /// comes straight back to the list. A picker that stayed open would hide
    /// the change it just made behind itself.
    #[test]
    fn a_settings_row_drills_into_its_values_and_comes_back() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut config = ctx.config_options;
                config.set(vec![model_option()]);
            },
            chat(),
        );
        screen.press("title=\"Session settings\"");
        // No session, so no Title row: Model is the first row in the sheet.
        screen.press("class=\"setting-row\"");
        let drilled = screen.markup();
        assert!(
            drilled.contains("<h2>Model</h2>") && drilled.contains("Claude Opus 5"),
            "the Model row did not open the models the agent offers: {drilled}"
        );
        assert!(
            drilled.contains("class=\"choice selected\""),
            "the model in use is not marked as the current one, so the list \
             says nothing about where you are: {drilled}"
        );

        // The one that is not already selected.
        screen.press("class=\"choice\"");
        let back = screen.markup();
        assert!(
            back.contains("<h2>Session settings</h2>"),
            "picking a model left the drill-down open, hiding the sheet the \
             change was made in: {back}"
        );
        assert!(
            !back.contains("choice-list"),
            "the value list is still on screen after a value was picked: {back}"
        );
    }

    /// The mode picker is a one-shot: pick and it applies and closes. Mode is
    /// the setting you change mid-conversation, so leaving the picker up
    /// after a choice puts a sheet between the reader and the reply.
    #[test]
    fn choosing_a_mode_applies_it_and_puts_the_picker_away() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                open_chat(ctx);
                let mut config = ctx.config_options;
                config.set(vec![model_option(), mode_option()]);
            },
            chat(),
        );
        screen.press("title=\"Mode\"");
        assert!(screen.markup().contains("Select mode"));

        screen.press("class=\"choice\"");
        assert!(
            !screen.markup().contains("Select mode"),
            "picking a mode left the picker on screen: {}",
            screen.markup()
        );
    }

    /// Every sheet in this view closes on a tap beside it. That is the only
    /// dismissal an overlay with no visible close button has, and each one
    /// wires its own handler — so they are checked one by one rather than
    /// assumed from the first.
    #[test]
    fn a_tap_beside_a_sheet_closes_it() {
        let _alone = alone();
        let mut settings = Pressable::mount(
            |ctx| {
                open_chat(ctx);
                let mut config = ctx.config_options;
                config.set(vec![model_option(), mode_option()]);
            },
            chat(),
        );
        settings.press("title=\"Session settings\"");
        settings.press("class=\"modal-backdrop\"");
        assert!(
            !settings.markup().contains("<h2>Session settings</h2>"),
            "the settings sheet cannot be dismissed by tapping beside it: {}",
            settings.markup()
        );

        let mut mode = Pressable::mount(
            |ctx| {
                open_chat(ctx);
                let mut config = ctx.config_options;
                config.set(vec![model_option(), mode_option()]);
            },
            chat(),
        );
        mode.press("title=\"Mode\"");
        mode.press("class=\"modal-backdrop\"");
        assert!(
            !mode.markup().contains("Select mode"),
            "the mode picker cannot be dismissed by tapping beside it: {}",
            mode.markup()
        );

        let mut menu = Pressable::mount(open_chat, chat());
        menu.press("title=\"More\"");
        menu.press("class=\"modal-backdrop\"");
        assert!(
            !menu.markup().contains("Delete chat"),
            "the overflow menu cannot be dismissed by tapping beside it: {}",
            menu.markup()
        );
    }

    /// `session/delete` is not a soft delete and there is no undo, so the menu
    /// item asks before it acts — and it has to say what goes. Cancel is the
    /// half that must work: a confirmation you cannot back out of is not one.
    #[test]
    fn deleting_a_chat_asks_first_and_the_question_can_be_answered_either_way() {
        let _alone = alone();
        let mut backed_out = Pressable::mount(open_chat, chat());
        backed_out.press("title=\"More\"");
        backed_out.press("class=\"setting-row danger\"");
        let asked = backed_out.markup();
        assert!(
            asked.contains("<h2>Delete this chat?</h2>"),
            "picking Delete deleted it without asking: {asked}"
        );
        assert!(
            asked.contains("The whole conversation goes from the goose server")
                && asked.contains("This cannot be undone."),
            "the confirmation does not say what is about to be lost: {asked}"
        );
        assert!(
            !asked.contains("Delete chat"),
            "the menu is still open underneath its own confirmation: {asked}"
        );

        backed_out.press("class=\"btn secondary\"");
        assert!(
            !backed_out.markup().contains("Delete this chat?"),
            "Cancel left the confirmation on screen, so there is no way out \
             of it: {}",
            backed_out.markup()
        );

        let mut confirmed = Pressable::mount(open_chat, chat());
        confirmed.press("title=\"More\"");
        confirmed.press("class=\"setting-row danger\"");
        confirmed.press("class=\"btn danger\"");
        assert!(
            !confirmed.markup().contains("Delete this chat?"),
            "confirming left the dialog up, so the delete reads as one that \
             never happened: {}",
            confirmed.markup()
        );
    }

    /// Stop is a message to the agent, not a change of mind here. With no
    /// socket to send the cancel on there is nothing to report, and the turn
    /// is still running as far as anyone knows — so the screen must go on
    /// saying so. Clearing `running` on the press instead would be the app
    /// telling the reader something it has no way to know.
    #[test]
    fn stop_does_not_pretend_to_have_stopped_a_turn_it_could_not_reach() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut chat = ctx.chat;
                chat.set(ChatState {
                    session_id: Some("s-1".to_owned()),
                    running: true,
                    ..ChatState::default()
                });
            },
            chat(),
        );
        screen.press("title=\"Stop\"");
        let html = screen.markup();
        assert!(
            html.contains("<button class=\"send stop\" title=\"Stop\">"),
            "Stop cleared the turn locally, so a cancel that never left the \
             phone reads as one the agent acted on: {html}"
        );
        assert_eq!(
            html.matches("class=\"dot-anim\"").count(),
            3,
            "the typing indicator went away without the agent having been \
             told anything: {html}"
        );
    }

    /// The overflow is on the bar of every chat, including one that has never
    /// been opened and so has no session to delete. Confirming there has
    /// nothing to send, and the one thing it must not do is act as though it
    /// had: leaving for the chats list would report a deletion that never
    /// happened.
    #[test]
    fn deleting_a_chat_that_was_never_opened_deletes_nothing() {
        let _alone = alone();
        let mut screen = Pressable::mount(|_| {}, chat_and_state);
        screen.press("title=\"More\"");
        screen.press("class=\"setting-row danger\"");
        screen.press("class=\"btn danger\"");
        let html = screen.markup();
        assert!(
            !html.contains("Delete this chat?"),
            "the confirmation is still up after being answered: {html}"
        );
        assert!(
            html.contains("<p class=\"probe-left\">false</p>"),
            "confirming a delete with no session behind it walked back to the \
             chats list, which reports a deletion that never happened: {html}"
        );
    }
}
