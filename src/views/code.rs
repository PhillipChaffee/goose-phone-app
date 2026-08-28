//! Code tab views: the chat list (with lifecycle status), the new-session
//! form, the chat screen (cached-instant open, live streaming), the review
//! screen, the pull-request screen, and the permission modal for code chats.
//! Transcript items render through the same `chat::render_item` the Home tab
//! uses.

use std::collections::HashMap;

use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use opencode_client::{Checks, FileStatus, PullRequest};

use crate::attach::AttachTarget;
use crate::code::{
    answer_code_permission, ask_label, borrowed_agents_from, checks_label, delete_code_chat,
    ensure_code_agent_list, ensure_code_agents, ensure_code_branches, ensure_code_catalogue,
    ensure_code_models, expand_diff_gap, is_free_model, load_code_diff, mark_all_diff_seen,
    merge_pull, mergeability_label, new_code_chat, open_chat_allows_free_models, open_code_chat,
    open_code_pulls, pull_state_label, refresh_code_chats, refresh_code_permissions,
    repo_allows_free_models, reveal_removed_lines, send_code_prompt, set_code_agent,
    set_code_effort, set_code_model, start_code_poll, status_label, stop_code_turn,
    toggle_diff_file, toggle_diff_seen, BranchList, CodeScreen, DiffFile, DiffState,
    NewSessionSpec, NEW_CONVERSATION,
};
use crate::diff::Block;
use crate::external::open_external;
use crate::icons::Icon;
use crate::nav::Crumb;
use crate::state::{relative_time_secs, use_app_ctx, AppCtx, ConnState};
use crate::views::attach::{AttachButton, AttachTray};
use crate::views::chat::{format_tokens, render_transcript};
use crate::views::session_settings::{
    chip_effort, choice_label, mode_icon, ChoicePickerSheet, SessionSettingsSheet, SettingChoice,
    SettingRow,
};
use crate::views::{
    Confirm, ConfirmDelete, MenuItem, OverflowButton, OverflowSheet, ScrollToBottom, SwipeDelete,
};
use opencode_client::{
    resolve_agent, Agent, ChatMeta, CodePermission, ModelInfo, RepoEntry, DEFAULT_AGENT,
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
            // Named, so the three things that refresh a list know this one
            // has something to fetch and which fetch it is: the phone's pull
            // gesture, and on the desktop ⌘R and arriving here. They meet in
            // `viewport::refresh_named`.
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
        // The last-resort face, and still a statement rather than the name of
        // the control. The app has not been told which model this chat's
        // container is configured with — after `resolve_default_model` this is
        // only reached when `GET /config` was unavailable — so the turn will
        // carry no model and the container's own default runs. That is what
        // the word says. "Model" was the name of the control, which is the one
        // thing a chip must never be.
        return "Default".to_owned();
    };
    models
        .iter()
        .find(|m| m.reference() == reference)
        .map_or_else(
            || reference.rsplit('/').next().unwrap_or(reference).to_owned(),
            |m| m.name.clone(),
        )
}

/// The code tab's rows: two of the three settings `OpenCode` takes on a turn,
/// and the one number it only ever reports.
///
/// Model and thinking effort are both per-turn parameters of
/// `session/:id/prompt_async` (`model` and `variant`), which the server then
/// copies onto the session record — so "applies from your next message" is
/// literally the mechanism, not a hedge. Context length is not a parameter of
/// anything: it is catalogue metadata, and the one route that rewrites it
/// (`PATCH /config`) restarts the chat's server, killing the event stream the
/// app is reading. It is reported, not offered.
///
/// The goose sheet opens on a Provider row and this one does not, which is
/// the one place the two orders differ. There is no provider to choose here:
/// a model *is* `opencode/claude-sonnet-4-5`, provider and all, so picking
/// one picks both, and a Provider row would either duplicate the Model row or
/// offer a choice that decides nothing. Mode is missing from both, and for
/// the same reason on each — it is a chip in the composer row now.
fn code_setting_rows(ctx: &AppCtx, models: &[ModelInfo], loading: bool) -> Vec<SettingRow> {
    let (current, effort) = {
        let chat = ctx.code_chat.peek();
        (chat.model.clone(), chat.effort.clone())
    };
    let allow_free = open_chat_allows_free_models(ctx);

    let (offered, withheld) = offered_models(models, allow_free);
    let selected = current
        .as_deref()
        .and_then(|r| models.iter().find(|m| m.reference() == r));
    let unknown = || unknown_model_note(models, loading, current.is_some()).to_owned();

    let model_note = if let Some(note) = withheld_note(withheld) {
        Some(note)
    } else if models.is_empty() {
        Some(unknown())
    } else if current.is_none() {
        // The chip says "Default" and this is where that gets explained. Only
        // reachable once the catalogue is in hand and still nothing names a
        // model: the chat was created without one, has never been prompted,
        // and `GET /config` could not be read either.
        Some(
            "Running on the model this chat's container is configured with. \
             The app has not been told its name; picking one here names it \
             from your next message."
                .to_owned(),
        )
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
            "Fixed by the model. Nothing a message carries changes it.",
        ),
        None => SettingRow::fact("context_length", "Context length", "—", unknown()),
    });
    rows
}

/// The mode chip's face: the agent the next turn will run as.
///
/// Always a name, because a chat is always running as some agent — `OpenCode`
/// has no "no agent" state — so `Mode` was the app declining to answer a
/// question that always has an answer.
///
/// What it says now is the agent that *will* run: the reader's pick, the
/// session record's, or the one [`resolve_agent`] found in this server's own
/// list. Before that list lands it says `Build`, and that is not a guess
/// either — the prompt body carries no `agent` field until there is a resolved
/// name to put in it, and a turn that names none runs as
/// [`DEFAULT_AGENT`].
fn code_mode_label(agent: Option<&str>) -> String {
    let name = agent.unwrap_or(DEFAULT_AGENT);
    choice_label(name, name)
}

