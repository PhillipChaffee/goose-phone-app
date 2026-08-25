pub(crate) mod attach;

pub(crate) mod chat;

pub(crate) mod chrome;

pub(crate) mod code;

pub(crate) mod recipes;

pub(crate) mod skills;

// scheduler — PR 5 replaces this line

pub(crate) mod extensions;

// Session history (PR 7) adds no module: it is `sessions` growing kinds,
// rename and search.

pub(crate) mod session_settings;

pub(crate) mod sessions;

pub(crate) mod settings;

use dioxus::prelude::*;

use crate::icons::Icon;
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
#[component]
pub fn SwipeDelete(on_delete: EventHandler<()>) -> Element {
    rsx! {
        div { class: "session-actions",
            button {
                class: "swipe-action danger",
                onclick: move |e: Event<MouseData>| {
                    e.stop_propagation();
                    on_delete.call(());
                },
                Icon { name: "trash" }
                "Delete"
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

/// The confirmation a swipe earns, as a sheet rather than a row in the card.
/// Say it plainly, then Cancel and the thing itself.
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

/// The confirmation a swipe earns, as a sheet rather than a row in the card.
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
