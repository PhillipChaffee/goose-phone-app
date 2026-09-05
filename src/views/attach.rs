//! The three places an attachment shows up: the button that picks one, the
//! tray that holds it before it is sent, and the message it ends up in.
//!
//! Deliberately one small module. The look of a chip and a thumbnail is the
//! part of this feature that is still provisional — it is built plainly, in
//! the existing chip grammar, so restyling it means editing `.attach-*` in
//! `assets/shared.css` and these three components and nothing else.

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

#[cfg(test)]
mod tests {
    use super::{attachment_list, AttachButton, AttachTarget, AttachTray, Attachment};
    use crate::attach::PendingAttachment;
    use crate::state::AppCtx;
    use crate::testkit::{render, render_seeded};

    use dioxus::html::{
        set_event_converter, PlatformEventData, SerializedHtmlEventConverter, SerializedMouseData,
    };
    use dioxus::prelude::*;
    use std::any::Any;
    use std::rc::Rc;

    // -----------------------------------------------------------------------
    // Fixtures. Seeds are `fn` pointers and cannot capture, so everything a
    // seed needs is a free function it can call.

    /// A file in the tray. An empty `thumb` is "no picture", which is the
    /// branch deciding whether a chip shows an icon or the photo itself.
    fn pending(name: &str, mime: &str, size: u64, thumb: &str) -> PendingAttachment {
        PendingAttachment {
            record: Attachment {
                name: name.to_owned(),
                mime: mime.to_owned(),
                size,
                thumb: thumb.to_owned(),
            },
            data: "QUJD".to_owned(),
            text: None,
        }
    }

    /// One photo and one document, in that order, in the goose composer's
    /// tray.
    fn two_files(ctx: &AppCtx) {
        let mut tray = ctx.attachments;
        tray.set(vec![
            pending("IMG_0042.jpg", "image/jpeg", 18_003, "THUMBBYTES"),
            pending("spec.pdf", "application/pdf", 1_200, ""),
        ]);
    }

    fn goose_tray() -> Element {
        rsx! { AttachTray { target: AttachTarget::Goose, conversation: String::new() } }
    }

    /// An `ElementId` past the end is ignored rather than fatal
    /// (`Runtime::handle_event` does a `get`), so this only has to be larger
    /// than any screen here.
    const EVERY_ELEMENT: u32 = 60;

