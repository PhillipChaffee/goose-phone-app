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

The composer's chip row holds what is real on that screen: the token budget on
the goose tab, diff and PR on the code tab, and one chip per tab that opens the
session's settings. A control that does nothing is worse than no control.

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
reads as "this backend offers less", not as "the app forgot something".

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
  stops sending disappears honestly. (Mode *is* backed — an earlier version of
  this rule said nothing backed it, and that was wrong.) Switching is
  per-session; `session/prompt` carries no model field, so it applies from your
  next message.
- **the code tab.** `model` and `variant` are both parameters of
  `POST /session/:id/prompt_async`, and OpenCode copies whatever a turn asked
  for onto the session record. "From your next message" is the mechanism there,
  not a hedge. The catalogue behind them comes from `/config/providers`, with
  `/provider` as a fallback because the container tracks a rolling tag.

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

`node docs/measure-composer.js [width]` is separate because the audit cannot
see what it looks for: text spilling out of a chip is an anonymous text node
with no box, and no chip sets `overflow-x`. It builds the two composer rows
the app actually assembles, at whatever width you give it, with the longest
model names any server has offered and every effort tier that can reach a chip
— and fails if the send button leaves the screen, if the tier is squeezed out
through the side of the pill, if the tier takes more than the 40px it is
allowed, or if a name that has had to ellipsise is left holding less than
twice what the tier holds. That last pair is what the earlier version missed
by trying `Max` alone: it is the cheapest tier there is, and the only one
nothing goes wrong with. Run it at 360 and 375 as well as 402; 402 is the
width where the damage is smallest.

`node docs/measure-ptr.js both` and `node docs/measure-scroll-bottom.js both`
cover the two controls no screenshot will ever catch. Against a local mock a
refresh settles in about 120ms, so the pull indicator exists for two frames;
and the scroll-to-bottom button is only on screen while a transcript is
scrolled up, which is never a state the capture settles in. Both restate their
markup instead of reading the gallery — that is exactly the drift the gallery
exists to prevent, and it is accepted here only because the alternative is not
checking them at all, so keep them in step with the views. Both measure what a
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
