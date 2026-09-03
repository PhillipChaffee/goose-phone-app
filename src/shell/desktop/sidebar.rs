//! The plane's own list, in the sidebar.
//!
//! WHY THIS IS NOT THE LIST VIEW THAT ALREADY EXISTS, which was the plan until
//! it was measured. `views::sessions::SessionsView` and
//! `views::code::CodeSessionsView` are not lists — each emits a whole SCREEN:
//! a `header.topbar`, a `main.scroll.has-fab`, and a floating `.fab`. Dropped
//! into the sidebar, three things break rather than merely crowd. The `.fab`
//! escapes its containing block and lands in the window's bottom-right corner.
//! The `.pane`-scoped rules in `assets/desktop.css` stop matching, so a second
//! connection badge reappears and the empty-state rules inverse. And the row
//! itself does not fit: 245px of chrome is spent before the first character of
//! a title — 16 navpane + 16 navcard + 32 scroller edge + 2 border + 24 swipe
//! padding + 40 tile + 12 gap + 80 action gutter + 8 head gap + 13 age.
//!
//! Measured in Chromium against the real sheets, the title box is **0px wide
//! at the 212px sidebar** (clipped silently — `.session-title` is
//! `-webkit-line-clamp: 2; overflow: hidden`), 25px at 270, 85px at 330, and
//! only reaches today's parity at about 390. The design's sidebar is 268.
//!
//! So this is the mockup's row instead: a status mark, a title, a subtitle,
//! and an age — no tile, no action gutter, no fab, nothing that assumes a
//! pane. The full screens stay exactly as they are and remain the phone's.
//!
//! The grouping is pure and lives at the top of this file, because that is the
//! part worth testing hardest: a row landing under the wrong heading is the
//! failure a reader would notice and a render test would not.

use dioxus::prelude::*;

use dioxus::dioxus_core::spawn_forever;

use crate::icons::Icon;
use crate::nav::{self, Plane};
use crate::state::{relative_time, rfc3339_to_epoch, AppCtx};

/// The bands the chat plane's sessions fall into, newest first.
///
/// Four and not more: the mockup shows Today / Yesterday / Earlier, and
/// "Earlier" is doing real work — a list of dates is a list nobody scans. The
/// fourth is [`Band::Undated`], which exists because `SessionInfo::updated_at`
/// is an `Option` and a session the server sent no timestamp for still has to
/// go somewhere it can be seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Band {
    Today,
    Yesterday,
    Earlier,
    Undated,
}

impl Band {
    /// Display order. Iterated rather than derived, following `Group::ALL` and
    /// `Plane::ALL`: the order is stated once instead of following from where
    /// a row happens to sit.
    pub(crate) const ALL: [Self; 4] = [Self::Today, Self::Yesterday, Self::Earlier, Self::Undated];

    /// The heading, or `None` for the band that does not take one.
    ///
    /// [`Band::Undated`] is deliberately headless. "Undated" is a fact about
    /// our parsing, not about the reader's work, and a heading for it would
    /// put a word on screen that means nothing to anyone who did not write
    /// this file. The rows simply follow the dated ones.
    pub(crate) const fn header(self) -> Option<&'static str> {
        match self {
            Self::Today => Some("Today"),
            Self::Yesterday => Some("Yesterday"),
            Self::Earlier => Some("Earlier"),
            Self::Undated => None,
        }
    }
}

/// Which band an epoch falls in, given what "now" is.
///
/// `now` is a parameter rather than read from the clock, and that is the whole
/// reason this function can be tested at all: a version that called
/// `SystemTime::now` internally could only be checked by a test that also
/// called it, which is the assertion-supplies-its-own-needle shape
/// `crate::selfscan` exists to prevent.
///
/// The boundaries are CALENDAR days, not 24-hour windows. 23:30 yesterday and
/// 00:30 today are an hour apart and belong in different bands, because the
/// reader's question is "did I do this today", not "was this within 86400
/// seconds". Both are floored to a day number first.
pub(crate) const fn band_of(epoch: i64, now: i64) -> Band {
    let day = epoch.div_euclid(86_400);
    let today = now.div_euclid(86_400);
    match today - day {
        ..=0 => Band::Today,
        1 => Band::Yesterday,
        _ => Band::Earlier,
    }
}

/// A session's band, or [`Band::Undated`] when there is no usable timestamp.
///
/// Two ways to have none and both are real: the field is absent, or it is
/// present and unparseable. `rfc3339_to_epoch` answers `None` to both, and a
/// row that cannot be dated must still be reachable.
pub(crate) fn band_of_stamp(stamp: Option<&str>, now: i64) -> Band {
    stamp
        .and_then(rfc3339_to_epoch)
        .map_or(Band::Undated, |epoch| band_of(epoch, now))
}

/// What a sidebar row says, whichever plane it came from.
///
/// ONE shape for both halves, and that is not an abstraction over them — the
/// two planes stay completely separate everywhere it matters. It is that a ROW
/// is a row: a mark, a name, where it lives, and how old it is. Two structs
/// would be two sets of CSS to keep in step for no gain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Row {
    /// The id the row navigates to.
    pub id: String,
    pub title: String,
    /// Where it lives, or what was last said in it — the repo and branch on
    /// the code plane, the last message on the chat plane. `None` renders no
    /// second line at all rather than an empty one, so a row with nothing to
    /// add is short instead of padded.
    pub subtitle: Option<String>,
    /// Set where the subtitle is an IDENTIFIER rather than language: a repo, a
    /// branch. This sheet's own rule — a value the reader COMPARES or COPIES
    /// is mono — and a chat's last message is language, so it is not.
    pub subtitle_mono: bool,
    /// The age badge, already formatted.
    pub age: Option<String>,
    /// HOW LONG AN ASK HAS BEEN WAITING, where that is knowable at all.
    ///
    /// `Some("2m")` on the chat half and `None` on the code half, and the
    /// asymmetry is real rather than an omission: `ctx.lost_asks` records an
    /// `asked_at` the instant an ask arrives, and `opencode_client::
    /// CodePermission` is `{ id, session_id, title, kind, metadata }` with no
    /// time field anywhere on that wire. A blocked code row says it is blocked
    /// and does not guess at since when.
    pub blocked_for: Option<String>,
    /// What the mark says. See [`Mark`].
    pub mark: Mark,
    pub band: Band,
    /// True when the content pane is showing THIS row.
    ///
    /// The three-column layout had this and the restructure dropped it, which
    /// `assets/desktop.css`'s own note on `.session-item.on` had already
    /// argued against: "an unmarked list is a list that does not say where the
    /// pane beside it came from". With the list in the sidebar it is worse —
    /// the sidebar is on screen at ALL times now, so an unmarked list is
    /// permanently silent about what you are looking at.
    pub selected: bool,
}

