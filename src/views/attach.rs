//! The three places an attachment shows up: the button that picks one, the
//! tray that holds it before it is sent, and the message it ends up in.
//!
//! Deliberately one small module. The look of a chip and a thumbnail is the
//! part of this feature that is still provisional — it is built plainly, in
//! the existing chip grammar, so restyling it means editing `.attach-*` in
//! `assets/main.css` and these three components and nothing else.

use dioxus::prelude::*;

use crate::attach::{AttachTarget, Attachment};
use crate::icons::Icon;
use crate::state::use_app_ctx;

/// The `+` that opens iOS's photo / camera / Files sheet.
///
/// It carries no `onclick`. The sheet has to be opened from inside the real
/// touch event, and a Rust handler runs a round trip too late for that — so
/// the click is caught by a capture-phase listener in JavaScript, which finds
/// this button by its class and reads `data-attach` to know which composer
/// asked. See `crate::attach::PICK_FILES`.
///
/// `data-conversation` is the other half of that address: which composer is
/// not enough, because the read outlives the screen it was started on
/// (`crate::attach::conversation_key`).
#[component]
pub fn AttachButton(target: AttachTarget, conversation: String) -> Element {
    rsx! {
        button {
            class: "composer-chip action attach",
            "data-attach": "{target.as_str()}",
            "data-conversation": "{conversation}",
            title: "Attach an image or a file",
            "aria-label": "Attach an image or a file",
            Icon { name: "plus" }
        }
    }
}

/// What is attached to the message being written, and how to take it back
/// off.
///
/// A row of its own above the field rather than another chip in the control
/// row: the control row has a fixed budget of width that the send button has
/// to survive (`docs/measure-composer.js` exists because it once did not),
/// and a file name is server-length text. It scrolls sideways for the same
/// reason the action row does — wrapping would move the composer, and the
/// composer's position is the one thing on this screen a thumb learns.
#[component]
pub fn AttachTray(target: AttachTarget, conversation: String) -> Element {
    let ctx = use_app_ctx();
    // The conversation, not just the target: the Code tab's two composers hold
    // two different trays and this component renders both of them.
    let mut tray = crate::attach::tray_of(&ctx, target, &conversation);
    // Read without cloning: this holds the bytes of every picked file, and
    // the composer re-renders on every keystroke.
    let held = tray.read();
    // Only the picks made here. One left reading in a chat you walked out of
    // is going to land in that chat's tray, not this one, so announcing it
    // here would be a promise this composer cannot keep.
    let reading = crate::attach::reading_for(&ctx.attach_reading.read(), target, &conversation);
    if held.is_empty() && reading == 0 {
        return rsx! {};
    }

    let chips = held.iter().enumerate().map(|(index, file)| {
        let record = &file.record;
        let name = record.name.clone();
        let size = record.size_label();
        let thumb = record.thumb.clone();
        rsx! {
            div { key: "{index}-{name}", class: "attach-chip", role: "listitem",
                if thumb.is_empty() {
                    span { class: "attach-icon", Icon { name: "file" } }
                } else {
                    img {
                        class: "attach-thumb",
                        src: "data:image/jpeg;base64,{thumb}",
                        alt: "{name}",
                    }
                }
                span { class: "attach-meta",
                    span { class: "attach-name", "{name}" }
                    if !size.is_empty() {
                        span { class: "attach-size", "{size}" }
                    }
                }
                button {
                    class: "attach-remove",
                    title: "Remove {name}",
                    "aria-label": "Remove {name}",
                    onclick: move |_| {
                        let mut held = tray.write();
                        if index < held.len() {
                            held.remove(index);
                        }
                    },
                    Icon { name: "close" }
                }
            }
        }
    });

    rsx! {
        div { class: "attach-tray", role: "list", "aria-label": "Attachments",
            {chips}
            if reading > 0 {
                span { class: "attach-chip reading", "aria-live": "polite",
                    span { class: "attach-meta",
                        span { class: "attach-name",
                            if reading == 1 { "Reading a file…" } else { "Reading {reading} files…" }
                        }
                    }
                }
            }
        }
    }
}

/// Attachments as they appear inside the user's bubble in the transcript.
///
/// An image gets its thumbnail — that is the whole reason the phone keeps
/// one — and anything else gets a chip naming it, because a file the agent
/// read is a fact about the turn and the name is what makes it checkable.
pub(crate) fn attachment_list(attachments: &[Attachment]) -> Element {
    let items = attachments.iter().enumerate().map(|(index, record)| {
        let name = record.name.clone();
        let size = record.size_label();
        let thumb = record.thumb.clone();
        rsx! {
            if thumb.is_empty() {
                span { key: "{index}-{name}", class: "attach-file",
                    span { class: "attach-icon", Icon { name: "file" } }
                    span { class: "attach-name", "{name}" }
                    if !size.is_empty() {
                        span { class: "attach-size", "{size}" }
                    }
                }
            } else {
                img {
                    key: "{index}-{name}",
                    class: "attach-image",
                    src: "data:image/jpeg;base64,{thumb}",
                    alt: "{name}",
                }
            }
        }
    });
    rsx! {
        div { class: "attach-list", {items} }
    }
}