    /// Mount, click one element, and hand back the markup that produced.
    ///
    /// WHICH ELEMENT IS TAPPED IS NOT GUESSED AT: Dioxus addresses an element
    /// by an `ElementId` assigned in creation order and nothing in the markup
    /// maps back to one, so every element is tapped in its own fresh mount and
    /// the assertion is on how many of them did the thing. The same shape
    /// `views::extensions`'s tap tests use, and for the same reason — "exactly
    /// one control removes this chip" cannot rot into pointing at the wrong
    /// button when one is added above it.
    fn tap(app: fn() -> Element, target: u32) -> String {
        let _ = crate::testkit::storage_dir();
        // The listener an `onclick` installs takes a `PlatformEventData`, not
        // a `MouseData`: the shell that owns the window is what turns one into
        // the other, through a converter registered process-wide. There is no
        // shell here, so this registers the serialized one dioxus-html ships
        // for exactly that — a write of a global, but an idempotent one.
        set_event_converter(Box::new(SerializedHtmlEventConverter));
        let mut dom = VirtualDom::new(app);
        dom.rebuild_in_place();
        let data: Box<dyn Any> = Box::new(SerializedMouseData::default());
        let event: Rc<dyn Any> = Rc::new(PlatformEventData::new(data));
        dom.runtime().handle_event(
            "click",
            dioxus::dioxus_core::Event::new(event, false),
            dioxus::dioxus_core::ElementId(target as usize),
        );
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    /// The tray with two files in it, mounted so a tap can be delivered to it.
    fn tray_of_two() -> Element {
        let ctx = crate::state::use_app_ctx_provider();
        use_hook(|| two_files(&ctx));
        goose_tray()
    }

    // -----------------------------------------------------------------------
    // The button.

    /// The `+` carries no `onclick` — the sheet has to open inside the real
    /// touch event, so a capture-phase JavaScript listener finds this button by
    /// its class and reads its two data attributes to know who asked
    /// (`crate::attach::PICK_FILES`). Those three strings ARE the wiring: drop
    /// the class and the button stops opening anything at all, drop either
    /// attribute and a photo picked in the Code tab lands in the goose
    /// composer, or in whichever chat happens to be open when the read
    /// finishes seconds later.
    #[test]
    fn the_attach_button_carries_the_address_the_picker_reads_off_it() {
        let html = render(|| {
            rsx! {
                AttachButton { target: AttachTarget::Code, conversation: "chat-7".to_owned() }
            }
        });
        assert!(
            html.contains("class=\"composer-chip action attach\""),
            "the attach button has lost the class the capture-phase listener \
             finds it by, so tapping it opens no picker at all"
        );
        assert!(
            html.contains("data-attach=\"code\""),
            "the button does not say which composer asked, so the read lands \
             in the other one's tray"
        );
        assert!(
            html.contains("data-conversation=\"chat-7\""),
            "the button does not say which conversation asked, so a pick that \
             finishes after you walk out lands in whatever chat is open then"
        );
        assert!(
            html.contains("aria-label=\"Attach an image or a file\""),
            "the picker button is an unlabelled icon"
        );
    }

    /// The two composers address themselves differently, which is the whole
    /// point of the attribute — `AttachTarget::as_str` is what the JavaScript
    /// compares against.
    #[test]
    fn the_goose_composers_button_is_addressed_as_goose() {
        let html = render(|| {
            rsx! {
                AttachButton { target: AttachTarget::Goose, conversation: "s-1".to_owned() }
            }
        });
        assert!(
            html.contains("data-attach=\"goose\""),
            "both composers' buttons claim to be the same one"
        );
    }

    // -----------------------------------------------------------------------
    // The tray.

    /// Nothing attached and nothing being read means NO ROW, not an empty one.
    ///
    /// The tray sits above the field, so an always-rendered `.attach-tray`
    /// would push the composer up by its own height for the whole life of the
    /// screen — and the composer's position is the one thing on this screen a
    /// thumb learns.
    #[test]
    fn an_empty_tray_takes_no_room_above_the_composer() {
        let html = render(goose_tray);
        assert!(
            html.is_empty(),
            "the tray rendered something with nothing in it, so the composer \
             sits higher than it should on every screen that has never \
             attached anything"
        );
    }

    /// A chip has to say enough to check what is about to be sent: the name,
    /// what it weighs, and — for a photo — what it looks like. A picture is the
    /// only way to tell two files both called `image.jpg` apart, which is what
    /// every camera capture on iOS is called.
    #[test]
    fn a_photo_shows_its_picture_and_a_document_shows_an_icon() {
        let html = render_seeded(two_files, goose_tray);
        assert!(
            html.contains(
                "<img class=\"attach-thumb\" src=\"data:image/jpeg;base64,THUMBBYTES\" \
                 alt=\"IMG_0042.jpg\"/>"
            ),
            "the photo in the tray is not showing its thumbnail, so two camera \
             captures in one message are indistinguishable"
        );
        assert!(
            html.contains("<span class=\"attach-icon\">") && html.contains("spec.pdf"),
            "a file with no picture got no icon, leaving a nameless gap where \
             the chip should be"
        );
        assert_eq!(
            html.matches("attach-thumb").count(),
            1,
            "the document is being rendered as an image too, so its chip is a \
             broken picture"
        );
        assert!(
            html.contains("<span class=\"attach-size\">18 kB</span>")
                && html.contains("<span class=\"attach-size\">1 kB</span>"),
            "a chip does not say what its file weighs, which is the only \
             warning before a message is refused for being too heavy"
        );
        assert!(
            html.contains("aria-label=\"Remove IMG_0042.jpg\"")
                && html.contains("aria-label=\"Remove spec.pdf\""),
            "a chip's remove control does not name the file it removes, so on \
             a screen reader the tray is a row of identical buttons"
        );
    }

    /// A size of zero is "we do not know", not "an empty file" — a resource
    /// link names a file it does not carry. Printing `0 B` would be the phone
    /// asserting something the protocol never told it.
    #[test]
    fn a_file_of_unknown_size_says_nothing_rather_than_zero() {
        fn sizeless(ctx: &AppCtx) {
            let mut tray = ctx.attachments;
            tray.set(vec![pending("linked.txt", "text/plain", 0, "")]);
        }
        let html = render_seeded(sizeless, goose_tray);
        assert!(html.contains("linked.txt"), "the chip is missing entirely");
        assert!(
            !html.contains("attach-size"),
            "a file whose size is unknown is being described as 0 B"
        );
    }

    /// The × takes ITS OWN chip off. The handler closes over an index into the
    /// tray, so an off-by-one — or a shared index — deletes the file the reader
    /// meant to keep and leaves the one they meant to drop in the message.
    #[test]
    fn each_chips_remove_control_takes_that_chip_and_no_other() {
        let after: Vec<String> = (1..=EVERY_ELEMENT)
            .map(|target| tap(tray_of_two, target))
            .filter(|html| !html.contains("IMG_0042.jpg") || !html.contains("spec.pdf"))
            .collect();
        assert_eq!(
            after.len(),
            2,
            "a two-file tray does not have exactly two controls that remove \
             something — a chip has stopped being removable, or something that \
             is not a remove button removes one"
        );
        assert!(
            after
                .iter()
                .any(|html| html.contains("spec.pdf") && !html.contains("IMG_0042.jpg")),
            "no control removes the photo while leaving the document"
        );
        assert!(
            after
                .iter()
                .any(|html| html.contains("IMG_0042.jpg") && !html.contains("spec.pdf")),
            "both remove controls take the same chip off, so the file you meant \
             to drop is still in the message"
        );
    }

    /// Reading takes seconds — several photos have to be decoded, downscaled
    /// and base64'd — so the composer says so. Without the row it simply sits
    /// there after the picker closes, which reads as a tap that missed.
    #[test]
    fn a_read_in_flight_is_announced_in_the_composer_that_started_it() {
        fn one(ctx: &AppCtx) {
            crate::attach::receive(
                ctx,
                r#"{"pick":1,"target":"goose","conversation":"","reading":1}"#,
            );
        }
        fn several(ctx: &AppCtx) {
            crate::attach::receive(
                ctx,
                r#"{"pick":1,"target":"goose","conversation":"","reading":3}"#,
            );
        }
        let single = render_seeded(one, goose_tray);
        assert!(
            single.contains("Reading a file…"),
            "a read in flight leaves the composer silent, so the picker closing \
             looks like nothing happened"
        );
        assert!(
            single.contains("aria-live=\"polite\""),
            "the progress row is not announced, so a screen reader user is told \
             nothing at all"
        );
        let many = render_seeded(several, goose_tray);
        assert!(
            many.contains("Reading 3 files…"),
            "three files being read are announced as one"
        );
    }

    /// A read started in another chat must NOT be announced here. It is going
    /// to land in that chat's tray (`crate::attach::conversation_key`), so
    /// saying "Reading a file…" over this composer is a promise it cannot
    /// keep — the row would sit there and then nothing would arrive.
    #[test]
    fn a_read_started_in_another_conversation_is_not_announced_here() {
        fn elsewhere(ctx: &AppCtx) {
            crate::attach::receive(
                ctx,
                r#"{"pick":1,"target":"goose","conversation":"s-other","reading":2}"#,
            );
        }
        let html = render_seeded(elsewhere, goose_tray);
        assert!(
            html.is_empty(),
            "this composer is announcing a read belonging to a conversation the \
             reader has already left, so the row never resolves"
        );
    }

    /// The other composer's read is not this one's either. The Code tab keeps
    /// its own tray for the same reason it keeps its own draft.
    #[test]
    fn the_other_composers_read_is_not_announced_here() {
        fn code_side(ctx: &AppCtx) {
            crate::attach::receive(
                ctx,
                r#"{"pick":1,"target":"code","conversation":"","reading":2}"#,
            );
        }
        let html = render_seeded(code_side, goose_tray);
        assert!(
            html.is_empty(),
            "a pick made in the Code composer is being reported over the goose \
             one"
        );
    }

    // -----------------------------------------------------------------------
    // The message it ends up in.

    /// In the transcript an image is the picture and everything else is a named
    /// chip. The name is the point for a document: a file the agent read is a
    /// fact about the turn, and the name is what makes it checkable.
    #[test]
    fn a_sent_photo_is_shown_and_a_sent_document_is_named() {
        let html = render(|| {
            attachment_list(&[
                Attachment {
                    name: "IMG_0042.jpg".to_owned(),
                    mime: "image/jpeg".to_owned(),
                    size: 18_003,
                    thumb: "THUMBBYTES".to_owned(),
                },
                Attachment {
                    name: "spec.pdf".to_owned(),
                    mime: "application/pdf".to_owned(),
                    size: 1_200,
                    thumb: String::new(),
                },
            ])
        });
        assert!(
            html.contains(
                "<img class=\"attach-image\" src=\"data:image/jpeg;base64,THUMBBYTES\" \
                 alt=\"IMG_0042.jpg\"/>"
            ),
            "a photo in the transcript rendered as a chip, which throws away \
             the whole reason a thumbnail is kept at all"
        );
        assert!(
            html.contains("<span class=\"attach-file\">")
                && html.contains("<span class=\"attach-name\">spec.pdf</span>"),
            "a document the agent was sent is not named in the bubble, so there \
             is no record of what it read"
        );
        assert!(
            html.contains("<span class=\"attach-size\">1 kB</span>"),
            "a named file does not say what it weighed"
        );
        assert_eq!(
            html.matches("<img").count(),
            1,
            "the document is being rendered as a picture as well"
        );
    }

    /// A replayed attachment can arrive with no size — goose's resource link
    /// names a file it does not carry — and an invented `0 B` under it would be
    /// the transcript stating something the server never said.
    #[test]
    fn a_replayed_file_with_no_known_size_is_named_and_nothing_more() {
        let html = render(|| {
            attachment_list(&[Attachment {
                name: "notes.md".to_owned(),
                mime: "text/markdown".to_owned(),
                size: 0,
                thumb: String::new(),
            }])
        });
        assert!(
            html.contains("<span class=\"attach-name\">notes.md</span>"),
            "the replayed file lost its name"
        );
        assert!(
            !html.contains("attach-size"),
            "a file goose replayed without a size is being given one"
        );
    }
}
