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
inside it. The handler belongs on the `li`; the trailing control and the
confirm row stop propagation. With the handler on the inner block, 36% of a
card — the padding ring and the column under the trash — did nothing while
still lifting on press.

### 10. Long output folds

A run of two or more settled tool calls collapses to one line you can open. An
agent that reads four files and runs a command otherwise produces five stacked
cards and pushes the reply they were in service of off the screen. A run stays
open when anything in it failed or is still running — which is exactly when
you want to see it.

### 11. Only offer controls that do something

The composer's chip row holds facts that are real on that screen: the token
budget on the goose tab, diff and PR on the code tab. There is deliberately no
mode chip, because nothing backs it. A control that does nothing is worse than
no control.

The model chip is real: goose implements `session/set_config_option`, and the
model list arrives in the `configOptions` of `session/new` and `session/load`
with no extra call. Switching is per-session — `session/prompt` carries no
model field — so it applies from your next message.

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

Two things are easier to measure than to see, and `node docs/audit.js` checks
both — contrast against the first opaque background behind each element, and
geometry (overflow, clipped text, square-cornered surfaces, undersized tap
targets, radius nesting). It exits non-zero on a finding.

It audits `docs/gallery-states.json`, which is **captured out of the running
app**, not transcribed from it. Drive the app to the states you want and run
`scripts/capture-gallery.py /tmp/applog.txt`; every screen change prints its
`.app` subtree to the console (`src/domdump.rs`, debug builds only) and the
script writes both the JSON and `docs/style-gallery.html` from it.

That arrangement is deliberate. The gallery used to be a hand-written copy of
the views' markup and it drifted — far enough that a whole review pass
examined states the app no longer produced, while a clean audit said nothing
was wrong. Do not hand-edit the gallery; re-capture it.

What the gallery still cannot tell you: safe-area insets are zero in a
browser, so the floating chrome sits higher than it does on a device, and
`backdrop-filter` behaves differently. Positions and material are what the
simulator is for.

If you want numbers out of the real DOM rather than pixels, `document::eval`
reaches into the live WKWebView and can send `getBoundingClientRect` and
computed styles back to Rust — that is how the spacing in this design was
measured, and how the keyboard bug was finally diagnosed after two wrong
guesses. Give it ~1500ms: WebKit applies `env(safe-area-inset-*)` after first
paint, and an earlier read reports every bar 62px too high.
