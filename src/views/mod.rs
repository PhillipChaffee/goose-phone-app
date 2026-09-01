pub(crate) mod attach;

pub(crate) mod chat;

/// Mounting a view and pressing something in it. Test-only, and shared —
/// `viewport.rs` and `shell/mod.rs` reach for it too, because the handlers
/// they own are only reachable through an event.
#[cfg(test)]
pub(crate) mod press;

pub(crate) mod chrome;

pub(crate) mod code;

pub(crate) mod recipes;

pub(crate) mod skills;

pub(crate) mod scheduler;

pub(crate) mod extensions;

// Session history (PR 7) adds no module: it is `sessions` growing kinds,
// rename and search.

pub(crate) mod session_settings;

pub(crate) mod sessions;

pub(crate) mod settings;

use dioxus::prelude::*;

use crate::icons::Icon;
use crate::shell::Shell;
use crate::state::{AppCtx, ConnState};

/// Small colored dot + label reflecting the connection state.
#[component]
pub fn ConnBadge() -> Element {
    let ctx: AppCtx = crate::state::use_app_ctx();
    let (class, label) = match (ctx.conn)() {
        ConnState::Disconnected => ("dot off", "offline".to_string()),
        ConnState::Connecting => ("dot busy", "connecting…".to_string()),
        ConnState::Connected { agent } => ("dot on", agent),
        ConnState::Failed(_) => ("dot err", "error".to_string()),
    };
    rsx! {
        span { class: "conn-badge",
            span { class: "{class}" }
            span { class: "conn-label", "{label}" }
        }
    }
}

/// The tray behind a session row, revealed by dragging the row to the left.
///
/// It is the last item in the row's horizontal scroller, so at rest it sits
/// past the card's right edge and is clipped away entirely — there is no
/// always-visible destructive control on a list you scroll with your thumb.
/// The tap stops propagating because the row itself is the tap target
/// (design rule 9), and "Delete" is not "open".
///
/// The second tray renderer in the app, and the reason it has to know about
/// the shell too: `views/code.rs` builds its session rows by hand rather than
/// through `ListRow`, so this is the ONLY delete a Code session has. Left
/// alone it would be the one list on the desktop with no row action at all.
/// Converting that row to `ListRow` is the tidier end state and would delete
/// this component, but it changes the phone's markup, so it belongs to a pass
/// that re-captures the gallery.
#[component]
pub fn SwipeDelete(on_delete: EventHandler<()>) -> Element {
    let face = crate::views::chrome::RowFace::DELETE;
    let (word, title) = crate::views::chrome::row_action_words(Shell::CURRENT, face);
    rsx! {
        div { class: "session-actions",
            button {
                class: face.class(Shell::CURRENT),
                title,
                onclick: move |e: Event<MouseData>| {
                    e.stop_propagation();
                    on_delete.call(());
                },
                Icon { name: face.icon }
                "{word}"
            }
        }
    }
}

/// The way back to the bottom of a transcript you have read up from.
///
/// Rendered on every chat screen; whether it is *visible* is decided in JS
/// (`crate::viewport`), because the alternative is an `onscroll` handler in
/// Rust and a blocking round trip on every frame of every scroll. Rust owns
/// that the button exists and what a tap does, and nothing else about it.
///
/// The slot around it has no height and sits between the transcript and
/// everything below it, so the button hangs above whatever comes next and
/// moves with it. It has to: the composer grows with the draft, and the whole
/// shell tracks the visual viewport when the keyboard opens, so anything
/// placed against the bottom of the screen ends up behind one or the other.
#[component]
pub fn ScrollToBottom(scroller: &'static str) -> Element {
    rsx! {
        div { class: "scroll-bottom-slot",
            button {
                class: "scroll-bottom",
                title: "Jump to the latest",
                onclick: move |_| crate::viewport::scroll_to_bottom(scroller),
                Icon { name: "arrow-down" }
            }
        }
    }
}

/// The confirmation a destructive row action earns, as a sheet rather than a
/// row in the card. Say it plainly, then Cancel and the thing itself.
///
/// A swipe on the phone, a click on an always-visible icon on the desktop
/// (`views::chrome::ListRow`). The pointer's version is the easier one to hit
/// by accident, so the sheet matters more there, not less.
///
/// One component rather than one per action, because the difference between
/// confirming a delete and confirming a merge is two strings and which colour
/// the second button takes — and a copy of the modal per action is a copy of
/// the modal per action to keep in step.
#[component]
pub fn Confirm(
    title: String,
    body: String,
    /// The word on the button that does it. Never "OK": the label is the last
    /// chance to say what is about to happen.
    confirm_label: String,
    /// Destructive, and coloured as such (rule 7 — a control the user presses
    /// is what earns a saturated fill).
    danger: bool,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "modal-backdrop",
            div { class: "modal",
                h2 { "{title}" }
                p { class: "modal-body", "{body}" }
                div { class: "modal-actions",
                    button {
                        class: "btn secondary",
                        onclick: move |_| on_cancel.call(()),
                        "Cancel"
                    }
                    button {
                        class: if danger { "btn danger" } else { "btn primary" },
                        onclick: move |_| on_confirm.call(()),
                        "{confirm_label}"
                    }
                }
            }
        }
    }
}

