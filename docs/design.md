# Design guide

What this app is trying to look like, and the rules that get it there. If you
are about to change `assets/main.css`, read this first — most of the values in
that file are consequences of the rules below rather than free choices.

## Where the look comes from

Two sources, doing different jobs.

**The palette, type scale, radii and shadow ramp** mirror the goose desktop
app's design tokens (`ui/desktop/src/theme/theme-tokens.ts` in the goose
repo), so the phone and the desktop read as one product. Treat those values as
tracking an upstream source: change one to fix a real problem here, not to
taste, and change it in both themes.

**The shell — how the chrome behaves** follows current iOS. Since iOS 26
("Liquid Glass"), navigation bars, toolbars and tab bars are no longer opaque
bars bolted to the top and bottom edges: they are inset, translucent, floating
layers, and content scrolls underneath them. That single change is what stops
a phone UI reading as a stack of rectangles.

- [Liquid Glass reference](https://github.com/conorluddy/LiquidGlassReference)
  — a detailed writeup of the material, its shapes, and where Apple says to
  use it and not use it.
- [Apple Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines)
  — the authority on tap targets, bar behavior and typography.
- [Exploring tab bars on iOS 26](https://www.donnywals.com/exploring-tab-bars-on-ios-26-with-liquid-glass/)
  — what the floating tab bar actually does.

## The rules

### 1. Chrome floats; content scrolls under it

The top bar and the tab bar are inset capsules positioned over the page, not
rows in a column. They are translucent (`backdrop-filter`) with a solid
fallback, they cast a shadow, and they carry a hairline *ring* rather than a
border — on a white page a white bar is otherwise carried by its shadow alone.

Their heights are tokens (`--topbar-h`, `--tabbar-h`) because the scroll areas
pad themselves by exactly that much. Change one and you must change nothing
else; change the padding by hand and the two will drift.

### 2. Rounding is tiered, and nothing is square

| tier | radius | what |
|---|---|---|
| identity | `--radius-full` | icon buttons, tabs, chips, send, toasts, the bars |
| container | 16px | cards, session rows, bubbles, composer, modals, alerts |
| control / inset | 12px | buttons, fields, tool calls, code blocks, images |

**A control never sits at a tighter radius than the container around it.**
Mixed radii — a 4px block inside a 16px card — are what read as boxy.

Where something is clipped by a rounded parent, its own radius is *concentric*:
the parent's radius minus the border. The tool-call output slab is 11px inside
a 12px card for exactly this reason. iOS calls this corner concentricity, and
getting it wrong is visible even when nobody can say why.

### 3. Borders separate; shadows float

Anything in normal flow — tool cards, section rules, table cells — gets a 1px
hairline and no shadow. Only things that leave the flow — the bars, the
composer, modals, toasts, alerts — cast one. An element with both is rare and
deliberate (the modal is the one that earns it).

### 4. Glass is for chrome only

Translucency belongs on navigation, tab bars and floating controls. It does
**not** belong on content: lists, transcripts, code blocks and tables stay
opaque. Glass over glass is mud.

### 5. Controls are sized for thumbs

Tap targets are 40px minimum, which clears the 44px guidance once the
surrounding bar padding is counted. A label never goes inside a fixed-size
circular button — it wraps and overflows. That bug shipped twice here (`‹ Back`
and `⇪ PR`); labelled actions use `.icon-btn.action`, which is a pill sized to
its text.

### 6. State is a dot, numbers are monospace

Connection state, tool status and progress are carried by an 8px coloured dot
and monospace text — no chips, no badges with borders. Chips are reserved for
words that carry live state ("working", "crashed").

The dot is `display: inline-block`. It has to be: the session title is a
`-webkit-box` for line clamping rather than a flex row, and an inline span
ignores width and height — the per-session dot silently collapsed to nothing
until this was fixed.

Words are set as words. Backend status enums (`in_progress`, `fetch_unknown`)
are mapped to UI copy before they reach the screen; uppercased machine tokens
read as debug output and ate the width the tool title needed.

### 7. Anything that fills is a tint, not a slab

Errors and banners carry their colour as a 12% tint of the semantic colour
with a matching hairline and coloured text, not as a saturated fill. A full
red rectangle across the page is the loudest and most rectangular thing a
screen can contain, and it was sitting directly above an equally solid
button. Saturated fills are left to controls the user presses — the `Delete`
in a destructive confirm earns one; a status message does not.

### 8. Buttons come in twos

`.btn-row` and `.modal-actions` are two-column grids, and a row holding one
button — or an odd last one — spans the full width. Four stacked full-width
bars is a wall; buttons sized to their own labels is a ragged edge against a
page of full-width cards. Where two solid primaries land in the same row the
second is quietened by a sibling rule, so a row always has one obvious
default.

Disabled controls fade (`opacity`), they do not swap to a disabled colour
token. Painting `--text-disabled` on `--bg-disabled` put two near-identical
greys on each other and the label vanished; on the light glass bar the same
swap measured 1.5:1.

## Deviations, and why

All commented at the point of use in the stylesheet:

- **Font.** goose uses Cash Sans, which is proprietary and not in the repo. We
  fall back to the system stack.
- **Every accent colour this app also sets as text** is darkened (light) or
  lifted (dark) from the upstream token. Upstream tunes them as *fills*; as
  text they land on `--tool-surface`, not on the page, and at the upstream
  values `--text-success` measured 2.5:1 there and `--text-info` 2.7:1. The
  same goes for `--text-secondary`, which failed in *both* themes, and for
  `--bg-danger`, whose white label measured 3.4:1. These are measured, not
  taste — see below.
- **`.chip` reads one step brighter** than comparable secondary text, because
  secondary-on-secondary is unreadable at phone size.

## How to check your work

Open [`docs/style-gallery.html`](style-gallery.html) in a browser. It renders
all sixteen screen states — populated and empty lists, a full transcript,
markdown, every tool state, both permission modals, the waking banner, the diff
panel, settings, error states — in 390×844 frames against the real stylesheet,
in whichever colour scheme your OS is set to. Every state is visible at once,
with no build and no device.

Check both themes. Most mistakes in this file are contrast mistakes, and they
only ever show up in one of them.

Two things are worth measuring rather than judging by eye, because the gallery
cannot show you either:

- **Contrast.** Walk every element that has its own text, composite it against
  the first opaque background behind it, and compare against 4.5:1 (3:1 for
  large or bold text). Note that `color-mix()` resolves to
  `color(srgb r g b / a)` with components in 0..1 while `rgb()` gives 0..255 —
  scaling the wrong one makes every glass bar read as near-black and invents a
  screenful of failures that are not there.
- **Geometry.** Walk for anything that overflows the 390px viewport, any text
  clipped without an ellipsis, any filled or fully-bordered box left at radius
  0, any button under 32px, and any child rounded more than the parent
  clipping it.

The blur is the one thing you cannot check here at all: headless Chromium does
not composite `backdrop-filter`, in an iframe or out of one. That is why
`--glass-tint` is set high enough that a bar stays readable with the blur doing
nothing — which is also the case on a real device under Reduce Transparency or
Low Power Mode.

The README's screenshots come from the same place: `node docs/screenshots.js`
captures individual gallery frames at 390×844. Re-run it when a change alters
what those screens look like, so the README cannot quietly go stale the way
the previous device captures did.
