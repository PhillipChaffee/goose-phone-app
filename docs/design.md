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

The composer's chip row holds what is real on that screen: the model the next
message runs on, the mode it runs in, and — on the goose tab — how full the
context window is. A control that does nothing is worse than no control. (Diff
and PR are not in it; they act on the work rather than on your message, and
they have their own row above the composer.)

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

The sheet has exactly two kinds of row, and which one a setting gets is
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
model and mode names any server has offered — and fails if the send button
leaves the screen, if a label spills past its pill, or if a label is clipped
down to an ellipsis and nothing else. Run it at 360 as well as 402; 402 is the
width where the damage is smallest.

Both audit `docs/gallery-states.json`, which is **captured out of the running
app**, not transcribed from it. Drive the app to the states you want and run
`scripts/capture-gallery.py /tmp/applog.txt`; the app prints its `.app`
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
