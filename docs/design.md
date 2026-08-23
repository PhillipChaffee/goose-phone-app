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

## Deviations, and why

Three, all commented at the point of use in the stylesheet:

- **Font.** goose uses Cash Sans, which is proprietary and not in the repo. We
  fall back to the system stack.
- **`--text-success` / `--text-warning` in light mode** are darkened from the
  upstream tokens. Those are tuned as *fills*; as text on white they fail
  contrast, and this app sets them as text for tool-call status labels.
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

The README's screenshots come from the same place: `node docs/screenshots.js`
captures individual gallery frames at 390×844. Re-run it when a change alters
what those screens look like, so the README cannot quietly go stale the way
the previous device captures did.
