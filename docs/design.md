# Design guide

What this app is trying to look like, and the rules that get it there. If you
are about to change `assets/main.css`, read this first — most of the values in
that file are consequences of the rules below rather than free choices.

## Where the look comes from

Three sources, doing different jobs.

**The reading voice** is the Claude iOS app: agent prose set in a serif, on a
page with no panel around it, under chrome that floats rather than sits in a
bar. That app is the reference for what a long agent reply should feel like to
read on a phone.

**The shell** follows current iOS. Since iOS 26 ("Liquid Glass"), navigation
is not an opaque bar bolted to the top edge — it is inset, translucent and
floating, with content passing underneath. This app goes one step further and
drops the bar entirely: what floats is the individual controls.

**The palette, radii and shadow ramp** still mirror the goose desktop app's
design tokens (`ui/desktop/src/theme/theme-tokens.ts` in the goose repo), so
the phone and the desktop read as one product. Treat those as tracking an
upstream source: change one to fix a real problem here, not to taste, and
change it in both themes. Where a value is deliberately *not* upstream's, it
is listed under Deviations and the reason is a measurement.

- [Liquid Glass reference](https://github.com/conorluddy/LiquidGlassReference)
- [Apple Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines)
- [Exploring tab bars on iOS 26](https://www.donnywals.com/exploring-tab-bars-on-ios-26-with-liquid-glass/)

## The rules

### 1. Prose is serif; the interface is sans

`--font-serif` resolves to New York on iOS through `ui-serif`, so this costs
no font file and no download. It is the voice of anything the agent *says*:
markdown, message text, screen and list titles. Everything you *operate* —
buttons, labels, metadata, status, chips — stays in the sans stack, and code
stays monospace.

The split is the point. When the reply is set in a different voice from the
controls around it, the reply stops competing with them, and it does so
without drawing a box.

### 2. There is no top bar, and no bottom bar

The top of a screen is three detached things: a circular control at the
leading edge, the title floating over the page, and a trailing group pill.
Each carries its own glass. Nothing spans the width.

The title is centred by being taken **out of flow** — absolutely positioned at
`left: 50%`. Centring it "between the controls" is not centring: the leading
and trailing groups are different widths on every screen, and a flex title
lands half a back-button to one side. That was the bug the first attempt
shipped.

Navigation is a slide-over drawer, not a tab bar. A tab bar spent 100px of
every screen on two destinations and had no room for a third.

**Every root screen must carry the drawer's hamburger.** Settings once had
only a back chevron that rendered `if connected`, which sealed a disconnected
user into the one screen they would be on *because* they were not connected.

### 3. Chrome floats; the scrim is what makes it readable

With no bar behind the title, nothing hides content scrolling up past it. The
scroll-edge scrim (`.app::before`) covers the whole chrome band — safe area
plus gap plus bar height — in the same material as the controls, fading out at
its lower edge so there is no hard line across the page.

**The fade goes below the bar, never across it.** The band runs
`--scrim-fade` (24px) past the bar's bottom edge and the mask holds full tint
until then. Expressed as a percentage instead, the fade began at 55% of a
122px band — 67px down, with the bar occupying 70px to 122px — so the whole
bar sat in thinning material. Nothing looked wrong until a screen had enough
content to scroll: the icon buttons were fine either way because each carries
a glass pill of its own, but the title has none, and a dark code block passing
underneath stayed legible straight through the serif. `docs/audit.js` checks
that `scrim - fade` still reaches past the bar.

Do not thin the scrim to make the page feel more open. Without it, a message
bubble sails through the status bar and collides with the clock.

### 4. Rounding is tiered, and nothing is square

| tier | radius | what |
|---|---|---|
| identity | `--radius-full` | icon buttons, chips, the group pill, the floating action, toasts |
| container | 16–24px | cards, session rows, bubbles, the composer, modals, drawer items |
| control / inset | 12px | buttons, fields, tool calls, code blocks, tiles |

**A control never sits at a tighter radius than the container around it.**
Where something is clipped by a rounded parent its own radius is *concentric*:
the parent's minus the border. The tool-call output slab is 11px inside a 12px
card for exactly that reason.

**A surface that runs edge to edge is exempt, and only that.** The screen's
own edge has already cut the corner; curving it there notches the band rather
than rounding it. The review screen's file bands are the only place this
applies today — code is the only content on this phone worth the whole width
(47 monospace columns inside a card at 402pt, 52 without one) — and the head of
each band keeps the page's 16pt gutter, because the head is chrome and only
the code is content. `docs/audit.js` reads the exemption off the geometry
rather than off that list: full width, or full height, is a page or a panel
rather than an object on one, whatever screen it turns up on. That is the rule,
not a carve-out for this screen — anything spanning 402pt has had its corners
cut by the phone, so there is nothing left there to round.

### 5. Borders separate; shadows float

Anything in normal flow — cards, session rows, tool cards, table rules — gets
a 1px hairline and **no** shadow. Only things that leave the flow cast one:
the floating controls, the composer, the drawer, modals, toasts. A card that
carries both reads as trying too hard, and a card whose fill matches the page
is doing nothing at all with its shadow.

### 6. The agent's turn is never a panel

Prose on the page, in both themes. It is the longest thing on screen, and a
box around the longest thing on screen is the boxiest possible layout. Dark
mode used to give it a faint raised surface; the serif separates it now.

The user's turn *is* a bubble — it is short, and it wants to read as an
interjection.

### 7. Anything that fills is a tint, not a slab

Errors and banners carry their colour as a 12% tint of the semantic colour,
with a matching hairline and coloured text, plus a dot. A saturated
full-bleed rectangle is the loudest and most rectangular thing a screen can
contain. Saturated fills are reserved for controls the user presses — the
`Delete` in a destructive confirm earns one; a status message does not.

### 8. State is a dot; words are words

Connection state, tool status and lifecycle ride on an 8px coloured dot. In a
bar, the dot is *all* you get — the agent name and version are ~107px of text
that a centred title cannot clear, so Settings names the connection in its own
body instead.

Backend status enums are mapped to UI copy before they reach the screen.
`IN_PROGRESS` and `FETCH_UNKNOWN`, uppercased and monospaced, read as debug
output and ate the width the tool title needed.

### 9. Controls are sized for thumbs, and the whole row is the target

Icon buttons are 44px — the HIG's number. They were 40px on the argument that
surrounding bar padding made up the difference, which stopped being true the
moment the bar went away and the controls started floating on their own. A
label never goes inside a fixed-size circular button — it wraps and
overflows.

A list row is tappable across its **whole** area, not just the text block
inside it. The handler belongs on the `li`; anything else in the row that is
itself a control stops propagation. With the handler on the inner block, 36%
of a card — the padding ring and the column under the trailing button — did
nothing while still lifting on press.

The row's own destructive action is **behind** the row, not on it. A session
row is a horizontal snap scroller: the card body is a pane exactly as wide as
the row, and the action tray is the item after it, clipped away until you drag
the row left. Two things follow from making the drag a scroller instead of a
transform driven from Rust. The native renderer round-trips every listened-to
event through a synchronous XHR, so tracking a finger in `ontouchmove` costs
~60 blocking IPC calls and vdom diffs a second; WebKit does the tracking,
rubber band, momentum and snap on its own for none of that. And WebKit does
not synthesise a `click` from a touch that turned into a scroll, so the row's
tap handler stays exactly where this rule puts it and needs no threshold, no
suppression flag and no guard to keep a swipe from opening the session.

Two costs that come with it, both accepted: nothing closes a *sibling* open
row the way Mail does, because knowing another row's scroll offset means an
`onscroll` listener and that is the per-frame IPC again; and on desktop the
reveal wants a horizontal trackpad swipe or shift-wheel, which is a real
narrowing against a button that was always there.

### 10. Long output folds

A run of two or more settled tool calls collapses to one line you can open. An
agent that reads four files and runs a command otherwise produces five stacked
cards and pushes the reply they were in service of off the screen. A run stays
open when anything in it failed or is still running — which is exactly when
you want to see it.

### 11. Only offer controls that do something

The composer's chip row holds what is real on that screen: the model the next
message runs on, the mode it runs in, and — on the goose tab — how full the
context window is. A control that does nothing is worse than no control. (Diff
and PR are not in it; they act on the work rather than on your message, and
they have their own row above the composer.)

**A control that is missing still owes an explanation, and it is a chip, not a
paragraph.** The pull-request row offers Merge only where GitHub says merging
can happen, and there are four reasons it cannot: closed or merged, draft,
failing checks, and a conflict GitHub has found or not yet looked for. The
first three are already named by the two chips every row carries, so only the
fourth needs saying — `conflicts`, or `mergeability pending` while GitHub works
it out. Said in the row's existing grammar rather than a sentence under it: the
sentence was a fourth line of prose in a list you scan, and three quarters of
what it said was already on screen beside it.

Everything else adjustable lives behind the model chip rather than getting a
chip of its own — four settings would be four chips and a composer that is
mostly chrome. **Mode is the one exception**, on both backends. It is the
setting you change mid-conversation, several times, while the rest are set once
and forgotten; every reference app puts it beside the model for that reason.
The picker it opens is the settings sheet's value list with two additions,
because a mode is a way of working rather than a value: a leading icon, and the
backend's own one-line description under the name.

Two chips and a send button is what the composer row will hold, and no more.
That budget is why the goose tab's context readout is a percentage rather than
`128.0k/200.0k`: the long form is 106px of a 306px row at 360pt, which left
48px for two chip labels and rendered `Auto` as a bare ellipsis. The window
itself is still stated in full, as the Context length row of the sheet.
`docs/measure-composer.js` fails on a label clipped to nothing now, not just on
a send button pushed off the edge — the row was within its bounds the whole
time it was useless.

The chips wrap onto a second line when they do not fit; the send button never
does. It lives outside the wrapping box (`.chip-row`) rather than inside it, so
it stays pinned to the trailing edge and centred on however many lines of chips
there are — wrapping the row itself put the primary action alone on line two,
which is the one item that must never move.

**Which chip takes the second line is decided, not left to the row.** A line
breaks on an item's *unshrunk* width, so a chip sized to its own text can take
a line of its own without having given up a pixel — and at 402pt a long model
name did exactly that, leaving the attach button alone above it and pushing the
mode chip to a third line. The model chip is capped at the row minus the attach
button instead, so it always fits beside it and what wraps is the mode chip and
the context readout. A shrink-and-grow arrangement was tried first and is the
thing to avoid: where the line breaks and how much a chip gets afterwards are
two decisions, and they stopped agreeing between 375 and 402pt — a wider phone
showed *less* of the model name than a narrower one. `docs/measure-composer.js`
now runs several widths in one pass and fails on exactly that, because no
single width can see it.

The sheet has exactly two kinds of row, and which one a setting gets is
**The settings chip's face is the model, then the effort tier** — `Opus 5
Max`, the tier in the quieter of the two voices. Effort was invisible until
the sheet was open, which is a poor place to hide the one setting that changes
what a message costs and how long it takes. Nothing is said when there is no
tier to state: `OpenCode` writes "no variant was asked for" as the literal
string `default` and the app carries it as `None`, and goose ships a lone `off`
whenever the session's model cannot reason at all — a chip reading `Claude
Sonnet 5 Off` claims something was switched off rather than never offered. The
two are told apart by tone rather than by size, and the step is taken by
lifting the name rather than by dimming the tier: on a chip's `--bg-secondary`
fill there is no headroom under `--text-secondary` (4.64:1 light, 5.00:1 dark),
and `--text-tertiary` measures 2.03:1 and 1.85:1 there — under even the 3:1 a
non-text indicator gets. So the name goes to `--text-primary` (9.16:1, 9.93:1)
and the tier keeps the secondary the chip already had.

**Neither half may eat the other.** The name is what gives way when the chip
runs out of room — it is the long one, and the sheet the chip opens states it
in full — but only as far as it can give way and still be a name. The goose
row is the tight one: its chips and send button share 306px at 360pt, which
leaves the label 120 and `Claude Sonnet 5` wants 94 of them, so a tier that is
pinned *and* unbounded spends the whole difference and the chip comes back
reading `Claude…`, which cannot tell Opus from Sonnet. Three things hold the
tier to about a fifth of the label instead: `chip_effort` shortens the two long
tiers either backend serves (goose's `medium`, `OpenCode`'s `minimal`) on the
way to the chip, `.chip-effort` caps what arrives at five characters, and the
token chip stopped spending 20px on decimals it could not use — `128k/200k`,
not `128.0k/200.0k`, since a tenth of a thousand is a hundred tokens and no
one is deciding anything on it. `docs/measure-composer.js` fails if the tier
takes more than 40px, or if a name that has been ellipsised is left holding
less than twice what the tier holds.
The composer's chip row holds what is real on that screen: the token budget on
the goose tab, and one chip per tab that opens the session's settings. The code
tab's own two — the diff and the branch's pull requests — sit in the action row
above it, because they are about the work rather than about the message. A
control that does nothing is worse than no control.

Each of those two carries a number, and the number is the reason the chip is
worth its width: `+N −M` for the diff, a count for the pull requests. Both are
withheld until they can be backed. `0` is a claim like any other, and a chip
that printed one before the fetch landed would be stating something it had not
been told. The pull-request count is also *scoped* — it counts what this chat's
branch has open, never the repo's other pull requests — because a chip reading
"4" that included someone else's work would be worse than no chip at all.

Everything adjustable lives behind that one chip rather than getting a chip of
its own — four settings would be four chips and a composer that is mostly
chrome. The sheet has exactly two kinds of row, and which one a setting gets is
decided by whether choosing would change anything:

| row | shape | when |
|---|---|---|
| control | name, value, chevron; pressable, pushes the value list | more than one value |
| fact | name, value, and the reason underneath; no chevron, no press state | one value, or read-only |

The chevron is the only visual difference between them — same box, same
height, same name, same value, same note — so it is painted `--text-secondary`
and not `--text-tertiary`. Tertiary measures 2.20:1 in light and 2.87:1 in
dark, under the 3:1 this design asks of any non-text indicator, which would
leave the entire control/fact distinction below the legibility bar. It is the
same argument already made for the tool disclosure caret, and `docs/audit.js`
now checks every icon for it, since an icon carries no text and the contrast
walk skips it.

That single distinction is the rule made mechanical. Nothing that cannot change
ever renders as pressable, and nothing real ever disappears — both backends have
the same edge case, and both degrade to the same fact row. goose ships
`thinking_effort` as a select whose only value is `off` whenever the session's
model cannot reason; OpenCode returns no variants at all for the
minimax/qwen/glm/kimi families, which includes the default small model. In both
cases the user is told *why* effort is not adjustable here, rather than being
left to wonder where the setting went.

Neither tab is padded out to match the other. They share the chip, the sheet,
the row grammar and the "applies from your next message" semantics — which is
literally true on both — and the sheet names the backend, so a shorter list
reads as "this backend offers less", not as "the app forgot something". What
they do **not** get to differ in is presentation: both sheets are Provider /
Model / Thinking effort / Context length in that order, with the notes in one
voice. They were built by unrelated code and read like two products until they
were made to agree. The code tab has no Provider row and that is not a gap: a
model there *is* `opencode/claude-sonnet-4-5`, provider included, so the row
would either duplicate Model or decide nothing.

**The way back to the bottom of a transcript appears only when you are not at
it.** A transcript follows its own bottom as a turn streams, so for most of a
conversation a button offering to take you there would do nothing, and this
rule is why it is not on screen for that whole time. It hangs
out of a zero-height slot directly above the composer rather than being
positioned against the bottom of the screen: the composer grows with the draft
and the whole shell rides the visual viewport when the keyboard opens, and
either one would swallow a button anchored to the frame.

Whether it is visible is decided in JS, and so is whether new content still
pins the transcript — one fact, "the reader is at the bottom", rather than two
that can disagree (`src/viewport.rs`). Asking Rust would mean an `onscroll`
handler, which the native renderer answers with a synchronous XHR: a blocking
round trip on every frame of every scroll. The pin used to be unconditional,
which meant reading back during a turn was impossible, because the next
streamed part dragged you to the bottom again.

That fact is answered from the transcript's *height* as well as from its
scroll offset, because the keyboard moves the bottom of a conversation away
from a reader who has not scrolled at all: the shell shrinks to the visual
viewport, the transcript loses that height with `scrollTop` exactly where it
was, and no scroll event is fired to notice. A growing draft does the same
from the other end. Both are heard through a `ResizeObserver`, and a reader
who was at the bottom is taken there again rather than told they have left a
place they never moved from — being at the bottom is a place, which is what
every native chat means by it when the keyboard covers the last message.

**What backs each row.**

- **goose.** `session/set_config_option` routes exactly four ids — `provider`,
  `mode`, `model`, `thinking_effort` — and answers `invalid_params` to anything
  else. All four arrive with their current values and their choices in the
  `configOptions` of `session/new` and `session/load`, so no extra call is
  needed. The sheet renders that array in arrival order and names no ids of its
  own: a fifth option upstream shows up without an app change, and one goose
  stops sending disappears honestly. Mode is the single id it does name, to
  take it out of the list and give it to the chip — found by the `mode`
  *category* the ACP spec defines for exactly this placement decision, or by
  the id, so an agent that sends either is understood. An agent that sends
  neither simply has no mode chip. Switching is per-session; `session/prompt`
  carries no model field, so it applies from your next message.
- **the code tab.** `model`, `variant` and `agent` are all parameters of
  `POST /session/:id/prompt_async`, and OpenCode copies whatever a turn asked
  for onto the session record. "From your next message" is the mechanism there,
  not a hedge. The catalogue behind the first two comes from
  `/config/providers`, with `/provider` as a fallback because the container
  tracks a rolling tag; the modes behind the third come from `GET /agent`,
  filtered to the agents that may hold a session — an OpenCode `subagent`
  exists to be invoked by another agent, never chosen by a person. That list is
  re-fetched per chat rather than cached like the catalogue, because a
  repository can define agents of its own.

**Context length is not settable on either backend, and is reported as a
fact.** goose's four ids do not include it; every `contextLimit` in its wire
types is output only, and the ACP call site passes `None` for the one
`update_provider` parameter that could carry one. OpenCode takes no context
window on a turn either — `limit.context` is catalogue metadata, and the one
route that rewrites it (`PATCH /config`) restarts the chat's server, killing the
event stream the app is reading. The honest number is already on hand on both
tabs, so the sheet states it.

**Free models are withheld from the code tab's picker** unless the chat's repo
is flagged `public_throwaway`, and the sheet says so where they would have been.
The manager refuses them at chat-create time, but a per-turn model rides through
its transparent proxy unchecked — so a picker that offered them would be a way
around privacy hard rule 1.

### 12. An attachment costs something, and the interface says what

The `+` in both composers opens iOS's own photo / camera / Files sheet. Three
consequences shape how it looks, and all three are constraints rather than
taste.

**The tray is a row of its own, above the field.** A file name comes from a
photo library and can be anything; the control row is where the send button has
to survive whatever the server called the model, and adding a second
unbounded string to that budget is how it breaks.
`docs/measure-composer.js` now builds the composer with an empty tray, one
attachment and a full one, and fails if a name spills its chip or the composer
grows past the screen. Like `.action-row`, the tray scrolls sideways rather
than wrapping — a second row would move the composer, and where the composer
sits is the one thing on this screen a thumb learns.

**The transcript keeps a thumbnail, never the payload.** Both chat views clone
their whole state on every keystroke and the Code tab writes its transcript to
disk, so a 250 kB base64 photo in a transcript item is paid for on both, over
and over. The phone makes a ~200px thumbnail at pick time and that is what the
item holds; a payload replayed from a server is adopted only if it is already
that small, and otherwise the attachment renders as a named chip. A photo this
phone sent does not fall back to a chip, though: both planes carry their own
thumbnails across a history reload, because a photo turning into a grey chip a
second after a reconnect reads as the app having lost it. The rule and its
numbers are in `src/attach.rs`.

**Everything on the inverted user bubble takes its colour from the bubble.**
`--text-secondary` and friends are tuned against the page and land the wrong
way round on a bubble whose fill is `--text-primary`. The one piece of
secondary text in there — an attachment's size — is a 72% tint of the bubble's
own ink, which measures 6.0:1 in light and 6.1:1 in dark.

The chip and thumbnail styling itself is **provisional**: it is deliberately
plain, and confined to the `.attach-*` block in the stylesheet and to
`src/views/attach.rs`, so replacing it against a reference screenshot is a
local change.
### 12. A list only reports what its backend can be asked

The Code list marks a chat that is blocked on a permission: a dot on its tile,
and the ask itself inset in the card with `Approve` and `Deny` in it. The Chats
list marks nothing. That difference is deliberate, and it is a difference
between the two protocols rather than between how much care the two lists got.

**The code plane can be asked, but only in one shape.** A pending ask is state
inside a chat's own container and `GET /permission` reads it — one chat at a
time, through the manager's transparent proxy, which *wakes a stopped
container on any request to it*. Polling the list that way would hold every
container open and undo the idle spin-down the whole plane is built on. So the
manager aggregates instead (`GET /api/permissions`), over the containers that
are already running, and that restriction costs nothing real: a container that
is down has no live turn, so it has nothing parked on an ask. Every chat comes
from that aggregate except the one with a **live event stream**, which is both
faster and ordered and is left to speak for itself. Live is the word that
matters: a stream ends when the container spins down, when the tailnet roams,
or when the app was on another tab at the wrong moment, and the chat it was
speaking for is handed straight back to the aggregate. "The chat you opened
last" is not the same claim and is not good enough — nothing clears it, so it
would exempt one chat, permanently, from the only thing that can report it.

Which is also where the modal went: it interrupts you about the conversation
you are *reading* — the chat screen or its review — and the cards report every
other one, including the chat you were reading a moment ago and have left.
A row says so twice, in the two registers rule 8 gives it: a dot on the tile
for a scroll down the list, and `waiting on you` where the container's status
would otherwise be. That chip is not decoration. Nothing in the manager's index
reports a live turn on a chat the app does not have open, so without the ask
folded back into it the row read `idle` directly above its own panel asking for
a decision.

**The goose plane cannot be asked, and does not need to be.**
`session/request_permission` is a JSON-RPC request the *agent* makes of the
client over the one WebSocket. It is not a resource: no method lists
outstanding ones, and `session/list` reports titles, counts and a snippet with
nothing in it about a parked turn. What the connection does deliver is every
ask the agent raises on it, whichever session it belongs to — the event carries
a session id, the queue is not filtered by the open chat, and the modal names
the session when it is not the one on screen. On the goose side an unanswered
ask is therefore *always already on your screen*, and a dot in the list could
only ever be drawn behind the modal covering it.

The case the Code list exists for cannot arise there either. An ask lives only
as an outstanding request on a live socket: drop the connection and the server
resolves it as a transport error and the turn unwinds with it, which is why the
app clears that queue on disconnect. A goose session cannot be sitting blocked
while the app is away. Nothing to poll, and nothing to catch up on.

So the Chats list gets nothing — not a placeholder, not a greyed dot. Anything
there would be a lie or a duplicate.

## Deviations, and why

All commented at the point of use in the stylesheet:

- **Font.** goose uses Cash Sans, which is proprietary and not in the repo. We
  fall back to the system sans, and to the system serif for prose.
- **Every accent colour this app also sets as text** is darkened (light) or
  lifted (dark) from the upstream token. Upstream tunes them as *fills*; as
  text they land on `--tool-surface`, not on the page, and at the upstream
  values `--text-success` measured 2.5:1 there and `--text-info` 2.7:1.
  `--text-secondary` failed in *both* themes, and `--bg-danger`'s white label
  measured 3.4:1. Measured, not taste.
- **`--glass-tint` is thin (78/86%)** so the blur is visibly doing its job.
  An earlier value of 90/93% was chosen to keep bars readable with the blur
  absent — but the blur was never absent, it had been mismeasured, and
  thickening the pane removed the effect the shell is built on. Where the blur
  really is absent the answer is the `prefers-reduced-transparency` rule,
  which asks for the preference instead of guessing at it.
- **`.chip` reads one step brighter** than comparable secondary text, because
  secondary-on-secondary is unreadable at phone size.
- **`--diff-add` / `--diff-del` / `--code-muted` do not swap with the theme**,
  because the surface they sit on (`--code-bg`) does not either. Reaching for
  `--bg-danger` gives a deletion marker measuring 5.0:1 in dark and 2.9:1 in
  light — a marker that vanishes in one theme. Their values are set by the
  `+`/`−` glyph rather than by the leading rule: the glyph is text on the row
  tint and wants 4.5:1, which is much the stricter of the two bars.

## How to check your work

**Run it.** Build for a booted simulator, install, launch. An incremental
rebuild is a few seconds. Seed the settings so a reinstall does not mean
retyping four fields — these are read only in debug builds:

```sh
GOOSE_DEV_SERVER_URL=http://127.0.0.1:3285 \
GOOSE_DEV_SECRET_KEY=mock-secret \
GOOSE_DEV_WORKING_DIR=/tmp/goose-work \
GOOSE_DEV_CODE_URL=http://127.0.0.1:4399 \
GOOSE_DEV_CODE_PASSWORD=... \
  dx build --platform ios --no-default-features --features mobile
```

`cargo run -p mock-goose-server 3285` gives you the goose plane. For the code
plane, `scripts/verify/test-code-agent-manager.sh --serve` in the
personal-ai-setup repo stands up the manager against a mock OpenCode server
and prints the URL and password.

**Check both themes.** `xcrun simctl ui booted appearance dark|light`. Most
mistakes in the stylesheet are contrast mistakes, and they only ever show up
in one of them.

Some things are easier to measure than to see, and `node docs/audit.js both`
checks them: contrast against the first opaque background behind each element,
icon contrast against 3:1 (icons carry no text, so the contrast walk skips
them), geometry (overflow, clipped text, square corners, undersized tap
targets, radius nesting), the scrim still being opaque where the title sits,
and rows that render nothing and so measure nothing. It exits non-zero on a
finding.

It also repeats every state with the server-supplied strings swapped for the
longest plausible value, because a captured state only shows the one string the
app happened to be holding. What belongs in that list is any text this app does
not choose: a model name, a session title, the command a permission ask quotes
— and a filename, which is the agent's to pick and sits opposite a fixed-width
control on the review screen's file head.

`node docs/measure-composer.js [width…]` is separate because the audit cannot
see what it looks for: text spilling out of a chip is an anonymous text node
with no box, and no chip sets `overflow-x`. It builds the two composer rows the
app actually assembles, at every width you give it, with the longest model
names any server has offered and every effort tier that can reach a chip — and
fails if the send button leaves the screen, if a label spills past its pill, if
the tier is squeezed out through the side of the pill, if the tier takes more
than the 40px it is allowed, or if a name that has had to ellipsise is left
holding less than twice what the tier holds. That last pair is what the earlier
version missed by trying `Max` alone: it is the cheapest tier there is, and the
only one nothing goes wrong with.

It fails on three more things that were on screen the whole time it reported
clean, because none of them is a number going out of range: the send button
wrapping below the chips, the send button not centred on the chip block, and a
label clipped with no ellipsis. A wrapping row grows rather than overflowing,
so send could sit on a line of its own with `overflow` still reading 0; and a
label that clips itself measures as spilling a *negative* amount, so a hard cut
mid-word read as cleaner than a label with room to spare. The overflow it
measures is `.chip-row`'s, not `.composer-row`'s: the row holds a chip block
that shrinks and a send button that does not, so its own scrollWidth can never
exceed its clientWidth and asking it was asking a question with one answer.

And it fails on one thing no single width can show — a bigger phone rendering
*less* of the model name than a smaller one. That is the shape of failure a
wrapping row invites, because where a line breaks and what a chip gets
afterwards are two decisions that can stop agreeing; it read `Claude Son…` at
390pt and `Claude Sonnet 4.5` at 375pt while every number was in range at both.
So the script takes a list of widths and defaults to 320/360/375/390/393/402 —
390 and 393 are in it because the old habit of running 360, 375 and 402 stepped
straight over the two widths where it was worst.

`node docs/measure-ptr.js both` and `node docs/measure-scroll-bottom.js both`
cover the two controls no screenshot will ever catch. Against a local mock a
refresh settles in about 120ms, so the pull indicator exists for two frames;
and the scroll-to-bottom button is only on screen while a transcript is
scrolled up, which is never a state the capture settles in. Both restate their
markup instead of reading the gallery — that is exactly the drift the gallery
exists to prevent, and it is accepted here only because the alternative is not
checking them at all, so keep them in step with the views. The scroll-to-bottom
fixture builds its composer with a `.chip-row`, and one of its cases fills that
row until the chips take two lines: a composer whose chips are flat in
`.composer-row` cannot grow a second line whatever is put in it, so it would be
measuring a height the app never has. Both measure what a
screenshot would have shown: hidden when it should be, present and in the right
place when it should be, in both themes.

The scroll-to-bottom check goes one further and drives the button's *rule*,
because placement was never the half that broke. It reads the three scripts
out of `src/viewport.rs` — restating a script would let the copy drift while
still passing — and walks a real scroller through streaming, reading back, the
keyboard opening and closing under a reader in both positions, a draft growing
to four lines, and a tap. One rule is checked at every step: the button is on
screen precisely when the transcript is not at its bottom.

The audit reads `docs/gallery-states.json`, which is **captured out of the
running app**, not transcribed from it. Drive the app to the states you want
and run `scripts/capture-gallery.py /tmp/applog.txt`; the app prints its `.app`
subtree to the console whenever the UI settles in a new state
(`src/domdump.rs`, debug builds only) and the script writes both the JSON and
`docs/style-gallery.html` from it.

The state is not just the screen. A drawer, a settings sheet, a choice list, a
swiped-open row and a confirm dialog each get their own key, so they each get
audited — keying on the mounted view alone filed them all under the screen
behind them, the last dump won, and three branches' worth of new UI sat
outside everything the audit checked while it reported clean.

A capture **replaces** the gallery. Keeping states you did not visit is how
the old hand-written gallery went stale: a screen captured on another branch
survives every later run and nothing ever says the markup no longer exists.
`--merge` is there if you really want to build the set up over several runs,
and it names every key it carries over. Do not hand-edit the gallery;
re-capture it.

What the gallery still cannot tell you: safe-area insets are zero in a
browser, so the floating chrome sits higher than it does on a device, and the
font stack resolves to whatever is installed locally rather than to iOS's, so
every text measurement is approximate. Positions and material are what the
simulator is for. (Headless Chromium *does* composite `backdrop-filter` — a
controlled test measured a stdev of 7.03 with the blur against 47.11 without,
so a tint tuned as if the blur were absent comes out flat on a device.)

If you want numbers out of the real DOM rather than pixels, `document::eval`
reaches into the live WKWebView and can send `getBoundingClientRect` and
computed styles back to Rust — that is how the spacing in this design was
measured, and how the keyboard bug was finally diagnosed after two wrong
guesses. Give it ~1500ms: WebKit applies `env(safe-area-inset-*)` after first
paint, and an earlier read reports every bar 62px too high.