/// The selectable agents, in the order the server listed them.
///
/// Subagents are dropped: they exist to be invoked by another agent for a
/// sub-task, so pointing a chat at one would put the session on an agent that
/// cannot hold it. Each row carries the agent's own description — the field
/// its definition is meant to answer "when would I use this" with — and a row
/// whose author left it out is simply a name.
fn agent_choices(agents: &[Agent]) -> Vec<SettingChoice> {
    agents
        .iter()
        .filter(|a| a.is_primary())
        .map(|a| {
            SettingChoice::new(&a.name, choice_label(&a.name, &a.name))
                .with_note(a.description.clone())
                .with_icon(mode_icon(&a.name))
        })
        .collect()
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

/// The models a repo may see, and how many were withheld from it.
///
/// Zen's free models train on their input (privacy hard rule 1). The manager
/// refuses them when a chat is created against a repo that is not a public
/// throwaway, and a per-turn model rides through its transparent proxy with no
/// such check — so both the new-session picker and the session sheet filter
/// here rather than each having its own idea of the rule.
fn offered_models(models: &[ModelInfo], allow_free: bool) -> (Vec<&ModelInfo>, usize) {
    let offered: Vec<&ModelInfo> = models
        .iter()
        .filter(|m| allow_free || !is_free_model(&m.reference()))
        .collect();
    let withheld = models.len() - offered.len();
    (offered, withheld)
}

/// Why the list is shorter than the catalogue. Said plainly rather than
/// letting models silently go missing — in one voice, from one place.
fn withheld_note(withheld: usize) -> Option<String> {
    if withheld == 0 {
        return None;
    }
    let (count, verb) = if withheld == 1 {
        ("1 free model".to_owned(), "is")
    } else {
        (format!("{withheld} free models"), "are")
    };
    Some(format!(
        "{count} {verb} hidden — they train on their input, and this repo is \
         not a public throwaway."
    ))
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

/// Which pill's sheet is open. One signal rather than four booleans: only one
/// sheet can be up at a time, and four flags is four ways to say that wrongly.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NewPill {
    Repo,
    Branch,
    Model,
    Mode,
}

/// The field's placeholder: the sentence the reference writes there, naming
/// the parameters the pills below are holding.
///
/// It is the screen's title as well as its prompt. The topbar carries a
/// dismiss control and nothing else, because "New code session" said less than
/// this does and said it twice.
fn compose_placeholder(repo: &str, branch: Option<&str>) -> String {
    match (repo, branch) {
        ("", _) => "Start a task…".to_owned(),
        (repo, Some(branch)) => format!("Start a task in {repo} on branch {branch}…"),
        (repo, None) => format!("Start a task in {repo}…"),
    }
}

/// The repo pill's face: the bare name, owner dropped.
///
/// `PhillipChaffee/personal-ai-setup` is 31 characters on a row that also
/// carries a branch, and the owner is the half that is the same for every repo
/// you own. It is not lost: the picker states it under the name, and the
/// placeholder sentence above spells the whole thing out.
fn repo_chip_label(name: &str) -> &str {
    let bare = name.rsplit('/').next().unwrap_or(name);
    if bare.is_empty() {
        name
    } else {
        bare
    }
}

/// The branch pill's face. `Default` — not "Default branch" — when none has
/// been resolved: it is what `create_chat` with no base branch does, so it is
/// a value rather than a placeholder, and it fits a pill.
fn branch_chip_label(branch: Option<&str>) -> &str {
    branch.filter(|b| !b.is_empty()).unwrap_or("Default")
}

/// The model pill's face. `Model` before a choice, because until one is made
/// there is nothing true to say and the pill is the thing being asked for;
/// `Default` for the one case the picker offers the manager's own; the
/// catalogue name otherwise.
fn new_model_label(model: Option<&str>, models: &[ModelInfo]) -> String {
    match model {
        None => "Model".to_owned(),
        Some("") => "Default".to_owned(),
        Some(reference) => code_chip_label(Some(reference), models),
    }
}

/// Whether the session can be started.
///
/// The model is in here and it is the change: the old form called it
/// "Model (optional)", which meant most sessions ran on whatever the manager
/// happened to default to. It is the one parameter that decides what the work
/// costs, how good it is, and — through privacy hard rule 1 — who gets to see
/// the code, so it is chosen, not defaulted into.
///
/// The task is required where the chat composer takes a photo with nothing
/// said about it as a message, and the difference is not an oversight: this
/// text is also the session's name — the manager titles a chat from it, and
/// falls back to the chat id — so a session started on an attachment alone
/// arrives in the list called `personal-ai-setup-9f3403`. The attach button
/// beside the field is for the screenshot the sentence is *about*.
fn can_start(repo: &str, model: Option<&str>, task: &str) -> bool {
    !repo.is_empty() && model.is_some() && !task.trim().is_empty()
}

/// One repo per row: the bare name, the owner under it, and the one flag about
/// a repo this app acts on.
///
/// Owner *under* the name rather than above it, which is where the reference
/// puts it: this app's grammar is name-then-explanation everywhere else (the
/// mode picker, every settings row), and inverting one list would make the
/// same component read two ways.
fn repo_choices(repos: &[RepoEntry]) -> Vec<SettingChoice> {
    repos
        .iter()
        .map(|r| {
            let owner = r.name.rsplit_once('/').map(|(o, _)| o.to_owned());
            let note = match (owner, r.public_throwaway) {
                (Some(o), true) => Some(format!("{o} · public throwaway")),
                (Some(o), false) => Some(o),
                (None, true) => Some("public throwaway".to_owned()),
                (None, false) => None,
            };
            SettingChoice::new(&r.name, repo_chip_label(&r.name)).with_note(note)
        })
        .collect()
}

/// One branch per row, the repo's own marked `Default` and listed first — the
/// reference's grammar, and the only fact about a branch this screen knows
/// that the name does not already carry.
///
/// The manager already sorts and already puts the default first, so this only
/// has to mark it; filtering it out of the tail is what stops it appearing
/// twice.
fn branch_choices(list: &BranchList) -> Vec<SettingChoice> {
    let mut rows: Vec<SettingChoice> = Vec::with_capacity(list.names.len());
    if let Some(default) = list.default.as_deref() {
        rows.push(SettingChoice::new(default, default).with_note(Some("Default".to_owned())));
    }
    rows.extend(
        list.names
            .iter()
            .filter(|n| Some(n.as_str()) != list.default.as_deref())
            .map(|n| SettingChoice::new(n, n)),
    );
    rows
}

/// The rows the model sheet offers, and how many were withheld.
///
/// The empty catalogue is not an error state. `/config/providers` is a route
/// on a chat's own server, and a manager with no chat on it has none to ask —
/// so what is offered then is the one model that is reachable without a
/// catalogue, which is the manager's own default, as a real value rather than
/// as a field left blank. Choosing it is still choosing.
///
/// `loading` is the difference between "there is no catalogue" and "the
/// catalogue has not arrived", and it has to be asked because the fetch starts
/// on the same tap that opens this sheet — so on every first open the list is
/// empty and in flight at once. Offering the escape hatch then would make the
/// manager's default the fastest thing on screen every time, which is the
/// whole of what "a model is chosen, never defaulted into" is against. While
/// it is in flight this offers nothing and the sheet says why.
fn model_sheet_choices(
    models: &[ModelInfo],
    allow_free: bool,
    loading: bool,
) -> (Vec<SettingChoice>, usize) {
    if loading {
        return (Vec::new(), 0);
    }
    if models.is_empty() {
        return (
            vec![
                SettingChoice::new("", "The server's default model").with_note(Some(
                    "The manager picks it. The catalogue lives inside a session's \
                     container, and this one has none to ask yet."
                        .to_owned(),
                )),
            ],
            0,
        );
    }
    let (offered, withheld) = offered_models(models, allow_free);
    (model_choices(&offered), withheld)
}

/// What the mode pill starts on.
///
/// `None`, and the pill then reads `Build` — the agent a turn whose body names
/// none runs as, which is what a new session with nothing picked will do.
/// Resolved rather than hard-coded once the borrowed agent list lands.
const fn initial_mode() -> Option<String> {
    None
}

/// Point the session at `name`.
///
/// Three things belong to a repo rather than to the screen, and all three have
/// to move with it: its branches, the branch chosen out of them, and whether a
/// model that trains on its input may see it. That last one is the reason this
/// is a function rather than a `repo.set(...)` in the handler — a free model
/// picked while a public throwaway was selected must not ride into a repo that
/// is not one. The manager would refuse it at create time, and a refusal after
/// the fact is a worse way to learn hard rule 1 than the pill going blank in
/// front of you.
fn choose_repo(
    ctx: &AppCtx,
    name: &str,
    mut repo: Signal<String>,
    mut branch: Signal<Option<String>>,
    mut model: Signal<Option<String>>,
) {
    if *repo.peek() == name {
        return;
    }
    branch.set(None);
    // Bound to a local before the write: a `peek()` guard used as a match
    // scrutinee stays alive across the arms, and `set` on the same signal
    // inside one is what aborted the app the first time this shape was written
    // (see `send_code_prompt`).
    let picked = model.peek().clone();
    if let Some(reference) = picked {
        if !reference.is_empty() && is_free_model(&reference) && !repo_allows_free_models(ctx, name)
        {
            model.set(None);
            crate::state::show_toast(
                ctx,
                "That model trains on its input — cleared, because this repo is \
                 not a public throwaway.",
            );
        }
    }
    repo.set(name.to_owned());
    ensure_code_branches(ctx, name);
}

/// The new-session screen: a composer, not a form.
///
/// The reference app's flow, and the reason it is the right shape here: what
/// you are doing is writing the first message. The parameters of the session
/// are pills under the field, each opening a sheet, and the field's placeholder
/// is a sentence naming them — so the screen states its own configuration in
/// the one place you are already looking, instead of in three labelled boxes
/// you scroll past to reach the button.
///
/// What is deliberately absent. There is no **environment** pill: the reference
/// has cloud environments and this app has none — nothing in the manager, in
/// `repos.json` or in `ChatMeta` carries one — so a pill there would be chrome
/// that decides nothing (rule 11). There is no **microphone** either; voice is
/// out of scope. And there is no **thinking effort**: a tier belongs to a model
/// and `set_code_model` clears it on every switch, so picking one before the
/// model is settled is picking a value the next tap throws away. The chat's own
/// settings chip takes it from the first turn on.
/// What the window's own bar calls the draft screen.
///
/// The pane's own header has no title at all — it carries a dismiss control
/// and nothing else, because on a phone "New code session" said less than the
/// field's placeholder does and said it twice (see [`compose_placeholder`]).
/// The window's bar is a different surface: it is the only thing on the
/// desktop that says which of the seven destinations the window is in and what
/// it has open, and leaving it blank while a draft is up says neither. The
/// string is the one the screen already gives its own scroller as an
/// `aria-label`, so this is the name it already had rather than a new one.
pub(crate) fn new_crumb() -> Crumb {
    Crumb::plain("New code session")
}

#[component]
pub fn CodeNewView() -> Element {
    let ctx = use_app_ctx();
    let repos = (ctx.code_repos)();
    let models = (ctx.code_models)();
    let models_loading = (ctx.code_models_loading)();
    let agents = (ctx.code_agents)();
    let agents_loading = (ctx.code_agents_loading)();
    let branches = (ctx.code_branches)();

    let repo = use_signal(String::new);
    let mut branch = use_signal(|| None::<String>);
    let model = use_signal(|| None::<String>);
    let agent = use_signal(initial_mode);
    let mut task = use_signal(String::new);
    let mut sheet = use_signal(|| None::<NewPill>);

    // Default to the first allowlisted repo, as the form did — through
    // `choose_repo`, so the branch fetch it starts happens here too and the
    // pill can say `main` before anything is tapped.
    if repo.peek().is_empty() {
        if let Some(first) = repos.first() {
            choose_repo(&ctx, &first.name, repo, branch, model);
        }
    }
    // Seeded from the answer rather than fetched by the pill: the sentence in
    // the placeholder names the branch, so it has to be resolved before the
    // reader has any reason to open the sheet.
    if branch.peek().is_none() && branches.repo == *repo.peek() {
        if let Some(default) = branches.default.clone() {
            branch.set(Some(default));
        }
    }

    let placeholder = compose_placeholder(&repo(), branch().as_deref());
    let ready = can_start(&repo(), model().as_deref(), &task());
    // Resolved once and fed to all four places that would otherwise read the
    // raw signal — the chip's label, its icon, the picker's tick and the agent
    // the session is created with — exactly as `CodeChatView` does it. Reading
    // the signal raw in one place and `resolve_agent` in another is how the
    // chip came to say `Build` on a server whose list has no `build` while the
    // sheet it opened ticked `Plan`.
    //
    // `None` (no list yet) is not a gap: the label falls back to
    // `DEFAULT_AGENT` and the spec carries no agent, and those two agree —
    // a prompt body naming none runs as `build`.
    let resolved = resolve_agent(agent().as_deref(), &agents).map(str::to_owned);
    // A copy for the send handler, which is a `move` closure and so must own
    // what it puts in the spec; the chip below still has to read the original.
    let start_agent = resolved.clone();

    rsx! {
        header { class: "topbar",
            // A dismiss, not a back chevron: this screen has no stack of its
            // own and leaving it discards a draft, which is what ✕ means and
            // what ‹ does not. No title beside it — the placeholder below is
            // the sentence that names the screen.
            button {
                class: "icon-btn back",
                title: "Discard this session",
                "aria-label": "Discard this session",
                onclick: move |_| {
                    let mut screen = ctx.code_screen;
                    // Discard means discard: the draft dies with the scope,
                    // and so must the photos picked beside it. They are in a
                    // tray of their own, so this cannot reach into the chat
                    // you were last in.
                    ctx.new_attachments.clone().set(Vec::new());
                    screen.set(CodeScreen::List);
                },
                Icon { name: "close" }
            }
        }

        // The page IS the field. Not a `.scroll`: the textarea is its own
        // scroller, so a long draft scrolls inside it rather than sliding
        // under the pills, and there is nothing here to refresh — no
        // `data-refresh`, so neither the phone's pull nor the desktop's ⌘R
        // finds anything to fetch.
        main { class: "compose", "aria-label": "New code session",
            textarea {
                class: "compose-field",
                placeholder: "{placeholder}",
                value: "{task}",
                oninput: move |e| task.set(e.value()),
            }
        }

        // No card around it (`bare`): the field is not inside it, so a bordered
        // box here would be a rectangle drawn around six capsules that already
        // carry their own edges.
        footer { class: "composer bare",
            AttachTray { target: AttachTarget::Code, conversation: NEW_CONVERSATION.to_owned() }

            // Row one — what the session runs ON, and outlives every turn.
            div { class: "composer-row",
                div { class: "chip-row",
                    button {
                        class: "composer-chip action repo",
                        title: "Repository",
                        onclick: move |_| sheet.set(Some(NewPill::Repo)),
                        Icon { name: "repo" }
                        span { class: "chip-label",
                            span { class: "chip-name", "{repo_chip_label(&repo())}" }
                        }
                    }
                    button {
                        class: "composer-chip action branch",
                        title: "Base branch",
                        onclick: move |_| {
                            ensure_code_branches(&ctx, &repo.peek());
                            sheet.set(Some(NewPill::Branch));
                        },
                        Icon { name: "git-branch" }
                        span { class: "chip-label",
                            span { class: "chip-name", "{branch_chip_label(branch().as_deref())}" }
                        }
                    }
                }
            }

            // Row two — what its first turn runs AS, and the action. Same shape
            // as both chat composers: a one-line chip block, and a send button
            // outside it that stays pinned to the trailing edge.
            div { class: "composer-row",
                div { class: "chip-row",
                    AttachButton {
                        target: AttachTarget::Code,
                        conversation: NEW_CONVERSATION.to_owned(),
                    }
                    button {
                        // `needed` while it is empty: the send button beside it
                        // is disabled, and a disabled button says nothing about
                        // which of three pills is why. Rule 8 — state is a dot
                        // — and the same dot the list puts on a chat that is
                        // waiting on you.
                        class: if model().is_none() {
                            "composer-chip action model needed"
                        } else {
                            "composer-chip action model"
                        },
                        title: "Model — pick one to start",
                        onclick: move |_| {
                            ensure_code_catalogue(&ctx);
                            sheet.set(Some(NewPill::Model));
                        },
                        span { class: "chip-label",
                            span { class: "chip-model",
                                "{new_model_label(model().as_deref(), &models)}"
                            }
                        }
                        Icon { name: "chevron-down" }
                    }
                    button {
                        class: "composer-chip action mode",
                        title: "Mode",
                        onclick: move |_| {
                            ensure_code_agent_list(&ctx, &repo.peek());
                            sheet.set(Some(NewPill::Mode));
                        },
                        Icon { name: mode_icon(resolved.as_deref().unwrap_or(DEFAULT_AGENT)) }
                        span { class: "chip-label", "{code_mode_label(resolved.as_deref())}" }
                    }
                }
                button {
                    class: "send",
                    title: "Start the session",
                    disabled: !ready,
                    onclick: move |_| {
                        // Lifted before the create: by the time the first
                        // prompt goes out this screen is gone, replaced by the
                        // chat it made. `new_code_chat` empties the tray once
                        // the create has succeeded.
                        let files = ctx.new_attachments.peek().clone();
                        new_code_chat(
                            &ctx,
                            NewSessionSpec {
                                repo: repo.peek().clone(),
                                task: task.peek().trim().to_owned(),
                                model: model.peek().clone().filter(|m| !m.is_empty()),
                                // The resolved name, not the raw signal: what
                                // the chip claims is what the session is
                                // created with, and the first turn no longer
                                // depends on whether `GET /agent` for the new
                                // chat happened to land inside the second
                                // `new_code_chat` waits before sending.
                                agent: start_agent.clone(),
                                base_branch: branch.peek().clone(),
                            },
                            files,
                        );
                    },
                    Icon { name: "arrow-up" }
                }
            }
        }

        {new_session_sheet(
            &ctx,
            NewSheet { sheet, repo, branch, model, agent },
            &NewLists { repos: &repos, models: &models, agents: &agents, branches: &branches },
            models_loading,
            agents_loading,
        )}
    }
}

/// The five signals the new-session sheets write back into.
///
/// Grouped rather than passed one by one: they are one screen's worth of
/// state, they always travel together, and five bare `Signal`s in an argument
/// list is five chances to swap two of the same type.
#[derive(Clone, Copy)]
struct NewSheet {
    sheet: Signal<Option<NewPill>>,
    repo: Signal<String>,
    branch: Signal<Option<String>>,
    model: Signal<Option<String>>,
    agent: Signal<Option<String>>,
}

/// What the sheets read. Borrowed from the caller rather than re-read here:
/// every one of these is already `read` by the composer above, and reading
/// them again would subscribe the sheet to signals it is already subscribed
/// to.
struct NewLists<'a> {
    repos: &'a [RepoEntry],
    models: &'a [ModelInfo],
    agents: &'a [Agent],
    branches: &'a BranchList,
}

