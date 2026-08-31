//! Attaching an image or a file to the message you are about to send.
//!
//! Both backends already carry attachments — ACP's prompt is an array of
//! `ContentBlock`s and `OpenCode`'s is an array of parts — so the work here is
//! the three things neither protocol does for you: getting the bytes off the
//! phone, keeping them to a size a tailnet round trip can survive, and
//! deciding what the transcript remembers afterwards.
//!
//! **Picking** happens in JavaScript (see `PICK_FILES`), because a hidden
//! `<input type="file">` is iOS's photo library, camera and Files sheet for
//! free, and because the read and the downscale are a single round trip that
//! way instead of one per chunk of bytes.
//!
//! **The size rule** is the reason this module has constants at all. A phone
//! photo is several megabytes and base64 adds a third on top, and the code
//! plane's request travels through the session manager's proxy into a
//! container. So an image is downscaled rather than refused — you have just
//! taken the photo; being told it is too big is not an answer — and everything
//! else is capped and refused by name.
//!
//! **What the transcript keeps** is a *thumbnail*, never the payload. The chat
//! views clone their whole state on every keystroke and the Code tab persists
//! its transcript to disk, so an item holding a 250 kB base64 photo is paid
//! for on both, over and over. The full bytes live in the composer's tray
//! until they are sent and then only on the server.

use std::collections::HashMap;

use dioxus::prelude::*;
use goose_acp_client::ContentBlock;
use opencode_client::{Part, PromptPart};
use serde::{Deserialize, Serialize};

use crate::state::{show_toast, AppCtx, ChatItem};

/// How many files one message may carry.
pub(crate) const MAX_ATTACHMENTS: usize = 6;

/// The most one attachment may weigh, after any downscaling. Decimal
/// megabytes, because that is what iOS calls a file size and the cap should
/// read as the same number the phone shows you.
pub(crate) const MAX_FILE_BYTES: u64 = 4_000_000;

/// And across the whole message. Base64 makes the request body a third bigger
/// again, so this is really a ~10.7 MB POST through the gateway.
pub(crate) const MAX_TOTAL_BYTES: u64 = 8_000_000;

/// Longest side an attached image is resized to before it is sent. 1280px
/// keeps text in a screenshot readable, which is the thing most often
/// photographed for an agent, while turning a 12-megapixel camera file into
/// roughly 200 kB.
const IMAGE_EDGE: u32 = 1280;

/// Second try, for an image that is still over the cap at `IMAGE_EDGE` — a
/// panorama, or a screenshot of a very tall page.
const RETRY_EDGE: u32 = 800;

/// Longest side of the thumbnail kept in the transcript.
const THUMB_EDGE: u32 = 192;

/// The most base64 a transcript item will hold as a thumbnail.
///
/// A thumbnail this phone made is 4–8 kB, well inside it. The bound exists
/// for the *other* source: history replays an attachment at the size it was
/// sent, and adopting that would put the payload back into the structure this
/// module exists to keep it out of. Anything larger renders as a named chip.
const THUMB_MAX_CHARS: usize = 24_000;

/// Which composer a pick belongs to. The two keep separate trays for the same
/// reason they keep separate drafts: they are different screens, and a photo
/// picked for a code chat must not ride along on a goose message.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AttachTarget {
    Goose,
    Code,
}

impl AttachTarget {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Goose => "goose",
            Self::Code => "code",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "goose" => Some(Self::Goose),
            "code" => Some(Self::Code),
            _ => None,
        }
    }
}

/// What kind of thing an attachment is, as far as the two protocols care.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum AttachKind {
    Image,
    Text,
    Binary,
}

/// What the transcript remembers about one attachment.
///
/// Serde derives are for the Code tab's on-device cache, and `thumb` is
/// deliberately included: without it a chat re-opened from the cache would
/// show chips where it had shown photos a moment earlier. It stays small by
/// construction — see `THUMB_MAX_CHARS`.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Attachment {
    pub name: String,
    pub mime: String,
    /// Size of what was sent, in bytes. Zero when it is not known — a
    /// resource link names a file it does not carry.
    pub size: u64,
    /// Base64 JPEG thumbnail, or empty for anything without a picture.
    pub thumb: String,
}

impl Attachment {
    pub(crate) fn kind(&self) -> AttachKind {
        kind_of(&self.mime)
    }

    /// The size as a chip would say it, or nothing when there is no honest
    /// number to give.
    pub(crate) fn size_label(&self) -> String {
        if self.size == 0 {
            String::new()
        } else {
            format_bytes(self.size)
        }
    }

    /// What identifies this file across a history reload — see
    /// [`thumbnail_index`]. Name alone is not enough: every photo taken with
    /// the camera on iOS arrives called `image.jpg`. Mime is deliberately not
    /// part of it, because it is the one field that does not survive the
    /// round trip: a text file is sent declared `text/plain` whatever the
    /// picker called it (see [`code_parts`]).
    fn identity(&self) -> (String, u64) {
        (self.name.clone(), self.size)
    }
}

/// A file picked in the composer and not yet sent.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PendingAttachment {
    pub record: Attachment,
    /// Base64 of the bytes that will be sent.
    pub data: String,
    /// The decoded contents, for a text file. goose takes a text resource as
    /// text rather than as base64, and the browser has already had to decode
    /// the file to know it was text at all.
    pub text: Option<String>,
}

fn kind_of(mime: &str) -> AttachKind {
    if mime.starts_with("image/") {
        AttachKind::Image
    } else if mime.starts_with("text/") || mime == "application/json" || mime == "application/xml" {
        AttachKind::Text
    } else {
        AttachKind::Binary
    }
}

/// Bytes behind a base64 string, without decoding it. Every size this module
/// reports is derived here, so the number on a chip is the number that
/// travelled rather than the size of the file that was picked — which, for a
/// downscaled photo, are nothing like each other.
pub(crate) fn base64_len_to_bytes(data: &str) -> u64 {
    let padding = data.bytes().rev().take_while(|b| *b == b'=').count();
    let len = data.len().saturating_sub(padding) as u64;
    // Four base64 characters carry three bytes.
    len * 3 / 4
}

/// A byte count as a person reads it. Decimal units, matching iOS.
pub(crate) fn format_bytes(n: u64) -> String {
    #[expect(
        clippy::cast_precision_loss,
        reason = "attachment sizes are capped in the low megabytes, orders of \
                  magnitude below where f64 stops representing integers exactly"
    )]
    let bytes = n as f64;
    if n >= 1_000_000 {
        format!("{:.1} MB", bytes / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{} kB", n / 1_000)
    } else {
        format!("{n} B")
    }
}