/// The confirmation a destructive row action earns — a swipe on the phone, a
/// click on a row icon on the desktop — as a sheet rather than a row in the
/// card.
///
/// Both planes delete for good: goose's `session/delete` is not a soft
/// delete, and the code plane purges the container and the workspace with the
/// branch still in it. With no undo to offer, a drag on its own is not
/// consent. That is Messages' arrangement rather than Mail's — Mail can
/// afford a one-tap swipe because it has a Trash to fish things back out of.
#[component]
pub fn ConfirmDelete(
    title: String,
    body: String,
    /// The word on the button that does it. "Delete" is right for a list row
    /// and wrong for the things that are not deletions — killing a running
    /// job stops it, it does not remove it — and a confirmation whose button
    /// does not name the act is a confirmation of nothing in particular.
    #[props(default = "Delete".to_owned())]
    confirm_label: String,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    rsx! {
        Confirm {
            title,
            body,
            confirm_label,
            danger: true,
            on_confirm,
            on_cancel,
        }
    }
}

/// Correcting a name, as a sheet with one field in it.
///
/// A sheet at the view's root and never inside a bar, for the reason written
/// on [`OverflowSheet`]: the bar's controls carry `backdrop-filter`, which
/// makes them the containing block for every `position: fixed` descendant.
///
/// It saves on the button and never on a keystroke. That is the app's standing
/// convention for anything that writes to the server — `SearchField` is the
/// single exemption, and it is exempt because it writes nothing.
#[component]
pub fn RenameSheet(
    heading: String,
    /// What the name is now. Shown ready to be edited rather than as a
    /// placeholder: most renames are a correction to the existing title, not
    /// a replacement for it.
    value: String,
    on_save: EventHandler<String>,
    on_cancel: EventHandler<()>,
) -> Element {
    let mut draft = use_signal(|| value);
    let text = draft();
    // A blank title is not a rename, it is a session with no name — and goose
    // would take it.
    let ready = !text.trim().is_empty();
    let save = move |()| {
        if ready {
            on_save.call(draft.peek().trim().to_owned());
        }
    };

    rsx! {
        div { class: "modal-backdrop", onclick: move |_| on_cancel.call(()),
            div {
                // `rename` alongside `sheet` for the reason `OverflowSheet`
                // carries `menu`: two panes that both answer to `.modal.sheet`
                // file under one gallery state, and whichever was captured
                // last wins. The class is here, the stylesheet uses it, and
                // `domdump.rs`'s suffix ladder reads it — above the generic
                // `.modal.sheet`, which this pane would otherwise answer to.
                class: "modal sheet rename",
                onclick: move |e: Event<MouseData>| e.stop_propagation(),
                h2 { "{heading}" }
                label { class: "field-label", "Title" }
                input {
                    class: "field",
                    r#type: "text",
                    placeholder: "What this chat is about",
                    autocomplete: "off",
                    value: "{text}",
                    oninput: move |e| draft.set(e.value()),
                    // The keyboard's return key is an explicit action like any
                    // other button, and it is the one under the thumb.
                    onkeydown: move |e: Event<KeyboardData>| {
                        if e.key() == Key::Enter {
                            e.prevent_default();
                            save(());
                        }
                    },
                }
                div { class: "modal-actions",
                    button {
                        class: "btn secondary",
                        onclick: move |_| on_cancel.call(()),
                        "Cancel"
                    }
                    button {
                        class: "btn primary",
                        disabled: !ready,
                        onclick: move |_| save(()),
                        "Save"
                    }
                }
            }
        }
    }
}

/// One entry in an overflow menu.
#[derive(Clone, PartialEq)]
pub(crate) struct MenuItem {
    pub icon: &'static str,
    pub label: &'static str,
    /// Destructive, and coloured as such.
    pub danger: bool,
}