/// Whichever pill's sheet is open.
///
/// Built here rather than inline for the reason `code_setting_rows` is: this
/// screen re-renders on every keystroke in the field, and the model list is
/// n² in the catalogue with a `String` per model.
fn new_session_sheet(
    ctx: &AppCtx,
    mut s: NewSheet,
    lists: &NewLists<'_>,
    models_loading: bool,
    agents_loading: bool,
) -> Element {
    let ctx = *ctx;
    match (s.sheet)() {
        None => rsx! {},
        Some(NewPill::Repo) => {
            let count = lists.repos.len();
            rsx! {
                ChoicePickerSheet {
                    title: "Repositories ({count})",
                    backend: "code agent",
                    subtitle: "from the brain's allowlist",
                    choices: repo_choices(lists.repos),
                    current: Some((s.repo)()),
                    empty: "The manager's allowlist is empty — nothing to start a session on.",
                    onchoose: move |value: String| {
                        choose_repo(&ctx, &value, s.repo, s.branch, s.model);
                        s.sheet.set(None);
                    },
                    onclose: move |()| s.sheet.set(None),
                }
            }
        }
        Some(NewPill::Branch) => rsx! {
            ChoicePickerSheet {
                title: "Choose base branch",
                backend: "code agent",
                subtitle: "the session's own branch is cut from this one",
                // The manager stops at 500. Said above the rows, because with
                // a filter over a list that has been cut short, "Nothing
                // matches" about a branch that exists is a lie the reader has
                // no way to catch.
                note: lists.branches.truncated.then(|| {
                    format!(
                        "{} branches — this repo has more than the manager will \
                         read, so one that is missing here may still exist.",
                        lists.branches.names.len(),
                    )
                }),
                choices: branch_choices(lists.branches),
                current: (s.branch)(),
                empty: if lists.branches.loading {
                    "Asking GitHub for this repo's branches…"
                } else {
                    "This manager cannot list branches — the session starts on the repo's default."
                },
                onchoose: move |value: String| {
                    s.branch.set(Some(value));
                    s.sheet.set(None);
                },
                onclose: move |()| s.sheet.set(None),
            }
        },
        Some(NewPill::Model) => new_model_sheet(&ctx, s, lists.models, models_loading),
        Some(NewPill::Mode) => new_mode_sheet(&ctx, s, lists.agents, agents_loading),
    }
}

