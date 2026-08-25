use dioxus::dioxus_core::spawn_forever;
use dioxus::prelude::*;
use goose_acp_client::{SessionInfo, SessionKind};

use crate::icons::Icon;
use crate::state::{
    new_session, open_session, refresh_sessions, relative_time, rename_session, rfc3339_to_epoch,
    search_sessions, show_toast, use_app_ctx,
};
use crate::views::chrome::{ListRow, RowAction, RowFace, SearchField, TopBar};
use crate::views::{ConfirmDelete, RenameSheet};

#[component]
pub fn SessionsView() -> Element {
    let ctx = use_app_ctx();
    let sessions = (ctx.sessions)();
    let loading = (ctx.sessions_loading)();
    let query = (ctx.sessions_query)();
    let has_more = ctx.sessions_next.read().is_some();
    let mut confirm_delete = use_signal(|| None::<String>);
    let mut rename = use_signal(|| None::<(String, String)>);

    // A search box on a list with nothing in it and nothing searched is a
    // control offering to filter zero rows. It stays once a search is running,
    // though — otherwise the field that emptied the list disappears with it,
    // and there is no way back except reconnecting.
    let searchable = !sessions.is_empty() || !query.trim().is_empty();

    rsx! {
        TopBar { title: "Chats", conn: true }
        main {
            class: "scroll has-fab",
            // Named, so the pull-to-refresh listener knows this list has
            // something to fetch and which fetch it is.
            "data-refresh": "chats",
            "data-refreshing": "{loading}",
            if searchable {
                div { class: "session-search",
                    SearchField {
                        // goose searches message text, not titles, so the
                        // placeholder says messages. A box labelled "Search
                        // chats" that misses a chat named for the thing you
                        // typed reads as a broken search rather than a
                        // different one.
                        placeholder: "Search messages",
                        // The filter lives on the context and the box does
                        // not: opening a chat unmounts this screen, so the
                        // field has to be told what is already being searched
                        // or it comes back blank over a filtered list.
                        value: query.clone(),
                        on_search: move |text: String| {
                            spawn_forever(async move { search_sessions(&ctx, text).await });
                        },
                    }
                }
            }

            if let Some(sentence) = empty_state(&sessions, loading, &query) {
                p { class: "empty", "{sentence}" }
            }

            ul { class: "session-list",
                for info in sessions {
                    ListRow {
                        key: "{info.session_id}",
                        icon: session_icon(info.kind()),
                        title: info.display_title(),
                        trailing: info.updated_at.as_deref()
                            .and_then(rfc3339_to_epoch)
                            .map(relative_time),
                        // Rename before Delete because the tray is a
                        // scroller: a short drag reveals the first button and
                        // a full one reaches the last, so the destructive
                        // action is the deeper pull.
                        actions: vec![
                            RowAction::new(RowFace::plain("Rename", "pencil"), EventHandler::new({
                                let row = (info.session_id.clone(), info.display_title());
                                move |()| rename.set(Some(row.clone()))
                            })),
                            RowAction::delete(EventHandler::new({
                                let session_id = info.session_id.clone();
                                move |()| confirm_delete.set(Some(session_id.clone()))
                            })),
                        ],
                        on_open: EventHandler::new({
                            let info = info.clone();
                            move |()| open_session(&ctx, info.clone())
                        }),
                        // The `if let` is outside the wrapper, not inside it:
                        // both halves of the line are optional, and a server
                        // that omits `messageCount` on an ordinary chat would
                        // otherwise leave an empty .session-meta whose
                        // `margin-top` still opens a gap above the quote.
                        if let Some(parts) = session_meta(&info) {
                            div { class: "session-meta",
                                for part in parts {
                                    span { key: "{part}", "{part}" }
                                }
                            }
                        }
                        if let Some(snippet) = info.last_message_snippet() {
                            div { class: "session-quote", "{snippet}" }
                        }
                    }
                }
            }

            if has_more {
                div { class: "btn-row",
                    button {
                        class: "btn secondary grow",
                        disabled: loading,
                        onclick: move |_| {
                            spawn_forever(async move { refresh_sessions(&ctx, true).await });
                        },
                        "Load more"
                    }
                }
            }
        }

        button {
            class: "fab",
            onclick: move |_| new_session(&ctx),
            Icon { name: "plus" }
            "New chat"
        }

        if let Some((session_id, title)) = rename() {
            RenameSheet {
                key: "{session_id}",
                heading: "Rename chat",
                value: title,
                on_cancel: move |()| rename.set(None),
                on_save: move |title: String| {
                    let session_id = session_id.clone();
                    rename.set(None);
                    spawn_forever(async move { rename_session(&ctx, &session_id, &title).await });
                },
            }
        }

        if let Some(session_id) = confirm_delete() {
            ConfirmDelete {
                title: "Delete this chat?",
                body: "The whole conversation goes from the goose server. \
                       This cannot be undone.",
                on_cancel: move |()| confirm_delete.set(None),
                on_confirm: move |()| {
                    let session_id = session_id.clone();
                    confirm_delete.set(None);
                    spawn_forever(async move {
                        let Some(client) = ctx.client.peek().clone() else { return };
                        match client.session_delete(&session_id).await {
                            Ok(()) => {
                                let mut sessions = ctx.sessions;
                                sessions.write().retain(|s| s.session_id != session_id);
                            }
                            Err(e) => show_toast(&ctx, format!("Delete failed: {e}")),
                        }
                    });
                },
            }
        }
    }
}