// ------------------------------------------------------------- the picker

/// Open the sheet, read what was picked, downscale it, hand it back once.
///
/// The gesture is owned by JavaScript, exactly as the phone's swipe and pull
/// are and as the desktop's ⌘R is (`src/viewport.rs` says why at length, and
/// `src/shell/desktop/mod.rs` says it again for the chord): the native renderer round-trips
/// every listened-to event through a synchronous XHR, so by the time a Rust
/// `onclick` handler could call `document::eval` the user gesture is over —
/// and opening a file input outside one is the thing `WKWebView` refuses.
/// A capture-phase listener clicks the input inside the real event instead.
///
/// The bytes are read, resized and base64'd here for the same reason: one
/// message back, not one per chunk.
///
/// Which is also why *every* cap is substituted in, not just the per-file
/// one. The tray is still the authority — it is the side that knows what is
/// already in it — but a selection this side can already see the message will
/// never carry costs a base64 copy, two JSON encodings and a trip through the
/// bridge before Rust gets to say no. Bounding it here can only refuse files
/// [`accept`] would have refused anyway: the tray's headroom is never larger
/// than an empty tray's.
const PICK_FILES: &str = r#"
(() => {
  if (window.__attachWired) return;
  window.__attachWired = true;

  const MAX_BYTES = __MAX_FILE_BYTES__;
  const MAX_FILES = __MAX_ATTACHMENTS__;
  const MAX_TOTAL = __MAX_TOTAL_BYTES__;
  const EDGE = __IMAGE_EDGE__;
  const RETRY_EDGE = __RETRY_EDGE__;
  const THUMB_EDGE = __THUMB_EDGE__;
  const QUALITY = 0.72;
  const RETRY_QUALITY = 0.6;

  // A file the picker hands over with no type at all — common for Files
  // documents — is still worth taking if its name says what it is.
  const TEXTY = /\.(txt|text|md|markdown|rst|json|jsonl|ya?ml|toml|ini|cfg|conf|csv|tsv|log|patch|diff|lock|rs|ts|tsx|js|jsx|mjs|py|rb|go|java|kt|swift|c|h|cc|cpp|hpp|cs|php|sh|bash|zsh|sql|html|css|scss|xml)$/i;

  const kindOf = (file) => {
    const t = (file.type || '').toLowerCase();
    // SVG is markup, and markup that a canvas will happily rasterise into
    // something quite unlike the file the agent was asked about.
    if (t.startsWith('image/') && t !== 'image/svg+xml') return 'image';
    if (t.startsWith('text/') || t === 'application/json' || t === 'application/xml') return 'text';
    if (t === 'application/pdf') return 'binary';
    if (!t && TEXTY.test(file.name)) return 'text';
    return null;
  };

  const b64 = (buf) => {
    // Chunked: String.fromCharCode.apply with a multi-megabyte array
    // overflows the argument stack and throws.
    const view = new Uint8Array(buf);
    let out = '';
    for (let i = 0; i < view.length; i += 0x8000) {
      out += String.fromCharCode.apply(null, view.subarray(i, i + 0x8000));
    }
    return btoa(out);
  };

  const bytesOf = (data) => Math.floor(data.replace(/=+$/, '').length * 3 / 4);

  // Decoding is the step that can hang on a file the browser cannot make
  // sense of, so it is the step with a deadline. Falling out of it as a
  // failure is what turns a stall into a message.
  const decode = (file) => new Promise((resolve) => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    const finish = (value) => { clearTimeout(timer); resolve(value ? { img, url } : null); };
    const timer = setTimeout(() => { URL.revokeObjectURL(url); resolve(null); }, 15000);
    img.onload = () => finish(true);
    img.onerror = () => { URL.revokeObjectURL(url); finish(false); };
    img.src = url;
  });

  const render = (img, edge, quality) => {
    try {
      const scale = Math.min(1, edge / Math.max(img.width, img.height));
      const w = Math.max(1, Math.round(img.width * scale));
      const h = Math.max(1, Math.round(img.height * scale));
      const canvas = document.createElement('canvas');
      canvas.width = w;
      canvas.height = h;
      canvas.getContext('2d').drawImage(img, 0, 0, w, h);
      return canvas.toDataURL('image/jpeg', quality).split(',')[1] || null;
    } catch (err) {
      return null;
    }
  };

  const readImage = async (file) => {
    const decoded = await decode(file);
    if (!decoded) return { rejected: { name: file.name, reason: 'unreadable' } };
    try {
      let data = render(decoded.img, EDGE, QUALITY);
      if (data && bytesOf(data) > MAX_BYTES) {
        data = render(decoded.img, RETRY_EDGE, RETRY_QUALITY);
      }
      if (!data) return { rejected: { name: file.name, reason: 'unreadable' } };
      if (bytesOf(data) > MAX_BYTES) {
        return { rejected: { name: file.name, reason: 'too-big', bytes: bytesOf(data) } };
      }
      return {
        file: {
          name: file.name,
          mime: 'image/jpeg',
          data,
          thumb: render(decoded.img, THUMB_EDGE, RETRY_QUALITY) || '',
        },
      };
    } finally {
      URL.revokeObjectURL(decoded.url);
    }
  };

  const readBytes = async (file, kind) => {
    if (file.size > MAX_BYTES) {
      return { rejected: { name: file.name, reason: 'too-big', bytes: file.size } };
    }
    const buf = await file.arrayBuffer();
    const mime = kind === 'text' ? (file.type || 'text/plain') : file.type;
    return {
      file: {
        name: file.name,
        mime,
        data: b64(buf),
        thumb: '',
        text: kind === 'text' ? new TextDecoder('utf-8').decode(buf) : null,
      },
    };
  };

  const input = document.createElement('input');
  input.type = 'file';
  input.multiple = true;
  // No accept filter on purpose: with one, iOS narrows its sheet to the photo
  // routes and the Files route — the whole "or a file" half of this control —
  // stops being offered.
  input.style.cssText = 'position:fixed;left:-9999px;width:1px;height:1px;opacity:0';
  document.body.appendChild(input);

  let target = 'goose';
  let conversation = '';
  let seq = 0;

  document.addEventListener('click', (e) => {
    const btn = e.target.closest && e.target.closest('.attach');
    if (!btn) return;
    target = btn.dataset.attach || 'goose';
    // Which chat, not just which composer: the read finishes seconds later
    // and the tray it was picked for is emptied the moment you walk out of
    // the conversation.
    conversation = btn.dataset.conversation || '';
    // Picking the same file twice in a row is a real thing to do, and without
    // this the second pick changes nothing and fires no event.
    input.value = '';
    input.click();
  }, true);

  input.addEventListener('change', async () => {
    const chosen = [...input.files];
    if (!chosen.length) return;
    // Pinned for the whole read: this handler awaits, and a tap on the other
    // composer's button — or a walk into another chat — meanwhile would
    // otherwise redirect a pick already in progress into the wrong
    // conversation. The id is how two overlapping picks tell themselves
    // apart, so one finishing does not clear the other's progress row.
    const picked = target;
    const conv = conversation;
    const id = ++seq;
    // Reading and resizing several photos takes seconds. Say so, or the
    // composer simply sits there — but never promise to read more than one
    // message can carry.
    const reading = Math.min(chosen.length, MAX_FILES);
    dioxus.send(JSON.stringify({ pick: id, target: picked, conversation: conv, reading }));

    const files = [];
    const rejected = [];
    let total = 0;
    for (const file of chosen) {
      if (files.length >= MAX_FILES) {
        rejected.push({ name: file.name, reason: 'too-many' });
        continue;
      }
      const kind = kindOf(file);
      if (!kind) {
        rejected.push({ name: file.name, reason: 'unsupported', mime: file.type || '' });
        continue;
      }
      // Refused before it is read, where the weight is known up front. A
      // photo's is not — it weighs whatever the downscale makes it, usually a
      // twentieth of the file that was picked — so those are weighed after.
      if (kind !== 'image' && total + file.size > MAX_TOTAL) {
        rejected.push({ name: file.name, reason: 'too-heavy' });
        continue;
      }
      let result;
      try {
        result = kind === 'image' ? await readImage(file) : await readBytes(file, kind);
      } catch (err) {
        result = { rejected: { name: file.name, reason: 'unreadable' } };
      }
      if (result.rejected) {
        rejected.push(result.rejected);
        continue;
      }
      const weight = bytesOf(result.file.data);
      if (total + weight > MAX_TOTAL) {
        rejected.push({ name: file.name, reason: 'too-heavy' });
        continue;
      }
      total += weight;
      files.push(result.file);
    }
    dioxus.send(JSON.stringify({ pick: id, target: picked, conversation: conv, files, rejected }));
  });
})();
"#;