/// The model pill's sheet.
fn new_model_sheet(ctx: &AppCtx, mut s: NewSheet, models: &[ModelInfo], loading: bool) -> Element {
    let ctx = *ctx;
    let allow_free = repo_allows_free_models(&ctx, &(s.repo)());
    let (choices, withheld) = model_sheet_choices(models, allow_free, loading);
    // No loading line here: `fetch_models` never runs against a catalogue it
    // already has, so a fetch in flight is always an empty list, and the empty
    // state below is already saying it. Two sentences saying one thing is how
    // a sheet ends up with more chrome than choices.
    let note = withheld_note(withheld);
    rsx! {
        ChoicePickerSheet {
            title: "Select model",
            backend: "code agent",
            subtitle: "the session runs on this from its first message",
            note,
            choices,
            current: (s.model)(),
            // Two ways to reach this. In flight, where the note above says the
            // same thing in more words and the escape-hatch row is deliberately
            // not being offered yet. And settled, with a catalogue every model
            // of which this repo may not see — which is a dead end, so it says
            // the way out: the manager's own default is not offered as an
            // escape there, because it would be one of these same models with
            // the rule not applied to it.
            empty: if loading {
                "Asking a session's container for its model catalogue…"
            } else {
                "Every model this server offers trains on its input, and this \
                 repo is not a public throwaway — start this session on a repo \
                 flagged public_throwaway, or give the server a model that does \
                 not train."
            },
            onchoose: move |value: String| {
                s.model.set(Some(value));
                s.sheet.set(None);
            },
            onclose: move |()| s.sheet.set(None),
        }
    }
}

/// The mode pill's sheet.
fn new_mode_sheet(ctx: &AppCtx, mut s: NewSheet, agents: &[Agent], loading: bool) -> Element {
    let ctx = *ctx;
    // Whose list this is, said out loud when it is not this repo's. `GET
    // /agent` is a route on a chat's own server and this repo may have none, so
    // the app borrows one — and a repository can define agents of its own
    // (`.opencode/agent/`), which makes a borrowed list a good guess rather
    // than an answer. `ensure_code_agent_list` asks a container of this repo's
    // whenever one exists, so this line is the case where none did.
    let note = borrowed_agents_from(&ctx, &(s.repo)()).map(|donor| {
        format!(
            "Borrowed from {donor} — nothing on this repo to ask, and a \
             repository can define agents of its own."
        )
    });
    rsx! {
        ChoicePickerSheet {
            title: "Select mode",
            backend: "code agent",
            subtitle: "how the first turn runs",
            note,
            choices: agent_choices(agents),
            // Resolved, not raw — the same expression the chip's label and icon
            // are built from, so the tick and the pill cannot disagree about
            // which agent the first turn runs as.
            current: resolve_agent((s.agent)().as_deref(), agents).map(str::to_owned),
            empty: if loading {
                "Asking a session's container which agents it has…"
            } else {
                "No agent list yet — the session starts on the server's default."
            },
            onchoose: move |value: String| {
                s.agent.set(Some(value));
                s.sheet.set(None);
            },
            onclose: move |()| s.sheet.set(None),
        }
    }
}

/// The transcript's scroller, named so the pin and the scroll-to-bottom
/// button address the same element.
const SCROLL_ID: &str = "code-chat-scroll";

/// Where a code chat is: the repo, and the branch when there is one.
///
/// The same sentence the header below builds out of three rsx nodes, written
/// a second time rather than shared — and that is deliberate. Collapsing those
/// three into one interpolation would put a `<!--placeholder-->` where a
/// branchless repo renders nothing today, which is a change to the phone's DOM
/// for a screen this work is not about. `the_window_bar_says_what_the_pane_
/// says` below is what holds the two in step instead.
fn chat_where(repo: &str, branch: &str) -> Option<String> {
    match (repo, branch) {
        ("", _) => None,
        (repo, "") => Some(repo.to_owned()),
        (repo, branch) => Some(format!("{repo} · {branch}")),
    }
}