/// The tile a row wears.
///
/// A scheduled run is the one entry in the list nobody was present for, and
/// the clock says that at a glance. The others keep the conversation tile:
/// an ACP session was opened by another client, but it is still a chat with a
/// transcript you can read, and its "Agent" word carries the difference
/// without a second glyph having to be learned.
const fn session_icon(kind: Option<SessionKind>) -> &'static str {
    match kind {
        Some(SessionKind::Scheduled) => "clock",
        _ => "message",
    }
}

/// The small line under a row's title.
///
/// It used to end with the raw `session_id`. That is a uuid — 36 characters of
/// machine identifier on a list read with a thumb, which is design rule 8's
/// example of what not to do — and it was there because the row had nothing
/// else to say. Now it has: what kind of session this is, on the two kinds
/// where that is not obvious. An ordinary chat gets no word, because a label
/// every row carries is a label that distinguishes nothing.
///
/// `None`, not an empty `Vec`, when there is nothing to say — an ordinary chat
/// from a server that omits `messageCount` has neither half. The caller wants
/// that to mean *no wrapper at all*, because `.session-meta` keeps its
/// `margin-top` when it is empty and opens a gap above the quote.
fn session_meta(info: &SessionInfo) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    if let Some(count) = info.message_count() {
        parts.push(format!("{count} msgs"));
    }
    if let Some(label) = info.kind_label() {
        parts.push(label.to_owned());
    }
    (!parts.is_empty()).then_some(parts)
}

/// What an empty list says, or `None` when it is not empty.
///
/// Two different silences: a search with no hits is not an empty account, and
/// saying "start a new chat" to somebody looking at the word they just typed
/// is the screen answering a question nobody asked.
///
/// There is deliberately no third one for a server that cannot list at all.
/// `session/list` is base ACP rather than a goose extension, so its absence is
/// a broken server and not a feature switched off, and the app has no way to
/// tell the two apart — see the `-32601` note in `refresh_sessions`.
fn empty_state(sessions: &[SessionInfo], loading: bool, query: &str) -> Option<String> {
    if !sessions.is_empty() || loading {
        return None;
    }
    let query = query.trim();
    if query.is_empty() {
        return Some("No sessions yet — start a new chat.".to_owned());
    }
    Some(format!("No chats match “{query}”."))
}

#[cfg(test)]
mod tests {
    use super::{empty_state, session_icon, session_meta};
    use goose_acp_client::{SessionInfo, SessionKind};
    use serde_json::json;

    fn session(kind: Option<&str>, messages: u64) -> SessionInfo {
        let mut meta = json!({ "messageCount": messages });
        if let Some(kind) = kind {
            meta["sessionType"] = json!(kind);
        }
        SessionInfo {
            session_id: "3f2b7c1e-9a44-4f0e-8e2d-5c6a1b0d7e88".to_owned(),
            cwd: None,
            title: Some("Standup".to_owned()),
            updated_at: None,
            meta: Some(meta),
        }
    }

    #[test]
    fn a_scheduled_run_is_the_one_row_that_changes_tile() {
        assert_eq!(session_icon(Some(SessionKind::Scheduled)), "clock");
        assert_eq!(session_icon(Some(SessionKind::User)), "message");
        assert_eq!(session_icon(Some(SessionKind::Acp)), "message");
        assert_eq!(session_icon(None), "message");
    }

    /// The whole point of the line: it is read, so it holds words.
    #[test]
    fn the_meta_line_never_prints_an_identifier() {
        let info = session(Some("scheduled"), 12);
        let parts = session_meta(&info);
        assert_eq!(
            parts,
            Some(vec!["12 msgs".to_owned(), "Scheduled".to_owned()])
        );
        assert!(
            !parts
                .into_iter()
                .flatten()
                .any(|part| part.contains(&info.session_id)),
            "the uuid is back"
        );
    }

    #[test]
    fn only_the_unusual_kinds_are_named() {
        assert_eq!(
            session_meta(&session(Some("user"), 3)),
            Some(vec!["3 msgs".to_owned()])
        );
        assert_eq!(
            session_meta(&session(Some("acp"), 3)),
            Some(vec!["3 msgs".to_owned(), "Agent".to_owned()])
        );
        // A goose old enough not to send the type still lists.
        assert_eq!(
            session_meta(&session(None, 3)),
            Some(vec!["3 msgs".to_owned()])
        );
    }

    /// An empty `.session-meta` still carries its `margin-top`, so a row with
    /// nothing to put on the line must render no wrapper rather than an empty
    /// one.
    #[test]
    fn a_row_with_nothing_to_say_gets_no_line() {
        let mut bare = session(None, 0);
        bare.meta = None;
        assert_eq!(session_meta(&bare), None);
    }

    #[test]
    fn an_empty_search_is_not_an_empty_account() {
        let no_chats = empty_state(&[], false, "").unwrap_or_default();
        assert!(no_chats.contains("start a new chat"), "{no_chats}");

        let no_hits = empty_state(&[], false, " deploy ").unwrap_or_default();
        assert!(no_hits.contains("deploy"), "{no_hits}");
        assert!(!no_hits.contains("start a new chat"), "{no_hits}");

        assert_eq!(empty_state(&[], true, ""), None, "still loading");
        assert_eq!(
            empty_state(&[session(None, 1)], false, "deploy"),
            None,
            "a list with rows in it says nothing"
        );
    }
}