/// The `⋯` in the top bar.
///
/// Only the button: the sheet it opens is [`OverflowSheet`], rendered at the
/// view's root beside the other overlays. They cannot be one component,
/// because the bar's controls carry `backdrop-filter` — and a filtered element
/// becomes the containing block for every `position: fixed` descendant, so a
/// sheet rendered in here is trapped inside a 94px pill in the corner instead
/// of covering the screen. The same property is why `.app` deliberately avoids
/// a transform.
#[component]
pub fn OverflowButton(onopen: EventHandler<()>) -> Element {
    rsx! {
        button {
            class: "icon-btn",
            title: "More",
            onclick: move |_| onopen.call(()),
            Icon { name: "more" }
        }
    }
}

/// What the `⋯` opens. Render this at the root of a view, not inside the bar.
#[component]
pub fn OverflowSheet(
    items: Vec<MenuItem>,
    onpick: EventHandler<usize>,
    onclose: EventHandler<()>,
) -> Element {
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| onclose.call(()),
            div {
                // `menu` as well as `sheet`: it is the same pane, but the
                // capture harness has to be able to tell an overflow menu from
                // the settings sheet, or they file under one gallery state and
                // whichever was seen last wins.
                class: "modal sheet menu",
                onclick: move |e: Event<MouseData>| e.stop_propagation(),
                div { class: "setting-list",
                    for (i, item) in items.iter().enumerate() {
                        button {
                            key: "{item.label}",
                            class: if item.danger { "setting-row danger" } else { "setting-row" },
                            onclick: move |_| onpick.call(i),
                            Icon { name: "{item.icon}" }
                            span { class: "setting-main",
                                span { class: "setting-name", "{item.label}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use dioxus::prelude::*;

    use super::press::{alone, Js, Pressable};
    use crate::state::AppCtx;

    thread_local! {
        /// The document a probe installs, reachable from the test that mounted
        /// it. A `fn() -> Element` cannot capture, so the two ends meet here.
        static RECORDER: Js = Js::default();
    }

    /// A session row with a swipe tray behind it, arranged the way a list
    /// arranges one: the card underneath opens the session, the tray on top
    /// deletes it. The two outcomes are written into two signals so a test can
    /// say which of them a tap produced.
    fn swipe_row() -> Element {
        let ctx: AppCtx = crate::state::use_app_ctx();
        rsx! {
            div {
                class: "session-item",
                // One line on purpose: the whole point of the test below is
                // that this never runs, so a body spread over four lines is
                // four lines the coverage report will never account for.
                onclick: move |_| ctx.chat_draft.clone().set("the row opened it".to_owned()),
                super::SwipeDelete {
                    on_delete: move |()| {
                        let mut asked = ctx.sessions_query;
                        asked.set("delete was asked for".to_owned());
                    },
                }
            }
        }
    }

    /// Delete is the one control in the row that is not the row.
    ///
    /// The card behind it is the tap target for the whole row (design rule 9),
    /// so a Delete that let its click bubble would ask to delete the session
    /// AND walk into it — on the phone a transcript sliding in over a row that
    /// is about to vanish, on the desktop a detail column showing a session
    /// the list has already dropped. `stop_propagation` is the whole of that
    /// rule and nothing in the type system holds it in place.
    #[test]
    fn deleting_a_row_does_not_also_open_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(|_| {}, swipe_row);
        screen.press("title=\"Delete\"");

        assert_eq!(
            screen.with(|ctx| ctx.sessions_query.peek().clone()),
            "delete was asked for",
            "pressing the tray's button never reached `on_delete`, so the only \
             delete a Code session has does nothing at all"
        );
        assert!(
            screen.with(|ctx| ctx.chat_draft.peek().is_empty()),
            "the tap went through the tray and opened the row behind it as \
             well, so deleting a session also navigates into it"
        );
    }

    /// The jump-to-the-bottom button, over a document that records what the
    /// app evaluates in it.
    fn scroll_button() -> Element {
        use_hook(|| RECORDER.with(Js::clone)).install();
        rsx! { super::ScrollToBottom { scroller: "chat-scroll" } }
    }

    /// The button's whole job is the scroll, and the scroll is a
    /// `document::eval` against the scroller it was handed. A press that
    /// evaluated nothing — or evaluated against some other element's id —
    /// looks identical on screen: the transcript stays where the reader left
    /// it and the button stays up, which is exactly the state it exists to get
    /// them out of.
    #[test]
    fn the_jump_button_scrolls_the_transcript_it_was_named() {
        let _alone = alone();
        RECORDER.with(Js::clear);
        let mut screen = Pressable::mount(|_| {}, scroll_button);
        assert!(
            RECORDER.with(Js::scripts).is_empty(),
            "the button scrolled the transcript before anyone pressed it"
        );

        screen.press("class=\"scroll-bottom\"");
        let script = RECORDER.with(|js| js.script_with("getElementById"));
        assert!(
            script.contains("getElementById('chat-scroll')"),
            "the jump went to some element other than the transcript the \
             button was given: {script}"
        );
        assert!(
            script.contains("el.scrollTop = el.scrollHeight"),
            "the script the button ran does not take the transcript to its \
             bottom: {script}"
        );
        assert!(
            script.contains("window.__atBottom = true"),
            "the jump left `__atBottom` alone, so the pin will not follow new \
             content and the button that was just pressed stays on screen: \
             {script}"
        );
    }

    /// The rename sheet, with the two outcomes it can produce written where a
    /// test can read them: the saved name, and the fact that it was cancelled.
    fn rename_sheet() -> Element {
        let ctx: AppCtx = crate::state::use_app_ctx();
        rsx! {
            super::RenameSheet {
                heading: "Rename chat",
                value: "Rotate the certificate",
                on_save: move |name: String| {
                    let mut saved = ctx.chat_draft;
                    saved.set(name);
                },
                on_cancel: move |()| {
                    let mut toast = ctx.toast;
                    toast.set(Some("cancelled".to_owned()));
                },
            }
        }
    }

    /// The return key is the button under the thumb: on a short phone the
    /// keyboard is over the sheet's own Save, so a sheet that only saved from
    /// the button would be a rename you cannot commit without dismissing the
    /// keyboard first.
    ///
    /// And it saves what was typed, trimmed. A title that kept the leading
    /// space a phone keyboard leaves behind an autocorrect sorts and searches
    /// wrong for as long as the session exists.
    #[test]
    fn the_return_key_saves_the_name_that_was_typed() {
        let _alone = alone();
        let mut screen = Pressable::mount(|_| {}, rename_sheet);
        let opened = screen.markup();
        assert!(
            opened.contains("Rotate the certificate"),
            "the sheet opened on something other than the name it was given, \
             so every rename starts by retyping the title: {opened}"
        );

        screen.type_into("class=\"field\"", "  Rotate the wildcard certificate  ");
        screen.enter("class=\"field\"");
        assert_eq!(
            screen.with(|ctx| ctx.chat_draft.peek().clone()),
            "Rotate the wildcard certificate",
            "Enter in the field did not save the name that was typed, trimmed"
        );
    }

    /// A blank title is not a rename, it is a session with no name — and goose
    /// would take it. The button says so by going disabled; the return key has
    /// to say the same thing, or the one path that skips the button is the one
    /// path that can wipe a title.
    #[test]
    fn a_blank_title_is_not_a_rename_from_the_keyboard_either() {
        let _alone = alone();
        let mut screen = Pressable::mount(|_| {}, rename_sheet);
        screen.type_into("class=\"field\"", "   ");
        let blank = screen.markup();
        assert!(
            blank.contains("disabled"),
            "Save is still pressable over a blank title: {blank}"
        );

        screen.enter("class=\"field\"");
        assert!(
            screen.with(|ctx| ctx.chat_draft.peek().is_empty()),
            "Enter saved a blank title, so the session is left with no name at \
             all and the button's disabled state was decoration"
        );
    }

    /// Every sheet in this app dismisses by tapping the dark outside it, and
    /// this one is a modal over a list: without that tap a reader who opened
    /// it by accident has Cancel and nothing else — and on the desktop, where
    /// rename is an always-visible icon on the row, opening it by accident is
    /// the easy mistake.
    #[test]
    fn a_tap_outside_the_rename_sheet_cancels_it() {
        let _alone = alone();
        let mut screen = Pressable::mount(|_| {}, rename_sheet);
        screen.press("class=\"modal-backdrop\"");
        assert_eq!(
            screen.with(|ctx| ctx.toast.peek().clone()),
            Some("cancelled".to_owned()),
            "tapping outside the sheet did not dismiss it"
        );
        assert!(
            screen.with(|ctx| ctx.chat_draft.peek().is_empty()),
            "the dismissal saved the name on its way out as well"
        );
    }

    /// The sheet itself is not the backdrop. A tap inside it — reaching for
    /// the field, or missing the Save button — must not close it, which is
    /// what the inner `stop_propagation` is for and what nothing else holds in
    /// place.
    #[test]
    fn a_tap_inside_the_rename_sheet_keeps_it_open() {
        let _alone = alone();
        let mut screen = Pressable::mount(|_| {}, rename_sheet);
        screen.press("class=\"modal sheet rename\"");
        assert_eq!(
            screen.with(|ctx| ctx.toast.peek().clone()),
            None,
            "a tap on the sheet bubbled out to the backdrop and dismissed the \
             rename the reader was in the middle of typing"
        );
    }
}
