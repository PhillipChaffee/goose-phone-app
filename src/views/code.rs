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
    repo_allows_free_models, reveal_removed_lines, row_checks_label, row_pull_word,
    send_code_prompt, set_code_agent, set_code_effort, set_code_model, start_code_poll,
    status_label, stop_code_turn, toggle_diff_file, toggle_diff_seen, BranchList, CodeScreen,
    DiffFile, DiffState, NewSessionSpec, NEW_CONVERSATION,
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
    // Read once for the whole list rather than once per row, and read here
    // rather than inside `render_code_row` so that the row stays a function of
    // its arguments. This is also what subscribes the list to the plane-wide
    // sweep, so a build that turns red repaints the rows without a navigation.
    let plane_pulls = (ctx.code_pulls)();

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
                            render_code_row(
                                &ctx,
                                meta,
                                turn,
                                chat_ask(&asks, &meta.id),
                                plane_pulls.plane_pull(&meta.id),
                                confirm_delete,
                            )
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

/// One chat's row: what it is, what its container is doing, what its branch
/// has open on GitHub, and — when it is parked on a permission — the ask
/// itself.
///
/// `pull` is the newest pull request off this branch, from the plane-wide
/// sweep (`code::refresh_plane_pulls`), or `None` when the sweep has not
/// reached this chat or the branch has none. Passed in rather than read here
/// so that the row stays a function of its arguments — the list already reads
/// the signal once for the whole list rather than once per row.
///
/// **What is still absent, and why** (issues #81, #82). The mockups' working
/// tree row also carries `+34 −11`, a file count, a commit count and an
/// ahead-count. None of them is on this wire. The diffstat exists only per
/// SESSION and only through the container (`CodeClient::diff`), so a sweep
/// for it would wake every sleeping tree; the commit and ahead counts have no
/// endpoint at all — `ChatMeta` is id/repo/title/branch/base/status/model/
/// `last_active` and nothing else. Both are one container-free GitHub call
/// away on the manager's side — `compare/<base>...<branch>` answers
/// `ahead_by`, `behind_by`, `total_commits` and the summed `+/-` in a single
/// request that wakes nothing, and `pull_to_wire` already receives
/// `commits`/`additions`/`deletions`/`changed_files` from GitHub and drops
/// them. Filed as `PhillipChaffee/personal-ai-setup#28` and `#29`. Until one
/// of those lands the row says the number it has and no others, because a
/// plausible one is worse than none.
fn render_code_row(
    ctx: &AppCtx,
    meta: &ChatMeta,
    running_turn: bool,
    ask: Option<(CodePermission, usize)>,
    pull: Option<&PullRequest>,
    mut confirm_delete: Signal<Option<String>>,
) -> Element {
    let ctx = *ctx;
    let meta = meta.clone();
    let id = meta.id.clone();
    let waiting = ask.is_some();
    let (dot, label) = status_label(&meta, running_turn, waiting);
    let build = pull.and_then(row_checks_label);
    let pull_word = pull.map(row_pull_word);

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
                        // The build, ahead of the repo and the branch on
                        // purpose: a red one is the only thing on this line
                        // that is asking for something, and `.session-meta`
                        // wraps, so a row that has one does not push anything
                        // off the end — it gains a second line.
                        if let Some((build_dot, build_word)) = build {
                            span { class: "chip",
                                span { class: "{build_dot}" }
                                "{build_word}"
                            }
                        }
                        span { "{meta.repo}" }
                        if !meta.branch.is_empty() {
                            span { "{meta.branch}" }
                        }
                        if let Some(word) = pull_word {
                            span { "{word}" }
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
/// (`src/shell/desktop/mod.rs`, `assets/desktop/`).
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
        agent_choices, branch_chip_label, branch_choices, can_start, chat_ask, chat_crumb,
        chat_where, choose_repo, code_chip_label, code_mode_label, code_setting_rows,
        compose_placeholder, diff_crumb, initial_mode, merge_confirm_body, mode_icon,
        model_choices, model_sheet_choices, new_crumb, new_model_label, new_session_sheet,
        offered_models, pulls_crumb, pulls_subtitle, repo_chip_label, repo_choices, resolve_agent,
        unknown_model_note, withheld_note, Agent, BranchList, CodeChatView, CodeDiffView,
        CodeNewView, CodePermission, CodePermissionModal, CodePullsView, CodeSessionsView,
        FileStatus, ModelInfo, NewLists, NewPill, NewSheet, PullRequest, RepoEntry,
        SessionSettingsSheet, DEFAULT_AGENT,
    };
    use crate::code::{CodeChatState, DiffFile, DiffState, FileView, PullsState};
    use crate::state::{use_app_ctx, AppCtx, ChatItem, ConnState};
    use crate::testkit::{render as mount, render_seeded as mount_seeded};
    use dioxus::prelude::*;
    use opencode_client::{AgentMode, ChatMeta, Checks, FileDiff, ModelLimit, PullState};

    /// A failure message in this module shows what came out instead, as
    /// `{markup:.400}` — a `Display` precision on a string is the first 400
    /// CHARACTERS, so a screen whose markup contains a `·` cannot make the
    /// message itself panic on a byte boundary. Written as an inline format
    /// argument rather than as a call to a truncating helper, because a helper
    /// call in an `assert!`'s arguments only ever runs when the assertion
    /// fails: ninety of them would be ninety lines this file could never cover.
    ///
    /// A mount, with the escapes `dioxus_ssr` applies to TEXT undone.
    ///
    /// Every apostrophe in this app's copy comes back as `&#39;`, and a suite
    /// that spelled it that way would be asserting on the escaper rather than
    /// on the words. It matters most for the NEGATIVE assertions: written
    /// against the raw markup, `!html.contains("The server's default model")`
    /// passes whatever the screen says, because that string can never appear.
    ///
    /// Only entities that stand for text are undone; the quotes around an
    /// attribute value are written literally by the renderer and are still
    /// there, so `class="…"` is still assertable.
    fn render(view: fn() -> Element) -> String {
        unescape(&mount(view))
    }

    fn render_seeded(seed: fn(&AppCtx), view: fn() -> Element) -> String {
        unescape(&mount_seeded(seed, view))
    }

    pub(super) fn unescape(html: &str) -> String {
        html.replace("&#39;", "'").replace("&#34;", "\"")
    }

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

    pub(super) fn agent(name: &str, mode: AgentMode, description: Option<&str>) -> Agent {
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

    pub(super) fn model(provider: &str, id: &str, name: &str) -> ModelInfo {
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

    // ------------------------------------------------ the notes a picker gives

    /// Four different reasons a model row has nothing to say, and they are not
    /// interchangeable: "not loaded yet" is a wait, "no list" is a server that
    /// cannot answer, "not in the catalogue" is a model running right now
    /// under a name this app cannot resolve, and the fourth is simply an
    /// unanswered question. Collapse any two and the row tells the reader to
    /// do the wrong thing — wait for a list that is never coming, or go
    /// looking for a fault where there is none.
    #[test]
    fn a_missing_catalogue_says_which_kind_of_missing_it_is() {
        assert_eq!(
            unknown_model_note(&[], true, false),
            "Available once the model list has loaded."
        );
        assert_eq!(
            unknown_model_note(&[], false, false),
            "The chat server did not offer a model list."
        );
        let known = [model("opencode", "sonnet", "Sonnet")];
        assert_eq!(
            unknown_model_note(&known, false, true),
            "This model is not in the chat server's catalogue."
        );
        assert_eq!(
            unknown_model_note(&known, false, false),
            "Pick a model above and this follows from it."
        );
    }

    /// The one sentence that admits models are being kept back. It has to
    /// count in English — one model "is" hidden, several "are" — and it has to
    /// be absent entirely when nothing was withheld, because a picker that
    /// always explained itself would be explaining a rule that is not in
    /// force on the repo in front of you.
    #[test]
    fn withheld_models_are_counted_in_english_or_not_mentioned() {
        assert_eq!(withheld_note(0), None);
        assert_eq!(
            withheld_note(1).as_deref(),
            Some(
                "1 free model is hidden \u{2014} they train on their input, and \
                 this repo is not a public throwaway."
            )
        );
        assert_eq!(
            withheld_note(3).as_deref(),
            Some(
                "3 free models are hidden \u{2014} they train on their input, and \
                 this repo is not a public throwaway."
            )
        );
    }

    /// Privacy hard rule 1, at the level both pickers share. A model that
    /// trains on its input leaves the list for a repo that is not a public
    /// throwaway, and the count of what left comes back with it — without that
    /// number the sheet would silently come up short and the reader would
    /// think the server simply had less to offer.
    #[test]
    fn a_private_repo_is_offered_only_the_models_that_do_not_train() {
        let catalogue = [
            model("opencode", "claude-sonnet-4-5", "Claude Sonnet 4.5"),
            model("opencode", "big-pickle", "Big Pickle"),
            model("opencode", "grok-code-free", "Grok Code Free"),
        ];
        let (offered, withheld) = offered_models(&catalogue, false);
        assert_eq!(withheld, 2);
        let ids: Vec<&str> = offered.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            ["claude-sonnet-4-5"],
            "both the named free model and the one caught by the `free` \
             substring net have to go"
        );

        let (everything, none_held) = offered_models(&catalogue, true);
        assert_eq!(everything.len(), 3, "a throwaway repo sees the lot");
        assert_eq!(none_held, 0);
    }

    /// What the mode pill reads before any list lands. A constant here rather
    /// than a literal in two places, so the pair below cannot drift.
    const DEFAULT_LABEL: &str = "Build";

    /// The mode pill starts on nothing chosen, and that is what makes it read
    /// `Build`: a turn whose body names no agent runs as `build`. Seeding it
    /// with a literal name instead would make the pill a claim about a server
    /// whose agent list has not been read yet.
    #[test]
    fn a_new_session_starts_with_no_agent_chosen() {
        assert_eq!(initial_mode(), None);
        assert_eq!(code_mode_label(initial_mode().as_deref()), DEFAULT_LABEL);
    }

    // ------------------------------------------------------- merging a pull

    pub(super) fn pull(
        number: u64,
        title: &str,
        state: PullState,
        checks: Checks,
        mergeable: Option<bool>,
    ) -> PullRequest {
        PullRequest {
            number,
            title: title.to_owned(),
            state,
            checks,
            mergeable,
            base: "main".to_owned(),
            url: "https://github.example/acme/infra/pull/42".to_owned(),
            ..PullRequest::default()
        }
    }

    /// The merge sheet is the last thing between a tap and a commit on GitHub,
    /// and neither fact that decides whether merging *now* is right is on the
    /// button: where it lands, and what the checks said.
    #[test]
    fn the_merge_confirm_names_the_base_and_the_checks() {
        let body = merge_confirm_body(&pull(
            7,
            "Rotate the cert",
            PullState::Open,
            Checks::Pending,
            Some(true),
        ));
        assert!(
            body.contains("\u{201c}Rotate the cert\u{201d}"),
            "the sheet must quote the pull request it is about: {body}"
        );
        assert!(
            body.contains("merges into main on GitHub, straight away."),
            "the base branch is where the merge lands and it is nowhere else \
             on screen: {body}"
        );
        assert!(
            body.contains("Its checks are still running."),
            "the merge is offered WHILE checks run, so this is the only place a \
             reader learns they have not answered yet: {body}"
        );
        assert!(
            body.contains("It cannot be undone from here."),
            "a one-way action has to say so: {body}"
        );
    }

    /// Each check state gets a sentence of its own — "nothing runs checks here"
    /// and "the checks could not be read" are opposite advice — and a pull
    /// request whose base the manager did not send still reads as English
    /// rather than as "merges into  on GitHub".
    #[test]
    fn every_check_state_has_its_own_sentence_and_a_missing_base_still_reads() {
        for (checks, sentence) in [
            (Checks::Passing, "Its checks have passed."),
            (Checks::Failing, "Its checks are failing."),
            (Checks::None, "Nothing runs checks on this repo."),
            (Checks::Unknown, "Its check results could not be read."),
        ] {
            let mut subject = pull(7, "t", PullState::Open, checks, Some(true));
            subject.base = String::new();
            let body = merge_confirm_body(&subject);
            assert!(
                body.contains(sentence),
                "this check state is being described as some other one: {body}"
            );
            assert!(
                body.contains("merges into the base branch on GitHub"),
                "a pull request with no base named still has to read as a \
                 sentence: {body}"
            );
        }
    }

    // -------------------------------------------------------- the chat list

    pub(super) fn chat_meta(
        id: &str,
        title: &str,
        repo: &str,
        branch: &str,
        status: &str,
    ) -> ChatMeta {
        ChatMeta {
            id: id.to_owned(),
            repo: repo.to_owned(),
            title: title.to_owned(),
            branch: branch.to_owned(),
            base: String::new(),
            status: status.to_owned(),
            model: None,
            last_active: 0.0,
        }
    }

    pub(super) fn permission(id: &str, title: &str) -> CodePermission {
        CodePermission {
            id: id.to_owned(),
            title: title.to_owned(),
            ..CodePermission::default()
        }
    }

    pub(super) fn connect(ctx: &AppCtx) {
        let mut conn = ctx.code_conn;
        conn.set(ConnState::Connected {
            agent: "opencode".to_owned(),
        });
    }

    /// Before a URL has been typed there is nothing to connect to, and the tab
    /// says where to type it. An empty list here would read as "you have no
    /// sessions" to someone who has plenty on a server the app cannot reach.
    #[test]
    fn an_unconfigured_code_tab_points_at_settings() {
        let html = render(|| rsx! { CodeSessionsView {} });
        assert!(
            html.contains("Set the code server URL and password in Settings"),
            "an offline Code tab must say what is missing: {html:.400}"
        );
        assert!(
            html.contains("conn-label\">offline<"),
            "and the badge must agree with it: {html:.400}"
        );
        assert!(
            !html.contains("New session"),
            "there is nothing to start a session on"
        );
    }

    /// A row is the whole of what a scroll down this list tells you: what the
    /// session is, which repo and branch it is on, and what its container is
    /// doing right now. `running` with no turn of ours in flight is idle, not
    /// working — the manager's index cannot see inside a container.
    #[test]
    fn a_code_row_states_the_repo_the_branch_and_what_the_container_is_doing() {
        let html = render_seeded(
            |ctx| {
                connect(ctx);
                let mut chats = ctx.code_chats;
                chats.set(vec![
                    chat_meta(
                        "c1",
                        "Rotate the Tailscale cert",
                        "acme/infra",
                        "agent/c1",
                        "running",
                    ),
                    chat_meta("c2", "Tidy the audit script", "acme/tools", "", "stopped"),
                ]);
            },
            || rsx! { CodeSessionsView {} },
        );
        assert!(
            html.contains("Rotate the Tailscale cert") && html.contains("Tidy the audit script"),
            "both seeded sessions have to be on screen: {html:.400}"
        );
        assert!(
            html.contains("acme/infra") && html.contains("agent/c1"),
            "a row that does not name its repo and branch cannot be told from \
             another session on the same repo: {html:.400}"
        );
        assert!(
            html.contains(">idle<"),
            "a container that is up with no turn of ours running is idle: {html:.400}"
        );
        assert!(
            html.contains(">asleep<"),
            "a stopped container reads as asleep, not as an error: {html:.400}"
        );
        assert!(
            !html.contains("No code sessions yet"),
            "the empty state rendered alongside two rows"
        );
        assert!(
            html.contains("New session"),
            "a connected list offers the way to start one"
        );
    }

    /// Issue #84: a row whose branch has a red build looked exactly like a row
    /// whose branch is green — both said "asleep" and nothing else — so the
    /// reader could not triage from the list and had to open every tree.
    ///
    /// Three rows, three answers, from one plane-wide sweep: the red build
    /// says so beside the container's own status, the landed branch names its
    /// number, and the tree with no pull request gains nothing at all. The
    /// last one is the assertion that matters most — it is what says the row
    /// is reporting the wire rather than decorating every row alike.
    #[test]
    fn a_row_whose_branch_has_a_red_build_no_longer_looks_like_a_green_one() {
        let html = render_seeded(
            |ctx| {
                connect(ctx);
                let mut chats = ctx.code_chats;
                chats.set(vec![
                    chat_meta(
                        "red",
                        "Add a --no-color flag",
                        "acme/app",
                        "agent/red",
                        "stopped",
                    ),
                    chat_meta(
                        "done",
                        "Fix the anchors",
                        "acme/notes",
                        "agent/done",
                        "stopped",
                    ),
                    chat_meta(
                        "bare",
                        "Tidy the audit script",
                        "acme/tools",
                        "agent/bare",
                        "stopped",
                    ),
                ]);
                let mut pulls = ctx.code_pulls;
                let mut state = PullsState::default();
                state.by_chat.insert(
                    "red".to_owned(),
                    vec![PullRequest {
                        number: 121,
                        state: PullState::Open,
                        checks: Checks::Failing,
                        mergeable: Some(true),
                        ..PullRequest::default()
                    }],
                );
                state.by_chat.insert(
                    "done".to_owned(),
                    vec![PullRequest {
                        number: 109,
                        state: PullState::Merged,
                        checks: Checks::Passing,
                        ..PullRequest::default()
                    }],
                );
                // Asked about, and the branch has none. The row must render
                // exactly as it did before this feature existed.
                state.by_chat.insert("bare".to_owned(), Vec::new());
                pulls.set(state);
            },
            || rsx! { CodeSessionsView {} },
        );
        assert!(
            html.contains("checks failing") && html.contains("#121 open"),
            "the red branch has to say both that it is open and that its build \
             failed: {html:.400}"
        );
        assert!(
            html.contains("dot err\"></span>checks failing"),
            "and the build has to carry the error dot, not just the words: \
             {html:.400}"
        );
        assert!(
            html.contains("#109 merged"),
            "a branch that has landed says so on the row: {html:.400}"
        );
        assert!(
            !html.contains("checks passing"),
            "a merged branch's green build is history and would be three \
             words on every finished row: {html:.400}"
        );
        assert_eq!(
            html.matches("#1").count(),
            2,
            "the tree with no pull request must gain nothing: {html:.400}"
        );
    }

    /// A chat parked on a permission is the one thing in this list that wants
    /// something from you, and the row has to say all of it: the dot on the
    /// tile, the status chip, the ask itself, and how many more of ITS OWN are
    /// behind it. It must not say "idle" — that line sat directly above the
    /// "Approve or deny" panel until the ask outranked the container status.
    #[test]
    fn a_row_with_an_ask_stops_claiming_it_is_idle() {
        let html = render_seeded(
            |ctx| {
                connect(ctx);
                let mut chats = ctx.code_chats;
                chats.set(vec![chat_meta(
                    "c1",
                    "Rotate the cert",
                    "acme/infra",
                    "agent/c1",
                    "running",
                )]);
                let mut asks = ctx.code_permissions;
                asks.set(vec![
                    ("c1".to_owned(), permission("p1", "Write to src/main.rs")),
                    ("c1".to_owned(), permission("p2", "Run cargo test")),
                    ("c2".to_owned(), permission("p3", "Push the branch")),
                ]);
            },
            || rsx! { CodeSessionsView {} },
        );
        assert!(
            html.contains("Approve or deny Write to src/main.rs"),
            "the front of this chat's queue is the ask on the card: {html:.400}"
        );
        assert!(
            !html.contains("Run cargo test"),
            "a card is not the place to work through a backlog — the rest is \
             counted, not listed"
        );
        assert!(
            html.contains("+1 more waiting"),
            "the count is this chat's own, and the third ask belongs to another \
             chat entirely: {html:.400}"
        );
        assert!(
            html.contains("session-tile attention"),
            "rule 8: the tile's dot is what makes a scroll answer \"which one \
             wants me\" without reading a word"
        );
        assert!(
            html.contains(">waiting on you<") && !html.contains(">idle<"),
            "an outstanding ask outranks the container status: {html:.400}"
        );
        assert!(
            html.contains(">Deny<") && html.contains(">Approve<"),
            "the two answers you can give without reading anything else: {html:.400}"
        );
        assert!(
            !html.contains("Always allow"),
            "the third answer changes what the agent may do unattended and \
             belongs to the modal, with the conversation in front of you"
        );
    }

    /// A connection that failed says what failed and offers the retry. It is
    /// not the offline state — that one sends you to Settings, and sending
    /// someone to Settings over a dropped TLS handshake is wrong advice.
    #[test]
    fn a_failed_code_connection_shows_the_error_and_a_way_back() {
        let html = render_seeded(
            |ctx| {
                let mut conn = ctx.code_conn;
                conn.set(ConnState::Failed("tls handshake failed".to_owned()));
            },
            || rsx! { CodeSessionsView {} },
        );
        assert!(
            html.contains("tls handshake failed"),
            "the server's own words are the only clue there is: {html:.400}"
        );
        assert!(
            html.contains(">Retry<"),
            "a failure with no retry leaves the tab dead until the app is \
             restarted: {html:.400}"
        );
        assert!(
            html.contains("conn-label\">error<"),
            "the badge has to agree with the box below it: {html:.400}"
        );
        assert!(
            !html.contains("Set the code server URL"),
            "a configured server that refused is not an unconfigured one"
        );
    }

    /// Connecting is neither offline nor connected, and saying either would be
    /// a claim the app cannot back yet: no Settings advice, no empty-list
    /// verdict, no way to start a session against a server that has not
    /// answered.
    #[test]
    fn a_connecting_code_tab_claims_nothing_yet() {
        let html = render_seeded(
            |ctx| {
                let mut conn = ctx.code_conn;
                conn.set(ConnState::Connecting);
            },
            || rsx! { CodeSessionsView {} },
        );
        assert!(
            html.contains("conn-label\">connecting\u{2026}<") && html.contains("dot busy"),
            "the badge is the whole of what this state says: {html:.400}"
        );
        assert!(
            !html.contains("Set the code server URL"),
            "connecting is not offline"
        );
        assert!(
            !html.contains("No code sessions yet"),
            "no verdict on a list nobody has read"
        );
        assert!(
            !html.contains("New session"),
            "nothing to create a session on until the server answers"
        );
    }

    /// "You have no sessions" is a verdict, and it is only true once the list
    /// has been read. Rendering it while the first fetch is in flight makes
    /// every cold open flash it before the rows arrive.
    #[test]
    fn an_empty_code_list_says_so_only_once_it_has_been_read() {
        let settled = render_seeded(connect, || rsx! { CodeSessionsView {} });
        assert!(
            settled.contains("No code sessions yet \u{2014} start one against a repo."),
            "a settled empty list says so and says what to do: {settled:.400}"
        );

        let inflight = render_seeded(
            |ctx| {
                connect(ctx);
                let mut loading = ctx.code_chats_loading;
                loading.set(true);
            },
            || rsx! { CodeSessionsView {} },
        );
        assert!(
            !inflight.contains("No code sessions yet"),
            "the verdict was rendered while the fetch was still in flight: {inflight:.400}"
        );
        assert!(
            inflight.contains("data-refreshing=\"true\""),
            "the scroller is what the pull gesture and \u{2318}R read the \
             in-flight state off: {inflight:.400}"
        );
    }

    // -------------------------------------------------- the permission modal

    /// The modal interrupts, so it is scoped to the conversation you are IN.
    /// With no chat open, or with a queue full of other containers' asks, it
    /// renders nothing at all — those are answered on their own cards.
    #[test]
    fn the_modal_stays_out_of_the_way_of_other_chats_asks() {
        assert_eq!(
            render(|| rsx! { CodePermissionModal {} }),
            "",
            "with no chat open there is nothing to be interrupted about"
        );

        let elsewhere = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    chat_id: Some("c1".to_owned()),
                    ..CodeChatState::default()
                });
                let mut asks = ctx.code_permissions;
                asks.set(vec![("c2".to_owned(), permission("p1", "Push the branch"))]);
            },
            || rsx! { CodePermissionModal {} },
        );
        assert_eq!(
            elsewhere, "",
            "an ask from a chat you are not in threw a modal over the one you \
             are"
        );
    }

    /// The open chat's ask, with the three answers, the session named the way
    /// the list names it, the tool's own arguments available, and the rest of
    /// THAT chat's queue counted.
    #[test]
    fn the_open_chats_ask_gets_the_modal_and_all_three_answers() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    chat_id: Some("c1".to_owned()),
                    ..CodeChatState::default()
                });
                let mut chats = ctx.code_chats;
                chats.set(vec![chat_meta(
                    "c1",
                    "Rotate the cert",
                    "acme/infra",
                    "agent/c1",
                    "running",
                )]);
                let mut asks = ctx.code_permissions;
                let mut first = permission("p1", "Write a file");
                first.metadata = serde_json::json!({ "target": "Cargo.toml" });
                asks.set(vec![
                    ("c1".to_owned(), first),
                    ("c1".to_owned(), permission("p2", "Run cargo test")),
                ]);
            },
            || rsx! { CodePermissionModal {} },
        );
        assert!(
            html.contains("Session: Rotate the cert"),
            "the sheet names the session by its title, not by its id: {html:.400}"
        );
        assert!(
            html.contains("modal-tool\">Write a file<"),
            "the ask's own title is what the sheet is about: {html:.400}"
        );
        assert!(
            html.contains("Cargo.toml") && html.contains("<pre>"),
            "the tool's arguments are the difference between approving a write \
             and approving A write: {html:.400}"
        );
        assert!(
            html.contains("Allow once") && html.contains("Always allow") && html.contains("Reject"),
            "all three answers belong here: {html:.400}"
        );
        assert!(
            html.contains("+1 more waiting"),
            "answering one of two must not look like answering the last: {html:.400}"
        );
    }

    /// A chat the index has not caught up with is still nameable, an ask with
    /// no title of its own is still named by the tool it is for, and an ask
    /// carrying no arguments must not render an empty disclosure triangle.
    #[test]
    fn an_ask_degrades_to_the_ids_it_has_rather_than_to_blanks() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    chat_id: Some("c9".to_owned()),
                    ..CodeChatState::default()
                });
                let mut asks = ctx.code_permissions;
                let mut bare = permission("p1", "");
                bare.kind = "bash".to_owned();
                asks.set(vec![("c9".to_owned(), bare)]);
            },
            || rsx! { CodePermissionModal {} },
        );
        assert!(
            html.contains("Session: c9"),
            "a chat missing from the index falls back to its id rather than to \
             \"Session: \": {html:.400}"
        );
        assert!(
            html.contains("modal-tool\">bash<"),
            "an untitled ask is named by its tool kind: {html:.400}"
        );
        assert!(
            !html.contains("<details"),
            "null metadata must not render an empty Details disclosure: {html:.400}"
        );
        assert!(!html.contains("more waiting"), "one ask is not a backlog");
    }

    // ------------------------------------------------ the new-session screen

    pub(super) fn seed_repos(ctx: &AppCtx) {
        let mut repos = ctx.code_repos;
        repos.set(vec![
            repo("PhillipChaffee/personal-ai-setup", false),
            repo("PhillipChaffee/scratch", true),
        ]);
    }

    /// The field's placeholder is a SENTENCE naming the repo and the branch, so
    /// both have to be settled before anything is tapped — otherwise the screen
    /// cannot describe itself until you have opened two sheets, which is the
    /// form this composer replaced.
    #[test]
    fn the_new_screen_settles_a_repo_and_a_branch_before_anything_is_tapped() {
        let html = render_seeded(
            |ctx| {
                seed_repos(ctx);
                let mut branches = ctx.code_branches;
                branches.set(BranchList {
                    repo: "PhillipChaffee/personal-ai-setup".to_owned(),
                    default: Some("main".to_owned()),
                    names: vec!["main".to_owned(), "release/2.x".to_owned()],
                    truncated: false,
                    loading: false,
                });
            },
            || rsx! { CodeNewView {} },
        );
        assert!(
            html.contains(
                "Start a task in PhillipChaffee/personal-ai-setup on branch main\u{2026}"
            ),
            "the first allowlisted repo and its default branch are what the \
             sentence is built from: {html:.400}"
        );
        assert!(
            html.contains("chip-name\">personal-ai-setup<"),
            "the repo pill drops the owner the placeholder above spells out: {html:.400}"
        );
        assert!(
            html.contains("chip-name\">main<"),
            "the branch pill says the resolved default rather than the word \
             \"Default\": {html:.400}"
        );
        assert!(
            html.contains("composer-chip action model needed"),
            "the one pill still holding the session back wears the dot, because \
             a disabled send button says nothing about which of three it is: {html:.400}"
        );
        assert!(
            html.contains("chip-model\">Model<"),
            "and it is the one place a chip may name its own control: {html:.400}"
        );
    }

    /// The pill and the picker it opens are built from ONE resolution, so on a
    /// server whose list has no `build` the pill must name an agent that list
    /// really has — not the default it would fall back to if the list were
    /// empty, which is how the chip came to say `Build` over a picker with no
    /// Build row in it.
    #[test]
    fn the_new_screens_mode_pill_names_an_agent_the_server_really_has() {
        let html = render_seeded(
            |ctx| {
                seed_repos(ctx);
                let mut agents = ctx.code_agents;
                agents.set(vec![
                    agent("plan", AgentMode::Primary, None),
                    agent("review", AgentMode::Primary, None),
                ]);
            },
            || rsx! { CodeNewView {} },
        );
        assert!(
            html.contains("chip-label\">Plan<"),
            "the pill has to name the first primary agent this server offers: {html:.400}"
        );
        assert!(
            !html.contains("chip-label\">Build<"),
            "the pill named an agent the picker beside it cannot offer"
        );
    }

    // ----------------------------------------------- the new-session sheets

    /// One screen's worth of new-session state, built where a renderer can hold
    /// it. `CodeNewView`'s own five signals are local to it and no seed can
    /// reach them, so the sheets are mounted from here instead — this is the
    /// same struct the composer hands `new_session_sheet`.
    fn new_sheet(pill: Option<NewPill>, repo: &str) -> NewSheet {
        let repo = repo.to_owned();
        NewSheet {
            sheet: use_signal(move || pill),
            repo: use_signal(move || repo),
            branch: use_signal(|| None),
            model: use_signal(|| None),
            agent: use_signal(|| None),
        }
    }

    fn repo_sheet_probe() -> Element {
        let ctx = use_app_ctx();
        let sheet = new_sheet(Some(NewPill::Repo), "PhillipChaffee/personal-ai-setup");
        let repos = (ctx.code_repos)();
        let branches = BranchList::default();
        new_session_sheet(
            &ctx,
            sheet,
            &NewLists {
                repos: &repos,
                models: &[],
                agents: &[],
                branches: &branches,
            },
            false,
            false,
        )
    }

    fn branch_sheet_probe() -> Element {
        let ctx = use_app_ctx();
        let sheet = new_sheet(Some(NewPill::Branch), "acme/infra");
        let branches = (ctx.code_branches)();
        new_session_sheet(
            &ctx,
            sheet,
            &NewLists {
                repos: &[],
                models: &[],
                agents: &[],
                branches: &branches,
            },
            false,
            false,
        )
    }

    fn model_sheet_probe() -> Element {
        let ctx = use_app_ctx();
        let sheet = new_sheet(Some(NewPill::Model), "acme/infra");
        let models = (ctx.code_models)();
        let loading = (ctx.code_models_loading)();
        let branches = BranchList::default();
        new_session_sheet(
            &ctx,
            sheet,
            &NewLists {
                repos: &[],
                models: &models,
                agents: &[],
                branches: &branches,
            },
            loading,
            false,
        )
    }

    fn mode_sheet_probe() -> Element {
        let ctx = use_app_ctx();
        let sheet = new_sheet(Some(NewPill::Mode), "acme/infra");
        let agents = (ctx.code_agents)();
        let loading = (ctx.code_agents_loading)();
        let branches = BranchList::default();
        new_session_sheet(
            &ctx,
            sheet,
            &NewLists {
                repos: &[],
                models: &[],
                agents: &agents,
                branches: &branches,
            },
            false,
            loading,
        )
    }

    fn closed_sheet_probe() -> Element {
        let ctx = use_app_ctx();
        let sheet = new_sheet(None, "acme/infra");
        let branches = BranchList::default();
        new_session_sheet(
            &ctx,
            sheet,
            &NewLists {
                repos: &[],
                models: &[],
                agents: &[],
                branches: &branches,
            },
            false,
            false,
        )
    }

    /// With no pill tapped there is no sheet. A backdrop rendered over the
    /// composer with nothing in it would swallow every tap on the screen.
    #[test]
    fn no_pill_open_means_no_sheet_at_all() {
        assert_eq!(render(closed_sheet_probe), "");
    }

    /// The repo sheet counts what it is offering in its own title, says whose
    /// list it is, and puts the owner under each name — the app's grammar is
    /// name-then-explanation, and the owner is what tells two forks apart.
    #[test]
    fn the_repo_sheet_counts_the_allowlist_and_states_each_owner() {
        let html = render_seeded(seed_repos, repo_sheet_probe);
        assert!(
            html.contains("Repositories (2)"),
            "the title carries the count: {html:.400}"
        );
        assert!(
            html.contains("from the brain's allowlist"),
            "whose list this is, said in place of \"applies from your next \
             message\" — there is no next message on this screen: {html:.400}"
        );
        assert!(
            html.contains(">personal-ai-setup<") && html.contains(">PhillipChaffee<"),
            "the bare name with the owner under it: {html:.400}"
        );
        assert!(
            html.contains("PhillipChaffee \u{b7} public throwaway"),
            "the one flag this app acts on is stated on the row it decides the \
             model list for: {html:.400}"
        );
    }

    /// The manager stops reading at 500 branches, and a filter over a list that
    /// has been cut short would answer "Nothing matches" about a branch that
    /// exists. Said above the rows so it is read before the filter is believed.
    #[test]
    fn a_truncated_branch_list_says_it_is_truncated() {
        let html = render_seeded(
            |ctx| {
                let mut branches = ctx.code_branches;
                branches.set(BranchList {
                    repo: "acme/infra".to_owned(),
                    default: Some("main".to_owned()),
                    names: vec!["main".to_owned(), "release/2.x".to_owned()],
                    truncated: true,
                    loading: false,
                });
            },
            branch_sheet_probe,
        );
        assert!(
            html.contains("Choose base branch"),
            "the branch pill's sheet: {html:.400}"
        );
        assert!(
            html.contains(
                "2 branches \u{2014} this repo has more than the manager will \
                 read, so one that is missing here may still exist."
            ),
            "a short list that looks complete is worse than no list: {html:.400}"
        );
        assert!(
            html.contains(">release/2.x<"),
            "the tail of the list is still offered: {html:.400}"
        );
    }

    /// "Asking GitHub" and "this manager cannot answer" want opposite responses
    /// — wait, or accept the repo's default — so the empty branch sheet has to
    /// know which one it is in.
    #[test]
    fn an_empty_branch_sheet_tells_a_wait_from_a_dead_end() {
        let inflight = render_seeded(
            |ctx| {
                let mut branches = ctx.code_branches;
                branches.set(BranchList {
                    repo: "acme/infra".to_owned(),
                    loading: true,
                    ..BranchList::default()
                });
            },
            branch_sheet_probe,
        );
        assert!(
            inflight.contains("Asking GitHub for this repo's branches\u{2026}"),
            "a fetch in flight reads as a wait: {inflight:.400}"
        );

        let settled = render(branch_sheet_probe);
        assert!(
            settled.contains(
                "This manager cannot list branches \u{2014} the session starts \
                 on the repo's default."
            ),
            "and a settled empty list says what happens instead: {settled:.400}"
        );
    }

    /// The model sheet offers nothing while the catalogue is in flight, and
    /// says so. The escape hatch — the manager's own default — is deliberately
    /// withheld then: the fetch starts on the tap that opens this sheet, so
    /// offering it would make it the fastest row on screen every single time.
    #[test]
    fn a_model_sheet_in_flight_offers_nothing_and_says_why() {
        let html = render_seeded(
            |ctx| {
                let mut loading = ctx.code_models_loading;
                loading.set(true);
            },
            model_sheet_probe,
        );
        assert!(
            html.contains("Asking a session's container for its model catalogue\u{2026}"),
            "the sheet has to account for its own empty list: {html:.400}"
        );
        assert!(
            !html.contains("The server's default model"),
            "the escape hatch must not be offered while the catalogue is still \
             coming"
        );
    }

    /// A catalogue every model of which this repo may not see is a dead end,
    /// and the sheet says the two ways out. The manager's own default is NOT
    /// offered here — it would be one of these same models with the rule not
    /// applied to it.
    #[test]
    fn a_catalogue_a_private_repo_may_not_see_names_the_way_out() {
        let html = render_seeded(
            |ctx| {
                let mut models = ctx.code_models;
                models.set(vec![
                    model("opencode", "big-pickle", "Big Pickle"),
                    model("opencode", "grok-code-free", "Grok Code Free"),
                ]);
            },
            model_sheet_probe,
        );
        assert!(
            html.contains("start this session on a repo flagged public_throwaway"),
            "a dead end has to say how to get out of it: {html:.400}"
        );
        assert!(
            !html.contains("The server's default model"),
            "the escape hatch would be one of the withheld models with the rule \
             not applied"
        );
    }

    /// When only SOME models are withheld the sheet lists the rest and says
    /// what went, in the same sentence the settings sheet uses.
    #[test]
    fn a_part_withheld_catalogue_lists_the_rest_and_says_what_went() {
        let html = render_seeded(
            |ctx| {
                let mut models = ctx.code_models;
                models.set(vec![
                    model("anthropic", "claude-sonnet-4-5", "Claude Sonnet 4.5"),
                    model("opencode", "big-pickle", "Big Pickle"),
                ]);
            },
            model_sheet_probe,
        );
        assert!(
            html.contains(">Claude Sonnet 4.5<"),
            "the models this repo may see are still offered: {html:.400}"
        );
        assert!(
            !html.contains(">Big Pickle<"),
            "a model that trains on its input reached a repo that is not a \
             public throwaway"
        );
        assert!(
            html.contains("1 free model is hidden"),
            "a list that is silently short reads as a server with less to \
             offer: {html:.400}"
        );
    }

    /// `GET /agent` is a route on a chat's own server, and a repo with no chat
    /// on it has none to ask — so the app borrows another repo's list. A
    /// repository can define agents of its own, which makes a borrowed list a
    /// good guess rather than an answer, and the sheet says whose it is.
    #[test]
    fn a_borrowed_agent_list_says_whose_it_is() {
        let borrowed = render_seeded(
            |ctx| {
                let mut agents = ctx.code_agents;
                agents.set(vec![
                    agent("build", AgentMode::Primary, Some("Full tool access.")),
                    agent("reviewer", AgentMode::Subagent, None),
                ]);
                let mut from = ctx.code_agents_from;
                from.set("c9".to_owned());
                let mut chats = ctx.code_chats;
                chats.set(vec![chat_meta(
                    "c9",
                    "Other work",
                    "other/x",
                    "",
                    "running",
                )]);
            },
            mode_sheet_probe,
        );
        assert!(
            borrowed.contains("Borrowed from other/x"),
            "a list from another repo passed off as this repo's is a claim the \
             app cannot back: {borrowed:.400}"
        );
        assert!(
            borrowed.contains(">Build<"),
            "the primary agents are still the choices: {borrowed:.400}"
        );
        assert!(
            !borrowed.contains(">Reviewer<"),
            "a subagent cannot hold a session and must not be offered"
        );

        let own = render_seeded(
            |ctx| {
                let mut agents = ctx.code_agents;
                agents.set(vec![agent("build", AgentMode::Primary, None)]);
            },
            mode_sheet_probe,
        );
        assert!(
            !own.contains("Borrowed from"),
            "a list that IS this repo's must not apologise for itself: {own:.400}"
        );
    }

    /// An empty agent list is not an error, and the two ways it can be empty
    /// want different words: one is a wait, the other is a server that has no
    /// agent to run a session on, in which case the session still starts.
    #[test]
    fn an_empty_mode_sheet_tells_a_wait_from_a_server_with_none() {
        let inflight = render_seeded(
            |ctx| {
                let mut loading = ctx.code_agents_loading;
                loading.set(true);
            },
            mode_sheet_probe,
        );
        assert!(
            inflight.contains("Asking a session's container which agents it has\u{2026}"),
            "a fetch in flight reads as a wait: {inflight:.400}"
        );

        let settled = render(mode_sheet_probe);
        assert!(
            settled.contains(
                "No agent list yet \u{2014} the session starts on the server's \
                 default."
            ),
            "and no list at all still leaves the session startable: {settled:.400}"
        );
    }

    // ------------------------------------------------ the chat settings sheet

    /// The code tab's settings sheet, mounted exactly as `CodeChatView` mounts
    /// it, so the rows under test are the rows the reader gets. The sheet's own
    /// open/closed state is local to the chat view and no seed can reach it.
    fn settings_sheet_probe() -> Element {
        let ctx = use_app_ctx();
        let models = (ctx.code_models)();
        let loading = (ctx.code_models_loading)();
        rsx! {
            SessionSettingsSheet {
                backend: "code agent",
                rows: code_setting_rows(&ctx, &models, loading),
                onchoose: move |_: (String, String)| {},
                onclose: move |()| {},
            }
        }
    }

    pub(super) fn sonnet() -> ModelInfo {
        let mut variants = std::collections::BTreeMap::new();
        variants.insert("low".to_owned(), serde_json::Value::Null);
        variants.insert("high".to_owned(), serde_json::Value::Null);
        ModelInfo {
            id: "claude-sonnet-4-5".to_owned(),
            provider_id: "anthropic".to_owned(),
            name: "Claude Sonnet 4.5".to_owned(),
            limit: ModelLimit { context: 200_000.0 },
            variants,
        }
    }

    /// With no catalogue and no model recorded, all three rows have to state
    /// what is unknown rather than showing blanks — and the effort and context
    /// rows must give the SAME reason as each other, because they are missing
    /// for the same one.
    #[test]
    fn a_settings_sheet_with_no_catalogue_states_what_it_does_not_know() {
        let html = render(settings_sheet_probe);
        assert!(
            html.contains("The chat server did not offer a model list."),
            "a settled empty catalogue is not a wait: {html:.400}"
        );
        assert!(
            html.contains("setting-value\">Default<"),
            "\"Default\" is the real value OpenCode records for a turn that \
             asked for no variant: {html:.400}"
        );
        assert!(
            html.contains("Thinking effort") && html.contains("Context length"),
            "all three rows are always present: {html:.400}"
        );
        assert!(
            html.contains("setting-value\">\u{2014}<"),
            "an unknown value is an em dash, not an empty span: {html:.400}"
        );
    }

    /// While the catalogue is in flight the same rows say so, which is a
    /// different instruction from "this server has no list".
    #[test]
    fn a_settings_sheet_waiting_on_the_catalogue_says_it_is_waiting() {
        let html = render_seeded(
            |ctx| {
                let mut loading = ctx.code_models_loading;
                loading.set(true);
            },
            settings_sheet_probe,
        );
        assert!(
            html.contains("Available once the model list has loaded."),
            "a fetch in flight must not read as a server that cannot answer: {html:.400}"
        );
    }

    /// With a model recognised in the catalogue every row has a real answer:
    /// its catalogue name, the tier the next turn will really ask for, and the
    /// context window the model is fixed at.
    #[test]
    fn a_recognised_model_fills_in_all_three_rows() {
        let html = render_seeded(
            |ctx| {
                let mut models = ctx.code_models;
                models.set(vec![sonnet(), model("opencode", "kimi-k2", "Kimi K2")]);
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    model: Some("anthropic/claude-sonnet-4-5".to_owned()),
                    effort: Some("high".to_owned()),
                    ..CodeChatState::default()
                });
            },
            settings_sheet_probe,
        );
        assert!(
            html.contains("setting-value\">Claude Sonnet 4.5<"),
            "the catalogue name, not the bare reference: {html:.400}"
        );
        assert!(
            html.contains("setting-value\">High<"),
            "the tier is shown as UI copy rather than as the wire's own word: {html:.400}"
        );
        assert!(
            html.contains("setting-value\">200k tokens<"),
            "the context window is catalogue metadata and is reported, not \
             offered: {html:.400}"
        );
        assert!(
            html.contains("Fixed by the model. Nothing a message carries changes it."),
            "and the row says why it is not a control: {html:.400}"
        );
        assert!(
            !html.contains("not in the chat server's catalogue"),
            "a model that IS in the catalogue was reported as missing from it"
        );
    }

    /// A model with no thinking-effort tiers says so. `OpenCode` returns no
    /// variants at all for several whole model families, so this is a normal
    /// answer and not a failure — and it must not borrow the wording for a
    /// catalogue that could not be read.
    #[test]
    fn a_model_without_tiers_says_it_has_none() {
        let html = render_seeded(
            |ctx| {
                let mut models = ctx.code_models;
                let mut kimi = model("opencode", "kimi-k2", "Kimi K2");
                kimi.limit = ModelLimit { context: 128_000.0 };
                models.set(vec![kimi]);
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    model: Some("opencode/kimi-k2".to_owned()),
                    ..CodeChatState::default()
                });
            },
            settings_sheet_probe,
        );
        assert!(
            html.contains("This model has no thinking-effort tiers."),
            "an empty variant list is a fact about the model, not a gap in the \
             catalogue: {html:.400}"
        );
        assert!(
            html.contains("setting-value\">128k tokens<"),
            "and the rest of the row still reports: {html:.400}"
        );
    }

    /// A chat running on a model the catalogue does not list is a real state —
    /// the session record names it and `/config/providers` never mentioned it —
    /// and the effort row is the one that has to explain why it can offer
    /// nothing.
    #[test]
    fn a_model_outside_the_catalogue_is_named_as_such() {
        let html = render_seeded(
            |ctx| {
                let mut models = ctx.code_models;
                models.set(vec![sonnet()]);
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    model: Some("opencode/ghost-model".to_owned()),
                    ..CodeChatState::default()
                });
            },
            settings_sheet_probe,
        );
        assert!(
            html.contains("This model is not in the chat server's catalogue."),
            "the reader is told the app cannot resolve what is running, rather \
             than being shown a blank: {html:.400}"
        );
        assert!(
            html.contains("setting-value\">Opencode/ghost-model<"),
            "and the row still names what the session record says is running: {html:.400}"
        );
    }

    /// The chip says "Default" and this row is where that gets explained: the
    /// chat was created without a model, has never been prompted, and
    /// `GET /config` could not be read either. Only reachable with the
    /// catalogue in hand, which is what makes it a statement rather than a
    /// guess.
    #[test]
    fn a_chat_with_no_model_named_explains_what_will_run() {
        let html = render_seeded(
            |ctx| {
                let mut models = ctx.code_models;
                models.set(vec![sonnet()]);
            },
            settings_sheet_probe,
        );
        assert!(
            html.contains("Running on the model this chat's container is configured with."),
            "the sheet has to account for a chip that says Default: {html:.400}"
        );
    }

    /// The withheld-models sentence outranks every other note on the Model row,
    /// because it is the only one that says the list itself is short.
    #[test]
    fn the_withheld_sentence_outranks_the_other_model_notes() {
        let html = render_seeded(
            |ctx| {
                let mut repos = ctx.code_repos;
                repos.set(vec![repo("acme/infra", false)]);
                let mut models = ctx.code_models;
                models.set(vec![
                    sonnet(),
                    model("opencode", "big-pickle", "Big Pickle"),
                ]);
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    repo: "acme/infra".to_owned(),
                    ..CodeChatState::default()
                });
            },
            settings_sheet_probe,
        );
        assert!(
            html.contains("1 free model is hidden"),
            "the sheet has to admit the list is short: {html:.400}"
        );
        assert!(
            !html.contains("Running on the model this chat's container is configured with."),
            "two notes on one row is how a row ends up with more explanation \
             than value"
        );
    }

    // -------------------------------------------------------- the chat screen

    /// The heading, the subtitle and the window bar's crumb are three readings
    /// of one chat, and the pane's own heading is literally `chat_crumb`'s
    /// title. This mounts the crumb beside the pane so a change to either half
    /// shows up as a disagreement between them.
    fn chat_crumb_probe() -> Element {
        let ctx = use_app_ctx();
        let crumb = chat_crumb(&ctx);
        let subtitle = crumb.subtitle.clone().unwrap_or_default();
        rsx! {
            span { class: "probe", "{crumb.title}|{subtitle}" }
            CodeChatView {}
        }
    }

    fn seed_open_chat(ctx: &AppCtx) {
        let mut chat = ctx.code_chat;
        chat.set(CodeChatState {
            chat_id: Some("c1".to_owned()),
            title: "Rotate the Tailscale cert".to_owned(),
            repo: "acme/infra".to_owned(),
            branch: "agent/c1".to_owned(),
            items: vec![
                ChatItem::User {
                    text: "rotate the cert".to_owned(),
                    attachments: Vec::new(),
                },
                ChatItem::Assistant {
                    message_id: None,
                    text: "Rotated it.".to_owned(),
                },
            ],
            ..CodeChatState::default()
        });
    }

    /// A code chat names itself and says where it is, in the pane and in the
    /// window's bar, from one expression — and the transcript under it is the
    /// conversation, not a placeholder.
    #[test]
    fn an_open_code_chat_names_itself_and_where_it_is() {
        let html = render_seeded(seed_open_chat, chat_crumb_probe);
        assert!(
            html.contains("probe\">Rotate the Tailscale cert|acme/infra \u{b7} agent/c1<"),
            "the window's bar has stopped describing the chat the pane has \
             open: {html:.400}"
        );
        assert!(
            html.contains("<h1 class=\"title ellipsis\">Rotate the Tailscale cert</h1>"),
            "the pane's own heading is the crumb's title: {html:.400}"
        );
        assert!(
            html.contains("acme/infra") && html.contains("agent/c1"),
            "the subtitle carries the repo and the branch: {html:.400}"
        );
        assert!(
            html.contains("rotate the cert") && html.contains("Rotated it."),
            "the transcript is the screen: {html:.400}"
        );
        assert!(
            !html.contains("Loading history\u{2026}"),
            "a loaded transcript must not also claim to be loading"
        );
        assert!(
            !html.contains("dot-anim"),
            "nothing is running, so there is no typing indicator"
        );
    }

    /// A container that is asleep is woken behind the cached transcript, and
    /// the screen has to say so — otherwise cached history reads as a live
    /// conversation that has stopped answering. The composer is closed while it
    /// wakes, because a cached transcript is read-only until the server is
    /// authoritative.
    #[test]
    fn a_waking_chat_says_the_transcript_is_cached_and_closes_the_composer() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    chat_id: Some("c1".to_owned()),
                    title: "Rotate the cert".to_owned(),
                    waking: true,
                    ..CodeChatState::default()
                });
            },
            || rsx! { CodeChatView {} },
        );
        assert!(
            html.contains("Waking the container \u{2014} showing the cached transcript\u{2026}"),
            "a cached transcript passed off as live is the whole failure this \
             banner exists for: {html:.400}"
        );
        assert!(
            html.contains("placeholder=\"Waking\u{2026}\""),
            "the field says why it is closed: {html:.400}"
        );
    }

    /// An empty transcript that is still loading says so; one that has items
    /// already must not, or every open of a cached chat flashes "Loading
    /// history" over history that is on screen.
    #[test]
    fn loading_history_is_said_only_when_there_is_nothing_to_show() {
        let cold = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    chat_id: Some("c1".to_owned()),
                    loading: true,
                    ..CodeChatState::default()
                });
            },
            || rsx! { CodeChatView {} },
        );
        assert!(
            cold.contains("Loading history\u{2026}"),
            "a cold open with nothing cached has to say what it is doing: {cold:.400}"
        );

        let warm = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    chat_id: Some("c1".to_owned()),
                    loading: true,
                    items: vec![ChatItem::Assistant {
                        message_id: None,
                        text: "Cached answer.".to_owned(),
                    }],
                    ..CodeChatState::default()
                });
            },
            || rsx! { CodeChatView {} },
        );
        assert!(
            warm.contains("Cached answer.") && !warm.contains("Loading history\u{2026}"),
            "a cached transcript is shown instantly and the loading line stays \
             out of its way: {warm:.400}"
        );
    }

    /// A turn in flight replaces send with stop. Both at once would offer to
    /// queue a message the container cannot take; neither would leave a running
    /// turn unstoppable.
    #[test]
    fn a_running_turn_swaps_send_for_stop() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    chat_id: Some("c1".to_owned()),
                    running: true,
                    ..CodeChatState::default()
                });
            },
            || rsx! { CodeChatView {} },
        );
        assert!(
            html.contains("dot-anim"),
            "a running turn shows the transcript is alive: {html:.400}"
        );
        assert!(
            html.contains("send stop") && html.contains("title=\"Stop\""),
            "a running turn has to be stoppable: {html:.400}"
        );
        assert!(
            !html.contains("title=\"Send\""),
            "send and stop were offered at the same time"
        );
    }

    /// "+0 −0" and a pull-request count of 0 are claims, and before either
    /// fetch lands the app cannot back them — so the chips carry no numbers at
    /// all until there is an answer, and the real ones once there is.
    #[test]
    fn the_action_chips_carry_numbers_only_once_there_is_an_answer() {
        let silent = render(|| rsx! { CodeChatView {} });
        assert!(
            silent.contains("Diff</button>") && silent.contains("Pull requests</button>"),
            "both chips are always offered — the pull chip is answered from \
             GitHub and works on a chat that is fast asleep: {silent:.400}"
        );
        assert!(
            !silent.contains("stat add") && !silent.contains("stat count"),
            "a count nobody has read is not zero: {silent:.400}"
        );

        let answered = render_seeded(
            |ctx| {
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files: vec![
                        diff_file("src/a.rs", FileStatus::Modified, 9, 2, MODIFIED_PATCH),
                        diff_file("src/b.rs", FileStatus::Added, 3, 1, MODIFIED_PATCH),
                    ],
                    ..DiffState::default()
                });
                let mut pulls = ctx.code_pulls;
                pulls.set(PullsState {
                    pulls: vec![
                        pull(42, "Rotate", PullState::Open, Checks::Passing, Some(true)),
                        pull(43, "Retry", PullState::Merged, Checks::Passing, None),
                    ],
                    loaded: true,
                    ..PullsState::default()
                });
            },
            || rsx! { CodeChatView {} },
        );
        assert!(
            answered.contains("stat add\">+12<") && answered.contains("stat del\">\u{2212}3<"),
            "the diff chip totals every file in the session's diff: {answered:.400}"
        );
        assert!(
            answered.contains("stat count\">2<"),
            "the pull chip counts what GitHub answered for this branch: {answered:.400}"
        );
    }

    /// The composer's two chips name what the NEXT message will run on. The
    /// model comes from the catalogue when it is known, the tier is shortened
    /// to fit beside it, and the mode is the resolved agent — the same
    /// resolution the picker ticks.
    #[test]
    fn the_composer_chips_name_what_the_next_message_runs_on() {
        let html = render_seeded(
            |ctx| {
                let mut models = ctx.code_models;
                models.set(vec![sonnet()]);
                let mut agents = ctx.code_agents;
                agents.set(vec![agent("plan", AgentMode::Primary, None)]);
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    chat_id: Some("c1".to_owned()),
                    model: Some("anthropic/claude-sonnet-4-5".to_owned()),
                    effort: Some("medium".to_owned()),
                    agent: Some("plan".to_owned()),
                    ..CodeChatState::default()
                });
            },
            || rsx! { CodeChatView {} },
        );
        assert!(
            html.contains("chip-model\">Claude Sonnet 4.5<"),
            "the chip names the model, not its reference: {html:.400}"
        );
        assert!(
            html.contains("chip-effort\">Med<"),
            "`medium` is shortened because the model name beside it needs the \
             room: {html:.400}"
        );
        assert!(
            html.contains("chip-label\">Plan<"),
            "the mode chip names the resolved agent: {html:.400}"
        );

        let bare = render(|| rsx! { CodeChatView {} });
        assert!(
            bare.contains("chip-model\">Default<"),
            "with no model known the chip states what will happen rather than \
             naming the control: {bare:.400}"
        );
        assert!(
            !bare.contains("chip-effort"),
            "a default tier is not worth saying — spelling it out turns the one \
             place effort is visible at a glance into noise"
        );
    }

    // ------------------------------------------------------ the review screen

    const MODIFIED_PATCH: &str = "@@ -1,4 +1,4 @@\n fn main() {\n-    old();\n+    new();\n }\n";

    pub(super) const DELETED_PATCH: &str = "@@ -1,2 +0,0 @@\n-gone one\n-gone two\n";

    const METADATA_ONLY_PATCH: &str = "@@ -0,0 +0,0 @@\n";

    const TRUNCATED_LINE_PATCH: &str =
        "@@ -1,1 +1,1 @@\n-old\n+new\n\\ No newline at end of file\n";

    pub(super) fn diff_file(
        file: &str,
        status: FileStatus,
        additions: u32,
        deletions: u32,
        patch: &str,
    ) -> DiffFile {
        DiffFile::from(FileDiff {
            file: file.to_owned(),
            patch: patch.to_owned(),
            additions,
            deletions,
            status,
        })
    }

    /// A band's head is the whole of what a scroll down a twenty-file review
    /// tells you, and its body is the change itself.
    #[test]
    fn a_diff_band_shows_the_path_the_counts_and_the_lines() {
        let html = render_seeded(
            |ctx| {
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files: vec![diff_file(
                        "src/main.rs",
                        FileStatus::Modified,
                        1,
                        1,
                        MODIFIED_PATCH,
                    )],
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            html.contains("diff-dir\">src/<") && html.contains("diff-name\">main.rs<"),
            "the filename is what a reader scans for and the directory is the \
             part allowed to give way: {html:.400}"
        );
        assert!(
            html.contains("add\">+1<") && html.contains("del\">\u{2212}1<"),
            "the head carries the file's own counts: {html:.400}"
        );
        assert!(
            html.contains("diff-line add")
                && html.contains("diff-line del")
                && html.contains("diff-line ctx"),
            "an open band renders all three kinds of row: {html:.400}"
        );
        assert!(
            html.contains("diff-code\">    new();<"),
            "the changed line itself has to be on screen: {html:.400}"
        );
        assert!(
            html.contains("aria-pressed=\"false\"") && html.contains("Viewed"),
            "unreviewed is an empty labelled box, not a bare tick that is \
             present either way: {html:.400}"
        );
        assert!(
            html.contains("subtitle ellipsis\">1 file \u{b7} +1 \u{2212}1<"),
            "one file is counted in the singular: {html:.400}"
        );
        assert!(
            !html.contains("diff-progress"),
            "a one-file review has no progress to report"
        );
    }

    /// The window's bar and the review pane must count the same diff. Two call
    /// sites of one expression, with nothing in the compiler holding them
    /// together.
    fn diff_crumb_probe() -> Element {
        let ctx = use_app_ctx();
        let crumb = diff_crumb(&ctx);
        let subtitle = crumb.subtitle.clone().unwrap_or_default();
        rsx! {
            span { class: "probe", "{crumb.title}|{subtitle}" }
            CodeDiffView {}
        }
    }

    pub(super) fn seed_three_files(ctx: &AppCtx) {
        let files = vec![
            diff_file("src/a.rs", FileStatus::Modified, 1, 1, MODIFIED_PATCH),
            diff_file("src/b.rs", FileStatus::Added, 2, 0, MODIFIED_PATCH),
            diff_file("src/c.rs", FileStatus::Modified, 3, 1, MODIFIED_PATCH),
        ];
        let mut view = std::collections::HashMap::new();
        view.insert(
            "src/a.rs".to_owned(),
            FileView {
                seen: Some(files[0].fingerprint),
                ..FileView::default()
            },
        );
        let mut diff = ctx.code_diff;
        diff.set(DiffState {
            files,
            view,
            ..DiffState::default()
        });
    }

    /// A multi-file review keeps score, folds what you have finished with, and
    /// offers the bulk mark only while there is something left to mark. The
    /// window's bar carries the same count.
    #[test]
    fn a_multi_file_review_keeps_score_and_folds_what_is_done() {
        let html = render_seeded(seed_three_files, diff_crumb_probe);
        assert!(
            html.contains("1 of 3 files reviewed"),
            "the progress line is how a long review stays finishable: {html:.400}"
        );
        assert!(
            html.contains("style=\"width: 33%\""),
            "the bar has to follow the count it sits under: {html:.400}"
        );
        assert!(
            html.contains(">Mark all<"),
            "with files left unreviewed the bulk action is offered: {html:.400}"
        );
        assert!(
            html.contains("probe\">Review|3 files \u{b7} +6 \u{2212}2<"),
            "the window's bar has stopped counting the diff the pane is \
             showing: {html:.400}"
        );
        assert_eq!(
            html.matches("diff-body").count(),
            2,
            "marking a file reviewed folds it away, which is what stops a long \
             diff making you scroll past work you have finished with"
        );
        assert!(
            html.contains("aria-pressed=\"true\""),
            "the reviewed file's box is ticked: {html:.400}"
        );
    }

    /// Nothing left to mark, no button to mark it with — rule 11. The progress
    /// line still reports, because "2 of 2" is the answer the reader came for.
    #[test]
    fn a_finished_review_stops_offering_the_bulk_mark() {
        let html = render_seeded(
            |ctx| {
                let files = vec![
                    diff_file("src/a.rs", FileStatus::Modified, 1, 1, MODIFIED_PATCH),
                    diff_file("src/b.rs", FileStatus::Modified, 1, 1, MODIFIED_PATCH),
                ];
                let mut view = std::collections::HashMap::new();
                for file in &files {
                    view.insert(
                        file.info.file.clone(),
                        FileView {
                            seen: Some(file.fingerprint),
                            ..FileView::default()
                        },
                    );
                }
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files,
                    view,
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            html.contains("2 of 2 files reviewed") && html.contains("style=\"width: 100%\""),
            "a finished review still reports: {html:.400}"
        );
        assert!(
            !html.contains(">Mark all<"),
            "a control that can do nothing must not be on screen"
        );
    }

    /// The three kinds of file nobody reviews line by line, each with the note
    /// that stands in for a body — and the deleted one with the way to see it
    /// anyway, because "not shown by default" is not "not available".
    #[test]
    fn the_bodies_not_worth_rendering_say_so_in_their_own_words() {
        let html = render_seeded(
            |ctx| {
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files: vec![
                        diff_file("assets/logo.png", FileStatus::Modified, 0, 0, ""),
                        diff_file("src/old.rs", FileStatus::Deleted, 0, 2, DELETED_PATCH),
                        diff_file(
                            "src/moved.rs",
                            FileStatus::Modified,
                            0,
                            0,
                            METADATA_ONLY_PATCH,
                        ),
                    ],
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            html.contains("Binary file \u{2014} not shown.")
                && html.contains("diff-badge\">binary<"),
            "a binary file's badge IS its stat — there are no counts to show: {html:.400}"
        );
        assert!(
            html.contains("File deleted \u{b7} 2 lines removed")
                && html.contains("diff-badge\">deleted<"),
            "a deletion's patch is one `-` row per line of the file that used to \
             be there, and nobody reviews that: {html:.400}"
        );
        assert!(
            html.contains("Show removed lines"),
            "not shown by default is not unavailable: {html:.400}"
        );
        assert!(
            html.contains("No line changes \u{2014} file metadata only."),
            "a file whose patch carries no rows still needs a body that says \
             something: {html:.400}"
        );
        assert!(
            !html.contains("gone one"),
            "the deleted file's lines were rendered without being asked for"
        );
    }

    /// Asking for a deletion's lines gives them, and the note that stood in for
    /// them goes.
    #[test]
    fn a_deletion_hands_its_lines_over_when_they_are_asked_for() {
        let html = render_seeded(
            |ctx| {
                let mut view = std::collections::HashMap::new();
                view.insert(
                    "src/old.rs".to_owned(),
                    FileView {
                        show_removed: true,
                        ..FileView::default()
                    },
                );
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files: vec![diff_file(
                        "src/old.rs",
                        FileStatus::Deleted,
                        0,
                        2,
                        DELETED_PATCH,
                    )],
                    view,
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            html.contains("diff-code\">gone one<") && html.contains("diff-code\">gone two<"),
            "the removed lines are what the reveal is for: {html:.400}"
        );
        assert!(
            !html.contains("Show removed lines"),
            "the reveal is still offered after it has been taken"
        );
    }

    /// The added badge and the no-newline note: two facts about a file that
    /// nothing else on the row carries.
    #[test]
    fn an_added_file_and_a_file_with_no_trailing_newline_both_say_so() {
        let html = render_seeded(
            |ctx| {
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files: vec![diff_file(
                        "src/new.rs",
                        FileStatus::Added,
                        1,
                        1,
                        TRUNCATED_LINE_PATCH,
                    )],
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            html.contains("diff-badge\">added<"),
            "a new file is a different reading from a heavily edited one: {html:.400}"
        );
        assert!(
            html.contains("No newline at end of file"),
            "the patch's own note about the line above it: {html:.400}"
        );
    }

    pub(super) fn gapped_patch() -> String {
        let mut patch = String::from("@@ -1,22 +1,22 @@\n+added at the top\n");
        for i in 0..20 {
            patch.push_str(" context line ");
            patch.push_str(&i.to_string());
            patch.push('\n');
        }
        patch.push_str("+added at the bottom\n");
        patch
    }

    /// `Snapshot.diffFull` sends the WHOLE file in one hunk, so a three-line
    /// change arrives as twelve hundred rows. The band standing in for the
    /// untouched middle is what makes that readable, and it has to say how much
    /// it is hiding.
    #[test]
    fn an_untouched_middle_collapses_into_a_band_that_says_how_much() {
        let html = render_seeded(
            |ctx| {
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files: vec![diff_file(
                        "src/wide.rs",
                        FileStatus::Modified,
                        2,
                        0,
                        &gapped_patch(),
                    )],
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            html.contains("\u{22ef} 14 unchanged lines"),
            "the band has to say how much it is hiding: {html:.400}"
        );
        assert!(
            html.contains("added at the top") && html.contains("added at the bottom"),
            "the changes either side of the band are the point of it: {html:.400}"
        );
        assert!(
            !html.contains("diff-code\">context line 10<"),
            "the middle of the band was rendered after all"
        );
    }

    /// Expanding a band gives lines back from BOTH ends, so the context grows
    /// towards the changes either side of it rather than out of one of them.
    #[test]
    fn expanding_a_band_gives_lines_back_from_both_of_its_ends() {
        let html = render_seeded(
            |ctx| {
                let mut expanded = std::collections::HashMap::new();
                expanded.insert(4_usize, 6_usize);
                let mut view = std::collections::HashMap::new();
                view.insert(
                    "src/wide.rs".to_owned(),
                    FileView {
                        expanded,
                        ..FileView::default()
                    },
                );
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files: vec![diff_file(
                        "src/wide.rs",
                        FileStatus::Modified,
                        2,
                        0,
                        &gapped_patch(),
                    )],
                    view,
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            html.contains("\u{22ef} 8 unchanged lines"),
            "the band has to shrink by what it gave back: {html:.400}"
        );
        assert!(
            html.contains("diff-code\">context line 3<"),
            "the top of the band grew towards the change above it: {html:.400}"
        );
        assert!(
            html.contains("diff-code\">context line 16<"),
            "and the bottom towards the change below it: {html:.400}"
        );
    }

    /// One capped file, rendered. The patch travels through a thread-local
    /// because a seed is a plain `fn` pointer and cannot carry one.
    fn render_capped(patch: String) -> String {
        CAPPED_PATCH.with(|slot| slot.replace(patch));
        render_seeded(
            |ctx| {
                let patch = CAPPED_PATCH.with(|slot| slot.borrow().clone());
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files: vec![diff_file(
                        "src/huge.rs",
                        FileStatus::Modified,
                        900,
                        0,
                        &patch,
                    )],
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        )
    }

    thread_local! {
        static CAPPED_PATCH: std::cell::RefCell<String> =
            const { std::cell::RefCell::new(String::new()) };
    }

    /// A file too long to render in one screen is capped, and the note has to
    /// say whether the screen is hiding EDITS or only context. The cap fills in
    /// document order, so a file whose changes are scattered through it can
    /// exhaust the budget early and lose changes off the end — "too long to
    /// render" would describe that as mere overflow.
    #[test]
    fn a_capped_render_says_whether_it_dropped_changes_or_only_context() {
        let mut all_changes = String::from("@@ -1,900 +1,900 @@\n");
        for i in 0..900 {
            all_changes.push_str("+added ");
            all_changes.push_str(&i.to_string());
            all_changes.push('\n');
        }
        let html = render_capped(all_changes);
        assert!(
            html.contains("100 more lines, 100 of them changes"),
            "a reader has to know the screen is hiding edits, not just lines: {html:.400}"
        );

        let mut context_tail = String::from("@@ -1,900 +1,900 @@\n");
        for i in 0..800 {
            context_tail.push_str("+added ");
            context_tail.push_str(&i.to_string());
            context_tail.push('\n');
        }
        for i in 0..100 {
            context_tail.push_str(" tail ");
            context_tail.push_str(&i.to_string());
            context_tail.push('\n');
        }
        let html = render_capped(context_tail);
        assert!(
            html.contains("100 more unchanged lines"),
            "dropping only context is a milder claim and gets its own sentence: {html:.400}"
        );
    }

    /// The review screen has three ways to have nothing on it and they are not
    /// the same: the fetch failed, the fetch is running, or the branch really
    /// has no changes. Only the last is good news.
    #[test]
    fn an_empty_review_says_which_kind_of_empty_it_is() {
        let broken = render_seeded(
            |ctx| {
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    error: Some("container is not responding".to_owned()),
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            broken.contains("container is not responding"),
            "the failure's own words: {broken:.400}"
        );
        assert!(
            !broken.contains("Nothing has changed on this branch yet."),
            "a failed fetch was reported as a clean branch, which is the one \
             wrong answer here"
        );

        let inflight = render_seeded(
            |ctx| {
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    loading: true,
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            inflight.contains(
                "Reading the working tree \u{2014} waking the container if it \
                 was asleep\u{2026}"
            ),
            "reading a diff can wake a container, which is why the wait is \
             explained: {inflight:.400}"
        );
        assert!(
            !inflight.contains("Nothing has changed on this branch yet."),
            "a verdict was given before the answer arrived"
        );

        let clean = render(|| rsx! { CodeDiffView {} });
        assert!(
            clean.contains("Nothing has changed on this branch yet."),
            "a settled empty diff is the good case and says so: {clean:.400}"
        );
    }

    /// The soft-wrap toggle is the reader's and the body has to follow it — a
    /// no-wrap body is its own horizontal scrollport. The control names what
    /// pressing it will do, not what is already true.
    #[test]
    fn the_wrap_toggle_reaches_the_body_and_the_control_names_the_other_way() {
        let wrapped = render_seeded(
            |ctx| {
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files: vec![diff_file(
                        "src/main.rs",
                        FileStatus::Modified,
                        1,
                        1,
                        MODIFIED_PATCH,
                    )],
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            wrapped.contains("class=\"diff-body\"") && !wrapped.contains("diff-body nowrap"),
            "the app wraps by default: {wrapped:.400}"
        );
        assert!(
            wrapped.contains("title=\"Scroll long lines instead of wrapping\""),
            "the control names the other way, not the way it already is: {wrapped:.400}"
        );

        let scrolling = render_seeded(
            |ctx| {
                let mut wrap = ctx.code_diff_wrap;
                wrap.set(false);
                let mut diff = ctx.code_diff;
                diff.set(DiffState {
                    files: vec![diff_file(
                        "src/main.rs",
                        FileStatus::Modified,
                        1,
                        1,
                        MODIFIED_PATCH,
                    )],
                    ..DiffState::default()
                });
            },
            || rsx! { CodeDiffView {} },
        );
        assert!(
            scrolling.contains("diff-body nowrap"),
            "the toggle has to reach the body: {scrolling:.400}"
        );
        assert!(
            scrolling.contains("title=\"Wrap long lines\""),
            "and the title flips with it: {scrolling:.400}"
        );
    }

    /// With no diff read there is nothing to count, so the review screen falls
    /// back to naming the chat it belongs to rather than reporting "0 files".
    #[test]
    fn a_review_with_no_diff_read_names_the_chat_instead() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    title: "Rotate the cert".to_owned(),
                    ..CodeChatState::default()
                });
            },
            diff_crumb_probe,
        );
        assert!(
            html.contains("probe\">Review|Rotate the cert<"),
            "a count of nothing is not a subtitle: {html:.400}"
        );
        assert!(
            html.contains("subtitle ellipsis\">Rotate the cert<"),
            "and the pane below the bar says the same: {html:.400}"
        );
        assert!(
            !html.contains("0 files"),
            "the count was reported before there was anything to count"
        );
    }

    // ------------------------------------------------ the pull-request screen

    fn pulls_crumb_probe() -> Element {
        let ctx = use_app_ctx();
        let crumb = pulls_crumb(&ctx);
        let subtitle = crumb.subtitle.clone().unwrap_or_default();
        rsx! {
            span { class: "probe", "{crumb.title}|{subtitle}" }
            CodePullsView {}
        }
    }

    /// A row says where the pull request stands, what its checks said, and the
    /// one reason for not offering the merge that neither of those chips can
    /// carry. The merge itself is offered only where it would work.
    #[test]
    fn a_pull_row_states_where_it_stands_and_offers_merge_only_where_it_works() {
        let html = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    repo: "acme/infra".to_owned(),
                    branch: "agent/c1".to_owned(),
                    ..CodeChatState::default()
                });
                let mut pulls = ctx.code_pulls;
                pulls.set(PullsState {
                    pulls: vec![
                        pull(
                            42,
                            "Rotate the cert",
                            PullState::Open,
                            Checks::Passing,
                            Some(true),
                        ),
                        pull(
                            43,
                            "Retry logic",
                            PullState::Open,
                            Checks::Pending,
                            Some(false),
                        ),
                    ],
                    loaded: true,
                    ..PullsState::default()
                });
            },
            pulls_crumb_probe,
        );
        assert!(
            html.contains("probe\">Pull requests|acme/infra \u{b7} agent/c1<"),
            "the screen is scoped to this branch and both places have to say \
             so: {html:.400}"
        );
        assert!(
            html.contains("Rotate the cert") && html.contains("session-age\">#42<"),
            "a pull request is identified by its number: {html:.400}"
        );
        assert!(
            html.contains(">open<") && html.contains(">checks passing<"),
            "state and checks are the two chips every row carries: {html:.400}"
        );
        assert!(
            html.contains(">conflicts<"),
            "an open pull request GitHub says conflicts renders exactly like one \
             it says can merge, minus the button — this chip is that fact: {html:.400}"
        );
        assert_eq!(
            html.matches("pull-actions").count(),
            1,
            "merge was offered on a pull request that cannot take it"
        );
        assert!(
            html.contains(">Merge<"),
            "the mergeable one is offered the merge: {html:.400}"
        );
    }

    /// A merge in flight says so on the button that started it, and on that
    /// button alone — the other rows stay usable.
    #[test]
    fn a_merge_in_flight_is_reported_on_its_own_row() {
        let html = render_seeded(
            |ctx| {
                let mut pulls = ctx.code_pulls;
                pulls.set(PullsState {
                    pulls: vec![
                        pull(42, "Rotate", PullState::Open, Checks::Passing, Some(true)),
                        pull(44, "Tidy", PullState::Open, Checks::None, Some(true)),
                    ],
                    loaded: true,
                    merging: Some(42),
                    ..PullsState::default()
                });
            },
            || rsx! { CodePullsView {} },
        );
        assert!(
            html.contains("Merging\u{2026}"),
            "the row that is merging has to say it is: {html:.400}"
        );
        assert_eq!(
            html.matches(">Merge<").count(),
            1,
            "the other row's merge button was taken away by somebody else's \
             merge"
        );
        assert!(
            html.contains(">no checks<"),
            "\"nothing runs checks here\" is a different fact from \"the checks \
             could not be read\": {html:.400}"
        );
    }

    /// GitHub computes mergeability asynchronously, and a `null` answer is a
    /// wait rather than a refusal — the row says which, because the merge
    /// button's absence alone cannot.
    #[test]
    fn a_mergeability_github_has_not_worked_out_reads_as_a_wait() {
        let html = render_seeded(
            |ctx| {
                let mut pulls = ctx.code_pulls;
                pulls.set(PullsState {
                    pulls: vec![
                        pull(42, "Rotate", PullState::Open, Checks::Unknown, None),
                        pull(43, "Old work", PullState::Merged, Checks::Passing, None),
                        pull(
                            44,
                            "Sketch",
                            PullState::Unknown,
                            Checks::Failing,
                            Some(true),
                        ),
                    ],
                    loaded: true,
                    ..PullsState::default()
                });
            },
            || rsx! { CodePullsView {} },
        );
        assert!(
            html.contains(">mergeability pending<"),
            "a wait must not read as a refusal: {html:.400}"
        );
        assert!(
            html.contains(">checks unknown<"),
            "the manager's PAT cannot read check runs on a private repo, and \
             that is a real answer rather than a parse failure: {html:.400}"
        );
        assert!(
            html.contains(">merged<") && html.contains(">state unknown<"),
            "a state this client has not heard of must not read as anything \
             reassuring: {html:.400}"
        );
        assert!(
            !html.contains("pull-actions"),
            "none of these three can be merged from here"
        );
    }

    /// Three ways for this screen to be empty, and only one of them means the
    /// agent has not pushed yet.
    #[test]
    fn an_empty_pull_screen_says_which_kind_of_empty_it_is() {
        let broken = render_seeded(
            |ctx| {
                let mut pulls = ctx.code_pulls;
                pulls.set(PullsState {
                    error: Some("GitHub said 403".to_owned()),
                    ..PullsState::default()
                });
            },
            || rsx! { CodePullsView {} },
        );
        assert!(
            broken.contains("GitHub said 403"),
            "the failure's own words: {broken:.400}"
        );
        assert!(
            !broken.contains("Nothing from this branch yet"),
            "a refused request was reported as a branch with no pull requests"
        );

        let inflight = render_seeded(
            |ctx| {
                let mut pulls = ctx.code_pulls;
                pulls.set(PullsState {
                    loading: true,
                    ..PullsState::default()
                });
            },
            || rsx! { CodePullsView {} },
        );
        assert!(
            inflight.contains("Asking GitHub\u{2026}")
                && !inflight.contains("Nothing from this branch yet"),
            "a verdict was given before GitHub answered: {inflight:.400}"
        );

        let settled = render(|| rsx! { CodePullsView {} });
        assert!(
            settled.contains("the push is permission-gated, so it will ask you first"),
            "the empty state says what to do next and what that will cost: {settled:.400}"
        );
    }

    /// The screen is scoped to one branch and names it, in all four states the
    /// open chat can be in. A repo's other pull requests have nothing to do
    /// with this conversation, so a subtitle that lost the branch would make
    /// the chip's count read as a number about the repo.
    fn pulls_subtitle_probe() -> Element {
        let ctx = use_app_ctx();
        rsx! { span { class: "probe", "{pulls_subtitle(&ctx)}" } }
    }

    #[test]
    fn the_pull_screen_names_whatever_it_can_of_the_branch_it_is_scoped_to() {
        let both = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    repo: "acme/infra".to_owned(),
                    branch: "agent/c1".to_owned(),
                    ..CodeChatState::default()
                });
            },
            pulls_subtitle_probe,
        );
        assert!(
            both.contains("probe\">acme/infra \u{b7} agent/c1<"),
            "repo and branch, joined: {both:.400}"
        );

        let repo_only = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    repo: "acme/infra".to_owned(),
                    ..CodeChatState::default()
                });
            },
            pulls_subtitle_probe,
        );
        assert!(
            repo_only.contains("probe\">acme/infra<"),
            "a chat with no branch yet still says where it is: {repo_only:.400}"
        );

        let branch_only = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    branch: "agent/c1".to_owned(),
                    ..CodeChatState::default()
                });
            },
            pulls_subtitle_probe,
        );
        assert!(
            branch_only.contains("probe\">agent/c1<"),
            "and so does one whose repo the app has not been told: {branch_only:.400}"
        );

        let neither = render_seeded(
            |ctx| {
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    title: "Rotate the cert".to_owned(),
                    ..CodeChatState::default()
                });
            },
            pulls_subtitle_probe,
        );
        assert!(
            neither.contains("probe\">Rotate the cert<"),
            "with neither known the chat's own name is the last thing left to \
             say: {neither:.400}"
        );
    }

    // ------------------------------------ pointing a draft session at a repo

    /// Three things belong to the repo rather than to the new-session screen,
    /// and this is the one place that moves all three at once. The probe reads
    /// the signals back out after the move, because that — not any markup — is
    /// what `choose_repo` produces.
    fn moved_to_a_private_repo() -> Element {
        let ctx = use_app_ctx();
        let repo = use_signal(|| "PhillipChaffee/scratch".to_owned());
        let branch = use_signal(|| Some("release/2.x".to_owned()));
        let model = use_signal(|| Some("opencode/big-pickle".to_owned()));
        choose_repo(&ctx, "acme/infra", repo, branch, model);
        let (name, base, chosen) = (repo(), branch(), model());
        rsx! { span { class: "probe", "{name}|{base:?}|{chosen:?}" } }
    }

    fn moved_to_a_throwaway_repo() -> Element {
        let ctx = use_app_ctx();
        let repo = use_signal(|| "acme/infra".to_owned());
        let branch = use_signal(|| Some("release/2.x".to_owned()));
        let model = use_signal(|| Some("opencode/big-pickle".to_owned()));
        choose_repo(&ctx, "PhillipChaffee/scratch", repo, branch, model);
        let (name, base, chosen) = (repo(), branch(), model());
        rsx! { span { class: "probe", "{name}|{base:?}|{chosen:?}" } }
    }

    fn moved_to_the_repo_it_is_already_on() -> Element {
        let ctx = use_app_ctx();
        let repo = use_signal(|| "acme/infra".to_owned());
        let branch = use_signal(|| Some("release/2.x".to_owned()));
        let model = use_signal(|| Some("opencode/big-pickle".to_owned()));
        choose_repo(&ctx, "acme/infra", repo, branch, model);
        let (name, base, chosen) = (repo(), branch(), model());
        rsx! { span { class: "probe", "{name}|{base:?}|{chosen:?}" } }
    }

    fn seed_two_repos(ctx: &AppCtx) {
        let mut repos = ctx.code_repos;
        repos.set(vec![
            repo("acme/infra", false),
            repo("PhillipChaffee/scratch", true),
        ]);
    }

    /// Privacy hard rule 1, at the moment it can be broken: a free model picked
    /// while a public throwaway was selected must not ride into a repo that is
    /// not one. The manager would refuse it at create time, and a refusal after
    /// the fact is a worse way to learn the rule than the pill going blank in
    /// front of you.
    #[test]
    fn moving_a_draft_to_a_private_repo_clears_a_model_that_trains() {
        let html = render_seeded(seed_two_repos, moved_to_a_private_repo);
        assert!(
            html.contains("probe\">acme/infra|None|None<"),
            "the repo moved, its branch went with it, and the free model was \
             cleared: {html:.400}"
        );

        let toast = render_seeded(seed_two_repos, moved_to_a_private_repo_toast);
        assert!(
            toast.contains("That model trains on its input"),
            "a pill that empties itself without saying why is the app changing \
             a decision behind your back: {toast:.400}"
        );
    }

    fn moved_to_a_private_repo_toast() -> Element {
        let ctx = use_app_ctx();
        let repo = use_signal(|| "PhillipChaffee/scratch".to_owned());
        let branch = use_signal(|| Some("release/2.x".to_owned()));
        let model = use_signal(|| Some("opencode/big-pickle".to_owned()));
        choose_repo(&ctx, "acme/infra", repo, branch, model);
        let said = ctx.toast.peek().clone().unwrap_or_default();
        rsx! { span { class: "probe", "{said}" } }
    }

    /// The same move onto a repo the rule allows keeps the model. The branch
    /// still goes, because a branch belongs to the repo it was read from.
    #[test]
    fn moving_a_draft_to_a_throwaway_keeps_the_model_and_still_drops_the_branch() {
        let html = render_seeded(seed_two_repos, moved_to_a_throwaway_repo);
        assert!(
            html.contains("probe\">PhillipChaffee/scratch|None|Some(\"opencode/big-pickle\")<"),
            "a public throwaway is exactly where a model that trains on its \
             input is allowed: {html:.400}"
        );
    }

    /// Re-choosing the repo already selected changes nothing. Without the
    /// guard, every re-render of the picker would throw away a base branch the
    /// reader had chosen by hand.
    #[test]
    fn choosing_the_repo_already_selected_throws_nothing_away() {
        let html = render_seeded(seed_two_repos, moved_to_the_repo_it_is_already_on);
        assert!(
            html.contains(
                "probe\">acme/infra|Some(\"release/2.x\")|Some(\"opencode/big-pickle\")<"
            ),
            "a no-op move cleared the branch and the model: {html:.400}"
        );
    }

    /// A repo name with no owner in it and no flag on it has nothing to say
    /// under its name, and must get no note rather than an empty line.
    #[test]
    fn a_repo_with_nothing_to_add_carries_no_note() {
        let rows = repo_choices(&[repo("testrepo", false)]);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].label, "testrepo");
        assert_eq!(rows[0].note, None);
    }

    /// "Working" is the one status the manager's index cannot tell you: it
    /// knows a container is up, not that a turn is in flight inside it. Only
    /// the chat the app has open can say that, and only about itself.
    #[test]
    fn the_chat_the_app_has_open_is_the_only_one_that_can_read_as_working() {
        let html = render_seeded(
            |ctx| {
                connect(ctx);
                let mut chats = ctx.code_chats;
                chats.set(vec![
                    chat_meta("c1", "Rotating", "acme/infra", "agent/c1", "running"),
                    chat_meta("c2", "Idle work", "acme/tools", "agent/c2", "running"),
                ]);
                let mut chat = ctx.code_chat;
                chat.set(CodeChatState {
                    chat_id: Some("c1".to_owned()),
                    running: true,
                    ..CodeChatState::default()
                });
            },
            || rsx! { CodeSessionsView {} },
        );
        assert!(
            html.contains(">working<"),
            "the open chat's turn is in flight and its row has to say so: \
             {html:.400}"
        );
        assert_eq!(
            html.matches(">idle<").count(),
            1,
            "the OTHER running container must still read as idle — nothing in \
             the index says whether it is mid-turn"
        );
    }
    // APPEND-HERE
}