/// The dot at the head of a row.
///
/// Colour is the whole signal here — there is no room for a word — so the
/// variants are the three states a reader acts on differently, and nothing
/// else. `Waiting` is the one that must never be missed: it is a question the
/// agent is blocked on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mark {
    Waiting,
    Running,
    Idle,
}

impl Mark {
    /// The class the sheet paints. A value rather than a literal in the rsx,
    /// following `Plane::label` — a rule taken as a value is a rule a test can
    /// hold.
    pub(crate) const fn class(self) -> &'static str {
        match self {
            Self::Waiting => "mark waiting",
            Self::Running => "mark running",
            Self::Idle => "mark idle",
        }
    }

    /// What a screen reader says, because the colour is the only other carrier
    /// and a dot with no name is a state nobody who cannot see it can read.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Waiting => "waiting on you",
            Self::Running => "running",
            Self::Idle => "idle",
        }
    }
}

/// The chat plane's rows, newest first inside each band.
///
/// `now` is threaded through from the caller for [`band_of`]'s reason.
pub(crate) fn chat_rows(ctx: &AppCtx, now: i64) -> Vec<Row> {
    // What the pane is showing, so a row can say it is the one. Read once
    // rather than per row: `ctx.chat` is a signal and a list of fifty rows
    // would otherwise take fifty subscriptions to the same value.
    let open_chat = (ctx.chat)().session_id;
    let running = (ctx.running_sessions)();
    let waiting: std::collections::HashSet<String> = (ctx.permission)()
        .iter()
        .map(|p| p.session_id.clone())
        .collect();

    // THE OLDEST OPEN ASK PER SESSION, which is the only "blocked since" this
    // app has and is a real one. `crate::ask_journal`'s whole design decision
    // is that a record is written the INSTANT an ask arrives — before it is
    // queued, because the case the journal exists for is the app being killed
    // — so `asked_at` is an arrival time and not a render time. `is_open`
    // filters out asks that were answered or withdrawn, so an acknowledged
    // loss from last week cannot date a live row.
    let asked_at: std::collections::HashMap<String, i64> = (ctx.lost_asks)()
        .iter()
        .filter(|record| record.is_open())
        .fold(std::collections::HashMap::new(), |mut acc, record| {
            let at = acc
                .entry(record.session_id.clone())
                .or_insert(record.asked_at);
            *at = (*at).min(record.asked_at);
            acc
        });

    let mut rows: Vec<Row> = (ctx.sessions)()
        .iter()
        .map(|info| {
            let epoch = info.updated_at.as_deref().and_then(rfc3339_to_epoch);
            Row {
                id: info.session_id.clone(),
                title: info
                    .title
                    .clone()
                    .filter(|t| !t.trim().is_empty())
                    .unwrap_or_else(|| "Untitled chat".to_owned()),
                // THE LAST THING SAID IN IT. Every row read "2 messages" —
                // six characters carrying nothing — while the snippet was on
                // the same struct the row is built from, on the wire, served
                // by the mock, and already rendered by the phone's own list.
                //
                // The count is the FALLBACK rather than the answer: a session
                // the server sent no snippet for still needs a second line,
                // and the count is what that row said before.
                subtitle: info
                    .last_message_snippet()
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        info.message_count().map(|n| {
                            if n == 1 {
                                "1 message".to_owned()
                            } else {
                                format!("{n} messages")
                            }
                        })
                    }),
                subtitle_mono: false,
                blocked_for: waiting
                    .contains(&info.session_id)
                    .then(|| asked_at.get(&info.session_id).copied().map(relative_time))
                    .flatten(),
                age: epoch.map(relative_time),
                mark: if waiting.contains(&info.session_id) {
                    Mark::Waiting
                } else if running.contains(&info.session_id) {
                    Mark::Running
                } else {
                    Mark::Idle
                },
                band: band_of_stamp(info.updated_at.as_deref(), now),
                selected: open_chat.as_deref() == Some(info.session_id.as_str()),
            }
        })
        .collect();
    // Newest first, and undated last within its own band — a stable sort so
    // the server's own order survives between equal timestamps rather than
    // being shuffled by the sort itself.
    rows.sort_by_key(|row| {
        std::cmp::Reverse(
            (ctx.sessions)()
                .iter()
                .find(|s| s.session_id == row.id)
                .and_then(|s| s.updated_at.as_deref())
                .and_then(rfc3339_to_epoch)
                .unwrap_or(i64::MIN),
        )
    });
    rows
}