/// The picker script with this module's limits substituted in, so the numbers
/// the browser enforces and the numbers the messages quote cannot drift.
pub(crate) fn picker_js() -> String {
    PICK_FILES
        .replace("__MAX_FILE_BYTES__", &MAX_FILE_BYTES.to_string())
        .replace("__MAX_ATTACHMENTS__", &MAX_ATTACHMENTS.to_string())
        .replace("__MAX_TOTAL_BYTES__", &MAX_TOTAL_BYTES.to_string())
        .replace("__IMAGE_EDGE__", &IMAGE_EDGE.to_string())
        .replace("__RETRY_EDGE__", &RETRY_EDGE.to_string())
        .replace("__THUMB_EDGE__", &THUMB_EDGE.to_string())
}

/// One message from the picker: either "I am reading n files", or the result.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Picked {
    /// The browser's id for this pick, carried on both messages so a result
    /// can only ever end the read it belongs to.
    pick: u64,
    target: String,
    /// The conversation the button was tapped in — see [`conversation_key`].
    conversation: String,
    reading: Option<usize>,
    files: Vec<PickedFile>,
    rejected: Vec<PickedRejection>,
}

/// A pick the browser is still reading.
///
/// A list rather than a slot, because two can be in flight at once: the
/// change handler awaits every file and nothing stops a second tap while it
/// does. With one slot the first result to arrive cleared the second pick's
/// progress row, and the composer went quiet for the rest of its read.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Pick {
    id: u64,
    target: AttachTarget,
    conversation: String,
    count: usize,
}

/// Fold one message from the picker into the reads still in flight.
///
/// Keyed on the pick's own id, so a result ends its own read and no other.
fn track(
    picks: &mut Vec<Pick>,
    id: u64,
    target: AttachTarget,
    conversation: &str,
    reading: Option<usize>,
) {
    match reading {
        Some(count) if count > 0 => picks.push(Pick {
            id,
            target,
            conversation: conversation.to_owned(),
            count,
        }),
        // A pick of nothing at all never opened a row to close.
        Some(_) => {}
        None => picks.retain(|pick| pick.id != id),
    }
}

/// How many files a composer should say it is reading.
///
/// Summed over the picks made in *this* conversation: two overlapping picks
/// are one honest count, and one made in a chat you have since left says
/// nothing here — the tray it is going to land in is not this one.
pub(crate) fn reading_for(picks: &[Pick], target: AttachTarget, conversation: &str) -> usize {
    picks
        .iter()
        .filter(|pick| pick.target == target && pick.conversation == conversation)
        .map(|pick| pick.count)
        .sum()
}