/// What the open code chat is called, once.
///
/// Read by two things that are never on screen together: the header below,
/// and — on the desktop — the window's own bar, which takes the heading out of
/// the pane and paints it in `.shell-chrome` instead
/// (`src/shell/desktop.rs`, `assets/desktop.css`).
pub(crate) fn chat_crumb(ctx: &AppCtx) -> Crumb {
    let chat = ctx.code_chat.read();
    Crumb::detailed(chat.title.clone(), chat_where(&chat.repo, &chat.branch))
}

#[component]
pub fn CodeChatView() -> Element {
    let ctx = use_app_ctx();
    let chat = (ctx.code_chat)();
    let mut draft = ctx.code_draft;

    use_effect(move || {
        let _ = ctx.code_chat.read().items.len();
        crate::viewport::pin_transcript(SCROLL_ID);
    });

    let running = chat.running;
    // Cached transcript is read-only until the server is authoritative (A5).
    let can_send = !running && !chat.waking && !chat.loading;
    // Which conversation the composer's picks belong to — see the goose
    // composer for why it is passed down rather than read inside the pieces.
    let conversation = chat.chat_id.clone().unwrap_or_default();

    let models = (ctx.code_models)();
    let models_loading = (ctx.code_models_loading)();
    let agents = (ctx.code_agents)();
    let agents_loading = (ctx.code_agents_loading)();
    let mut sheet = use_signal(|| false);
    let mut mode_sheet = use_signal(|| false);
    let mut chat_confirm_delete = use_signal(|| false);
    let mut menu = use_signal(|| false);
    let chip_label = code_chip_label(chat.model.as_deref(), &models);
    // Resolved once and fed to all three places that used to read `chat.agent`
    // raw — the label, the chip's icon and the picker's tick — so the chip,
    // its mark and the checked row can never disagree about which agent the
    // next turn runs as.
    let agent = resolve_agent(chat.agent.as_deref(), &agents).map(str::to_owned);
    let mode_label = code_mode_label(agent.as_deref());
    // No catalogue lookup: a model switch clears the tier (`set_code_model`)
    // and the server's own record arrives already filtered, so whatever is
    // here is a tier the next turn will really ask for.
    let effort = chip_effort(chat.effort.as_deref());
    // None until the fetch on chat open lands, and None for a session that has
    // changed nothing — the chip says "Diff" alone rather than "+0 −0", which
    // would be a claim it cannot back before the diff has been read.
    let diff_totals = {
        let d = ctx.code_diff.read();
        (!d.files.is_empty()).then(|| d.totals())
    };
    // Likewise None until GitHub has answered — and `0` is a claim too, so it
    // is only shown once there is an answer to back it.
    let pull_count = {
        let p = ctx.code_pulls.read();
        p.loaded.then(|| p.pulls.len())
    };

    let mut submit = move || {
        let text = draft.peek().trim().to_string();
        let files = ctx.code_attachments.peek().clone();
        if text.is_empty() && files.is_empty() {
            return;
        }
        // Emptied once the message is on its way; a request that then fails
        // is `send_code_prompt`'s to put right, since it is answered long
        // after this returns (see the goose composer for the same shape).
        if send_code_prompt(&ctx, text, &files) {
            draft.set(String::new());
            // Your own message always takes you back to the bottom, whatever
            // you had scrolled up to read.
            crate::viewport::scroll_to_bottom(SCROLL_ID);
            ctx.code_attachments.clone().set(Vec::new());
        }
    };

    let heading = chat_crumb(&ctx).title;

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
                // The same expression the window's bar reads. The substituted
                // value is byte-identical to the `{chat.title}` it replaces,
                // so the phone's captured markup is unchanged.
                h1 { class: "title ellipsis", "{heading}" }
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

        main { class: "scroll chat", id: SCROLL_ID,
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

        ScrollToBottom { scroller: SCROLL_ID }

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
            // Never disabled, unlike the composer beside it: the manager
            // answers this from GitHub, so it works while the container is
            // still waking and while a turn is running.
            button {
                class: "action-chip",
                title: "Pull requests from this branch",
                onclick: move |_| open_code_pulls(&ctx),
                Icon { name: "pull-request" }
                "Pull requests"
                if let Some(count) = pull_count {
                    span { class: "stat count", "{count}" }
                }
            }
        }

        footer { class: "composer",
            AttachTray { target: AttachTarget::Code, conversation: conversation.clone() }
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
                // This box is one line and never more. The send button is
                // outside it, which is what keeps it pinned to the trailing
                // edge whatever the chips inside do.
                div { class: "chip-row",
                    AttachButton { target: AttachTarget::Code, conversation }
                    button {
                        class: "composer-chip action model",
                        title: "Session settings",
                        onclick: move |_| {
                            ensure_code_models(&ctx);
                            sheet.set(true);
                        },
                        span { class: "chip-label",
                            span { class: "chip-model", "{chip_label}" }
                            if let Some(effort) = effort {
                                span { class: "chip-effort", "{effort}" }
                            }
                        }
                        Icon { name: "chevron-down" }
                    }
                    // Always offered, unlike goose's: whether this server has
                    // any agents is not known until the list is asked for, and
                    // an empty answer is reported inside the picker rather
                    // than by the chip going missing. The tap is a loud retry
                    // — `refresh_code_agents` has already asked quietly on
                    // open, because the chip's own label is resolved out of
                    // that list.
                    button {
                        class: "composer-chip action mode",
                        title: "Mode",
                        onclick: move |_| {
                            ensure_code_agents(&ctx);
                            mode_sheet.set(true);
                        },
                        Icon { name: mode_icon(agent.as_deref().unwrap_or(DEFAULT_AGENT)) }
                        span { class: "chip-label", "{mode_label}" }
                    }
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

        if mode_sheet() {
            ChoicePickerSheet {
                title: "Select mode",
                backend: "code agent",
                choices: agent_choices(&agents),
                // The resolved agent, not the raw one, so the picker opens
                // with a row already ticked rather than with nothing marked
                // while the chip beside it names something.
                current: agent,
                empty: if agents_loading {
                    "Asking the chat server which agents it has…"
                } else {
                    "This chat server offers no agent you can run a session on."
                },
                onchoose: move |value: String| {
                    set_code_agent(&ctx, &value);
                    mode_sheet.set(false);
                },
                onclose: move |()| mode_sheet.set(false),
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

/// The review screen: the session's cumulative diff, one collapsible band
/// per file.
///
/// **Unified, not split.** At 402px the two columns of a split view get
/// `(402 − 2×18 − 8) ÷ 2 = 179px`, about 24 monospace columns each.
/// `pub(crate) fn load_code_diff(` is 30 characters, so every real line wraps
/// on both sides — and to *different* heights on each side, which stops the
/// rows lining up. Lining the before and after up on one visual row is the
/// entire value of split view, so at this width it is not merely cramped, it
/// is self-defeating.
///
/// **Full width, and the only screen that is.** The scroller gives up the
/// page's gutter so each file's body can run edge to edge — 52 monospace
/// columns at 402pt where a card allowed 47 — while the head keeps that
/// gutter, because the head is chrome and the code is content. The soft-wrap
/// toggle is unaffected: a no-wrap body is still its own horizontal
/// scrollport, now one that starts at the screen's edge.
/// What the review screen counts under its own name.
///
/// One expression for the header's subtitle line and the window bar's.
fn diff_subtitle(ctx: &AppCtx) -> String {
    let diff = ctx.code_diff.read();
    let total = diff.files.len();
    let (added, removed) = diff.totals();
    if total == 0 {
        ctx.code_chat.read().title.clone()
    } else if total == 1 {
        format!("1 file · +{added} −{removed}")
    } else {
        format!("{total} files · +{added} −{removed}")
    }
}

/// What the window's own bar calls the review screen.
///
/// The title is a literal in both places rather than a shared constant,
/// because promoting the header's static text node to an interpolated one
/// would change the phone's captured markup for a screen whose rendering is
/// not what this is about. `the_window_bar_names_a_screen_the_way_the_pane_
/// does` below is what holds the two in step instead.
pub(crate) fn diff_crumb(ctx: &AppCtx) -> Crumb {
    Crumb::detailed("Review", Some(diff_subtitle(ctx)))
}

#[component]
pub fn CodeDiffView() -> Element {
    let ctx = use_app_ctx();
    let diff = (ctx.code_diff)();
    let wrap = (ctx.code_diff_wrap)();

    let total = diff.files.len();
    let reviewed = diff.reviewed();
    let subtitle = diff_subtitle(&ctx);
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

/// One file's band: a head you can scan, and a body that folds away
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

    // Only for a band that is actually showing them. Re-hunking every file on
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
            class: "diff-file",
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
                // One line, GitLab's: chevron, path, counts, checkbox. The path
                // is the only thing that gives way — everything else is either
                // a fixed control or a number that cannot be shortened.
                //
                // The status badge rides in .diff-stat rather than in
                // .diff-path for two reasons. For a binary file the badge *is*
                // the stat — there are no counts — so putting it there keeps
                // every head the same three-part shape and the checkbox column
                // never moves down the list. And .diff-path's whole shrink
                // budget is promised to the filename by .diff-name's measured
                // max-width; an unshrinkable badge sharing that line would
                // overflow it.
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
                // A trailing control inside a row-sized target: it stops the
                // click reaching the summary, the same way the list's trash
                // button does (design rule 9).
                //
                // Labelled, not a bare circle: a checked circle beside a
                // filename is the app's word for "reviewed" and nothing on
                // screen says so, which is a thing to be learned rather than
                // read. This screen borrows its whole grammar from GitLab's
                // review view, and GitLab writes the word beside a plain box —
                // which is exactly what this is.
                //
                // A real checkbox, not a lone tick: unchecked is an empty box,
                // so the unreviewed state carries no checkmark at all. A bare
                // tick that is present either way is what this replaced — it
                // read as decoration rather than as a control with a state.
                //
                // The box is drawn by CSS and only the tick is an icon. Drawing
                // the box as an SVG too put its fill somewhere the contrast
                // audit cannot follow — it walks up for the first opaque
                // background, and an SVG fill is not one, so it compared the
                // white tick against the head behind it and called 3.48:1
                // 1.08:1. A CSS background is a background, and measures.
                //
                // The tick is absent rather than transparent when unchecked:
                // an element held at opacity 0 is still measured, and it has
                // nothing to be measured against.
                //
                // The word is a <span> rather than a bare text node because
                // the button is a flex container: a bare node becomes an
                // anonymous flex item, which the gap does reach but no rule
                // ever can. It needs no class — .diff-seen styles it.
                button {
                    class: "diff-seen",
                    "aria-pressed": "{seen}",
                    title: if seen { "Reviewed — tap to unmark" } else { "Mark reviewed" },
                    onclick: move |e: Event<MouseData>| {
                        e.stop_propagation();
                        e.prevent_default();
                        toggle_diff_seen(&ctx, &path, fingerprint);
                    },
                    span { class: "cbox",
                        if seen {
                            Icon { name: "check" }
                        }
                    }
                    span { "Viewed" }
                }
            }
            // Rendered only when open: a closed band costs nothing, which is
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

/// The pull-request screen: what this chat's branch has on GitHub.
///
/// **Scoped to the branch, and it says so.** A repo has pull requests that
/// have nothing to do with this conversation — someone else's, an earlier
/// chat's, your own from last week — and a screen that listed them would make
/// the count on the chip a number about the repo rather than about this work.
/// The subtitle names the branch for the same reason.
///
/// Nothing here goes near the chat's container: the manager answers both
/// routes from GitHub with its own credential, so this screen works on a chat
/// that is fast asleep and does not wake it by being looked at.
/// Which branch's pull requests these are.
///
/// One expression for the header's subtitle line and the window bar's.
fn pulls_subtitle(ctx: &AppCtx) -> String {
    let chat = ctx.code_chat.read();
    match (chat.repo.as_str(), chat.branch.as_str()) {
        ("", "") => chat.title.clone(),
        (repo, "") => repo.to_owned(),
        ("", branch) => branch.to_owned(),
        (repo, branch) => format!("{repo} · {branch}"),
    }
}

/// What the window's own bar calls the pull-request screen. See
/// [`diff_crumb`] for why the title is a literal in both places.
pub(crate) fn pulls_crumb(ctx: &AppCtx) -> Crumb {
    Crumb::detailed("Pull requests", Some(pulls_subtitle(ctx)))
}

#[component]
pub fn CodePullsView() -> Element {
    let ctx = use_app_ctx();
    let state = (ctx.code_pulls)();
    let mut confirm = use_signal(|| None::<u64>);

    let subtitle = pulls_subtitle(&ctx);
    let show_empty = !state.loading && state.pulls.is_empty() && state.error.is_none();
    let confirming = confirm().and_then(|number| {
        state
            .pulls
            .iter()
            .find(|pull| pull.number == number)
            .cloned()
    });
    let rows = state
        .pulls
        .iter()
        .map(|pull| render_pull(&ctx, pull, state.merging, confirm));

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
                h1 { class: "title ellipsis", "Pull requests" }
                span { class: "subtitle ellipsis", "{subtitle}" }
            }
        }

        main {
            class: "scroll",
            "data-refresh": "pulls",
            "data-refreshing": "{state.loading}",
            if let Some(error) = state.error.as_ref() {
                p { class: "error-box", "{error}" }
            }
            if state.loading && state.pulls.is_empty() {
                p { class: "empty", "Asking GitHub…" }
            }
            if show_empty {
                p { class: "empty",
                    "Nothing from this branch yet. Ask the agent to push and open one — "
                    "the push is permission-gated, so it will ask you first."
                }
            }
            ul { class: "session-list", {rows} }
        }

        if let Some(pull) = confirming {
            Confirm {
                title: "Merge #{pull.number}?",
                body: merge_confirm_body(&pull),
                confirm_label: "Merge",
                danger: false,
                on_cancel: move |()| confirm.set(None),
                on_confirm: move |()| {
                    confirm.set(None);
                    merge_pull(&ctx, pull.number);
                },
            }
        }
    }
}

/// One pull request: what it is, where it stands, and the one thing that can
/// be done to it from a phone.
///
/// The row itself opens GitHub, the way a session row opens its session
/// (design rule 9) — everything a pull request is for beyond merging it lives
/// there, and none of it belongs in a 402px column. Merge stops the click so
/// the two do not fight.
fn render_pull(
    ctx: &AppCtx,
    pull: &PullRequest,
    merging: Option<u64>,
    mut confirm: Signal<Option<u64>>,
) -> Element {
    let ctx = *ctx;
    let number = pull.number;
    let url = pull.url.clone();
    let (state_dot, state_word) = pull_state_label(pull);
    let (checks_dot, checks_word) = checks_label(pull.checks);
    let merge_block = mergeability_label(pull);
    let offer_merge = pull.is_mergeable();
    let busy = merging == Some(number);

    rsx! {
        li {
            key: "{number}",
            class: "session-item",
            title: "Open on GitHub",
            onclick: move |_| {
                if crate::external::is_web_url(&url) {
                    open_external(&url);
                } else {
                    crate::state::show_toast(&ctx, "This pull request came without a web address");
                }
            },
            div { class: "session-swipe",
                div { class: "session-tile", Icon { name: "pull-request" } }
                div { class: "session-main",
                    div { class: "session-head",
                        div { class: "session-title", "{pull.title}" }
                        span { class: "session-age", "#{number}" }
                    }
                    div { class: "session-meta",
                        span { class: "chip",
                            span { class: "{state_dot}" }
                            "{state_word}"
                        }
                        span { class: "chip",
                            span { class: "{checks_dot}" }
                            "{checks_word}"
                        }
                        // The fourth reason merging is not offered, and the
                        // only one the two chips above cannot say. A chip
                        // rather than the paragraph this used to be: rule 11
                        // says do not render a control that does nothing, not
                        // that the reader must guess — and the row already has
                        // a grammar for a one-word fact.
                        if let Some((dot, word)) = merge_block {
                            span { class: "chip",
                                span { class: "{dot}" }
                                "{word}"
                            }
                        }
                    }
                    if offer_merge {
                        div { class: "pull-actions",
                            button {
                                class: "btn small primary",
                                disabled: busy,
                                onclick: move |e: Event<MouseData>| {
                                    e.stop_propagation();
                                    confirm.set(Some(number));
                                },
                                if busy { "Merging…" } else { "Merge" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// What the merge confirm says out loud.
///
/// The base branch and the check status are both in it because they are the
/// two facts that decide whether merging *now* is a good idea, and neither is
/// on the button. Pending checks especially: the merge is offered while they
/// run — they are checks that have not answered, not checks that said no —
/// and this is where that gets said.
fn merge_confirm_body(pull: &PullRequest) -> String {
    let base = if pull.base.is_empty() {
        "the base branch"
    } else {
        pull.base.as_str()
    };
    let checks = match pull.checks {
        Checks::Passing => "Its checks have passed.",
        Checks::Pending => "Its checks are still running.",
        Checks::None => "Nothing runs checks on this repo.",
        Checks::Unknown => "Its check results could not be read.",
        Checks::Failing => "Its checks are failing.",
    };
    format!(
        "“{}” merges into {base} on GitHub, straight away. {checks} \
         It cannot be undone from here.",
        pull.title
    )
}

#[cfg(test)]
mod tests {
    use super::{
        agent_choices, branch_chip_label, branch_choices, can_start, chat_ask, chat_where,
        code_chip_label, code_mode_label, compose_placeholder, mode_icon, model_choices,
        model_sheet_choices, new_crumb, new_model_label, repo_chip_label, repo_choices,
        resolve_agent, Agent, BranchList, CodePermission, ModelInfo, RepoEntry, DEFAULT_AGENT,
    };
    use opencode_client::AgentMode;

    /// The window's bar and the pane below it must call a screen the same
    /// thing, and for three of this file's screens nothing in the compiler
    /// makes them.
    ///
    /// The code chat is safe by construction — its heading reads `chat_crumb`
    /// directly. These three are literals written twice on purpose: promoting
    /// the headers' static text nodes to interpolated ones would change the
    /// phone's captured markup for screens this work is not about (see
    /// `diff_crumb`), and the New screen's name lives on an `aria-label`
    /// rather than in a heading at all.
    ///
    /// What this proves, exactly: each string the window's bar paints is the
    /// argument to a `Crumb` constructor AND the text of the pane's own name
    /// node — its `h1.title`, or for the New screen the `aria-label` that
    /// stands in for one. Two named halves, one assertion each, either of
    /// which can fail on its own.
    ///
    /// COUNTING WAS TRIED AND IT DOES NOT WORK, twice over. The first version
    /// read `include_str!("code.rs")` and asked for two occurrences of the
    /// title, one for the crumb and one for the header — but `include_str!` of
    /// this file includes this module, so `"Review".to_owned()` on the line
    /// below was one of the two it counted: delete
    /// `h1 { class: "title ellipsis", "Review" }` from `CodeDiffView` and the
    /// test passed. Scanning `code_of` fixes that one and not the general
    /// case, because a count is the wrong question: "Pull requests" is also
    /// the label on the chip that opens the screen, so it stands at three
    /// occurrences with the tests cut off and survives losing its heading at
    /// two. Naming the node is what a count was standing in for.
    ///
    /// The cost is that it is coupled to how the header is written. Change
    /// `h1.title.ellipsis` to something else and this goes red on a screen
    /// that is fine — red on a refactor, which is the direction a guard is
    /// allowed to be wrong in, and the message says which of the two shapes
    /// it was looking for.
    #[test]
    fn the_window_bar_names_a_screen_the_way_the_pane_does() {
        let source = crate::selfscan::code_of("src/views/code.rs", include_str!("code.rs"));
        for title in [
            new_crumb().title,
            "Review".to_owned(),
            "Pull requests".to_owned(),
        ] {
            let quoted = format!("{title:?}");
            assert!(
                source.contains(&format!("Crumb::detailed({quoted}"))
                    || source.contains(&format!("Crumb::plain({quoted}")),
                "no Crumb in views/code.rs is built from {quoted}, so the \
                 window's bar has stopped calling a screen that"
            );
            let heading = format!("h1 {{ class: \"title ellipsis\", {quoted} }}");
            let labelled = format!("\"aria-label\": {quoted}");
            assert!(
                source.contains(&heading) || source.contains(&labelled),
                "{quoted} is what the window's bar paints, and no screen in \
                 views/code.rs carries it as `{heading}` or as `{labelled}` — \
                 the bar calls a screen something the screen no longer calls \
                 itself"
            );
        }
    }

    /// The sentence the window's bar puts beside a code chat's title, which
    /// the header below it builds out of three rsx nodes instead. Written
    /// twice because collapsing those three into one interpolation would
    /// change the phone's DOM (see [`super::chat_where`]), so the shape of the
    /// two is all a test can hold — and the empty cases are where they would
    /// drift.
    #[test]
    fn a_code_chat_says_where_it_is_the_way_its_header_does() {
        assert_eq!(chat_where("", ""), None);
        assert_eq!(chat_where("", "goose/x"), None, "no repo, no sentence");
        assert_eq!(chat_where("acme/w", ""), Some("acme/w".to_owned()));
        assert_eq!(
            chat_where("acme/w", "goose/x"),
            Some("acme/w \u{b7} goose/x".to_owned()),
        );
    }

    fn agent(name: &str, mode: AgentMode, description: Option<&str>) -> Agent {
        Agent {
            name: name.to_owned(),
            description: description.map(str::to_owned),
            mode,
            built_in: true,
        }
    }

    /// A subagent is invoked by another agent, never chosen by a person —
    /// offering one would put the chat on an agent that cannot hold it.
    #[test]
    fn only_agents_a_session_can_run_on_are_offered() {
        let agents = [
            agent("build", AgentMode::Primary, Some("Full tool access.")),
            agent("reviewer", AgentMode::Subagent, Some("Reviews a diff.")),
            agent("general", AgentMode::All, None),
        ];
        let choices = agent_choices(&agents);
        let values: Vec<&str> = choices.iter().map(|c| c.value.as_str()).collect();
        assert_eq!(values, ["build", "general"]);
        assert_eq!(choices[0].label, "Build");
        assert_eq!(choices[0].note.as_deref(), Some("Full tool access."));
        assert_eq!(choices[0].icon.as_deref(), Some("wrench"));
        assert_eq!(choices[1].note, None);
    }

    /// A chat is always running as *some* agent — `OpenCode` has no "no agent"
    /// state — so the chip always names one. Before any list has landed it
    /// says `Build`, which is not a guess: the prompt body carries no `agent`
    /// field until there is a resolved name to put in it, and a turn that
    /// names none runs as `build`.
    #[test]
    fn the_mode_chip_always_names_an_agent() {
        assert_eq!(code_mode_label(None), "Build");
        assert_eq!(code_mode_label(Some("plan")), "Plan");
        let custom = code_mode_label(Some("accept-edits"));
        assert!(!custom.is_empty());
        assert_ne!(custom, "Mode", "the chip must never name the control");
    }

    /// A rename of the default must not silently drop the chip back to the
    /// generic bolt: `Build` is a mode that edits, and the wrench is what says
    /// so beside the word.
    #[test]
    fn the_mode_chip_never_wears_the_generic_mark() {
        assert_eq!(mode_icon(DEFAULT_AGENT), "wrench");
    }

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
    /// model the catalogue does not list. With no model known at all it states
    /// what will happen — the container's own default runs — rather than
    /// naming the control.
    #[test]
    fn the_chip_falls_back_to_the_bare_id() {
        assert_eq!(code_chip_label(None, &[]), "Default");
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

    // ------------------------------------------- the new-session composer

    /// Both new pills name an icon that exists. `Icon` renders nothing for a
    /// name it does not know, so a typo here would be a silently empty pill.
    #[test]
    fn the_new_session_pills_name_real_icons() {
        assert!(crate::icons::path_for("repo").is_some());
        assert!(crate::icons::path_for("git-branch").is_some());
    }

    /// The placeholder is the screen's title as well as its prompt, so it has
    /// to read as a sentence in all three states it can be in.
    #[test]
    fn the_placeholder_names_whatever_is_settled() {
        assert_eq!(compose_placeholder("", None), "Start a task…");
        assert_eq!(
            compose_placeholder("personal-ai-setup", None),
            "Start a task in personal-ai-setup…"
        );
        assert_eq!(
            compose_placeholder("personal-ai-setup", Some("main")),
            "Start a task in personal-ai-setup on branch main…"
        );
    }

    /// The pill drops the owner, which is the half that is the same for every
    /// repo you own — and survives a name that has none.
    #[test]
    fn the_repo_pill_drops_the_owner() {
        assert_eq!(
            repo_chip_label("PhillipChaffee/personal-ai-setup"),
            "personal-ai-setup"
        );
        assert_eq!(repo_chip_label("testrepo"), "testrepo");
        assert_eq!(repo_chip_label("owner/"), "owner/", "never an empty pill");
    }

    /// `Default` is a value, not a placeholder: it is exactly what creating
    /// with no base branch does.
    #[test]
    fn the_branch_pill_states_the_default_as_a_value() {
        assert_eq!(branch_chip_label(None), "Default");
        assert_eq!(branch_chip_label(Some("")), "Default");
        assert_eq!(branch_chip_label(Some("release/2.x")), "release/2.x");
    }

    /// Three faces, and only the first is the name of the control — which is
    /// allowed exactly here, because until one is picked there is nothing true
    /// to say and the pill is the thing being asked for.
    #[test]
    fn the_new_model_pill_says_what_is_settled() {
        assert_eq!(new_model_label(None, &[]), "Model");
        assert_eq!(new_model_label(Some(""), &[]), "Default");
        let known = model("opencode", "deepseek-v4-flash", "DeepSeek V4 Flash");
        assert_eq!(
            new_model_label(Some("opencode/deepseek-v4-flash"), &[known]),
            "DeepSeek V4 Flash"
        );
    }

    /// The whole of the change: a session cannot start without a model, even
    /// with a repo and a task in hand.
    #[test]
    fn a_session_cannot_start_without_a_model() {
        assert!(!can_start("repo", None, "do the thing"));
        assert!(!can_start("", Some("opencode/x"), "do the thing"));
        assert!(!can_start("repo", Some("opencode/x"), "   "));
        assert!(can_start("repo", Some("opencode/x"), "do the thing"));
        // The manager's own default is a real choice, so it starts a session.
        assert!(can_start("repo", Some(""), "do the thing"));
    }

    fn repo(name: &str, public_throwaway: bool) -> RepoEntry {
        RepoEntry {
            name: name.to_owned(),
            url: String::new(),
            edit_only: false,
            allow_push: false,
            public_throwaway,
        }
    }

    /// The owner rides as the note, and the one flag this app acts on is
    /// stated there too — it is what decides the model list.
    #[test]
    fn a_repo_row_carries_its_owner_and_its_flag() {
        let rows = repo_choices(&[
            repo("PhillipChaffee/personal-ai-setup", false),
            repo("scratch", true),
        ]);
        assert_eq!(rows[0].label, "personal-ai-setup");
        assert_eq!(rows[0].value, "PhillipChaffee/personal-ai-setup");
        assert_eq!(rows[0].note.as_deref(), Some("PhillipChaffee"));
        assert_eq!(rows[1].note.as_deref(), Some("public throwaway"));
    }

    /// The default is listed first and marked, and never twice — the manager
    /// already puts it at the front of `names`.
    #[test]
    fn the_default_branch_is_listed_once_and_marked() {
        let list = BranchList {
            repo: "r".to_owned(),
            default: Some("main".to_owned()),
            names: vec!["main".to_owned(), "release/2.x".to_owned()],
            truncated: false,
            loading: false,
        };
        let rows = branch_choices(&list);
        let values: Vec<&str> = rows.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(values, ["main", "release/2.x"]);
        assert_eq!(rows[0].note.as_deref(), Some("Default"));
        assert_eq!(rows[1].note, None);
    }

    /// A manager that could not name a default still gets a usable list.
    #[test]
    fn a_branch_list_with_no_default_still_lists() {
        let list = BranchList {
            repo: "r".to_owned(),
            default: None,
            names: vec!["main".to_owned()],
            truncated: false,
            loading: false,
        };
        assert_eq!(branch_choices(&list).len(), 1);
    }

    /// With no catalogue to show — no chat exists to ask one of — the picker
    /// offers the one model that is reachable without one, as a real value
    /// with the reason under it.
    #[test]
    fn an_empty_catalogue_still_offers_a_choice() {
        let (rows, withheld) = model_sheet_choices(&[], false, false);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].value, "",
            "the empty reference is the manager's default"
        );
        assert!(rows[0].note.is_some(), "and it says who picks it");
        assert_eq!(withheld, 0);
    }

    /// Privacy hard rule 1, applied to the repo selected on THIS screen: a
    /// model that trains on its input is offered only to a public throwaway.
    #[test]
    fn free_models_reach_a_throwaway_and_nothing_else() {
        let catalogue = [
            model("opencode", "deepseek-v4-flash", "DeepSeek V4 Flash"),
            model("opencode", "big-pickle", "Big Pickle"),
        ];
        let (private, withheld) = model_sheet_choices(&catalogue, false, false);
        assert_eq!(private.len(), 1);
        assert_eq!(private[0].label, "DeepSeek V4 Flash");
        assert_eq!(withheld, 1);

        let (throwaway, none_held) = model_sheet_choices(&catalogue, true, false);
        assert_eq!(throwaway.len(), 2);
        assert_eq!(none_held, 0);
    }

    /// While the catalogue is in flight the picker offers nothing at all.
    ///
    /// The fetch starts on the tap that opens this sheet, so an empty list and
    /// a request in flight are the same instant on every first open — and the
    /// escape hatch offered then would be the fastest row on screen every
    /// single time, which is the whole of what "a model is chosen, never
    /// defaulted into" is against.
    #[test]
    fn a_catalogue_in_flight_offers_nothing_yet() {
        let (rows, withheld) = model_sheet_choices(&[], false, true);
        assert!(rows.is_empty(), "the default is not offered while loading");
        assert_eq!(withheld, 0);
        // And the moment it settles empty, it is.
        assert_eq!(model_sheet_choices(&[], false, false).0.len(), 1);
    }

    /// The new-session chip and the sheet it opens are built from ONE
    /// resolution, so they cannot name different agents.
    ///
    /// The case that breaks a raw read is a server whose list has no `build`:
    /// the chip would say `Build` off `DEFAULT_AGENT` while the picker ticked
    /// the first primary, and no `Build` row would exist to tick.
    #[test]
    fn the_new_session_mode_chip_agrees_with_its_picker() {
        let list = [
            agent("general", AgentMode::Subagent, None),
            agent("plan", AgentMode::Primary, None),
            agent("review", AgentMode::Primary, None),
        ];
        let resolved = resolve_agent(None, &list);
        assert_eq!(resolved, Some("plan"), "no build on this server");
        assert_eq!(code_mode_label(resolved), "Plan");
        let rows = agent_choices(&list);
        assert!(
            rows.iter().any(|r| r.value == "plan"),
            "the ticked row is one the picker really offers"
        );
        assert!(
            !rows.iter().any(|r| r.value == DEFAULT_AGENT),
            "and the chip must not have named one it does not"
        );
    }
}