/// The code plane's rows: one per working tree, newest first.
///
/// Banded by `last_active` like the chat plane, rather than grouped by repo as
/// the mockup's wide home screen does. The sidebar is 268px and a repo heading
/// per tree would spend more of it on headings than on trees; the repo goes on
/// the row's own second line instead, where it is still on screen.
pub(crate) fn code_rows(ctx: &AppCtx, now: i64) -> Vec<Row> {
    let open_chat = (ctx.code_chat)().chat_id;
    let waiting: std::collections::HashSet<String> = (ctx.code_permissions)()
        .iter()
        .map(|(chat, _)| chat.clone())
        .collect();

    let mut rows: Vec<Row> = (ctx.code_chats)()
        .iter()
        .map(|chat| Row {
            id: chat.id.clone(),
            title: if chat.title.trim().is_empty() {
                chat.id.clone()
            } else {
                chat.title.clone()
            },
            // Repo AND branch, which is what a working tree IS. Both are
            // identifiers, so the row sets them in mono — see `subtitle_mono`.
            subtitle: {
                let repo = chat.repo.trim();
                let branch = chat.branch.trim();
                match (repo.is_empty(), branch.is_empty()) {
                    (false, false) => Some(format!("{repo} \u{b7} {branch}")),
                    (false, true) => Some(repo.to_owned()),
                    (true, false) => Some(branch.to_owned()),
                    (true, true) => None,
                }
            },
            subtitle_mono: true,
            // NO TIMESTAMP EXISTS on this wire. The row still says it is
            // blocked — that is the one state a reader must not miss — and
            // says nothing about how long, rather than reaching into
            // `CodePermission::metadata` for a field this app does not model.
            blocked_for: None,
            age: (chat.last_active > 0.0).then(|| {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "an epoch in seconds is many orders of magnitude inside i64, and a \
                              row badge has no use for the fraction"
                )]
                relative_time(chat.last_active as i64)
            }),
            mark: if waiting.contains(&chat.id) {
                Mark::Waiting
            } else if chat.is_running() {
                Mark::Running
            } else {
                Mark::Idle
            },
            band: {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "see the age badge above; the same epoch, the same reasoning"
                )]
                if chat.last_active > 0.0 {
                    band_of(chat.last_active as i64, now)
                } else {
                    Band::Undated
                }
            },
            selected: open_chat.as_deref() == Some(chat.id.as_str()),
        })
        .collect();
    rows.sort_by(|a, b| {
        let key = |row: &Row| {
            (ctx.code_chats)()
                .iter()
                .find(|c| c.id == row.id)
                .map_or(f64::MIN, |c| c.last_active)
        };
        key(b).total_cmp(&key(a))
    });
    rows
}

/// The plane's rows, whichever plane it is.
pub(crate) fn rows_for(ctx: &AppCtx, plane: Plane, now: i64) -> Vec<Row> {
    match plane {
        Plane::Chat => chat_rows(ctx, now),
        Plane::Code => code_rows(ctx, now),
    }
}

/// What the list says when it has nothing to show.
///
/// Per plane, because the two halves have different vocabulary and a shared
/// "Nothing here" would be the one place they leaked into each other.
pub(crate) const fn empty_line(plane: Plane) -> &'static str {
    match plane {
        Plane::Chat => "No chats yet",
        Plane::Code => "No working trees yet",
    }
}

/// Now, in seconds. Split out so the component has one clock and the pure
/// functions above have none.
fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs().cast_signed())
}