/// The conversation a composer's picks belong to.
///
/// `AttachTarget` says which composer; this says which chat inside it, and
/// the two are not the same thing. A read takes seconds, `open_session`
/// empties the tray the moment you walk into another session, and a pick
/// bound only to "goose" then lands in whichever session is open when it
/// finishes — carrying a photo picked for one conversation into the next
/// message sent in another.
///
/// The Code tab keys on the chat id and not on `code_epoch`, matching
/// `open_code_chat`: re-opening the same chat deliberately keeps its tray, so
/// an epoch would throw away a pick that is still for the chat on screen.
pub(crate) fn conversation_key(ctx: &AppCtx, target: AttachTarget) -> String {
    match target {
        AttachTarget::Goose => ctx.chat.peek().session_id.clone().unwrap_or_default(),
        // The new-session composer has no chat of its own yet, and `code_chat`
        // still holds whichever one you last visited — so without this a photo
        // picked for a session that does not exist is either dropped as
        // belonging to another conversation, or worse, accepted into it. It is
        // its own conversation until it becomes one, which is also why
        // `tray_of` gives it a tray of its own: this decides which picks are
        // accepted, and that decides where the accepted ones sit.
        AttachTarget::Code if *ctx.code_screen.peek() == crate::code::CodeScreen::New => {
            crate::code::NEW_CONVERSATION.to_owned()
        }
        AttachTarget::Code => ctx.code_chat.peek().chat_id.clone().unwrap_or_default(),
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PickedFile {
    name: String,
    mime: String,
    data: String,
    thumb: String,
    text: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PickedRejection {
    name: String,
    reason: String,
    bytes: u64,
    mime: String,
}

/// Fold one message from the picker into the composer it came from.
pub(crate) fn receive(ctx: &AppCtx, payload: &str) {
    let Ok(msg) = serde_json::from_str::<Picked>(payload) else {
        show_toast(ctx, "The file picker sent something this app cannot read");
        return;
    };
    let Some(target) = AttachTarget::parse(&msg.target) else {
        return;
    };

    track(
        &mut ctx.attach_reading.clone().write(),
        msg.pick,
        target,
        &msg.conversation,
        msg.reading,
    );
    if msg.reading.is_some() {
        return;
    }

    // The tray this was picked for is not necessarily the one on screen: the
    // read takes seconds and walking into another chat empties the tray as it
    // goes. Landing the files anyway would put a photo picked in one
    // conversation on the next message sent in another.
    if conversation_key(ctx, target) != msg.conversation {
        if !msg.files.is_empty() {
            show_toast(ctx, "Not attached: the chat they were picked in is closed");
        }
        return;
    }

    let mut refused: Vec<String> = msg.rejected.iter().map(refusal).collect();
    let picked: Vec<PendingAttachment> =
        msg.files.into_iter().map(PendingAttachment::from).collect();
    {
        let mut tray = tray_of(ctx, target, &msg.conversation);
        let mut held = tray.write();
        refused.extend(accept(&mut held, picked));
    }
    if let Some(message) = refusal_summary(&refused) {
        show_toast(ctx, message);
    }
}

impl From<PickedFile> for PendingAttachment {
    fn from(file: PickedFile) -> Self {
        let size = base64_len_to_bytes(&file.data);
        Self {
            record: Attachment {
                name: file.name,
                mime: file.mime,
                size,
                thumb: file.thumb,
            },
            data: file.data,
            text: file.text,
        }
    }
}

/// Which tray a pick belongs in.
///
/// Keyed by conversation and not by target alone, for the reason
/// [`conversation_key`] exists: the Code tab has two composers and only one of
/// them belongs to a chat. A pick made on the new-session screen is for a
/// session that does not exist yet, so it cannot share a Vec with the last
/// chat you had open — that Vec is rendered by that chat's tray and lifted
/// into its next message. `conversation_key` gates which picks are *accepted*;
/// this is what keeps the ones already accepted apart.
pub(crate) fn tray_of(
    ctx: &AppCtx,
    target: AttachTarget,
    conversation: &str,
) -> Signal<Vec<PendingAttachment>> {
    match target {
        AttachTarget::Goose => ctx.attachments,
        AttachTarget::Code if conversation == crate::code::NEW_CONVERSATION => ctx.new_attachments,
        AttachTarget::Code => ctx.code_attachments,
    }
}

/// Why a file past the count cap was turned away. Shared, because both sides
/// enforce that cap now — the browser so a selection the message can never
/// carry is not read and shipped first, the tray because it is the only side
/// that knows what is already in it — and a reader should not be able to tell
/// which one refused.
fn too_many(name: &str) -> String {
    format!("{name} — one message carries at most {MAX_ATTACHMENTS} attachments")
}

/// And past the whole-message cap.
fn too_heavy(name: &str) -> String {
    format!(
        "{name} — one message carries at most {} in total",
        format_bytes(MAX_TOTAL_BYTES)
    )
}

/// Take what fits and say, by name, why the rest did not.
///
/// The last word on every cap: the browser applies the same ones to a single
/// pick, but only this side knows what the tray already holds.
fn accept(tray: &mut Vec<PendingAttachment>, picked: Vec<PendingAttachment>) -> Vec<String> {
    let mut refused = Vec::new();
    for file in picked {
        let name = &file.record.name;
        if tray.len() >= MAX_ATTACHMENTS {
            refused.push(too_many(name));
            continue;
        }
        if file.record.size > MAX_FILE_BYTES {
            refused.push(format!(
                "{name} is {} — the limit is {} a file",
                format_bytes(file.record.size),
                format_bytes(MAX_FILE_BYTES)
            ));
            continue;
        }
        let total = tray
            .iter()
            .map(|a| a.record.size)
            .sum::<u64>()
            .saturating_add(file.record.size);
        if total > MAX_TOTAL_BYTES {
            refused.push(too_heavy(name));
            continue;
        }
        tray.push(file);
    }
    refused
}

fn refusal(rejection: &PickedRejection) -> String {
    let name = &rejection.name;
    match rejection.reason.as_str() {
        "too-big" => format!(
            "{name} is {} — the limit is {} a file",
            format_bytes(rejection.bytes),
            format_bytes(MAX_FILE_BYTES)
        ),
        "unsupported" => {
            let what = if rejection.mime.is_empty() {
                "an unrecognised type".to_owned()
            } else {
                rejection.mime.clone()
            };
            format!("{name} is {what} — only images, text files and PDFs can be attached")
        }
        "unreadable" => format!("{name} could not be read"),
        "too-many" => too_many(name),
        "too-heavy" => too_heavy(name),
        other => format!("{name} — {other}"),
    }
}

/// One toast for a whole pick. Naming the first refusal and counting the rest
/// keeps the message short without hiding that more than one thing failed.
fn refusal_summary(refused: &[String]) -> Option<String> {
    match refused {
        [] => None,
        [only] => Some(format!("Not attached: {only}")),
        [first, rest @ ..] => Some(format!("Not attached: {first} (and {} more)", rest.len())),
    }
}

// ------------------------------------------------------- what gets sent

/// The ACP prompt array for a message: the text, then its attachments.
///
/// Text first because that is the message and the attachments are what it is
/// about — and because the transcript, the mock server and `OpenCode`'s own
/// history all read the first text part as the thing the user said.
pub(crate) fn goose_blocks(text: &str, files: &[PendingAttachment]) -> Vec<ContentBlock> {
    let mut blocks = Vec::with_capacity(files.len() + 1);
    if !text.is_empty() {
        blocks.push(ContentBlock::text(text));
    }
    for file in files {
        // Not a `file://` URI: the file is on this phone and the agent is not,
        // so a path-shaped name is an invitation to try opening one and report
        // that it does not exist.
        let uri = format!("attachment:{}", file.record.name);
        blocks.push(match file.record.kind() {
            AttachKind::Image => ContentBlock::image(&file.data, &file.record.mime),
            AttachKind::Text => ContentBlock::resource_text(
                &uri,
                &file.record.mime,
                file.text.as_deref().unwrap_or_default(),
            ),
            AttachKind::Binary => ContentBlock::resource_blob(&uri, &file.record.mime, &file.data),
        });
    }
    blocks
}

/// The `OpenCode` prompt parts for a message.
pub(crate) fn code_parts(text: &str, files: &[PendingAttachment]) -> Vec<PromptPart> {
    let mut parts = Vec::with_capacity(files.len() + 1);
    if !text.is_empty() {
        parts.push(PromptPart::text(text));
    }
    for file in files {
        parts.push(match file.record.kind() {
            // `text_file` and not `file`: the server inlines a data URL whose
            // mime is exactly `text/plain` and passes anything else through as
            // an opaque attachment, so a `.md` sent as `text/markdown` reaches
            // a model that cannot open it.
            AttachKind::Text => PromptPart::text_file(&file.record.name, &file.data),
            _ => PromptPart::file(&file.record.mime, &file.record.name, &file.data),
        });
    }
    parts
}

/// What the transcript keeps once a message has been sent.
pub(crate) fn records(files: &[PendingAttachment]) -> Vec<Attachment> {
    files.iter().map(|f| f.record.clone()).collect()
}

// -------------------------------------------------- what comes back again

/// An attachment as it arrives back from goose, in a replayed user message.
///
/// goose replays the image at the size it was sent, which is exactly what
/// this module keeps out of the transcript — so the payload is adopted as a
/// thumbnail only when it is already thumbnail-sized, and otherwise the
/// attachment renders as a named chip.
pub(crate) fn from_content_block(block: &ContentBlock) -> Attachment {
    match block {
        ContentBlock::Image {
            data,
            mime_type,
            uri,
            ..
        } => Attachment {
            name: uri.clone().unwrap_or_else(|| "Image".to_owned()),
            mime: mime_type.clone(),
            size: base64_len_to_bytes(data),
            thumb: small_thumb(data),
        },
        ContentBlock::Audio {
            mime_type, data, ..
        } => Attachment {
            name: "Audio".to_owned(),
            mime: mime_type.clone(),
            size: base64_len_to_bytes(data),
            thumb: String::new(),
        },
        ContentBlock::ResourceLink { uri, name, .. } => Attachment {
            name: name.clone().unwrap_or_else(|| uri.clone()),
            mime: String::new(),
            size: 0,
            thumb: String::new(),
        },
        ContentBlock::Resource { resource, .. } => {
            let field = |key: &str| {
                resource
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let blob = field("blob").unwrap_or_default();
            let size = field("text").map_or_else(
                || base64_len_to_bytes(&blob),
                |text| text.len().try_into().unwrap_or(u64::MAX),
            );
            Attachment {
                name: display_name(&field("uri").unwrap_or_default()),
                mime: field("mimeType").unwrap_or_default(),
                size,
                thumb: String::new(),
            }
        }
        ContentBlock::Text { .. } => Attachment::default(),
    }
}

/// An attachment as it arrives back from `OpenCode`, as a `file` part.
pub(crate) fn from_part(part: &Part) -> Attachment {
    let data = part.data_url_base64().unwrap_or_default();
    let mime = part.mime.clone().unwrap_or_default();
    let is_image = mime.starts_with("image/");
    Attachment {
        name: part
            .filename
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| display_name(part.url.as_deref().unwrap_or_default())),
        mime,
        size: base64_len_to_bytes(data),
        thumb: if is_image {
            small_thumb(data)
        } else {
            String::new()
        },
    }
}

/// The payload, but only if it is small enough to be a thumbnail.
fn small_thumb(data: &str) -> String {
    if data.len() <= THUMB_MAX_CHARS {
        data.to_owned()
    } else {
        String::new()
    }
}

/// The last path-ish segment of a URI, for something whose only name is where
/// it came from. A `data:` URL has no name at all, so it gets a word.
fn display_name(uri: &str) -> String {
    if uri.is_empty() || uri.starts_with("data:") {
        return "Attachment".to_owned();
    }
    uri.rsplit(['/', ':']).find(|s| !s.is_empty()).map_or_else(
        || uri.to_owned(),
        |segment| segment.trim_start_matches("//").to_owned(),
    )
}

/// Thumbnails already on screen, keyed by what identifies the file.
///
/// History is authoritative and replaces the transcript wholesale, but it
/// replaces a thumbnail this phone made with a payload too big to keep — so
/// the attachment would silently turn from a photo into a chip a second after
/// the chat opened. This carries them across instead. `OpenCode` only: goose
/// replays too little to key on, which is what [`sent_attachments`] is for.
pub(crate) fn thumbnail_index(items: &[ChatItem]) -> HashMap<(String, u64), String> {
    let mut out = HashMap::new();
    for item in items {
        if let ChatItem::User { attachments, .. } = item {
            for attachment in attachments {
                if !attachment.thumb.is_empty() {
                    out.insert(attachment.identity(), attachment.thumb.clone());
                }
            }
        }
    }
    out
}

/// Put those thumbnails back onto a freshly folded transcript.
pub(crate) fn restore_thumbnails(thumbs: &HashMap<(String, u64), String>, items: &mut [ChatItem]) {
    if thumbs.is_empty() {
        return;
    }
    for item in items {
        if let ChatItem::User { attachments, .. } = item {
            for attachment in attachments.iter_mut() {
                if attachment.thumb.is_empty() {
                    if let Some(thumb) = thumbs.get(&attachment.identity()) {
                        attachment.thumb.clone_from(thumb);
                    }
                }
            }
        }
    }
}

/// What the goose transcript knows about the files it sent, in the order it
/// shows them, for the replay that is about to overwrite it.
///
/// The Code tab can match a replayed attachment to the one it replaces by
/// name and size, because `OpenCode` echoes a file part back with the
/// filename it was given. goose cannot: an ACP image block carries its bytes
/// and its mime type and nothing else — `ContentBlock::image` leaves `uri`
/// off deliberately, since the agent cannot resolve a path on this phone — so
/// a replayed photo arrives nameless and at a size this module will not keep.
/// Without carrying this across, a reconnect turned every photo in the
/// transcript into a grey chip called "Image".
///
/// Only the ones with a picture: a text file or a PDF replays with its `uri`
/// intact and comes back correctly named on its own.
pub(crate) fn sent_attachments(items: &[ChatItem]) -> Vec<Attachment> {
    items
        .iter()
        .filter_map(|item| match item {
            ChatItem::User { attachments, .. } => Some(attachments),
            _ => None,
        })
        .flatten()
        .filter(|a| !a.thumb.is_empty())
        .cloned()
        .collect()
}

/// Give a replayed attachment back the name and the picture it was sent with.
///
/// Matched on type and weight, which is all goose replays, and consumed on
/// use: two photos of the same size in one conversation then keep their own
/// thumbnails, because the replay walks the transcript in the order this list
/// was taken from.
pub(crate) fn adopt_sent(sent: &mut Vec<Attachment>, record: &mut Attachment) {
    let Some(index) = sent
        .iter()
        .position(|a| a.mime == record.mime && a.size == record.size)
    else {
        return;
    };
    let known = sent.remove(index);
    record.name = known.name;
    record.thumb = known.thumb;
}

// ------------------------------------------------------- a send that failed

/// Put a failed message's attachments back in the composer they were picked
/// in, and say what became of them.
///
/// The transcript is no help here: the bubble keeps the words but only a
/// thumbnail of the files, and the bytes lived in the tray that was emptied
/// when the message went out. Anything picked while the request was in flight
/// stays where it is and keeps its place — it is what the reader is looking
/// at — so the caps decide how much of the failed message fits back in.
///
/// A prompt can take tens of seconds to fail, which is long enough to have
/// opened something else, so `conversation` is the chat the message went out
/// in and the files go back only while that is still the one on screen. The
/// alternative is what [`conversation_key`] exists to prevent: one
/// conversation's photo riding out on another's next message.
pub(crate) fn return_to_tray(
    ctx: &AppCtx,
    target: AttachTarget,
    conversation: &str,
    files: Vec<PendingAttachment>,
) -> String {
    let wanted = files.len();
    if wanted == 0 {
        return String::new();
    }
    if conversation_key(ctx, target) != conversation {
        return " — that chat is no longer open, so its attachments were not put back".to_owned();
    }
    let refused = {
        let mut tray = tray_of(ctx, target, conversation);
        let mut held = tray.write();
        accept(&mut held, files).len()
    };
    restored_note(wanted - refused, wanted)
}

/// The clause a failed send's toast adds about the files it was carrying.
/// One toast, not two: the second would erase the first, and the reason and
/// the recovery are both things the reader needs.
fn restored_note(restored: usize, wanted: usize) -> String {
    match (restored, wanted) {
        (_, 0) => String::new(),
        (0, _) => " — the composer is full, so its attachments were not put back".to_owned(),
        (n, w) if n == w => " — its attachments are back in the composer".to_owned(),
        (n, w) => format!(" — {n} of {w} attachments are back in the composer"),
    }
}

#[cfg(test)]
#[expect(
    clippy::unwrap_used,
    reason = "test assertions: a fixture that does not serialise is the failing check"
)]
mod tests {
    use super::{
        accept, adopt_sent, base64_len_to_bytes, code_parts, display_name, format_bytes,
        from_content_block, from_part, goose_blocks, picker_js, reading_for, refusal,
        refusal_summary, restore_thumbnails, restored_note, sent_attachments, thumbnail_index,
        track, AttachTarget, Attachment, ChatItem, PendingAttachment, Picked, PickedRejection,
        MAX_ATTACHMENTS, MAX_FILE_BYTES, MAX_TOTAL_BYTES, THUMB_MAX_CHARS,
    };
    use goose_acp_client::ContentBlock;
    use opencode_client::Part;

    fn pending(name: &str, mime: &str, size: u64) -> PendingAttachment {
        PendingAttachment {
            record: Attachment {
                name: name.to_owned(),
                mime: mime.to_owned(),
                size,
                thumb: "THUMB".to_owned(),
            },
            data: "QUJD".to_owned(),
            text: Some("hello".to_owned()),
        }
    }

    /// The limits the browser enforces are the limits the messages quote, and
    /// they get there by substitution — a token that stops matching would
    /// otherwise ship as a `ReferenceError` inside a listener nobody watches.
    #[test]
    fn the_picker_script_carries_no_unfilled_placeholders() {
        let js = picker_js();
        assert!(
            !js.replace("__attachWired", "").contains("__"),
            "a __TOKEN__ in PICK_FILES was not substituted"
        );
        assert!(js.contains(&MAX_FILE_BYTES.to_string()));
    }

    /// Every cap reaches the browser, and every one that reaches it is used.
    ///
    /// A pick is read, downscaled, base64'd and JSON-encoded twice before it
    /// crosses the bridge, so a cap the script does not know is a cap paid for
    /// in full and then refused on this side: six files at the per-file limit
    /// is a 24 MB crossing — 56 MB for text, which travels decoded as well —
    /// to enforce an 8 MB rule.
    #[test]
    fn the_picker_script_enforces_every_cap_and_not_just_the_per_file_one() {
        let js = picker_js();
        for (name, value) in [
            ("MAX_FILES", MAX_ATTACHMENTS.to_string()),
            ("MAX_TOTAL", MAX_TOTAL_BYTES.to_string()),
        ] {
            assert!(
                js.contains(&format!("{name} = {value}")),
                "{name} never reaches the browser"
            );
            // Declared is not enforced: it has to be read somewhere too.
            assert!(
                js.matches(name).count() > 1,
                "{name} is declared and never used"
            );
        }
    }

    /// A cap refused in the browser has to read exactly like the same cap
    /// refused by the tray. They are one rule enforced twice, and a reason
    /// name the Rust side does not know falls through to the raw string —
    /// "notes.md — too-many".
    #[test]
    fn a_cap_refused_in_the_browser_reads_like_one_refused_by_the_tray() {
        let browser = |reason: &str| {
            refusal(&PickedRejection {
                name: "notes.md".to_owned(),
                reason: reason.to_owned(),
                ..PickedRejection::default()
            })
        };

        let mut full: Vec<PendingAttachment> = (0..MAX_ATTACHMENTS)
            .map(|i| pending(&format!("f{i}.txt"), "text/plain", 10))
            .collect();
        let tray = accept(&mut full, vec![pending("notes.md", "text/plain", 10)]);
        assert_eq!(tray, [browser("too-many")]);

        let mut heavy = vec![pending("big.pdf", "application/pdf", MAX_TOTAL_BYTES - 10)];
        let tray = accept(&mut heavy, vec![pending("notes.md", "text/plain", 100)]);
        assert_eq!(tray, [browser("too-heavy")]);

        assert!(
            !browser("too-many").contains("too-many"),
            "the reason name leaked into the toast"
        );
    }

    /// Two picks can be in flight at once — the change handler awaits every
    /// file, and nothing stops a second tap while it does — so a result has
    /// to end its own read and no other.
    #[test]
    fn one_pick_finishing_does_not_cancel_another_that_is_still_reading() {
        let mut picks = Vec::new();
        track(&mut picks, 1, AttachTarget::Code, "chat-1", Some(5));
        track(&mut picks, 2, AttachTarget::Goose, "sess-1", Some(1));
        assert_eq!(reading_for(&picks, AttachTarget::Goose, "sess-1"), 1);

        track(&mut picks, 1, AttachTarget::Code, "chat-1", None);
        assert_eq!(reading_for(&picks, AttachTarget::Code, "chat-1"), 0);
        assert_eq!(
            reading_for(&picks, AttachTarget::Goose, "sess-1"),
            1,
            "the other composer went quiet for the rest of its read"
        );
    }

    /// And a read belongs to the conversation it was started in, not just to
    /// the composer: walking into another session empties that tray, and a
    /// pick still running would land in it.
    #[test]
    fn a_read_is_only_announced_in_the_conversation_it_was_started_in() {
        let mut picks = Vec::new();
        track(&mut picks, 1, AttachTarget::Goose, "sess-a", Some(3));
        assert_eq!(reading_for(&picks, AttachTarget::Goose, "sess-b"), 0);
        assert_eq!(reading_for(&picks, AttachTarget::Code, "sess-a"), 0);

        // Two picks in one composer are one honest count, not the second
        // hiding the first.
        track(&mut picks, 2, AttachTarget::Goose, "sess-a", Some(2));
        assert_eq!(reading_for(&picks, AttachTarget::Goose, "sess-a"), 5);

        // A pick of nothing opens no row, so nothing is left to close.
        track(&mut picks, 3, AttachTarget::Goose, "sess-a", Some(0));
        assert_eq!(picks.len(), 2);
    }

    /// The contract with `PICK_FILES`, captured from a real run of it against
    /// a photo, a markdown file, a video and an oversized PDF. Rename a key on
    /// either side and this is what says so.
    #[test]
    fn a_picked_payload_decodes_the_way_the_script_writes_it() {
        // Doubled hashes: the markdown heading in the fixture is a literal
        // `"#`, which closes an r#"…"# string.
        let raw = r##"{"pick":7,"target":"code","conversation":"chat-1",
            "files":[
              {"name":"IMG_0042.jpg","mime":"image/jpeg","data":"QUJDRA==","thumb":"QUJD","text":null},
              {"name":"notes.md","mime":"text/markdown","data":"QUJD","thumb":"","text":"# notes"}],
            "rejected":[
              {"name":"clip.mov","reason":"unsupported","mime":"video/quicktime"},
              {"name":"huge.pdf","reason":"too-big","bytes":5000009}]}"##;
        let msg: Picked = serde_json::from_str(raw).unwrap();
        assert_eq!(msg.target, "code");
        assert_eq!(msg.reading, None);
        // Which pick, and which chat it was made in. Without the second, a
        // result lands in whatever conversation is open when it arrives.
        assert_eq!(msg.pick, 7);
        assert_eq!(msg.conversation, "chat-1");

        let picked: Vec<PendingAttachment> =
            msg.files.into_iter().map(PendingAttachment::from).collect();
        assert_eq!(picked[0].record.size, 4, "size comes from the payload");
        assert_eq!(picked[0].record.thumb, "QUJD");
        assert_eq!(picked[1].text.as_deref(), Some("# notes"));

        let refused: Vec<String> = msg.rejected.iter().map(refusal).collect();
        assert!(
            refused[0].contains("video/quicktime"),
            "got: {}",
            refused[0]
        );
        assert!(refused[1].contains("5.0 MB"), "got: {}", refused[1]);
    }

    #[test]
    fn base64_length_gives_the_byte_count_back() {
        assert_eq!(base64_len_to_bytes("QUJD"), 3); // "ABC"
        assert_eq!(base64_len_to_bytes("QUJDRA=="), 4); // "ABCD"
        assert_eq!(base64_len_to_bytes(""), 0);
    }

    #[test]
    fn sizes_read_the_way_the_phone_writes_them() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(4_096), "4 kB");
        assert_eq!(format_bytes(4_000_000), "4.0 MB");
    }

    /// The caps that depend on what is already in the tray are this side's to
    /// enforce, and every refusal has to name the file it is about.
    #[test]
    fn the_tray_fills_up_and_says_what_it_turned_away() {
        let mut tray = Vec::new();
        let picked: Vec<PendingAttachment> = (0..MAX_ATTACHMENTS + 2)
            .map(|i| pending(&format!("f{i}.txt"), "text/plain", 10))
            .collect();
        let refused = accept(&mut tray, picked);
        assert_eq!(tray.len(), MAX_ATTACHMENTS);
        assert_eq!(refused.len(), 2);
        assert!(refused[0].starts_with("f6.txt —"), "got: {}", refused[0]);
    }

    #[test]
    fn a_message_that_would_be_too_heavy_altogether_is_refused_by_name() {
        let mut tray = vec![pending("big.pdf", "application/pdf", MAX_TOTAL_BYTES - 10)];
        let refused = accept(&mut tray, vec![pending("more.pdf", "application/pdf", 100)]);
        assert_eq!(tray.len(), 1, "the second file must not be taken");
        assert_eq!(
            refused,
            ["more.pdf — one message carries at most 8.0 MB in total"]
        );
    }

    #[test]
    fn every_refusal_reason_becomes_something_a_person_can_act_on() {
        let reject = |reason: &str, bytes: u64, mime: &str| {
            refusal(&PickedRejection {
                name: "clip.mov".to_owned(),
                reason: reason.to_owned(),
                bytes,
                mime: mime.to_owned(),
            })
        };
        assert_eq!(
            reject("too-big", 9_000_000, ""),
            "clip.mov is 9.0 MB — the limit is 4.0 MB a file"
        );
        assert_eq!(
            reject("unsupported", 0, "video/quicktime"),
            "clip.mov is video/quicktime — only images, text files and PDFs can be attached"
        );
        assert_eq!(reject("unreadable", 0, ""), "clip.mov could not be read");
        assert!(reject("unsupported", 0, "").contains("an unrecognised type"));
    }

    #[test]
    fn one_toast_names_the_first_failure_and_counts_the_rest() {
        assert_eq!(refusal_summary(&[]), None);
        assert_eq!(
            refusal_summary(&["a is big".to_owned()]).as_deref(),
            Some("Not attached: a is big")
        );
        assert_eq!(
            refusal_summary(&["a is big".to_owned(), "b too".to_owned(), "c".to_owned()])
                .as_deref(),
            Some("Not attached: a is big (and 2 more)")
        );
    }

    /// Each backend gets the encoding it can actually read: an image inline,
    /// a text file as text, and anything else as bytes with a name.
    #[test]
    fn each_kind_is_encoded_the_way_its_backend_takes_it() {
        let files = vec![
            pending("shot.jpg", "image/jpeg", 100),
            pending("notes.md", "text/markdown", 20),
            pending("spec.pdf", "application/pdf", 30),
        ];
        let blocks = goose_blocks("look", &files);
        assert_eq!(
            serde_json::to_value(&blocks).unwrap(),
            serde_json::json!([
                {"type": "text", "text": "look"},
                {"type": "image", "data": "QUJD", "mimeType": "image/jpeg"},
                {"type": "resource", "resource": {
                    "uri": "attachment:notes.md", "mimeType": "text/markdown", "text": "hello"}},
                {"type": "resource", "resource": {
                    "uri": "attachment:spec.pdf", "mimeType": "application/pdf", "blob": "QUJD"}},
            ])
        );

        assert_eq!(
            serde_json::to_value(code_parts("look", &files)).unwrap(),
            serde_json::json!([
                {"type": "text", "text": "look"},
                {"type": "file", "mime": "image/jpeg", "filename": "shot.jpg",
                 "url": "data:image/jpeg;base64,QUJD"},
                {"type": "file", "mime": "text/plain", "filename": "notes.md",
                 "url": "data:text/plain;base64,QUJD"},
                {"type": "file", "mime": "application/pdf", "filename": "spec.pdf",
                 "url": "data:application/pdf;base64,QUJD"},
            ])
        );
    }

    /// A message can be attachments alone, and an empty text block is not a
    /// message — ACP would carry a blank turn and `OpenCode` records one.
    #[test]
    fn attachments_alone_send_no_empty_text_block() {
        let files = vec![pending("shot.jpg", "image/jpeg", 100)];
        assert_eq!(goose_blocks("", &files).len(), 1);
        assert_eq!(code_parts("", &files).len(), 1);
    }

    /// Anything replayed at full size is a payload, not a thumbnail: keeping
    /// it would put the bytes back into the structure the chat clones on
    /// every keystroke.
    #[test]
    fn a_replayed_payload_is_only_kept_when_it_is_thumbnail_sized() {
        let small = ContentBlock::image("Q".repeat(100), "image/jpeg");
        assert_eq!(from_content_block(&small).thumb.len(), 100);

        let big = ContentBlock::image("Q".repeat(THUMB_MAX_CHARS + 1), "image/jpeg");
        let record = from_content_block(&big);
        assert!(record.thumb.is_empty(), "a full-size image is not a thumb");
        assert!(record.size > 0, "but it still reports what it weighs");
    }

    #[test]
    fn a_replayed_resource_keeps_its_name_and_type() {
        let block = ContentBlock::resource_text("attachment:notes.md", "text/markdown", "hello");
        let record = from_content_block(&block);
        assert_eq!(record.name, "notes.md");
        assert_eq!(record.mime, "text/markdown");
        assert_eq!(record.size, 5);
        assert_eq!(display_name("data:image/png;base64,QQ=="), "Attachment");
        assert_eq!(display_name("file:///a/b/c.png"), "c.png");
    }

    #[test]
    fn an_opencode_file_part_becomes_an_attachment() {
        let part = Part {
            kind: "file".to_owned(),
            mime: Some("image/jpeg".to_owned()),
            filename: Some("IMG_0042.jpg".to_owned()),
            url: Some("data:image/jpeg;base64,QUJD".to_owned()),
            ..Part::default()
        };
        let record = from_part(&part);
        assert_eq!(record.name, "IMG_0042.jpg");
        assert_eq!(record.size, 3);
        assert_eq!(record.thumb, "QUJD");
    }

    /// Two photos taken with the camera are both called `image.jpg`, so the
    /// name alone cannot decide which thumbnail belongs to which.
    #[test]
    fn thumbnails_survive_a_history_reload_without_being_swapped() {
        let attachment = |size: u64, thumb: &str| Attachment {
            name: "image.jpg".to_owned(),
            mime: "image/jpeg".to_owned(),
            size,
            thumb: thumb.to_owned(),
        };
        let before = vec![ChatItem::User {
            text: "these two".to_owned(),
            attachments: vec![attachment(100, "FIRST"), attachment(200, "SECOND")],
        }];
        let index = thumbnail_index(&before);

        let mut after = vec![ChatItem::User {
            text: "these two".to_owned(),
            attachments: vec![attachment(200, ""), attachment(100, "")],
        }];
        restore_thumbnails(&index, &mut after);
        let ChatItem::User { attachments, .. } = &after[0] else {
            unreachable!()
        };
        assert_eq!(attachments[0].thumb, "SECOND");
        assert_eq!(attachments[1].thumb, "FIRST");
    }

    /// goose replays an image as bytes and a mime type and nothing else: no
    /// name, and at the size it was sent, which is far too big to keep. The
    /// transcript the replay is about to overwrite is the only thing left
    /// that knows what the photo was called and what it looked like.
    #[test]
    fn a_replayed_photo_takes_back_the_name_and_picture_it_was_sent_with() {
        let photo = |name: &str, thumb: &str| Attachment {
            name: name.to_owned(),
            mime: "image/jpeg".to_owned(),
            // Two photos of exactly the same weight: the hard case, because
            // the weight is half of all the replay gives us to match on.
            size: 18_003,
            thumb: thumb.to_owned(),
        };
        let before = vec![ChatItem::User {
            text: "these two, and the spec".to_owned(),
            attachments: vec![
                photo("IMG_0042.jpg", "FIRST"),
                photo("IMG_0043.jpg", "SECOND"),
                Attachment {
                    name: "spec.pdf".to_owned(),
                    mime: "application/pdf".to_owned(),
                    size: 18_003,
                    thumb: String::new(),
                },
            ],
        }];
        let mut sent = sent_attachments(&before);
        assert_eq!(
            sent.len(),
            2,
            "a resource replays with its uri, so it needs no help"
        );

        let block = ContentBlock::image("Q".repeat(THUMB_MAX_CHARS + 4), "image/jpeg");
        let mut first = from_content_block(&block);
        assert_eq!(first.name, "Image", "which is all the replay itself knows");
        assert!(first.thumb.is_empty());

        adopt_sent(&mut sent, &mut first);
        assert_eq!(first.name, "IMG_0042.jpg");
        assert_eq!(first.thumb, "FIRST");

        let mut second = from_content_block(&block);
        adopt_sent(&mut sent, &mut second);
        assert_eq!(
            second.thumb, "SECOND",
            "a match is consumed, so the second photo is not given the first's"
        );
    }

    /// A send that fails on the wire has to say two things at once — why, and
    /// what became of the files it was carrying. Two toasts would be one:
    /// there is a single slot and the second erases the first.
    #[test]
    fn a_failed_send_says_what_became_of_its_attachments() {
        assert_eq!(restored_note(0, 0), "", "a message with no files says less");
        assert_eq!(
            restored_note(2, 2),
            " — its attachments are back in the composer"
        );
        assert_eq!(
            restored_note(1, 3),
            " — 1 of 3 attachments are back in the composer"
        );
        assert!(
            restored_note(0, 2).contains("not put back"),
            "a full tray keeps what the reader picked since, and says so"
        );
    }
}