/// Pressing the code tab, because everything left unmeasured in this file is
/// a handler.
///
/// The module above mounts a view and reads its markup back, which reaches
/// every arm the context decides and none of the ones the READER decides. Two
/// hundred lines of this file are closures a render never runs: the four pills
/// and their sheets, the composer's send and its Enter key, the review's fold,
/// tick, reveal and expand bands, both confirms, the overflow menu, the three
/// answers on a permission modal and the two on a card. A suite that stopped
/// at markup would report all of them as covered by nothing — which is what
/// they were.
///
/// So this dispatches real events into a live `VirtualDom` and renders again.
/// The two problems that has to solve are solved the way
/// `crate::views::chat`'s `pressing` module solves them, and the long-form
/// account of WHY each half is shaped this way is written there:
/// `dioxus_ssr::pre_render` numbers the hydratable elements, [`hydration_ids`]
/// is `dioxus-web`'s own rehydration walk over public `dioxus-core` API, and
/// the k-th element that walk visits is the element the renderer numbered k.
/// Taking `rebuild_to_vec`'s listener order instead is measurably wrong — it
/// lands a press on a different button — so it is not done here either.
///
/// It is a second copy rather than a shared one because that module is a
/// private `#[cfg(test)]` module of another file, and widening test
/// scaffolding into `pub(crate)` to reach it would put a third file's markup
/// in this file's blast radius. When a third screen wants it, that is when it
/// earns a home in `crate::testkit`.
///
/// WHAT A PRESS IS ALLOWED TO REACH. Every handler here ends in
/// `crate::code`, and every one of those functions goes through
/// `ctx.code_client`. Left `None`, each has a documented refusal — a toast, an
/// error written into the screen's own state — and that refusal is a thing a
/// reader sees, so it is asserted rather than avoided. Where a press has to
/// get PAST that guard, [`stub_client`] puts a real `CodeClient` in the slot
/// pointed at a base that is not a URL: `CodeClient::new` only checks that the
/// string is non-empty, so the request fails inside `reqwest`'s builder,
/// immediately, with no socket and no name lookup. Nothing in this module
/// talks to anything.
#[cfg(test)]
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test scaffolding: a press that cannot find its button has \
              nothing to assert, so failing loudly there IS the check"
)]
mod pressing {
    use std::any::Any;
    use std::rc::Rc;
    use std::time::Duration;