/// The plane's list, banded, as it appears in the sidebar.
///
/// A component of its own rather than more rsx inside `AppShell`, and the
/// reason is testability rather than tidiness: `AppShell` calls
/// `dioxus::desktop::window()`, which is `consume_context()` and panics
/// without a real event loop, so nothing that lives inside it can be mounted.
/// This reads `AppCtx` and nothing else, so `crate::testkit` can mount it and
/// assert on what it paints — which is what keeps this file inside the 95%
/// bar the workspace is now held to.
#[component]
pub(crate) fn SidebarList(plane: Plane) -> Element {
    let ctx = crate::state::use_app_ctx();
    let rows = rows_for(&ctx, plane, now_secs());

    // The two sheets a row can raise. Held here rather than on `AppCtx`,
    // following the rule `views/chat.rs` and the rest already follow: a sheet
    // belongs to the screen that raises it, and one on the context outlives
    // the unmount — which is the defect the extensions credential sheet
    // shipped with (an index into drafts that no longer existed).
    let mut renaming: Signal<Option<(String, String)>> = use_signal(|| None);
    let mut deleting: Signal<Option<(String, String)>> = use_signal(|| None);

    rsx! {
        // SEARCH, and it is here rather than in the pane because it filters
        // THIS list. It was in `views::sessions::SessionsView`, which the
        // desktop no longer mounts once the pane becomes a home screen; losing
        // it would be the same class of regression as losing rename.
        //
        // Chat only, like the actions: `ctx.sessions_query` is the chat
        // plane's filter and the code plane has no equivalent to drive.
        if plane == Plane::Chat {
            div { class: "nav-search",
                crate::views::chrome::SearchField {
                    placeholder: "Search messages",
                    value: (ctx.sessions_query)(),
                    on_search: move |text: String| {
                        spawn_forever(async move {
                            crate::state::search_sessions(&ctx, text).await;
                        });
                    },
                }
            }
        }

        div { class: "nav-sessions",
            if rows.is_empty() {
                p { class: "nav-sessions-empty", "{empty_line(plane)}" }
            }
            for band in Band::ALL {
                // The band renders only if it holds something. A heading over
                // a gap is the app promising a section it does not have —
                // `shell::render_group` states the same rule for the drawer.
                if rows.iter().any(|row| row.band == band) {
                    if let Some(header) = band.header() {
                        div { class: "nav-band",
                            span { class: "nav-band-name", "{header}" }
                            span { class: "nav-band-count",
                                "{rows.iter().filter(|row| row.band == band).count()}"
                            }
                        }
                    }
                    for row in rows.iter().filter(|row| row.band == band) {
                        // A DIV WRAPPING A BUTTON, not a button holding
                        // buttons.
                        //
                        // This shipped as `button.nav-row` with the actions
                        // nested inside it, and that is invalid HTML: a
                        // `<button>` takes phrasing content, so an HTML parser
                        // meeting a `<div>` — let alone two more `<button>`s —
                        // closes the button and HOISTS them out. Dioxus builds
                        // the real DOM through API calls and never parses, so
                        // the running app looked correct; anything that
                        // round-trips the markup does not. `docs/audit.js`
                        // does, and reported 1600 CHROME-SLOT findings because
                        // the re-parsed tree put `.pane-main` directly under
                        // `<body>`, on top of the traffic lights.
                        //
                        // `views::chrome::ListRow` had this right already: its
                        // row is a div and `.session-actions` is a sibling.
                        div {
                            key: "{row.id}",
                            class: if row.selected { "nav-row on" } else { "nav-row" },
                            button {
                            class: "nav-row-open",
                            "aria-current": if row.selected { "true" } else { "false" },
                            title: "{row.title}",
                            // OPENING IS THE POINT, and this is dispatched by
                            // plane rather than held on the row.
                            //
                            // A `Row` carries an id and no behaviour on
                            // purpose: the two halves open different things
                            // through different clients, and giving the struct
                            // a callback would be the one place their
                            // vocabularies met. The id is looked back up in
                            // the plane's own list, so a row can only ever
                            // open something that is still there — a stale row
                            // clicked after a refresh does nothing rather than
                            // opening a session the server has forgotten.
                            onclick: {
                                let id = row.id.clone();
                                move |_| {
                                    // ENTER THE PLANE FIRST, and this line is
                                    // the whole of a bug that unit tests could
                                    // not see.
                                    //
                                    // `open_session` sets `ctx.screen`; it does
                                    // not set `ctx.tab`. `nav::current` reads
                                    // the TAB first, so with Skills or Recipes
                                    // open the sidebar's row set the screen,
                                    // the library destination went on winning,
                                    // and clicking a chat did visibly nothing.
                                    // On a phone that could not happen — the
                                    // list is only on screen when you are
                                    // already in its tab — and the sidebar is
                                    // the first place in this app where a
                                    // session row is reachable from anywhere.
                                    //
                                    // Found by driving the real app, not by a
                                    // test: the harness mounts with the default
                                    // context, which is already on the chat
                                    // plane, so the assertion held while the
                                    // app did not.
                                    (nav::primary(plane).go)(&ctx);
                                    match plane {
                                        Plane::Chat => {
                                            if let Some(info) = (ctx.sessions)()
                                                .iter()
                                                .find(|s| s.session_id == id)
                                            {
                                                crate::state::open_session(&ctx, info.clone());
                                            }
                                        }
                                        Plane::Code => {
                                            if let Some(meta) =
                                                (ctx.code_chats)().iter().find(|c| c.id == id)
                                            {
                                                crate::code::open_code_chat(&ctx, meta.clone());
                                            }
                                        }
                                    }
                                }
                            },
                            span {
                                class: "{row.mark.class()}",
                                "aria-label": "{row.mark.label()}",
                            }
                            span { class: "nav-row-text",
                                span { class: "nav-row-title", "{row.title}" }
                                // THE META LINE. The age used to be a third
                                // flex sibling aligned to the TITLE, which put
                                // a timestamp on the row's most important line
                                // and left the second one running the full
                                // width. The mockups put both on the second
                                // line, which is also what frees the top-right
                                // for nothing at all.
                                span { class: "nav-row-meta",
                                    if let Some(subtitle) = row.subtitle.clone() {
                                        span {
                                            class: if row.subtitle_mono {
                                                "nav-row-sub mono"
                                            } else {
                                                "nav-row-sub"
                                            },
                                            "{subtitle}"
                                        }
                                    }
                                    if let Some(age) = row.age.clone() {
                                        span { class: "nav-row-age", "{age}" }
                                    }
                                }
                                // A THIRD LINE, and only on a row that is
                                // blocked. It is the one state that is about
                                // the READER rather than about the agent, and
                                // a coloured dot alone cannot say "for two
                                // minutes".
                                if row.mark == Mark::Waiting {
                                    span { class: "nav-row-needs",
                                        if let Some(since) = row.blocked_for.clone() {
                                            "Needs input \u{b7} {since}"
                                        } else {
                                            "Needs input"
                                        }
                                    }
                                }
                            }

                            }

                            // RENAME AND DELETE, in the age's place under the
                            // pointer. `assets/desktop.css` swaps them; this
                            // renders both and lets the sheet decide, so there
                            // is no hover state in Rust and no `onmouseover`
                            // paying a synchronous XHR per frame.
                            //
                            // Chat only for now. The code plane's equivalents
                            // are a container stop and a tree delete, which is
                            // a different and more destructive pair than
                            // "rename this thread" — it wants its own pass
                            // rather than a shared one that assumes they match.
                            if plane == Plane::Chat {
                                div { class: "nav-row-actions",
                                    button {
                                        class: "nav-row-act",
                                        title: "Rename",
                                        "aria-label": "Rename {row.title}",
                                        onclick: {
                                            let id = row.id.clone();
                                            let title = row.title.clone();
                                            move |e: Event<MouseData>| {
                                                // Or the press opens the row it
                                                // is sitting on as well.
                                                e.stop_propagation();
                                                renaming.set(Some((id.clone(), title.clone())));
                                            }
                                        },
                                        Icon { name: "pencil" }
                                    }
                                    button {
                                        class: "nav-row-act danger",
                                        title: "Delete",
                                        "aria-label": "Delete {row.title}",
                                        onclick: {
                                            let id = row.id.clone();
                                            let title = row.title.clone();
                                            move |e: Event<MouseData>| {
                                                e.stop_propagation();
                                                deleting.set(Some((id.clone(), title.clone())));
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
        }

        // THE SHEETS THE ROWS RAISE.
        //
        // Deliberately the SAME components `views::sessions` uses, with the
        // same words and the same wire calls — `RenameSheet` and
        // `ConfirmDelete` out of `crate::views`. Two dialogs asking the same
        // question in two wordings is how a reader learns to distrust both,
        // and `Confirm`'s markup is also what `DISMISS_KEY` keys Escape off
        // (`p.modal-body` is the discriminator), so a hand-rolled sheet here
        // would be one Escape does not close.
        if let Some((session_id, title)) = renaming() {
            crate::views::RenameSheet {
                key: "{session_id}",
                heading: "Rename chat",
                value: title,
                on_cancel: move |()| renaming.set(None),
                on_save: move |title: String| {
                    let session_id = session_id.clone();
                    renaming.set(None);
                    spawn_forever(async move {
                        crate::state::rename_session(&ctx, &session_id, &title).await;
                    });
                },
            }
        }

        if let Some((session_id, _)) = deleting() {
            crate::views::ConfirmDelete {
                title: "Delete this chat?",
                body: "The whole conversation goes from the goose server. \
                       This cannot be undone.",
                on_cancel: move |()| deleting.set(None),
                on_confirm: move |()| {
                    let session_id = session_id.clone();
                    deleting.set(None);
                    spawn_forever(async move {
                        let Some(client) = ctx.client.peek().clone() else {
                            return;
                        };
                        match client.session_delete(&session_id).await {
                            Ok(()) => {
                                let mut sessions = ctx.sessions;
                                sessions.write().retain(|s| s.session_id != session_id);
                            }
                            Err(e) => {
                                crate::state::show_toast(&ctx, format!("Delete failed: {e}"));
                            }
                        }
                    });
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        band_of, band_of_stamp, chat_rows, code_rows, empty_line, Band, Mark, SidebarList,
    };
    use crate::nav::Plane;
    use crate::views::press::Pressable;
    use dioxus::prelude::*;

    /// 2026-08-31T12:00:00Z, so every stamp below is a readable date rather
    /// than a number nobody can check by eye.
    const NOW: i64 = 1_788_177_600;
    const DAY: i64 = 86_400;

    fn session(id: &str, title: &str, updated: Option<&str>) -> goose_acp_client::SessionInfo {
        goose_acp_client::SessionInfo {
            session_id: id.to_owned(),
            cwd: None,
            title: Some(title.to_owned()),
            updated_at: updated.map(ToOwned::to_owned),
            meta: None,
        }
    }

    /// The bands are CALENDAR days, not 24-hour windows, and that is the whole
    /// reason `band_of` takes `now` instead of reading the clock.
    ///
    /// 23:30 yesterday and 00:30 today are an hour apart and belong in
    /// different bands, because the reader's question is "did I do this today"
    /// and not "was this within 86400 seconds". A duration-based version passes
    /// a naive test and puts last night's work under Today every morning.
    #[test]
    fn the_bands_are_calendar_days_and_not_rolling_windows() {
        assert_eq!(band_of(NOW, NOW), Band::Today);
        assert_eq!(band_of(NOW - DAY, NOW), Band::Yesterday);
        assert_eq!(band_of(NOW - 6 * DAY, NOW), Band::Earlier);

        // The pair that separates the two definitions: 90 minutes apart,
        // straddling midnight.
        let just_after_midnight = NOW - 12 * 3_600 + 1_800;
        let just_before_midnight = NOW - 12 * 3_600 - 1_800;
        assert_eq!(band_of(just_after_midnight, NOW), Band::Today);
        assert_eq!(
            band_of(just_before_midnight, NOW),
            Band::Yesterday,
            "an hour and a half earlier fell on the other side of midnight, so \
             it is yesterday's work however recent it is"
        );
    }

    /// A row the server sent no usable timestamp for still has to be reachable.
    /// Both ways of having none are real — the field absent, and the field
    /// present but unparseable — and a `None` that silently became Today would
    /// put undated rows above everything the reader did this morning.
    #[test]
    fn a_row_with_no_usable_stamp_is_still_reachable() {
        assert_eq!(band_of_stamp(None, NOW), Band::Undated);
        assert_eq!(band_of_stamp(Some("not a date"), NOW), Band::Undated);
        assert_eq!(band_of_stamp(Some(""), NOW), Band::Undated);
        assert_eq!(
            band_of_stamp(Some("2026-08-31T09:00:00Z"), NOW),
            Band::Today
        );
    }

    /// `Band::Undated` renders no heading, and that is deliberate rather than
    /// an oversight: "Undated" is a fact about our parsing, not about the
    /// reader's work. Every other band names itself, or its rows arrive under
    /// nothing.
    #[test]
    fn every_band_but_the_undated_one_names_itself() {
        for band in Band::ALL {
            assert_eq!(
                band.header().is_none(),
                band == Band::Undated,
                "{band:?} disagrees with the rule that only the undated band is \
                 headless"
            );
        }
    }

    /// The mark is the only thing on a row that says what state it is in —
    /// there is no room for a word — so each state needs its own class AND its
    /// own accessible name. Two states sharing either is a state nobody can
    /// tell apart, by eye or by screen reader.
    #[test]
    fn every_mark_is_distinguishable_two_ways() {
        let marks = [Mark::Waiting, Mark::Running, Mark::Idle];
        let mut classes: Vec<&str> = marks.iter().map(|m| m.class()).collect();
        let mut labels: Vec<&str> = marks.iter().map(|m| m.label()).collect();
        classes.sort_unstable();
        classes.dedup();
        labels.sort_unstable();
        labels.dedup();
        assert_eq!(classes.len(), marks.len(), "two marks share a class");
        assert_eq!(labels.len(), marks.len(), "two marks share a name");
        for mark in marks {
            assert!(
                mark.class().starts_with("mark "),
                "{mark:?} does not carry the base class the sheet sizes it with"
            );
        }
    }

    /// Waiting outranks running, and that ordering is the point of the mark.
    ///
    /// A session can be both — the agent is mid-turn AND blocked on a
    /// permission ask — and the one the reader must act on is the ask. Showing
    /// it as merely running is how a blocked agent sits unnoticed, which is the
    /// failure the whole cross-plane count exists to prevent one level up.
    fn seed_running_and_asking(ctx: &crate::state::AppCtx) {
        let mut sessions = ctx.sessions;
        sessions.set(vec![session("s1", "Blocked", Some("2026-08-31T09:00:00Z"))]);
        let mut running = ctx.running_sessions;
        running.set(std::iter::once("s1".to_owned()).collect());
        let mut permission = ctx.permission;
        permission.set(vec![goose_acp_client::PermissionRequest {
            request_id: serde_json::Value::from(7),
            session_id: "s1".to_owned(),
            tool_call: goose_acp_client::ToolCallUpdate {
                tool_call_id: "call-1".to_owned(),
                ..goose_acp_client::ToolCallUpdate::default()
            },
            options: Vec::new(),
        }]);
    }

    #[test]
    fn a_session_that_is_running_and_asking_reads_as_asking() {
        let rows = crate::testkit::with_ctx(seed_running_and_asking, |ctx| chat_rows(ctx, NOW));
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].mark,
            Mark::Waiting,
            "a session that is running and also has an ask outstanding read as \
             running, so the question it is blocked on is invisible"
        );
    }

    /// Newest first. A list ordered by whatever the server happened to send is
    /// a list the reader has to search rather than scan.
    #[test]
    fn the_newest_row_is_the_first_one() {
        let rows = crate::testkit::with_ctx(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![
                    session("old", "Older", Some("2026-08-25T09:00:00Z")),
                    session("new", "Newer", Some("2026-08-31T09:00:00Z")),
                    session("mid", "Middle", Some("2026-08-30T09:00:00Z")),
                ]);
            },
            |ctx| chat_rows(ctx, NOW),
        );
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["new", "mid", "old"]);
        assert_eq!(rows[0].band, Band::Today);
        assert_eq!(rows[1].band, Band::Yesterday);
        assert_eq!(rows[2].band, Band::Earlier);
    }

    /// An untitled session still needs a name on screen. A row rendering an
    /// empty string is a row that looks like a rendering bug.
    #[test]
    fn a_session_with_no_title_still_has_a_name() {
        let rows = crate::testkit::with_ctx(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![
                    goose_acp_client::SessionInfo {
                        session_id: "a".to_owned(),
                        cwd: None,
                        title: None,
                        updated_at: None,
                        meta: None,
                    },
                    session("b", "   ", None),
                ]);
            },
            |ctx| chat_rows(ctx, NOW),
        );
        for row in &rows {
            assert!(
                !row.title.trim().is_empty(),
                "a row rendered a blank title, which reads as a broken row"
            );
        }
    }

    /// The code plane's rows carry the repo AND the branch, because neither
    /// alone says which tree it is: three repos can hold the same branch name,
    /// and one repo holds a tree per branch. Both are identifiers, so the row
    /// marks them for the mono face.
    #[test]
    fn a_code_row_says_which_tree_it_is() {
        let rows = crate::testkit::with_ctx(
            |ctx| {
                let mut chats = ctx.code_chats;
                chats.set(vec![opencode_client::ChatMeta {
                    id: "c1".to_owned(),
                    repo: "goose-phone-app".to_owned(),
                    title: "inbox-triage".to_owned(),
                    branch: "agent/x".to_owned(),
                    base: String::new(),
                    status: "running".to_owned(),
                    model: None,
                    last_active: 1_788_177_600.0,
                }]);
            },
            |ctx| code_rows(ctx, NOW),
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "inbox-triage");
        assert_eq!(
            rows[0].subtitle.as_deref(),
            Some("goose-phone-app \u{b7} agent/x"),
            "a working tree is a repo and a branch; naming one of them makes \
             two trees of the same repo indistinguishable in the list"
        );
        assert!(
            rows[0].subtitle_mono,
            "a repo and a branch are things the reader compares character by \
             character, and this sheet sets those in mono"
        );
        assert_eq!(rows[0].mark, Mark::Running);
    }

    /// AND IT DEGRADES A FIELD AT A TIME rather than rendering a stray
    /// separator. A tree with no branch is reachable — `ChatMeta.branch` is a
    /// plain `String` and the manager sends `""` for a session that has not
    /// cut one yet — and "goose-phone-app · " reads as a truncation bug.
    #[test]
    fn a_tree_missing_half_its_name_still_reads_as_a_name() {
        let rows = crate::testkit::with_ctx(
            |ctx| {
                let mut chats = ctx.code_chats;
                chats.set(vec![
                    tree("c1", "goose-phone-app", ""),
                    tree("c2", "", "agent/y"),
                    tree("c3", "", ""),
                ]);
            },
            |ctx| code_rows(ctx, NOW),
        );
        let subs: Vec<Option<&str>> = rows.iter().map(|r| r.subtitle.as_deref()).collect();
        assert!(
            subs.contains(&Some("goose-phone-app")) && subs.contains(&Some("agent/y")),
            "a missing half should drop out, not leave its separator: {subs:?}"
        );
        assert!(
            subs.contains(&None),
            "a tree with neither should render no second line at all: {subs:?}"
        );
        assert!(
            !subs
                .iter()
                .flatten()
                .any(|s| s.contains(" \u{b7} ") && s.split(" \u{b7} ").any(str::is_empty)),
            "a separator was painted with nothing on one side of it: {subs:?}"
        );
    }

    fn tree(id: &str, repo: &str, branch: &str) -> opencode_client::ChatMeta {
        opencode_client::ChatMeta {
            id: id.to_owned(),
            repo: repo.to_owned(),
            title: id.to_owned(),
            branch: branch.to_owned(),
            base: String::new(),
            status: "stopped".to_owned(),
            model: None,
            last_active: 1_788_177_600.0,
        }
    }

    /// The two halves keep their own vocabulary down to the empty state. A
    /// shared "Nothing here" would be the one place they leaked into each
    /// other, and it would also be less use: what is missing is different.
    #[test]
    fn each_half_says_its_own_name_for_nothing() {
        assert_ne!(empty_line(Plane::Chat), empty_line(Plane::Code));
        assert!(empty_line(Plane::Code).contains("tree"));
    }

    #[component]
    fn ChatSidebar() -> Element {
        rsx! { SidebarList { plane: Plane::Chat } }
    }

    /// The list renders its bands, its rows and nothing it does not have.
    ///
    /// A band with no rows must not paint a heading — a heading over a gap is
    /// the app promising a section it does not have, which is the rule
    /// `shell::render_group` already states for the drawer.
    #[test]
    fn the_list_paints_only_the_bands_it_has_rows_for() {
        let html = crate::testkit::render_seeded(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![goose_acp_client::SessionInfo {
                    session_id: "s1".to_owned(),
                    cwd: None,
                    title: Some("Only an old one".to_owned()),
                    updated_at: Some("2020-01-02T09:00:00Z".to_owned()),
                    meta: None,
                }]);
            },
            || rsx! { SidebarList { plane: Plane::Chat } },
        );
        assert!(html.contains("Only an old one"), "the row never rendered");
        assert!(html.contains("Earlier"), "its band did not name itself");
        assert!(
            !html.contains("Yesterday"),
            "a band with no rows in it painted a heading anyway"
        );
        assert!(
            !html.contains("No chats yet"),
            "the list rendered rows AND its empty state at once"
        );
    }

    /// With nothing to show it says so, per plane.
    #[test]
    fn an_empty_list_says_which_half_is_empty() {
        let html = crate::testkit::render(|| rsx! { SidebarList { plane: Plane::Code } });
        assert!(html.contains("No working trees yet"));
        assert!(
            !html.contains("No chats yet"),
            "the code half borrowed the chat half's words"
        );
    }

    /// PRESSING A ROW OPENS IT, and this test exists because it did not.
    ///
    /// The list shipped once with the markup complete and no `onclick` at all:
    /// bands, marks, titles, ages, every class the sheet styles — and clicking
    /// a row did nothing. Nothing caught it, because every other test here
    /// asks what the list PAINTS, and it painted correctly. It was found by
    /// driving the real app and noticing the screen did not change.
    ///
    /// So this asks the other question. A list you cannot open is not a list,
    /// and "it renders" is not "it works".
    #[test]
    fn pressing_a_row_opens_that_session() {
        let _guard = crate::views::press::alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![
                    session("s1", "First thread", Some("2026-08-31T09:00:00Z")),
                    session("s2", "Second thread", Some("2026-08-30T09:00:00Z")),
                ]);
            },
            ChatSidebar,
        );

        assert!(
            screen.with(|ctx| (ctx.screen)() == crate::state::Screen::Settings),
            "the harness did not start where the app starts"
        );

        screen.press("Second thread");
        screen.settle();

        assert!(
            screen.with(|ctx| (ctx.screen)() == crate::state::Screen::Chat),
            "pressing a row did not open a chat — the row renders but does \
             nothing, which is the defect this test was written for"
        );
        assert_eq!(
            screen.with(|ctx| (ctx.chat)().session_id),
            Some("s2".to_owned()),
            "pressing the second row opened a different session than the one \
             under the pointer"
        );
    }

    /// The row the pane is showing says so, on both planes.
    ///
    /// The three-column layout marked its list and the restructure dropped it.
    /// It matters more in the sidebar than it did in a pane, because the
    /// sidebar is on screen at every width and in every state — an unmarked
    /// list is permanently silent about what you are looking at.
    #[test]
    fn the_open_row_is_the_marked_one() {
        let rows = crate::testkit::with_ctx(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![
                    session("s1", "First", Some("2026-08-31T09:00:00Z")),
                    session("s2", "Second", Some("2026-08-30T09:00:00Z")),
                ]);
                let mut chat = ctx.chat;
                chat.write().session_id = Some("s2".to_owned());
            },
            |ctx| chat_rows(ctx, NOW),
        );
        let marked: Vec<&str> = rows
            .iter()
            .filter(|r| r.selected)
            .map(|r| r.id.as_str())
            .collect();
        assert_eq!(
            marked,
            ["s2"],
            "exactly the open session should be marked; the sidebar is on \
             screen always and an unmarked list never says what is open"
        );
    }

    /// And it reaches the markup, not just the struct.
    #[test]
    fn the_marked_row_carries_the_class_the_sheet_paints() {
        let html = crate::testkit::render_seeded(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![session(
                    "s1",
                    "The open one",
                    Some("2026-08-31T09:00:00Z"),
                )]);
                let mut chat = ctx.chat;
                chat.write().session_id = Some("s1".to_owned());
            },
            || rsx! { SidebarList { plane: Plane::Chat } },
        );
        assert!(
            html.contains(r#"class="nav-row on""#),
            "the open row does not carry `nav-row on`, so assets/desktop.css \
             has nothing to paint the selection with: {}",
            &html[..html.len().min(500)]
        );
        assert!(
            html.contains(r#"aria-current="true""#),
            "the selection is colour only — a reader who cannot see the fill \
             is told nothing about which row is open"
        );
    }

    /// Rename and delete are on the row, and search is above the list.
    ///
    /// Both used to live in `views::sessions::SessionsView`, which the desktop
    /// stops mounting once the pane becomes a home screen. Losing them would
    /// be the same class of regression the "Nothing open" placeholder already
    /// caused once — a restructure may move a control, not drop it.
    #[test]
    fn the_chat_rows_carry_the_controls_the_pane_used_to() {
        let html = crate::testkit::render_seeded(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![session(
                    "s1",
                    "Renamable",
                    Some("2026-08-31T09:00:00Z"),
                )]);
            },
            || rsx! { SidebarList { plane: Plane::Chat } },
        );
        assert!(
            html.contains("nav-row-actions"),
            "the row has no actions, so there is nowhere on the desktop left \
             to rename or delete a chat"
        );
        assert!(
            html.contains("Rename Renamable"),
            "rename has no accessible name"
        );
        assert!(
            html.contains("Delete Renamable"),
            "delete has no accessible name"
        );
        assert!(
            html.contains("Search messages"),
            "the list has no search field, and the pane that used to carry one \
             is about to become a home screen"
        );
    }

    /// The code plane gets neither, and that is a decision rather than an
    /// oversight.
    ///
    /// Its equivalents are a container stop and a working-tree delete — a
    /// different and more destructive pair than "rename this thread" — and
    /// `ctx.sessions_query` is the chat plane's filter with no code-side
    /// counterpart to drive. Sharing the markup would mean shipping buttons
    /// that either do nothing or do something the label does not say.
    #[test]
    fn the_code_plane_does_not_borrow_the_chat_planes_controls() {
        let html = crate::testkit::render_seeded(
            |ctx| {
                let mut chats = ctx.code_chats;
                chats.set(vec![opencode_client::ChatMeta {
                    id: "c1".to_owned(),
                    repo: "repo".to_owned(),
                    title: "a tree".to_owned(),
                    branch: "agent/x".to_owned(),
                    base: String::new(),
                    status: "stopped".to_owned(),
                    model: None,
                    last_active: 1_788_177_600.0,
                }]);
            },
            || rsx! { SidebarList { plane: Plane::Code } },
        );
        assert!(html.contains("a tree"), "the code row never rendered");
        assert!(
            !html.contains("nav-row-actions"),
            "the code plane borrowed the chat plane's row actions, which are \
             worded for a thread and would be wired to a working tree"
        );
        assert!(
            !html.contains("Search messages"),
            "the code plane rendered the chat plane's search field, which \
             drives `ctx.sessions_query` and would filter nothing here"
        );
    }

    /// NO INTERACTIVE ELEMENT INSIDE ANOTHER ONE, which is not a style rule.
    ///
    /// A `<button>` takes phrasing content. Put a `<div>` or a second
    /// `<button>` in one and an HTML parser closes the first and HOISTS the
    /// rest out — the tree it yields is not the tree that was written. Dioxus
    /// builds the real DOM through API calls and never parses, so the running
    /// app looked perfect while every consumer of the captured markup saw
    /// something else.
    ///
    /// This row shipped exactly that: `button.nav-row` wrapping
    /// `div.nav-row-actions` and two more buttons. `docs/audit.js` re-parses,
    /// and reported **1600 CHROME-SLOT findings** — `.pane-main` re-parented
    /// to sit directly under `<body>`, on top of the traffic lights. A pile of
    /// findings about the wrong element is what an invalid tree looks like
    /// from the outside, and it cost far more to read than this test costs to
    /// run.
    ///
    /// Shown to fail: nest the actions back inside the open button.
    #[test]
    fn no_control_is_nested_inside_another_control() {
        let html = crate::testkit::render_seeded(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![session("s1", "A row", Some("2026-08-31T09:00:00Z"))]);
            },
            || rsx! { SidebarList { plane: Plane::Chat } },
        );
        // Walk the tags and keep a depth count of open buttons. Crude on
        // purpose: this markup is machine-written, never hand-edited, and has
        // no `<button/>` self-closing form to confuse the count.
        let mut depth = 0_i32;
        let mut rest = html.as_str();
        while let Some(at) = rest.find('<') {
            rest = &rest[at..];
            if rest.starts_with("</button>") {
                depth -= 1;
                rest = &rest[9..];
            } else if rest.starts_with("<button") {
                assert!(
                    depth == 0,
                    "a <button> is nested inside another <button>; an HTML \
                     parser will hoist it out and the tree that reaches the \
                     gallery, the audit and any snapshot is not the one this \
                     renders"
                );
                depth += 1;
                rest = &rest[7..];
            } else {
                rest = &rest[1..];
            }
        }
        assert_eq!(depth, 0, "the markup opens a <button> it never closes");
    }

    /// Pressing an action must not ALSO open the row it sits on.
    ///
    /// The buttons are children of the row's own button, so without
    /// `stop_propagation` a press bubbles: you would ask to rename a chat and
    /// be moved into it, with the sheet opening over a screen you did not
    /// choose. Cheap to get wrong, invisible in a render test.
    #[test]
    fn pressing_an_action_does_not_open_the_row_under_it() {
        let _guard = crate::views::press::alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![session(
                    "s1",
                    "Leave me shut",
                    Some("2026-08-31T09:00:00Z"),
                )]);
            },
            ChatSidebar,
        );

        screen.press("Rename Leave me shut");
        screen.settle();

        assert!(
            screen.with(|ctx| (ctx.chat)().session_id.is_none()),
            "pressing Rename opened the chat as well — the press bubbled from \
             the action to the row it sits on"
        );
        assert!(
            screen.markup().contains("Rename chat"),
            "pressing Rename did not raise the rename sheet"
        );
    }

    /// AND IT OPENS FROM ANYWHERE, which is the half the first version of this
    /// test could not see.
    ///
    /// The sidebar is the first place in this app where a session row is
    /// reachable while another destination is on screen — on a phone the list
    /// is only visible when you are already in its tab. `open_session` sets
    /// `ctx.screen` and not `ctx.tab`, and `nav::current` reads the tab first,
    /// so from Skills or Recipes the row set the screen, the library
    /// destination went on winning, and clicking a chat did visibly nothing.
    ///
    /// The first test missed it because `Pressable` mounts with the default
    /// context, which is already on the chat plane — the one starting point
    /// where the bug cannot show. It was found by driving the real app. This
    /// starts somewhere else on purpose.
    ///
    /// Shown to fail: drop the `(nav::primary(plane).go)(&ctx)` line from the
    /// row's handler and this goes red while the test above stays green.
    #[test]
    fn a_row_opens_even_when_another_destination_is_showing() {
        let _guard = crate::views::press::alone();
        let mut screen = Pressable::mount(
            |ctx| {
                let mut sessions = ctx.sessions;
                sessions.set(vec![session(
                    "s1",
                    "Reachable from Skills",
                    Some("2026-08-31T09:00:00Z"),
                )]);
                // Somewhere that is NOT the chat plane's primary.
                let mut tab = ctx.tab;
                tab.set(crate::state::Tab::Skills);
            },
            ChatSidebar,
        );

        assert_eq!(
            screen.with(|ctx| crate::nav::current(ctx).id),
            "skills",
            "the harness did not start on another destination, so this test \
             cannot see the defect it exists for"
        );

        screen.press("Reachable from Skills");
        screen.settle();

        assert_eq!(
            screen.with(|ctx| crate::nav::current(ctx).id),
            "chats",
            "pressing a session row from Skills left the window on Skills — \
             the row set the screen but not the tab, and `nav::current` reads \
             the tab first, so the chat never came up"
        );
        assert!(
            screen.with(|ctx| (ctx.screen)() == crate::state::Screen::Chat),
            "the plane was entered but the session was not opened"
        );
    }
}
