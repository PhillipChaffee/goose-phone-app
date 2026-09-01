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

    // Which row the desktop's detail column is showing. Ignored on the phone,
    // where the list is not on screen beside it
    // (`views::chrome::row_is_marked`).
    //
    // A memo and not a plain read, and that is the whole reason this line is
    // three: `ctx.chat` is one signal holding the entire transcript, so reading
    // any part of it here would re-render every row of this list on every
    // streamed delta — and on the desktop the chat is streaming in the column
    // next door. The memo re-runs on each of those and wakes this screen only
    // when the id it returns actually changes, which is once per open.
    let open_chat = use_memo(move || ctx.chat.read().session_id.clone());

    // A search box on a list with nothing in it and nothing searched is a
    // control offering to filter zero rows. It stays once a search is running,
    // though — otherwise the field that emptied the list disappears with it,
    // and there is no way back except reconnecting.
    let searchable = !sessions.is_empty() || !query.trim().is_empty();

    rsx! {
        TopBar { title: "Chats", conn: true }
        main {
            class: "scroll has-fab",
            // Named, so the three things that refresh a list know this one
            // has something to fetch and which fetch it is: the phone's pull
            // gesture, and on the desktop ⌘R and arriving here. They meet in
            // `viewport::refresh_named`.
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
                        selected: open_chat.read().as_deref() == Some(info.session_id.as_str()),
                        attention: crate::ask_journal::loss_count(
                            &ctx.lost_asks.read(),
                            &info.session_id,
                        ) > 0,
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
                        // The same panel the Code list draws for a chat that
                        // is blocked on an ask, minus the buttons — because
                        // there is nothing to press. The ask is not waiting;
                        // it is gone, along with the round it belonged to.
                        //
                        // This is the amendment to design rule 13, and the
                        // rule text says so: the rule's subject was a LIVE
                        // ask, which cannot sit unanswered while the app is
                        // away, and "the Chats list gets nothing" was right
                        // about that and silent about this.
                        if let Some(phrase) = lost_ask_phrase(
                            crate::ask_journal::loss_count(
                                &ctx.lost_asks.read(),
                                &info.session_id,
                            ),
                        ) {
                            div { class: "session-ask",
                                p { class: "session-ask-title", "{phrase}" }
                            }
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

/// What a row says about answers this app never got back to goose, or `None`
/// when it has none to report.
///
/// Past tense throughout, because the measurement is past tense: the round is
/// already discarded (`docs/permission-durability.md` section 0), so a phrase
/// like "waiting on you" — which is what the Code list's equivalent row says
/// — would be the one thing that is definitely not true.
fn lost_ask_phrase(count: usize) -> Option<String> {
    match count {
        0 => None,
        1 => Some("An answer never reached goose. That round was discarded.".to_owned()),
        n => Some(format!(
            "{n} answers never reached goose. Those rounds were discarded."
        )),
    }
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
    use super::{empty_state, lost_ask_phrase, session_icon, session_meta};
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

    /// A row with nothing lost draws no panel at all, and one that has lost
    /// something says so in the past tense — the round is already gone, so
    /// the Code list's "waiting on you" would be exactly wrong here.
    #[test]
    fn a_lost_answer_is_reported_as_something_that_already_happened() {
        assert_eq!(lost_ask_phrase(0), None);
        let one = lost_ask_phrase(1).unwrap_or_default();
        assert!(one.contains("discarded"), "{one}");
        assert!(!one.contains("waiting"), "{one}");
        let many = lost_ask_phrase(3).unwrap_or_default();
        assert!(many.starts_with("3 answers"), "{many}");
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

/// Pressing this screen, with a server behind it.
///
/// `crate::testkit` renders a view and reads its markup back, which reaches
/// every arm the context can be put into. It reaches none of this file's
/// HANDLERS: the search box, the two row actions, the row itself, "Load more",
/// the FAB and the four buttons on the two sheets are closures a render never
/// runs, and they were 33 of this file's lines — every one of them something a
/// thumb does, and three of them destructive.
///
/// Half of them do not finish without a server, so there is one: a plain
/// `ws://` JSON-RPC listener on a loopback port, the technique
/// `src/scheduler.rs` established (`ws_url` only reaches for TLS on an
/// `https://` base, so no certificate is involved). It is what turns "the row
/// leaves the list only after the server has agreed" from a hope into a check
/// — the failure it guards against is a list that reports a chat as deleted
/// while the server still has it.
///
/// `pub(crate)` because `views/settings.rs` mounts on it too. Settings is the
/// screen that CREATES the connection this one spends, its Save & Connect ends
/// on the chats list, and a second copy of the socket and the press machinery
/// would be a second copy to keep in step. It is deliberately not in
/// `crate::testkit`: that is the render-only harness the whole suite shares,
/// and folding the three private press harnesses (`views/chat.rs`,
/// `views/recipes.rs`, this one) into it would touch files this change does
/// not own.
#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test scaffolding: a press that cannot find its control, or a \
              socket that will not open, has nothing left to assert — failing \
              loudly there IS the check"
)]
#[expect(
    clippy::significant_drop_tightening,
    reason = "a mounted screen holds the one-at-a-time guard, and it has to \
              hold it until the dom is gone — dropping it at the last \
              assertion is exactly what would let the next test's dom start \
              rendering into the same process-wide storage subscription"
)]
pub(crate) mod pressing {
    use std::any::Any;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
    use std::time::Duration;

    use dioxus::dioxus_core::{AttributeValue, ElementId, Event, Mutation, NoOpMutations};
    use dioxus::html::{
        set_event_converter, PlatformEventData, SerializedFormData, SerializedHtmlEventConverter,
        SerializedMouseData,
    };
    use dioxus::prelude::*;
    use futures_util::{SinkExt as _, StreamExt as _};
    use goose_acp_client::{
        AcpClient, AcpEvent, ConnectConfig, SessionInfo, SessionKind, SessionListResponse,
        SessionQuery,
    };
    use serde_json::{json, Value};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    use crate::state::{AppCtx, ConnState, Screen};

    // ---------------------------------------------------------- the harness

    /// One press at a time, across every module that mounts on this harness.
    ///
    /// Rendering after an event runs the `VirtualDom`'s task queue, and
    /// `use_app_ctx_provider`'s ask journal is a `use_synced_storage` whose
    /// watch channel `dioxus-sdk-storage` keys in a `static` — process-wide, so
    /// every mount in the binary is on the same one. Two mounts rendering at
    /// once feed each other through it and never settle; `views/chat.rs`
    /// measured that at 7 wedged runs in 40, each a thread spinning inside
    /// `watch::Receiver::changed`, and 0 in 40 serialized. The guard lives on
    /// [`Mounted`] so it is held for exactly as long as a dom exists.
    ///
    /// `PoisonError::into_inner` because a test that fails while holding this
    /// has already reported what it exists to report; taking the rest of the
    /// module down behind it would only hide which one broke.
    fn alone() -> MutexGuard<'static, ()> {
        static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
        ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What to put in the context, and what to render over it. A thread-local
    /// rather than props because `cargo test` gives every test its own thread,
    /// and this way the component needs no props type of its own.
    type Mount = (fn(&AppCtx), fn() -> Element);

    thread_local! {
        static MOUNT: Cell<Option<Mount>> = const { Cell::new(None) };
        /// The context [`Probe`] built, so a test can read the app's state and
        /// not only the markup it produced.
        static PUBLISHED: RefCell<Option<AppCtx>> = const { RefCell::new(None) };
    }

    #[expect(
        non_snake_case,
        reason = "a Dioxus component is named like a component, not like a fn"
    )]
    fn Probe() -> Element {
        let ctx = crate::state::use_app_ctx_provider();
        let (seed, view) = MOUNT.with(Cell::get).expect("nothing was mounted");
        use_hook(|| seed(&ctx));
        PUBLISHED.with(|slot| *slot.borrow_mut() = Some(ctx));
        view()
    }

    /// A mounted view that can be pressed and typed into, with a runtime under
    /// it for the tasks a press starts.
    pub(crate) struct Mounted {
        dom: VirtualDom,
        /// Every mutation the dom has emitted since it was mounted, in order.
        /// A re-render describes only what CHANGED, so they accumulate — and
        /// every lookup starts at the END, which is what makes the newest thing
        /// on screen the one a press finds.
        edits: Vec<Mutation>,
        /// Where in `edits` the most recent render that BUILT A CONTROL starts,
        /// for the positional presses: they are about what that render put down
        /// — the sheet that just opened — rather than about the screen as a
        /// whole. Renders that only changed an attribute do not move it,
        /// because typing into a sheet's field must not make the sheet's own
        /// buttons unreachable.
        latest: usize,
        rt: tokio::runtime::Runtime,
        ctx: AppCtx,
        /// The connection's event stream, parked for the harness's lifetime.
        /// The client's actor gives up the socket when this end goes away, so a
        /// harness that dropped it would hold a connection that died between
        /// the handshake and the first request.
        events: Option<tokio::sync::mpsc::Receiver<AcpEvent>>,
        _alone: MutexGuard<'static, ()>,
    }

    impl Mounted {
        pub(crate) fn mount(seed: fn(&AppCtx), view: fn() -> Element) -> Self {
            let guard = alone();
            // The listener `dioxus-html` installs converts the platform's own
            // event into a `MouseData` before the handler sees it, through a
            // process-global converter that panics when it is missing. Setting
            // it twice is setting it once.
            set_event_converter(Box::new(SerializedHtmlEventConverter));
            // The one owner of the storage directory: `set_directory` writes a
            // process-wide `OnceLock` and unwraps it, so a second setter panics.
            let _ = crate::testkit::storage_dir();
            MOUNT.set(Some((seed, view)));
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread tokio runtime with a timer and a reactor");
            let mut dom = VirtualDom::new(Probe);
            let edits = dom.rebuild_to_vec().edits;
            let ctx = PUBLISHED
                .with(|slot| *slot.borrow())
                .expect("the probe rendered, so it published its context");
            Self {
                dom,
                edits,
                latest: 0,
                rt,
                ctx,
                events: None,
                _alone: guard,
            }
        }

        /// Read or write the context. Signals belong to the virtual dom's
        /// runtime and panic outside it, so every touch goes through here.
        pub(crate) fn with<T>(&self, f: impl FnOnce(&AppCtx) -> T) -> T {
            let ctx = self.ctx;
            self.dom.in_runtime(|| f(&ctx))
        }

        pub(crate) fn html(&self) -> String {
            dioxus_ssr::render(&self.dom)
        }

        pub(crate) fn runtime(&self) -> &tokio::runtime::Runtime {
            &self.rt
        }

        /// A goose on a loopback port, answering `script`.
        pub(crate) fn serve(&self, script: Script) -> Server {
            let listener = self
                .rt
                .block_on(async { TcpListener::bind("127.0.0.1:0").await.unwrap() });
            let port = listener.local_addr().unwrap().port();
            let calls: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
            let log = Arc::clone(&calls);
            self.rt.spawn(async move {
                while let Ok((socket, _)) = listener.accept().await {
                    let log = Arc::clone(&log);
                    tokio::spawn(async move { session_loop(socket, log, script).await });
                }
            });
            Server {
                base_url: format!("http://127.0.0.1:{port}"),
                calls,
            }
        }

        /// The same, plus a live [`AcpClient`] on the context — the state the
        /// app is in once Settings has connected.
        pub(crate) fn connect(&mut self, script: Script) -> Server {
            let server = self.serve(script);
            let cfg = ConnectConfig {
                base_url: server.base_url.clone(),
                secret: String::new(),
                fingerprint: None,
            };
            let (client, events, _info) = self
                .rt
                .block_on(AcpClient::connect(&cfg))
                .expect("the mock server accepted the handshake");
            self.events = Some(events);
            self.with(|ctx| {
                let mut slot = ctx.client;
                let mut conn = ctx.conn;
                slot.set(Some(client));
                conn.set(ConnState::Connected {
                    agent: "mock 0".to_owned(),
                });
            });
            server
        }

        /// Let the queued Dioxus tasks — and the socket under them — run.
        ///
        /// Dioxus polls a spawned task from its own executor, so nothing a
        /// press started happens without this; the timeout is what lets the
        /// tokio runtime carrying the WebSocket actor make progress while the
        /// virtual dom has nothing to do. Each slice is 10 ms of *idle*, so the
        /// default budget is 400 ms — comfortably past `SEARCH_DEBOUNCE`, which
        /// is the one deliberate wait on this screen.
        pub(crate) fn settle(&mut self) {
            self.settle_for(40);
        }

        pub(crate) fn settle_for(&mut self, slices: usize) {
            let dom = &mut self.dom;
            let at = self.edits.len();
            let mut edits = Vec::new();
            self.rt.block_on(async {
                for _ in 0..slices {
                    let _ =
                        tokio::time::timeout(Duration::from_millis(10), dom.wait_for_work()).await;
                    edits.extend(dom.render_immediate_to_vec().edits);
                }
            });
            self.edits.extend(edits);
            self.mark_latest(at);
        }

        /// The control that owns `label`, whichever way the label reaches the
        /// screen — and the NEWEST one, so the confirm that just opened wins
        /// over the row that opened it.
        ///
        /// The two kinds of label sit on opposite sides of the element that
        /// owns them, and Dioxus's own ordering is what decides which:
        ///
        ///   - a rendered ATTRIBUTE — a row action's `title`, a row's `class` —
        ///     names its own element in the mutation, so there is nothing to
        ///     infer;
        ///   - a rendered TEXT CHILD — a confirm's `{confirm_label}` — is a node
        ///     of its own, created after the attributes and listeners of the
        ///     template it belongs to, so its owner is the last listener before
        ///     it.
        ///
        /// A literal word lives in the compiled template and is never a
        /// mutation at all; those are pressed positionally below.
        fn control_for(&self, label: &str) -> ElementId {
            let Some(at) = self.edits.iter().rposition(|edit| match edit {
                Mutation::CreateTextNode { value, .. }
                | Mutation::SetAttribute {
                    value: AttributeValue::Text(value),
                    ..
                } => value == label,
                _ => false,
            }) else {
                panic!("nothing on screen says {label:?}, so there is nothing to press")
            };
            if let Mutation::SetAttribute { id, .. } = self.edits[at] {
                return id;
            }
            self.edits[..at]
                .iter()
                .rev()
                .find_map(|edit| match edit {
                    Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
                    _ => None,
                })
                .unwrap_or_else(|| {
                    panic!("{label:?} is on screen but nothing around it is pressable")
                })
        }

        /// Every element carrying a dynamic `name` attribute, in document
        /// order, each named once however often it has been re-rendered.
        ///
        /// For the controls whose every word and class is a literal but whose
        /// availability is not: "Load more" is `disabled: loading` and the two
        /// buttons at the foot of Settings are `disabled: connecting`, and that
        /// one bound attribute is the only thing in the stream that is
        /// unmistakably them. A word in an `if` is not — a literal in either
        /// arm compiles to a template of its own and never becomes a text
        /// mutation, which is exactly why "Test connection" cannot be pressed
        /// by name.
        fn controls_with_attribute(&self, name: &str) -> Vec<ElementId> {
            let mut found: Vec<ElementId> = Vec::new();
            for edit in &self.edits {
                if let Mutation::SetAttribute {
                    name: attribute,
                    id,
                    ..
                } = edit
                {
                    if *attribute == name && !found.contains(id) {
                        found.push(*id);
                    }
                }
            }
            found
        }

        /// Press the nth control carrying a dynamic `name` attribute.
        pub(crate) fn press_with_attribute(&mut self, name: &str, nth: usize) {
            let found = self.controls_with_attribute(name);
            let id = *found.get(nth).unwrap_or_else(|| {
                panic!(
                    "{} elements have a bound {name:?} attribute, so there is \
                     no {nth} to press",
                    found.len()
                )
            });
            self.press_id(id);
        }

        pub(crate) fn press(&mut self, label: &str) {
            let id = self.control_for(label);
            self.press_id(id);
        }

        /// Press the FIRST control the most recent render built: a sheet's
        /// backdrop, a confirm's Cancel.
        pub(crate) fn press_first(&mut self) {
            let id = self.clicks(self.latest).next();
            self.press_id(id.unwrap_or_else(|| panic!("that render built nothing pressable")));
        }

        /// Press the LAST control the most recent render built — a sheet's
        /// Save, whose word is a literal rather than a rendered one.
        pub(crate) fn press_last(&mut self) {
            let id = self.clicks(self.latest).next_back();
            self.press_id(id.unwrap_or_else(|| panic!("that render built nothing pressable")));
        }

        pub(crate) fn press_id(&mut self, id: ElementId) {
            self.dispatch(id, "click", Box::new(SerializedMouseData::default()));
        }

        /// Type into the nth field on screen, exactly as a `WebView` reports
        /// it: the field's whole new value.
        ///
        /// Positional because a text field carries no rendered label of its own
        /// — a `placeholder` is a literal, so it lives in the compiled template
        /// and never appears in this stream. The position is document order: an
        /// element's dynamic attributes, listeners included, are written in the
        /// order the `rsx!` names them, and every field on both of this
        /// harness's screens sits in one template. It is not taken on trust
        /// anywhere — each caller asserts on WHICH value moved, so a keystroke
        /// that landed in the wrong field fails rather than passing quietly.
        pub(crate) fn type_into_nth(&mut self, nth: usize, text: &str) {
            let fields = self.fields();
            let id = *fields.get(nth).unwrap_or_else(|| {
                panic!(
                    "the screen has {} fields, so there is no field {nth}",
                    fields.len()
                )
            });
            self.dispatch(
                id,
                "input",
                Box::new(SerializedFormData::new(text.to_owned(), Vec::new())),
            );
        }

        /// The text fields the most recent render built, in document order.
        ///
        /// Scoped to that render for the same reason the positional presses
        /// are: once a sheet is open, "the first field" means the sheet's, not
        /// the search box still behind it. On a screen that has opened nothing
        /// this is every field it has, because the render that built them is
        /// still the latest one.
        fn fields(&self) -> Vec<ElementId> {
            self.edits[self.latest..]
                .iter()
                .filter_map(|edit| match edit {
                    Mutation::NewEventListener { name, id } if name == "input" => Some(*id),
                    _ => None,
                })
                .collect()
        }

        fn clicks(&self, from: usize) -> impl DoubleEndedIterator<Item = ElementId> + '_ {
            self.edits[from..].iter().filter_map(|edit| match edit {
                Mutation::NewEventListener { name, id } if name == "click" => Some(*id),
                _ => None,
            })
        }

        fn dispatch(&mut self, id: ElementId, name: &str, data: Box<dyn Any>) {
            let payload: Rc<dyn Any> = Rc::new(PlatformEventData::new(data));
            let _timer = self.rt.enter();
            self.dom
                .runtime()
                .handle_event(name, Event::new(payload, true), id);
            let at = self.edits.len();
            self.edits.extend(self.dom.render_immediate_to_vec().edits);
            self.mark_latest(at);
        }

        /// A render starting at `at` becomes "the latest" only if it built
        /// something pressable.
        fn mark_latest(&mut self, at: usize) {
            if self.edits[at..]
                .iter()
                .any(|edit| matches!(edit, Mutation::NewEventListener { .. }))
            {
                self.latest = at;
            }
        }

        /// Deliver one non-bubbling click to one element id, for [`taps_that`].
        /// Without bubbling, or a press would fire the same handler once per
        /// element inside the control and the count would mean nothing.
        fn tap_only(&mut self, id: ElementId) {
            let payload: Rc<dyn Any> = Rc::new(PlatformEventData::new(Box::new(
                SerializedMouseData::default(),
            )));
            let _timer = self.rt.enter();
            self.dom
                .runtime()
                .handle_event("click", Event::new(payload, false), id);
            self.dom.render_immediate(&mut NoOpMutations);
        }
    }

    /// An `ElementId` past the end is ignored rather than fatal
    /// (`Runtime::handle_event` does a `get`), so this only has to be larger
    /// than any screen these tests mount.
    const EVERY_ELEMENT: u32 = 160;

    /// How many of the screen's elements, tapped one at a time in a mount of
    /// their own, leave the app in a state `outcome` recognises.
    ///
    /// The `views/extensions.rs` technique, and it is stronger than naming a
    /// button: "exactly one control on this screen does that" cannot rot into
    /// pointing at the wrong element the way a positional press can, and it is
    /// the only way to reach a control whose every word and class is a literal.
    pub(crate) fn taps_that(
        seed: fn(&AppCtx),
        view: fn() -> Element,
        outcome: fn(&AppCtx) -> bool,
    ) -> usize {
        every_element(seed, view, outcome).count()
    }

    /// The elements that, tapped alone, satisfy `outcome` — the iterator
    /// [`taps_that`] counts, so a caller that needs the id itself can take the
    /// first one.
    pub(crate) fn every_element(
        seed: fn(&AppCtx),
        view: fn() -> Element,
        outcome: fn(&AppCtx) -> bool,
    ) -> impl Iterator<Item = ElementId> {
        (1..=EVERY_ELEMENT).filter_map(move |target| {
            let id = ElementId(target as usize);
            let mut screen = Mounted::mount(seed, view);
            screen.tap_only(id);
            screen.with(outcome).then_some(id)
        })
    }

    // ----------------------------------------------------------- the server

    /// What a mock goose answers, per method. A plain `fn` and never a closure,
    /// so a test's whole script is one readable `match`.
    pub(crate) type Script = fn(&str, &Value) -> Result<Value, Value>;

    pub(crate) struct Server {
        pub(crate) base_url: String,
        calls: Arc<Mutex<Vec<(String, Value)>>>,
    }

    impl Server {
        /// Every request this server was sent, in order. The handshake is left
        /// out: it is the harness's, not the screen's.
        pub(crate) fn methods(&self) -> Vec<String> {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .map(|(method, _)| method.clone())
                .filter(|method| method != "initialize")
                .collect()
        }

        pub(crate) fn count(&self, method: &str) -> usize {
            self.methods().iter().filter(|m| *m == method).count()
        }

        /// The params of the `n`th call to `method`.
        pub(crate) fn params(&self, method: &str, n: usize) -> Value {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == method)
                .nth(n)
                .map(|(_, params)| params.clone())
                .expect("the call the assertion is about was never made")
        }
    }

    async fn session_loop(
        socket: tokio::net::TcpStream,
        log: Arc<Mutex<Vec<(String, Value)>>>,
        script: Script,
    ) {
        let Ok(ws) = tokio_tungstenite::accept_async(socket).await else {
            return;
        };
        let (mut sink, mut stream) = ws.split();
        let (out, mut outbox) = tokio::sync::mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(text) = outbox.recv().await {
                if sink.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        });
        while let Some(Ok(msg)) = stream.next().await {
            let Message::Text(text) = msg else { continue };
            let Ok(frame) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            let Some(id) = frame.get("id").cloned() else {
                continue;
            };
            let method = frame
                .get("method")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let params = frame.get("params").cloned().unwrap_or(Value::Null);
            log.lock().unwrap().push((method.clone(), params.clone()));
            let body = if method == "initialize" {
                Ok(json!({
                    "protocolVersion": 1,
                    "agentInfo": { "name": "mock", "version": "0" },
                }))
            } else {
                script(&method, &params)
            };
            let reply = match body {
                Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
            };
            let _ = out.send(reply.to_string());
        }
    }

    /// A goose that agrees to everything and has one chat.
    ///
    /// The `Result` is not optional even though this arm of it never fails: a
    /// script is a [`Script`], and a server that could not be handed a happy
    /// one would leave every test here running against a refusal.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the signature belongs to `Script`, not to this function"
    )]
    fn happy(method: &str, _params: &Value) -> Result<Value, Value> {
        match method {
            "session/list" => Ok(json!({ "sessions": [wire("s-1", "Rotate the certificate")] })),
            "session/new" => Ok(json!({ "sessionId": "s-new" })),
            _ => Ok(json!({})),
        }
    }

    /// A goose that refuses whatever it is asked to change.
    fn refuses(method: &str, params: &Value) -> Result<Value, Value> {
        match method {
            "session/delete" | "_goose/unstable/session/rename" => {
                Err(json!({ "code": -32000, "message": "read-only session store" }))
            }
            _ => happy(method, params),
        }
    }

    fn wire(id: &str, title: &str) -> Value {
        json!({ "sessionId": id, "cwd": "/home/demo", "title": title })
    }

    // ---------------------------------------------------------------- seeds

    fn session(id: &str, title: &str) -> SessionInfo {
        serde_json::from_value(wire(id, title)).unwrap()
    }

    /// Two chats on screen, which is what makes the search box, the rows and
    /// their trays exist at all.
    fn two_chats(ctx: &AppCtx) {
        let mut sessions = ctx.sessions;
        sessions.set(vec![
            session("s-1", "Rotate the certificate"),
            session("s-2", "Rewrite the audit"),
        ]);
    }

    /// The same, plus the cursor a previous page left behind.
    fn a_page_with_more_behind_it(ctx: &AppCtx) {
        two_chats(ctx);
        let page: SessionListResponse =
            serde_json::from_value(json!({ "sessions": [], "nextCursor": "page-2" })).unwrap();
        let mut next = ctx.sessions_next;
        next.set(SessionQuery::new(&SessionKind::ALL, None).next_page(&page));
    }

    /// A list with a working directory set, so the FAB gets past its first
    /// guard and reaches the server.
    fn two_chats_and_a_working_dir(ctx: &AppCtx) {
        two_chats(ctx);
        let mut settings = ctx.settings;
        settings.write().working_dir = "/home/demo".to_owned();
    }

    fn list() -> Element {
        rsx! { super::SessionsView {} }
    }

    fn toast_says(ctx: &AppCtx, phrase: &str) -> bool {
        ctx.toast
            .peek()
            .as_ref()
            .is_some_and(|toast| toast.contains(phrase))
    }

    fn titles(ctx: &AppCtx) -> Vec<String> {
        ctx.sessions
            .peek()
            .iter()
            .map(SessionInfo::display_title)
            .collect()
    }

    // ------------------------------------------------------------ searching

    /// The box is wired to goose's search, not to the rows already on screen.
    ///
    /// It is a *server* filter — goose matches the text of the messages, which
    /// is the whole reason the placeholder says "messages" — so a box that
    /// quietly filtered the local list instead would answer "deploy" with the
    /// chats whose titles happen to say it and hide every conversation that
    /// discussed it. What proves it is not a local filter is the request.
    #[test]
    fn typing_in_the_search_box_asks_the_server_for_that_search() {
        let mut screen = Mounted::mount(two_chats, list);
        let server = screen.connect(happy);
        assert_eq!(
            server.count("session/list"),
            0,
            "the screen asked for a list before anything was typed"
        );

        screen.type_into_nth(0, "deploy");
        screen.settle_for(80);

        assert_eq!(
            screen.with(|ctx| ctx.sessions_query.peek().clone()),
            "deploy",
            "the query the rest of the app reads never moved, so leaving this \
             screen and coming back would show an unfiltered list under a \
             filtered box"
        );
        assert_eq!(
            server.count("session/list"),
            1,
            "one word typed once should be one request: {:?}",
            server.methods()
        );
        assert_eq!(
            server.params("session/list", 0)["_meta"]["query"],
            json!("deploy"),
            "the search text never reached the server, so the box filters \
             nothing: {}",
            server.params("session/list", 0)
        );
    }

    /// The one search that must not take its own box away with it.
    ///
    /// `searchable` keeps the field once a query is running precisely because
    /// the list it filtered is empty — and if it did not, the control that
    /// emptied the screen would vanish with the rows and the only way back
    /// would be to reconnect.
    #[test]
    fn a_search_that_matches_nothing_keeps_the_box_that_emptied_the_list() {
        fn nothing_matches(method: &str, params: &Value) -> Result<Value, Value> {
            match method {
                "session/list" => Ok(json!({ "sessions": [] })),
                _ => happy(method, params),
            }
        }
        let mut screen = Mounted::mount(two_chats, list);
        let _server = screen.connect(nothing_matches);

        screen.type_into_nth(0, "deploy");
        screen.settle_for(80);

        let html = screen.html();
        assert!(
            html.contains("No chats match “deploy”."),
            "an empty result said nothing about the search that emptied it: {html}"
        );
        assert!(
            html.contains(r#"type="search""#),
            "the search box went away with the rows it filtered, so there is \
             no way back to the full list: {html}"
        );
        assert!(
            html.contains(r#"value="deploy""#),
            "the box came back blank over a filtered list, so the screen and \
             the field disagree about what is being searched: {html}"
        );
    }

    // ----------------------------------------------------------------- rows

    /// A tap on a row opens THAT chat. Each row closes over its own clone of
    /// the session it names, so a list that shared one would open the first
    /// chat whichever row was tapped — and the transcript would be somebody
    /// else's conversation under the title that was pressed. The SECOND row is
    /// pressed throughout this module for exactly that reason.
    #[test]
    fn a_tap_on_a_row_opens_that_rows_chat() {
        let mut screen = Mounted::mount(two_chats, list);
        screen.press("session-item");

        screen.with(|ctx| {
            let chat = ctx.chat.peek();
            assert_eq!(
                chat.session_id.as_deref(),
                Some("s-2"),
                "the second row opened a different chat from the one it names"
            );
            assert_eq!(
                chat.cwd, "/home/demo",
                "the transcript would replay against the wrong directory"
            );
            assert!(
                matches!(*ctx.screen.peek(), Screen::Chat),
                "the row set a chat up and left the reader on the list"
            );
        });
    }

    /// Rename opens on the name it is about to change, because most renames are
    /// a correction to the title goose guessed from the first message. A sheet
    /// that opened empty would make every rename a retype.
    #[test]
    fn rename_opens_the_sheet_on_the_rows_own_title() {
        let mut screen = Mounted::mount(two_chats, list);
        assert!(
            !screen.html().contains("Rename chat"),
            "the rename sheet is up before anything asked for it"
        );

        screen.press("Rename");
        let html = screen.html();
        assert!(
            html.contains("Rename chat"),
            "the row's Rename opened nothing: {html}"
        );
        assert!(
            html.contains(r#"value="Rewrite the audit""#),
            "the sheet opened on an empty field, or on the wrong row's title, \
             so a correction is a retype: {html}"
        );
    }

    /// Saving sends the new title AND puts it on the row, without re-fetching.
    ///
    /// Both halves matter: a rename that reached the server and not the list
    /// shows the old name until the next connection, and a list updated without
    /// the request is a rename that never happened.
    #[test]
    fn saving_a_rename_sends_it_and_the_row_takes_the_new_name() {
        let mut screen = Mounted::mount(two_chats, list);
        let server = screen.connect(happy);

        screen.press("Rename");
        screen.type_into_nth(0, "  Certificate rotation, done  ");
        screen.press_last();
        screen.settle();

        assert_eq!(
            server.params("_goose/unstable/session/rename", 0),
            json!({ "sessionId": "s-2", "title": "Certificate rotation, done" }),
            "the rename that went out is not the one that was typed, or not \
             for the row it was typed on: {:?}",
            server.methods()
        );
        assert_eq!(
            screen.with(titles),
            vec![
                "Rotate the certificate".to_owned(),
                "Certificate rotation, done".to_owned()
            ],
            "the row still shows the name the server no longer has"
        );
        assert!(
            !screen.html().contains("Rename chat"),
            "the sheet stayed up after it was answered"
        );
        assert_eq!(
            server.count("session/list"),
            0,
            "the list was re-fetched to show one changed word, which throws \
             away where the reader had scrolled to"
        );
    }

    /// A rename goose refuses must not be painted as done. The list is updated
    /// in place rather than re-fetched, so an optimistic row would keep a title
    /// the server has never heard of until the next reconnect.
    #[test]
    fn a_refused_rename_leaves_the_row_alone_and_says_why() {
        let mut screen = Mounted::mount(two_chats, list);
        let _server = screen.connect(refuses);

        screen.press("Rename");
        screen.type_into_nth(0, "Certificate rotation, done");
        screen.press_last();
        screen.settle();

        assert_eq!(
            screen.with(titles)[1],
            "Rewrite the audit",
            "the row took a name the server refused to give it"
        );
        assert!(
            screen.with(|ctx| toast_says(ctx, "Rename failed")),
            "a refused rename said nothing at all: {:?}",
            screen.with(|ctx| ctx.toast.peek().clone())
        );
    }

    /// The sheet has a way out that is not Save. Tapping the backdrop is the
    /// phone's own gesture for "not this", and a sheet that answered it by
    /// renaming would turn a mis-tap into an edit of somebody's history.
    #[test]
    fn dismissing_the_rename_sheet_renames_nothing() {
        let mut screen = Mounted::mount(two_chats, list);
        let server = screen.connect(happy);

        screen.press("Rename");
        screen.press_first();
        screen.settle();

        assert!(
            !screen.html().contains("Rename chat"),
            "the sheet stayed up after it was dismissed"
        );
        assert_eq!(
            server.count("_goose/unstable/session/rename"),
            0,
            "dismissing the sheet renamed the chat anyway"
        );
        assert_eq!(screen.with(titles)[1], "Rewrite the audit");
    }

    /// goose's `session/delete` is not a soft delete and there is no undo, so a
    /// swipe — or, on the desktop, a click on an always-visible icon — is not
    /// consent on its own. The sentence has to say the conversation goes from
    /// the server, because it is not on the phone in the first place.
    #[test]
    fn deleting_a_chat_is_asked_before_it_is_done() {
        let mut screen = Mounted::mount(two_chats, list);
        let server = screen.connect(happy);

        screen.press("Delete");
        let html = screen.html();
        assert!(
            html.contains("Delete this chat?"),
            "the row's delete went straight to the server with no question in \
             between: {html}"
        );
        assert!(
            html.contains("The whole conversation goes from the goose server."),
            "the confirm does not say what is destroyed or where: {html}"
        );
        assert_eq!(
            server.count("session/delete"),
            0,
            "the question was asked after the deletion"
        );

        screen.press_first();
        screen.settle();
        assert!(
            !screen.html().contains("Delete this chat?"),
            "Cancel left the question on screen"
        );
        assert_eq!(
            server.count("session/delete"),
            0,
            "declining the confirm deleted the chat anyway"
        );
        assert_eq!(screen.with(titles).len(), 2, "a declined delete took a row");
    }

    /// The row leaves only once the server has agreed, and only that row.
    #[test]
    fn a_confirmed_delete_takes_that_row_after_the_server_agrees() {
        let mut screen = Mounted::mount(two_chats, list);
        let server = screen.connect(happy);

        screen.press("Delete");
        screen.press("Delete");
        screen.settle();

        assert_eq!(
            server.params("session/delete", 0),
            json!({ "sessionId": "s-2" }),
            "the delete that went out is not the row that was swiped: {:?}",
            server.methods()
        );
        assert_eq!(
            screen.with(titles),
            vec!["Rotate the certificate".to_owned()],
            "the confirmed row is still in the list, or it took its neighbour \
             with it"
        );
    }

    /// A delete goose refuses must leave the row where it is. Removing it
    /// anyway would report a chat as gone that the server still has, and the
    /// list is not re-fetched afterwards — so the lie would survive until the
    /// next connection.
    #[test]
    fn a_refused_delete_keeps_the_row_and_says_why() {
        let mut screen = Mounted::mount(two_chats, list);
        let _server = screen.connect(refuses);

        screen.press("Delete");
        screen.press("Delete");
        screen.settle();

        assert_eq!(
            screen.with(titles).len(),
            2,
            "the row went from the list on a delete the server refused"
        );
        assert!(
            screen.with(|ctx| toast_says(ctx, "Delete failed")),
            "a refused delete said nothing at all: {:?}",
            screen.with(|ctx| ctx.toast.peek().clone())
        );
    }

    // ------------------------------------------------------------- the tail

    /// "Load more" fetches the NEXT page and appends it. goose ties a cursor to
    /// a hash of the filters it was minted under, so the button has to send
    /// that cursor back — and appending is the point: a button that replaced
    /// the list would scroll the reader to the top of a page they had already
    /// read past.
    #[test]
    fn load_more_sends_the_cursor_back_and_appends_the_page() {
        fn second_page(method: &str, params: &Value) -> Result<Value, Value> {
            match method {
                "session/list" => Ok(json!({ "sessions": [wire("s-3", "Older business")] })),
                _ => happy(method, params),
            }
        }
        let mut screen = Mounted::mount(a_page_with_more_behind_it, list);
        let server = screen.connect(second_page);
        assert!(
            screen.html().contains("Load more"),
            "a list with a next page offered no way to reach it"
        );

        screen.press_with_attribute("disabled", 0);
        screen.settle();

        assert_eq!(
            server.params("session/list", 0)["cursor"],
            json!("page-2"),
            "the next page was asked for without the cursor that identifies \
             it: {}",
            server.params("session/list", 0)
        );
        assert_eq!(
            screen.with(titles),
            vec![
                "Rotate the certificate".to_owned(),
                "Rewrite the audit".to_owned(),
                "Older business".to_owned()
            ],
            "the second page replaced the first instead of following it"
        );
        assert!(
            !screen.html().contains("Load more"),
            "the button is still offering a page the server said was the last"
        );
    }

    /// The FAB is the only way to start a chat from this screen, and with no
    /// working directory configured it has to say so. goose refuses a relative
    /// `cwd` outright, so the alternative is a button that looks broken.
    ///
    /// Counted rather than pressed by name: every word and class on this
    /// control is a literal, and "exactly one control does this" is a stronger
    /// claim than "this one does".
    #[test]
    fn exactly_one_control_starts_a_new_chat_and_it_says_what_is_missing() {
        fn asked_for_a_directory(ctx: &AppCtx) -> bool {
            toast_says(ctx, "Set an absolute working directory")
        }
        assert_eq!(
            taps_that(two_chats, list, asked_for_a_directory),
            1,
            "either nothing on the chats list starts a new chat, or more than \
             one thing does"
        );
    }

    /// With a directory set, the same press reaches the server and lands in the
    /// conversation the server made.
    ///
    /// The FAB is found the same way it is counted above, and by the state it
    /// leaves with no connection under it: it asks for one, and — unlike a row,
    /// which asks for the same thing — it opens no chat while doing so.
    #[test]
    fn the_new_chat_button_opens_the_chat_the_server_made() {
        fn wanted_a_connection_without_opening_anything(ctx: &AppCtx) -> bool {
            toast_says(ctx, "Not connected") && ctx.chat.peek().session_id.is_none()
        }
        let candidates: Vec<_> = every_element(
            two_chats_and_a_working_dir,
            list,
            wanted_a_connection_without_opening_anything,
        )
        .collect();
        assert_eq!(
            candidates.len(),
            1,
            "either nothing on the chats list starts a new chat, or more than \
             one thing does"
        );
        let fab = candidates[0];

        let mut screen = Mounted::mount(two_chats_and_a_working_dir, list);
        let server = screen.connect(happy);
        screen.press_id(fab);
        screen.settle();

        assert_eq!(
            server.params("session/new", 0)["cwd"],
            json!("/home/demo"),
            "the new chat was started somewhere other than the configured \
             working directory: {}",
            server.params("session/new", 0)
        );
        screen.with(|ctx| {
            assert_eq!(
                ctx.chat.peek().session_id.as_deref(),
                Some("s-new"),
                "the server made a session and the app never opened it"
            );
            assert!(
                matches!(*ctx.screen.peek(), Screen::Chat),
                "a new chat was created and the reader was left on the list"
            );
        });
    }
}