    use dioxus::dioxus_core::{
        DynamicNode, ElementId, NoOpMutations, ScopeState, TemplateAttribute, TemplateNode, VNode,
    };
    use dioxus::html::{
        Code, Key, Location, Modifiers, PlatformEventData, SerializedFormData,
        SerializedHtmlEventConverter, SerializedKeyboardData, SerializedMouseData,
    };
    use dioxus::prelude::*;
    use opencode_client::{AgentMode, Checks, CodeClient, CodeConfig, PullState};

    use super::tests::{
        agent, chat_meta, connect, diff_file, gapped_patch, model, permission, pull, seed_repos,
        seed_three_files, sonnet, unescape, DELETED_PATCH,
    };
    use super::{
        CodeChatView, CodeDiffView, CodeNewView, CodePermissionModal, CodePullsView,
        CodeSessionsView, FileStatus,
    };
    use crate::code::{CodeChatState, CodeScreen, DiffState, PullsState};
    use crate::state::{use_app_ctx, AppCtx, ChatItem, ConnState, Settings};

    // ----------------------------------------------------------- the harness

    /// The two process-wide things a press needs, set up exactly once.
    ///
    /// The converter because `dioxus-html` routes every listener through a
    /// global one that a renderer installs at launch, and the `.unwrap()`
    /// inside `ListenerCallback` panics without it. The runtime because a
    /// refusal ends in `show_toast`, which arms a `tokio::time::sleep` to take
    /// the toast away again — and polling that with no reactor panics.
    ///
    /// Both are `OnceLock`ed rather than built per mount for the reason
    /// `views/chat.rs` records: `cargo test` runs on every core at once, and a
    /// converter reinstalled under a reader wedged that whole binary while
    /// every test in it still passed alone.
    fn install_once() -> &'static tokio::runtime::Runtime {
        static TIMERS: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
        TIMERS.get_or_init(|| {
            dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread tokio runtime for the toast timer")
        })
    }

    /// These tests run one at a time.
    ///
    /// [`Pressable::settle`] runs the `VirtualDom`'s task queue, and the ask
    /// journal's `use_synced_storage` puts two tasks on that queue talking to
    /// a watch channel `dioxus-sdk-storage` keys in a `static` — process-wide,
    /// so every mount in the binary shares it. Two of them draining at once
    /// feed each other and never settle. See `views/chat.rs` for the
    /// measurement.
    fn alone() -> std::sync::MutexGuard<'static, ()> {
        static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
        ONE_AT_A_TIME
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
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

    /// A mounted view with a way in.
    struct Pressable {
        dom: VirtualDom,
        /// The render with `data-node-hydration` left in, and the elements it
        /// numbers, in the same order. Recomputed after every event, because
        /// an event that opens a sheet creates elements.
        hydrated: String,
        ids: Vec<ElementId>,
    }

    impl Pressable {
        fn mount(seed: fn(&AppCtx), view: fn() -> Element) -> Self {
            // The same one owner as `crate::testkit`: `set_directory` writes a
            // process-wide `OnceLock` and unwraps, so a second caller panics.
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

        /// What the screen says right now, with the escapes `dioxus_ssr`
        /// applies to text undone — so a negative assertion about a sentence
        /// carrying an apostrophe is a claim about the screen rather than one
        /// about the escaper.
        fn markup(&self) -> String {
            unescape(&dioxus_ssr::render(&self.dom))
        }

        /// Let the work a press STARTED finish before the markup is read.
        ///
        /// A handler that spawns is only half a press: `refresh_code_chats`,
        /// `code_connect`, `client.abort` and the rest all answer after the
        /// render that dispatched them has returned. The loop is bounded for
        /// the reason `testkit::render_settled`'s is — a view whose tasks
        /// never settle must not be able to hang the suite.
        fn settle(&mut self) {
            const PASSES: usize = 8;
            const SLICE: Duration = Duration::from_millis(20);
            install_once().block_on(async {
                for _ in 0..PASSES {
                    let _ = tokio::time::timeout(SLICE, self.dom.wait_for_work()).await;
                    self.dom.render_immediate(&mut NoOpMutations);
                }
            });
            self.reread();
        }

        /// The `ElementId` of the `nth` element whose opening tag contains
        /// `needle` and which carries an `event` listener.
        fn locate(&self, event: &str, needle: &str, nth: usize) -> ElementId {
            const MARK: &str = " data-node-hydration=\"";
            let mut at = 0;
            let mut seen = 0;
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
                    if seen == nth {
                        return *self.ids.get(number).expect(
                            "the markup numbers an element the hydration walk never \
                             reached, so the two are out of step and a press would \
                             land somewhere else entirely",
                        );
                    }
                    seen += 1;
                }
                at = end;
            }
            panic!(
                "there is no #{nth} element matching {needle:?} with an {event} \
                 listener:\n{}",
                self.hydrated
            )
        }

        fn dispatch(&mut self, event: &str, needle: &str, nth: usize, data: Box<dyn Any>) {
            let id = self.locate(event, needle, nth);
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
        fn press(&mut self, needle: &str) {
            self.press_nth(needle, 0);
        }

        /// Tap the `nth`, for the lists whose rows are indistinguishable from
        /// their markup — a picker's choices carry their label as a child, not
        /// as an attribute, so "the second model offered" is the only way to
        /// name one.
        fn press_nth(&mut self, needle: &str, nth: usize) {
            self.dispatch(
                "click",
                needle,
                nth,
                Box::new(SerializedMouseData::default()),
            );
        }

        /// Type into the first field whose opening tag contains `needle`,
        /// exactly as a `WebView` reports it: the field's whole new value.
        fn type_into(&mut self, needle: &str, value: &str) {
            self.dispatch(
                "input",
                needle,
                0,
                Box::new(SerializedFormData::new(value.to_owned(), Vec::new())),
            );
        }

        /// Press Return in the first field whose opening tag contains
        /// `needle`. The physical key is given as well as the logical one
        /// because a browser sends both.
        fn enter(&mut self, needle: &str, modifiers: Modifiers) {
            self.dispatch(
                "keydown",
                needle,
                0,
                Box::new(SerializedKeyboardData::new(
                    Key::Enter,
                    Code::Enter,
                    Location::Standard,
                    false,
                    modifiers,
                    false,
                )),
            );
        }
    }

    // ------------------------------------------------------------- the probe

    /// The state a press changes that no screen in this file paints.
    ///
    /// Rendered above whichever view is under test, so one `markup()` answers
    /// both "what does the reader see" and "where did that press leave the
    /// app". Every field is here because some assertion below would otherwise
    /// have nothing to stand on.
    fn probe(ctx: &AppCtx) -> Element {
        let screen = match (ctx.code_screen)() {
            CodeScreen::List => "list",
            CodeScreen::New => "new",
            CodeScreen::Chat => "chat",
            CodeScreen::Diff => "diff",
            CodeScreen::Pulls => "pulls",
        };
        let toast = (ctx.toast)().unwrap_or_default();
        let drawer = (ctx.drawer_open)();
        let drafts = ctx.new_attachments.read().len();
        let pulls_error = ctx.code_pulls.read().error.clone().unwrap_or_default();
        let open = ctx.code_chat.read().title.clone();
        rsx! {
            p { class: "probe",
                "{screen}|{toast}|{drawer}|new:{drafts}|open:{open}|pulls:{pulls_error}"
            }
        }
    }

    fn sessions_probe() -> Element {
        let ctx = use_app_ctx();
        rsx! {
            {probe(&ctx)}
            CodeSessionsView {}
        }
    }

    fn new_probe() -> Element {
        let ctx = use_app_ctx();
        rsx! {
            {probe(&ctx)}
            CodeNewView {}
        }
    }

    fn chat_probe() -> Element {
        let ctx = use_app_ctx();
        rsx! {
            {probe(&ctx)}
            CodeChatView {}
        }
    }

    fn diff_probe() -> Element {
        let ctx = use_app_ctx();
        rsx! {
            {probe(&ctx)}
            CodeDiffView {}
        }
    }

    fn pulls_probe() -> Element {
        let ctx = use_app_ctx();
        rsx! {
            {probe(&ctx)}
            CodePullsView {}
        }
    }

    fn modal_probe() -> Element {
        let ctx = use_app_ctx();
        rsx! {
            {probe(&ctx)}
            CodePermissionModal {}
        }
    }

    // --------------------------------------------------------------- seeds

    /// A client that exists and can reach nothing.
    ///
    /// The half of `crate::code` behind `ctx.code_client` is unreachable
    /// without one — `send_code_prompt` returns false, `merge_pull` toasts,
    /// `answer_code_permission` returns before it has taken the ask off the
    /// queue — so a press against an empty slot can only ever assert the
    /// refusal. `CodeClient::new` checks that the base is non-empty and
    /// nothing else, and `code-plane` prefixed to a path is not a URL, so
    /// every request fails inside `reqwest`'s builder without opening a socket
    /// or asking a resolver anything.
    fn stub_client(ctx: &AppCtx) {
        let client = CodeClient::new(&CodeConfig {
            base_url: "code-plane".to_owned(),
            password: "secret".to_owned(),
        })
        .expect("a client for a non-empty base");
        ctx.code_client.clone().set(Some(client));
    }

    fn seed_list(ctx: &AppCtx) {
        connect(ctx);
        let mut chats = ctx.code_chats;
        chats.set(vec![
            chat_meta("c1", "Rotate the cert", "acme/infra", "agent/c1", "running"),
            chat_meta("c2", "Tidy the audit", "acme/tools", "", "stopped"),
        ]);
    }

    fn seed_list_with_ask(ctx: &AppCtx) {
        seed_list(ctx);
        stub_client(ctx);
        let mut asks = ctx.code_permissions;
        asks.set(vec![
            ("c1".to_owned(), permission("p1", "Write to src/main.rs")),
            ("c1".to_owned(), permission("p2", "Run cargo test")),
        ]);
    }

    fn seed_modal(ctx: &AppCtx) {
        stub_client(ctx);
        let mut chat = ctx.code_chat;
        chat.set(CodeChatState {
            chat_id: Some("c1".to_owned()),
            ..CodeChatState::default()
        });
        let mut asks = ctx.code_permissions;
        asks.set(vec![
            ("c1".to_owned(), permission("p1", "Write a file")),
            ("c1".to_owned(), permission("p2", "Run cargo test")),
        ]);
    }

    fn seed_new(ctx: &AppCtx) {
        seed_repos(ctx);
        let mut branches = ctx.code_branches;
        branches.set(crate::code::BranchList {
            repo: "PhillipChaffee/personal-ai-setup".to_owned(),
            default: Some("main".to_owned()),
            names: vec!["main".to_owned(), "release/2.x".to_owned()],
            truncated: false,
            loading: false,
        });
        let mut models = ctx.code_models;
        models.set(vec![sonnet()]);
        let mut agents = ctx.code_agents;
        agents.set(vec![
            agent("build", AgentMode::Primary, None),
            agent("plan", AgentMode::Primary, None),
        ]);
    }

    /// A chat screen the reader has arrived on, so "the press left the screen
    /// where it was" is a claim with something behind it.
    fn open_chat(ctx: &AppCtx) {
        ctx.code_screen.clone().set(CodeScreen::Chat);
        let mut chat = ctx.code_chat;
        chat.set(CodeChatState {
            chat_id: Some("c1".to_owned()),
            session_id: Some("s-1".to_owned()),
            title: "Rotate the cert".to_owned(),
            repo: "acme/infra".to_owned(),
            branch: "agent/c1".to_owned(),
            items: vec![ChatItem::Assistant {
                message_id: None,
                text: "Rotated it.".to_owned(),
            }],
            ..CodeChatState::default()
        });
    }

    /// A chat the app has finished opening: a client, a session, a catalogue
    /// and the agent list this chat's own container answered.
    ///
    /// Two models rather than one because `SettingRow::select` demotes a
    /// one-item list to a fact — with nothing to choose between, the row is
    /// where the setting is stuck rather than a control. And
    /// `code_agents_from` because that is what says the held list belongs to
    /// THIS chat; without it `ensure_code_agents` throws the seeded list away
    /// and asks the container again, which is a fair reading of an empty
    /// `agents_from` and not the state under test.
    fn seed_chat(ctx: &AppCtx) {
        stub_client(ctx);
        open_chat(ctx);
        let mut models = ctx.code_models;
        models.set(vec![
            sonnet(),
            model("anthropic", "claude-opus-4", "Claude Opus 4"),
        ]);
        let mut agents = ctx.code_agents;
        agents.set(vec![
            agent("build", AgentMode::Primary, None),
            agent("plan", AgentMode::Primary, None),
        ]);
        ctx.code_agents_from.clone().set("c1".to_owned());
    }
    // ------------------------------------------------------- the chat list

    /// Opening the Code tab with a server already saved connects to it,
    /// without being asked and without a Retry first.
    ///
    /// That is a `use_hook` on first render, and it is the whole of what makes
    /// the tab usable after a cold start: without it the list sits on the
    /// offline arm — "set the code server URL in Settings" — over a URL that
    /// is already set, and nothing on the screen would ever change.
    ///
    /// The saved base here cannot be turned into a request, so the round trip
    /// ends where `code_connect` ends it: the badge and the error box, which
    /// is the state a reader would see with a gateway that has moved.
    #[test]
    fn a_saved_server_is_dialled_the_first_time_the_tab_is_opened() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut settings = ctx.settings;
                settings.set(Settings {
                    code_server_url: "code-plane".to_owned(),
                    code_password: "secret".to_owned(),
                    ..Settings::default()
                });
            },
            sessions_probe,
        );
        assert!(
            screen.markup().contains("conn-label\">offline<"),
            "nothing has been dialled yet, so the badge cannot claim otherwise"
        );

        screen.settle();
        let html = screen.markup();
        assert!(
            html.contains("conn-label\">error<"),
            "the tab never dialled the server it already has saved, so a cold \
             start leaves it offline for ever: {html:.400}"
        );
        assert!(
            html.contains("error-box"),
            "a failed connection has to say what failed: {html:.400}"
        );
        assert!(
            !html.contains("Set the code server URL and password in Settings"),
            "a configured server that could not be reached is not an \
             unconfigured one"
        );
    }

    /// The one way out of the Code tab on a phone. Nothing else in this view
    /// opens the drawer, so a handler that stopped writing the signal would
    /// strand the reader on a tab with no navigation at all.
    #[test]
    fn the_lists_menu_button_opens_the_drawer() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_list, sessions_probe);
        assert!(
            screen.markup().contains("|false|new:0"),
            "the drawer is open before anything was pressed"
        );

        screen.press("class=\"icon-btn menu\"");
        let said = screen.markup();
        assert!(
            said.contains("|true|new:0"),
            "the menu button no longer opens the drawer, which is the only way \
             off this tab: {said:.400}",
        );
    }

    /// Retry has to ASK AGAIN, not merely repaint. A handler that cleared the
    /// error without redialling would leave the tab looking recovered and
    /// still holding no client, which is worse than the failure it replaced.
    ///
    /// The proof is that the message CHANGES: the seeded failure is one the
    /// app could not have produced from the settings it has, and the one that
    /// replaces it is `build_client`'s own verdict on an empty URL.
    #[test]
    fn retry_dials_again_rather_than_only_clearing_the_message() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut conn = ctx.code_conn;
                conn.set(ConnState::Failed("tls handshake failed".to_owned()));
            },
            sessions_probe,
        );
        assert!(
            screen.markup().contains("tls handshake failed"),
            "the seeded failure is what the retry is being offered for"
        );

        screen.press("class=\"btn primary grow\"");
        screen.settle();
        let html = screen.markup();
        assert!(
            html.contains("code server URL is empty"),
            "Retry did not reach `code_connect` — the box still shows the old \
             failure rather than this attempt's: {html:.400}"
        );
        assert!(
            !html.contains("tls handshake failed"),
            "the previous failure survived a fresh attempt: {html:.400}"
        );
    }

    /// The only route to the draft screen from the list.
    #[test]
    fn the_fab_leaves_the_list_for_the_draft_screen() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_list, sessions_probe);
        assert!(
            screen.markup().contains("probe\">list|"),
            "starts on the list"
        );

        screen.press("class=\"fab\"");
        let said = screen.markup();
        assert!(
            said.contains("probe\">new|"),
            "the FAB is the only way to start a session and it went nowhere: \
             {said:.400}",
        );
    }

    /// Tapping a row opens THAT row's chat — the whole tile is the target
    /// (design rule 9), and what it opens is the session the row names rather
    /// than whatever was last open.
    #[test]
    fn a_row_opens_the_session_it_names() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_list, sessions_probe);

        screen.press_nth("class=\"session-item\"", 1);
        let html = screen.markup();
        assert!(
            html.contains("probe\">chat|"),
            "a tap on a row did not open its chat: {html:.400}"
        );
        assert!(
            html.contains("open:Tidy the audit|"),
            "the second row opened the first row's session: {html:.400}"
        );
    }

    /// Deleting a session takes the container and the branch with it and there
    /// is no undo, so the row action asks first — and Cancel has to mean
    /// cancel.
    #[test]
    fn deleting_from_the_list_asks_first_and_cancel_backs_out() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_list, sessions_probe);
        assert!(
            !screen.markup().contains("Delete this session?"),
            "the confirm is up before the row action was pressed"
        );

        screen.press("class=\"row-action danger\"");
        let asked = screen.markup();
        assert!(
            asked.contains("Delete this session?"),
            "a destructive row action with no confirmation behind it: {asked:.400}"
        );
        assert!(
            asked.contains("any work on the branch that has not been pushed goes with them"),
            "the confirm has to say what else goes: {asked:.400}"
        );

        screen.press("class=\"btn secondary\"");
        let after = screen.markup();
        assert!(
            !after.contains("Delete this session?"),
            "Cancel left the confirm on screen: {after:.400}"
        );
        assert!(
            after.contains("Rotate the cert"),
            "Cancel took the row away anyway: {after:.400}"
        );
    }

    /// Confirming closes the sheet and asks the server. It must NOT take the
    /// row off the list on its own — the manager purges the container and the
    /// workspace, and a row that vanished before that answered would report a
    /// deletion that had not happened.
    #[test]
    fn confirming_a_delete_hands_it_to_the_server_and_keeps_the_row() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                seed_list(ctx);
                stub_client(ctx);
            },
            sessions_probe,
        );

        screen.press("class=\"row-action danger\"");
        screen.press("class=\"btn danger\"");
        let html = screen.markup();
        assert!(
            !html.contains("Delete this session?"),
            "the confirm stayed up after it was answered: {html:.400}"
        );
        assert!(
            html.contains("Rotate the cert"),
            "the row was removed locally rather than by the server's answer: \
             {html:.400}"
        );
    }

    /// The two answers a card offers, and the reason the card can offer them
    /// at all: they stop the click, so approving does not also open the chat.
    ///
    /// That `stop_propagation` is one line with nothing else holding it in
    /// place, and losing it would send the reader into a conversation they did
    /// not ask for every time they cleared an ask from the list.
    #[test]
    fn approving_from_a_card_answers_the_ask_without_opening_the_chat() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_list_with_ask, sessions_probe);
        assert!(
            screen
                .markup()
                .contains("Approve or deny Write to src/main.rs"),
            "the card starts parked on the first ask"
        );

        screen.press("class=\"btn small primary\"");
        let html = screen.markup();
        assert!(
            html.contains("Approve or deny Run cargo test"),
            "answering the front of the queue did not move the card on to the \
             next ask: {html:.400}"
        );
        assert!(
            !html.contains("more waiting"),
            "one ask left is not a backlog: {html:.400}"
        );
        assert!(
            html.contains("probe\">list|"),
            "the answer's click reached the row underneath it, so clearing an \
             ask also navigated: {html:.400}"
        );
    }

    /// Deny is the other half, and it is a different string on the wire — a
    /// handler that sent `once` for both would read as working right up until
    /// the agent did the thing you refused.
    #[test]
    fn denying_from_a_card_also_takes_the_ask_off_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_list_with_ask, sessions_probe);

        screen.press("class=\"btn small danger-outline\"");
        let html = screen.markup();
        assert!(
            html.contains("Approve or deny Run cargo test"),
            "Deny left the answered ask on the card: {html:.400}"
        );
        assert!(
            html.contains("probe\">list|"),
            "Deny navigated into the chat as well as answering: {html:.400}"
        );
    }

    // ------------------------------------------------ the permission modal

    /// All three answers, each on its own mount, because each is a different
    /// word on the wire and the buttons are otherwise identical markup.
    /// Answering the front of a queue of two leaves the second one up, which
    /// is what says the modal answered ONE ask rather than emptied the queue.
    #[test]
    fn the_modal_answers_one_ask_at_a_time_whichever_button_is_pressed() {
        let _alone = alone();
        for (nth, needle, answer) in [
            (0, "class=\"btn primary\"", "Allow once"),
            (1, "class=\"btn primary\"", "Always allow"),
            (0, "class=\"btn danger-outline\"", "Reject"),
        ] {
            let mut screen = Pressable::mount(seed_modal, modal_probe);
            assert!(
                screen.markup().contains("modal-tool\">Write a file<"),
                "{answer}: the modal starts on the front of the queue"
            );

            screen.press_nth(needle, nth);
            let html = screen.markup();
            assert!(
                html.contains("modal-tool\">Run cargo test<"),
                "{answer} did not answer the ask it was on: {html:.400}"
            );
            assert!(
                !html.contains("more waiting"),
                "{answer} left the count claiming a backlog that is now the \
                 whole queue: {html:.400}"
            );
        }
    }
    // ---------------------------------------------- the new-session screen

    /// Four pills, four sheets, and nothing else on the screen opens any of
    /// them. The tap beside a sheet is the only way back out — this screen has
    /// no Cancel row — so a lost `onclose` would trap the reader in a picker.
    #[test]
    fn each_pill_opens_its_own_sheet_and_a_tap_beside_it_closes_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_new, new_probe);
        assert!(
            !screen.markup().contains("modal sheet picker"),
            "a sheet is up before any pill was pressed"
        );

        for (pill, heading, row) in [
            (
                "title=\"Repository\"",
                "Repositories (2)",
                "personal-ai-setup",
            ),
            ("title=\"Base branch\"", "Choose base branch", "release/2.x"),
            ("title=\"Model", "Select model", "Claude Sonnet 4.5"),
            ("title=\"Mode\"", "Select mode", "Plan"),
        ] {
            screen.press(pill);
            let open = screen.markup();
            assert!(
                open.contains(heading) && open.contains(row),
                "the {pill} pill did not open a sheet offering {row}: {open:.400}"
            );

            screen.press("class=\"modal-backdrop\"");
            let closed = screen.markup();
            assert!(
                !closed.contains(heading),
                "a tap beside the {pill} sheet left it on screen, which is the \
                 only way out of it: {closed:.400}"
            );
        }
    }

    /// Choosing a repo moves the branch with it. The sentence in the
    /// placeholder is the screen's own statement of what it is about to do, so
    /// a base branch read from the old repo surviving the move would make it
    /// name a branch the new repo may not even have.
    #[test]
    fn choosing_a_repo_rewrites_the_sentence_and_drops_the_branch_with_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_new, new_probe);
        assert!(
            screen.markup().contains(
                "Start a task in PhillipChaffee/personal-ai-setup on branch main\u{2026}"
            ),
            "the screen starts settled on the first allowlisted repo"
        );

        screen.press("title=\"Repository\"");
        screen.press_nth("class=\"choice", 1);
        let html = screen.markup();
        assert!(
            html.contains("Start a task in PhillipChaffee/scratch\u{2026}"),
            "picking a repo did not reach the sentence that names it: {html:.400}"
        );
        assert!(
            !html.contains("on branch main"),
            "the old repo's branch rode into the new one: {html:.400}"
        );
        assert!(
            !html.contains("Repositories (2)"),
            "the sheet stayed open over the choice it had just taken: {html:.400}"
        );
    }

    /// The branch is the other half of that sentence, and picking one has to
    /// reach it — otherwise the screen goes on promising the repo's default
    /// while the session is cut from something else.
    #[test]
    fn choosing_a_branch_names_it_in_the_sentence() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_new, new_probe);

        screen.press("title=\"Base branch\"");
        screen.press_nth("class=\"choice", 1);
        let html = screen.markup();
        assert!(
            html.contains("on branch release/2.x\u{2026}"),
            "the chosen branch never reached the placeholder: {html:.400}"
        );
        assert!(
            html.contains("chip-name\">release/2.x<"),
            "and the pill has to agree with it: {html:.400}"
        );
    }

    /// A model is chosen, never defaulted into: it decides what the work
    /// costs, how good it is and — through privacy hard rule 1 — who gets to
    /// see the code. The send button stays shut until one is picked, and the
    /// task text alone does not open it.
    #[test]
    fn a_session_cannot_be_started_until_a_model_is_picked() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_new, new_probe);
        let said = screen.markup();
        assert!(
            said.contains("title=\"Start the session\" disabled=true"),
            "an empty draft cannot start a session: {said:.400}",
        );

        screen.type_into("class=\"compose-field\"", "rotate the certificate");
        let typed = screen.markup();
        assert!(
            typed.contains("title=\"Start the session\" disabled=true"),
            "a task with no model behind it started a session anyway: {typed:.400}"
        );
        assert!(
            typed.contains("composer-chip action model needed"),
            "rule 8: the pill that is why the button is shut wears the dot: \
             {typed:.400}"
        );

        screen.press("title=\"Model");
        screen.press_nth("class=\"choice", 0);
        let picked = screen.markup();
        assert!(
            picked.contains("chip-model\">Claude Sonnet 4.5<"),
            "the picked model never reached its pill: {picked:.400}"
        );
        assert!(
            !picked.contains("title=\"Start the session\" disabled=true"),
            "every pill is answered and the button is still shut: {picked:.400}"
        );
        assert!(
            !picked.contains("composer-chip action model needed"),
            "the pill kept its dot after it was answered: {picked:.400}"
        );

        screen.press("title=\"Start the session\"");
        let sent = screen.markup();
        assert!(
            sent.contains("Code plane not connected"),
            "a create with no client behind it went silently nowhere: {sent:.400}"
        );
    }

    /// The mode pill is the agent the first turn runs as, and the picker is
    /// the only thing that can change it.
    #[test]
    fn choosing_a_mode_renames_the_pill() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_new, new_probe);
        assert!(
            screen.markup().contains("chip-label\">Build<"),
            "a turn naming no agent runs as build, and the pill says so"
        );

        screen.press("title=\"Mode\"");
        screen.press_nth("class=\"choice", 1);
        let html = screen.markup();
        assert!(
            html.contains("chip-label\">Plan<"),
            "the picked agent never reached the pill, so the chip and the \
             session would disagree about what the first turn runs as: \
             {html:.400}"
        );
    }

    /// Discard means discard, and the photos picked beside the draft go with
    /// it. They live in a tray of their own — leaving them behind would put
    /// them in the NEXT new session, on whatever repo that one is pointed at.
    #[test]
    fn discarding_a_draft_takes_the_photos_with_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(
            |ctx| {
                seed_new(ctx);
                ctx.new_attachments
                    .clone()
                    .set(vec![crate::attach::PendingAttachment {
                        record: crate::attach::Attachment {
                            name: "screenshot.png".to_owned(),
                            mime: "image/png".to_owned(),
                            size: 128,
                            thumb: String::new(),
                        },
                        data: "AAAA".to_owned(),
                        text: None,
                    }]);
            },
            new_probe,
        );
        assert!(
            screen.markup().contains("new:1"),
            "the draft starts with a photo beside it"
        );

        screen.press("title=\"Discard this session\"");
        let html = screen.markup();
        assert!(
            html.contains("probe\">list|"),
            "discarding the draft did not leave the screen: {html:.400}"
        );
        assert!(
            html.contains("new:0"),
            "the discarded draft's photo survived into the next session's \
             tray: {html:.400}"
        );
    }
    fn seed_chat_no_session(ctx: &AppCtx) {
        stub_client(ctx);
        open_chat(ctx);
        let mut chat = ctx.code_chat;
        chat.write().session_id = None;
    }

    fn seed_running_chat(ctx: &AppCtx) {
        seed_chat(ctx);
        let mut chat = ctx.code_chat;
        chat.write().running = true;
    }

    /// The chat screen before a chat has been opened on it: the manager's
    /// record has not landed, so there is no id to act on.
    fn seed_unopened_chat(ctx: &AppCtx) {
        stub_client(ctx);
        ctx.code_screen.clone().set(CodeScreen::Chat);
    }

    // -------------------------------------------------------- the chat screen

    /// The composer's round trip: what is typed reaches the draft, sending
    /// puts it in the transcript, and the field is emptied so the next message
    /// does not start with the last one.
    #[test]
    fn sending_moves_the_draft_into_the_transcript_and_empties_the_field() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);

        screen.type_into("class=\"input\"", "rotate the certificate");
        let said = screen.markup();
        assert!(
            said.contains("value=\"rotate the certificate\""),
            "what was typed never reached the draft: {said:.400}",
        );

        screen.press("title=\"Send\"");
        let html = screen.markup();
        assert!(
            !html.contains("value=\"rotate the certificate\""),
            "the field kept the message it had just sent, so the next one \
             would start with it: {html:.400}"
        );
        assert!(
            html.contains("rotate the certificate"),
            "the sent message is not in the transcript: {html:.400}"
        );
    }

    /// Return sends and Shift-Return writes a line, which is the difference
    /// between a composer and a form field. Getting it backwards sends half a
    /// paragraph to an agent.
    #[test]
    fn return_sends_and_shift_return_does_not() {
        let _alone = alone();
        let mut sent = Pressable::mount(seed_chat, chat_probe);
        sent.type_into("class=\"input\"", "ship it");
        sent.enter("class=\"input\"", Modifiers::empty());
        let said = sent.markup();
        assert!(
            !said.contains("value=\"ship it\""),
            "Return did not send: {said:.400}",
        );

        let mut kept = Pressable::mount(seed_chat, chat_probe);
        kept.type_into("class=\"input\"", "ship it");
        kept.enter("class=\"input\"", Modifiers::SHIFT);
        let said = kept.markup();
        assert!(
            said.contains("value=\"ship it\""),
            "Shift-Return sent the draft instead of writing a new line into \
             it: {said:.400}",
        );
    }

    /// Send with nothing to send does nothing at all — no empty bubble in the
    /// transcript, no request, no toast.
    #[test]
    fn an_empty_composer_sends_nothing() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);
        let before = screen.markup();

        screen.press("title=\"Send\"");
        assert_eq!(
            screen.markup(),
            before,
            "pressing Send on an empty composer changed the screen, so \
             something was sent"
        );
    }

    /// A send that never left keeps what you typed. The request is answered
    /// long after the handler returns, so the draft is cleared on the way
    /// out — and only when there was a way out.
    #[test]
    fn a_send_with_no_code_plane_says_so_and_keeps_the_draft() {
        let _alone = alone();
        let mut screen = Pressable::mount(open_chat, chat_probe);

        screen.type_into("class=\"input\"", "rotate the certificate");
        screen.press("title=\"Send\"");
        let html = screen.markup();
        assert!(
            html.contains("Code plane not connected"),
            "a send that went nowhere said nothing: {html:.400}"
        );
        assert!(
            html.contains("value=\"rotate the certificate\""),
            "the draft was thrown away by a send that never happened: \
             {html:.400}"
        );
    }

    /// Going back re-reads the list. The rows and the asks are one statement
    /// about it — refreshing half is how a chat that had gone quiet came back
    /// showing a fresh timestamp and no sign that it was blocked.
    #[test]
    fn the_back_arrow_leaves_for_the_list_and_re_reads_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);

        screen.press("class=\"icon-btn back\"");
        let said = screen.markup();
        assert!(
            said.contains("probe\">list|"),
            "the back arrow did not leave the chat: {said:.400}",
        );

        screen.settle();
        let said = screen.markup();
        assert!(
            said.contains("Failed to list code chats"),
            "leaving the chat did not ask the manager for the list again, so \
             the rows behind it stay as stale as they were: {said:.400}",
        );
    }

    /// The chat's own way to start another one.
    #[test]
    fn the_chat_offers_the_draft_screen_too() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);

        screen.press("title=\"New session\"");
        let said = screen.markup();
        assert!(
            said.contains("probe\">new|"),
            "the chat's plus button went nowhere: {said:.400}",
        );
    }

    /// The overflow is the only way to delete the chat you are in, and it
    /// asks before it does. Answering yes leaves for the list, because the
    /// screen you were on is about to stop existing.
    #[test]
    fn the_overflow_is_the_only_way_to_delete_the_open_chat() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);
        assert!(
            !screen.markup().contains("Delete session"),
            "the menu is open before it was asked for"
        );

        screen.press("title=\"More\"");
        let said = screen.markup();
        assert!(
            said.contains("Delete session"),
            "the overflow button did not open the menu: {said:.400}",
        );

        screen.press("class=\"setting-row danger\"");
        let asked = screen.markup();
        assert!(
            asked.contains("Delete this session?"),
            "the menu deleted the chat without asking: {asked:.400}"
        );
        assert!(
            !asked.contains("Delete session"),
            "the menu stayed up behind its own confirm: {asked:.400}"
        );

        screen.press("class=\"btn danger\"");
        let said = screen.markup();
        assert!(
            said.contains("probe\">list|"),
            "confirming the delete left the reader on a chat that is being \
             purged: {said:.400}",
        );
    }

    /// A tap beside the overflow menu closes it without picking anything.
    #[test]
    fn a_tap_beside_the_overflow_menu_closes_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);

        screen.press("title=\"More\"");
        screen.press("class=\"modal-backdrop\"");
        let html = screen.markup();
        assert!(
            !html.contains("Delete session"),
            "the menu survived a tap beside it: {html:.400}"
        );
        assert!(
            !html.contains("Delete this session?"),
            "closing the menu also armed the confirm: {html:.400}"
        );
    }

    /// A confirm the reader backs out of leaves the chat where it was.
    #[test]
    fn cancelling_the_chats_delete_leaves_the_chat_open() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);

        screen.press("title=\"More\"");
        screen.press("class=\"setting-row danger\"");
        screen.press("class=\"btn secondary\"");
        let html = screen.markup();
        assert!(
            !html.contains("Delete this session?"),
            "Cancel left the confirm up: {html:.400}"
        );
        assert!(
            html.contains("probe\">chat|"),
            "Cancel walked away from the chat anyway: {html:.400}"
        );
    }

    /// A chat with no id yet cannot be deleted, and the screen must not walk
    /// away as though it had been. The guard is a `let ... else` that runs
    /// BEFORE the navigation, and swapping those two lines would leave the
    /// reader on the list with the chat still there.
    #[test]
    fn deleting_a_chat_that_was_never_opened_deletes_nothing() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_unopened_chat, chat_probe);

        screen.press("title=\"More\"");
        screen.press("class=\"setting-row danger\"");
        screen.press("class=\"btn danger\"");
        let html = screen.markup();
        assert!(
            html.contains("probe\">chat|"),
            "a delete that could not name a chat navigated anyway: {html:.400}"
        );
        assert!(
            !html.contains("Delete this session?"),
            "the confirm stayed up: {html:.400}"
        );
    }

    /// The review chip navigates first and fetches after — waking a container
    /// can take the better part of a minute, and a chip that does nothing
    /// visible for that long reads as broken.
    #[test]
    fn the_review_chip_opens_the_review_screen() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);

        screen.press("title=\"Review the session");
        let said = screen.markup();
        assert!(
            said.contains("probe\">diff|"),
            "the Diff chip did not open the review: {said:.400}",
        );
    }

    /// A chat whose container has never been prompted has no session, so
    /// there is no working tree to read. That is said out loud rather than
    /// opening an empty review.
    #[test]
    fn the_review_chip_says_when_there_is_nothing_to_review_yet() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat_no_session, chat_probe);

        screen.press("title=\"Review the session");
        let html = screen.markup();
        assert!(
            html.contains("No changes yet \u{2014} the chat has no session"),
            "an unreviewable chat opened a blank review instead of saying \
             why: {html:.400}"
        );
        assert!(
            html.contains("probe\">chat|"),
            "and it navigated there as well: {html:.400}"
        );
    }

    /// The pull-request chip opens the screen AND asks GitHub. The manager
    /// answers that from its own credential, so it works on a chat that is
    /// fast asleep — which is exactly why the request is worth making on the
    /// tap rather than waiting for a wake.
    #[test]
    fn the_pull_request_chip_opens_the_screen_and_asks_github() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);
        assert!(
            screen.markup().contains("pulls:</p>"),
            "nothing has been asked of GitHub yet"
        );

        screen.press("title=\"Pull requests from this branch\"");
        let said = screen.markup();
        assert!(
            said.contains("probe\">pulls|"),
            "the chip did not open the screen: {said:.400}",
        );

        screen.settle();
        let said = screen.markup();
        assert!(
            !said.contains("pulls:</p>"),
            "the screen was opened without asking GitHub anything, so it would \
             sit on an empty branch for ever: {said:.400}",
        );
    }

    /// The settings chip opens the sheet it summarises, and a pick in it
    /// reaches the chip. The chip is the only place the model is visible from
    /// the composer, so the two disagreeing is the whole failure.
    #[test]
    fn the_settings_chip_opens_the_sheet_and_a_pick_comes_back_to_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);
        assert!(
            screen.markup().contains("chip-model\">Default<"),
            "nothing names this chat's model yet"
        );

        screen.press("title=\"Session settings\"");
        let sheet = screen.markup();
        assert!(
            sheet.contains("<h2>Session settings</h2>"),
            "the chip did not open its sheet: {sheet:.400}"
        );
        assert!(
            sheet.contains("code agent \u{b7} applies from your next message"),
            "the sheet does not say which backend it is about: {sheet:.400}"
        );

        screen.press_nth("class=\"setting-row\"", 0);
        let said = screen.markup();
        assert!(
            said.contains("Claude Sonnet 4.5"),
            "the Model row does not drill into the catalogue: {said:.400}",
        );

        screen.press_nth("class=\"choice", 0);
        let said = screen.markup();
        assert!(
            said.contains("chip-model\">Claude Sonnet 4.5<"),
            "the picked model never reached the composer's chip: {said:.400}",
        );

        // Row 1 is Thinking effort — the model row above it is 0, and Context
        // length below is a fact with nothing to press. Choice 1 is the
        // model's first real tier; choice 0 is `Default`, which is already
        // ticked and would prove nothing.
        screen.press_nth("class=\"setting-row\"", 1);
        screen.press_nth("class=\"choice", 1);
        let said = screen.markup();
        assert!(
            said.contains("chip-effort\">Low<"),
            "the picked thinking effort never reached the chip, which is the \
             one place it is visible at a glance: {said:.400}",
        );

        screen.press("class=\"modal-backdrop\"");
        let said = screen.markup();
        assert!(
            !said.contains("<h2>Session settings</h2>"),
            "a tap beside the sheet left it up: {said:.400}",
        );
    }

    /// The mode chip is the one setting you change mid-conversation, and the
    /// picker is the only way to.
    #[test]
    fn the_mode_chip_opens_its_picker_and_a_pick_renames_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);
        assert!(
            screen.markup().contains("chip-label\">Build<"),
            "the chip resolves to the agent a turn naming none runs as"
        );

        screen.press("title=\"Mode\"");
        let said = screen.markup();
        assert!(
            said.contains("Select mode"),
            "the mode chip did not open its picker: {said:.400}",
        );

        screen.press_nth("class=\"choice", 1);
        let html = screen.markup();
        assert!(
            html.contains("chip-label\">Plan<"),
            "the picked agent never reached the chip: {html:.400}"
        );
        assert!(
            !html.contains("Select mode"),
            "the picker stayed open over the choice it took: {html:.400}"
        );
    }

    /// A tap beside the mode picker closes it and changes nothing. This sheet
    /// has no Cancel row either, so the backdrop is the only way to look at
    /// the list and decide against it.
    #[test]
    fn a_tap_beside_the_mode_picker_leaves_the_agent_alone() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_chat, chat_probe);

        screen.press("title=\"Mode\"");
        screen.press("class=\"modal-backdrop\"");
        let html = screen.markup();
        assert!(
            !html.contains("Select mode"),
            "the picker survived a tap beside it: {html:.400}"
        );
        assert!(
            html.contains("chip-label\">Build<"),
            "closing the picker changed the agent anyway: {html:.400}"
        );
    }

    /// Stop does not pretend. `running` is cleared by the server's answer, so
    /// an abort that never left has to leave the turn where it was and say
    /// why — otherwise the transcript stops streaming on screen while the
    /// agent carries on.
    #[test]
    fn stop_reports_a_failure_rather_than_pretending_the_turn_ended() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_running_chat, chat_probe);
        assert!(
            screen.markup().contains("class=\"typing\""),
            "a running turn shows the dots"
        );

        screen.press("title=\"Stop\"");
        screen.settle();
        let html = screen.markup();
        assert!(
            html.contains("Stop failed:"),
            "an abort that never reached the server said nothing: {html:.400}"
        );
        assert!(
            html.contains("class=\"typing\""),
            "the turn was reported as stopped by a request that failed: \
             {html:.400}"
        );
    }
    fn seed_review(ctx: &AppCtx) {
        open_chat(ctx);
        ctx.code_screen.clone().set(CodeScreen::Diff);
        seed_three_files(ctx);
    }

    fn seed_deleted_file(ctx: &AppCtx) {
        ctx.code_screen.clone().set(CodeScreen::Diff);
        let mut diff = ctx.code_diff;
        diff.set(DiffState {
            files: vec![diff_file(
                "src/old.rs",
                FileStatus::Deleted,
                0,
                2,
                DELETED_PATCH,
            )],
            ..DiffState::default()
        });
    }

    fn seed_gapped_file(ctx: &AppCtx) {
        ctx.code_screen.clone().set(CodeScreen::Diff);
        let mut diff = ctx.code_diff;
        diff.set(DiffState {
            files: vec![diff_file(
                "src/wide.rs",
                FileStatus::Modified,
                2,
                0,
                &gapped_patch(),
            )],
            ..DiffState::default()
        });
    }

    fn seed_pulls(ctx: &AppCtx) {
        open_chat(ctx);
        ctx.code_screen.clone().set(CodeScreen::Pulls);
        let mut pulls = ctx.code_pulls;
        pulls.set(PullsState {
            pulls: vec![pull(
                42,
                "Rotate the cert",
                PullState::Open,
                Checks::Passing,
                Some(true),
            )],
            loaded: true,
            ..PullsState::default()
        });
    }

    fn seed_pull_without_a_url(ctx: &AppCtx) {
        seed_pulls(ctx);
        let mut pulls = ctx.code_pulls;
        pulls.write().pulls[0].url = String::new();
    }

    // ------------------------------------------------------ the review screen

    /// The review is a screen, so it has a way back to the conversation it is
    /// about.
    #[test]
    fn the_reviews_back_arrow_returns_to_the_chat() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_review, diff_probe);

        screen.press("class=\"icon-btn back\"");
        let said = screen.markup();
        assert!(
            said.contains("probe\">chat|"),
            "the review has no way back to the chat: {said:.400}",
        );
    }

    /// Soft wrap on and off. A no-wrap body is its own horizontal scrollport,
    /// which is the only way to read a long line on a 402px screen — and the
    /// control has to name the state it would move TO, not the one it is in.
    #[test]
    fn the_wrap_toggle_reaches_every_body_and_renames_itself() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_review, diff_probe);
        let wrapped = screen.markup();
        assert!(
            wrapped.contains("aria-pressed=\"true\"") && !wrapped.contains("diff-body nowrap"),
            "the review soft-wraps until somebody turns it off: {wrapped:.400}"
        );

        screen.press("title=\"Scroll long lines instead of wrapping\"");
        let scrolled = screen.markup();
        assert_eq!(
            scrolled.matches("diff-body nowrap").count(),
            2,
            "the toggle reached one open body and not the other: {scrolled:.400}"
        );
        assert!(
            scrolled.contains("title=\"Wrap long lines\""),
            "the control still offers the state it is already in: {scrolled:.400}"
        );
    }

    /// The bulk mark finishes the review and then stops being offered — a
    /// control that can do nothing must not be on screen (rule 11).
    #[test]
    fn marking_every_file_reviewed_finishes_the_review() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_review, diff_probe);
        assert!(
            screen.markup().contains("1 of 3 files reviewed"),
            "one of the three starts marked"
        );

        screen.press("class=\"btn small secondary\"");
        let html = screen.markup();
        assert!(
            html.contains("3 of 3 files reviewed") && html.contains("style=\"width: 100%\""),
            "Mark all did not mark them all: {html:.400}"
        );
        assert!(
            !html.contains(">Mark all<"),
            "there is nothing left to mark and the button is still there: \
             {html:.400}"
        );
        assert!(
            !html.contains("diff-body"),
            "marking a file reviewed folds it, which is what stops a long diff \
             making you scroll past work you have finished with: {html:.400}"
        );
    }

    /// A band folds and unfolds on its head, independently of whether it is
    /// marked reviewed. The native `<summary>` toggle is suppressed so `open`
    /// stays the app's to decide — a DOM that had toggled itself would not
    /// tell the app about it, and the next render would fold it back.
    #[test]
    fn a_bands_head_folds_it_and_unfolds_it_again() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_review, diff_probe);
        assert_eq!(
            screen.markup().matches("diff-body").count(),
            2,
            "the reviewed file starts folded and the other two open"
        );

        screen.press_nth("class=\"diff-file-head\"", 0);
        let said = screen.markup();
        assert_eq!(
            said.matches("diff-body").count(),
            3,
            "the reviewed file's head did not unfold it: {said:.400}",
        );

        screen.press_nth("class=\"diff-file-head\"", 0);
        let said = screen.markup();
        assert_eq!(
            said.matches("diff-body").count(),
            2,
            "the same head did not fold it again: {said:.400}",
        );
    }

    /// Ticking one file's box marks it reviewed, moves the count, and folds
    /// it. The tick is inside a row-sized target and has to stop the click
    /// reaching the head, or marking would also unfold what it just folded.
    #[test]
    fn ticking_a_file_marks_it_reviewed_and_folds_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_review, diff_probe);

        screen.press_nth("class=\"diff-seen\"", 1);
        let html = screen.markup();
        assert!(
            html.contains("2 of 3 files reviewed"),
            "ticking a file did not move the count: {html:.400}"
        );
        assert_eq!(
            html.matches("aria-pressed=\"true\"").count(),
            3,
            "the wrap toggle plus two ticked boxes — a different number means \
             the press landed on the wrong file: {html:.400}"
        );
        assert_eq!(
            html.matches("diff-body").count(),
            1,
            "the file that was just marked reviewed stayed open: {html:.400}"
        );
    }

    /// A deletion's lines are not shown by default — its patch is one `-` row
    /// per line of the file that used to be there — but "not shown" is not
    /// "not available", and the band is the way to ask for them.
    #[test]
    fn a_deletions_lines_come_back_when_the_band_is_pressed() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_deleted_file, diff_probe);
        assert!(
            !screen.markup().contains("gone one"),
            "a deletion's lines are not rendered until they are asked for"
        );

        screen.press("class=\"diff-skip\"");
        let html = screen.markup();
        assert!(
            html.contains("diff-code\">gone one<") && html.contains("diff-code\">gone two<"),
            "the reveal handed nothing back: {html:.400}"
        );
        assert!(
            !html.contains("Show removed lines"),
            "the reveal is still being offered after it was taken: {html:.400}"
        );
    }

    /// `Snapshot.diffFull` sends the whole file in one hunk, so the band
    /// standing in for the untouched middle is what makes a three-line change
    /// readable — and pressing it has to give those lines back.
    #[test]
    fn expanding_a_band_gives_back_the_lines_it_stood_for() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_gapped_file, diff_probe);
        assert!(
            screen.markup().contains("\u{22ef} 14 unchanged lines"),
            "the middle starts collapsed"
        );

        screen.press("class=\"diff-skip\"");
        let html = screen.markup();
        assert!(
            html.contains("diff-code\">context line 3<"),
            "expanding the band gave nothing back: {html:.400}"
        );
        assert!(
            !html.contains("\u{22ef} 14 unchanged lines"),
            "the band still claims to be hiding what it just revealed: \
             {html:.400}"
        );
    }

    // ------------------------------------------------ the pull-request screen

    /// The pull-request screen is a screen too.
    #[test]
    fn the_pull_screens_back_arrow_returns_to_the_chat() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_pulls, pulls_probe);

        screen.press("class=\"icon-btn back\"");
        let said = screen.markup();
        assert!(
            said.contains("probe\">chat|"),
            "the pull-request screen has no way back: {said:.400}",
        );
    }

    /// Merge is the last thing between a thumb and a commit on GitHub, so it
    /// asks — and the question carries the two facts that decide whether
    /// merging NOW is right, neither of which is on the button.
    #[test]
    fn merging_asks_first_and_names_where_it_lands() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_pulls, pulls_probe);
        assert!(
            !screen.markup().contains("Merge #42?"),
            "nothing has been asked yet"
        );

        screen.press("class=\"btn small primary\"");
        let asked = screen.markup();
        assert!(
            asked.contains("Merge #42?"),
            "Merge went straight to GitHub with no confirmation: {asked:.400}"
        );
        assert!(
            asked.contains("merges into main on GitHub, straight away")
                && asked.contains("Its checks have passed."),
            "the confirm has to say where it lands and what the checks said: \
             {asked:.400}"
        );

        screen.press("class=\"btn secondary\"");
        let after = screen.markup();
        assert!(
            !after.contains("Merge #42?"),
            "Cancel left the question up: {after:.400}"
        );
        assert!(
            after.contains(">Merge<"),
            "Cancel took the button away with the question: {after:.400}"
        );
    }

    /// Answering yes hands it to the manager. With nothing behind the manager
    /// the merge cannot happen, and saying so is the difference between a
    /// merge that failed and a tap that did nothing.
    #[test]
    fn confirming_a_merge_with_no_code_plane_says_so() {
        let _alone = alone();
        let mut screen = Pressable::mount(seed_pulls, pulls_probe);

        screen.press("class=\"btn small primary\"");
        screen.press("class=\"btn primary\"");
        let html = screen.markup();
        assert!(
            html.contains("Code plane not connected"),
            "a merge that never left said nothing: {html:.400}"
        );
        assert!(
            !html.contains("Merge #42?"),
            "the confirm stayed up after it was answered: {html:.400}"
        );
    }

    /// The row opens GitHub, the way a session row opens its session — and a
    /// pull request the manager sent without a web address says so instead of
    /// swallowing the tap. The URL is checked before it is opened because
    /// `javascript:` through `window.open` runs rather than opens.
    #[test]
    fn a_pull_row_opens_github_and_says_when_it_cannot() {
        let _alone = alone();
        let mut good = Pressable::mount(seed_pulls, pulls_probe);
        good.press("title=\"Open on GitHub\"");
        let said = good.markup();
        assert!(
            said.contains("probe\">pulls||"),
            "opening a real pull request should say nothing at all: {said:.400}",
        );

        let mut bare = Pressable::mount(seed_pull_without_a_url, pulls_probe);
        bare.press("title=\"Open on GitHub\"");
        let said = bare.markup();
        assert!(
            said.contains("This pull request came without a web address"),
            "a row with nowhere to go swallowed the tap silently: {said:.400}",
        );
    }
    // PRESS-APPEND-HERE
}
