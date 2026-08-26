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
(47 monospace columns inside a card at 402pt and a 16px root, 52 without one;
rule 14 and the Deviations section have the count at every text size) — and the head of
each band keeps the page's 16pt gutter, because the head is chrome and only
the code is content. `docs/audit.js` reads the exemption off the geometry
rather than off that list: full width, or full height, is a page or a panel
rather than an object on one, whatever screen it turns up on. That is the rule,
not a carve-out for this screen — anything spanning 402pt has had its corners
cut by the phone, so there is nothing left there to round.

Consecutive full-bleed bands **butt together and share a single hairline**,
rather than each carrying its own pair with a gap between. A gap turns a
full-bleed band back into an object sitting on a page, which is the thing this
exemption exists to deny — and two hairlines with a strip of page between them
is a double rule drawn between every two files.

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

`.btn` — the text button — did **not** come with them, and still has
`min-height: 40px`. It is under the floor in the height axis at every text
size and on every screen that has one (a confirm's two buttons, the Scheduler
detail's four, Settings). It is wide, so it is a much easier target than 40px
in both axes would be, and `docs/audit.js`'s `SMALL-TAP` threshold is 32px and
cannot see it. Recorded here as the open item it is rather than left to be
read as a decision: raising it to 44 is a one-line change to a rule that
reflows every screen at once, so it wants a pass of its own.

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

**A chip states a value, never its own name.** "Model" and "Mode" were the app
declining to answer questions that always have answers: a chat is always
running as some agent — `OpenCode` has no "no agent" state — and its container
is always configured with some model. Neither was read, only because neither
had been asked for. The mode chip now resolves the agent out of the server's
own list (`build` when it has one, else the first agent that may hold a
session) and `send_code_prompt` puts that name in the prompt body, so the chip
does not predict what the server will pick — the app tells it, and the claim
and the request agree by construction. The model chip asks the chat's own
container what it is configured with (`GET /chat/<id>/config`, the rendered
`opencode.json`) for the one case nothing else can answer: a chat created with
no model and never prompted, whose own record says `null` and whose session
record says nothing until a turn has been sent. Where even that fails the chip
reads `Default`, which is still a statement — the turn will carry no model and
the container's own default will run. The new-session screen's mode chip
resolves against the same list its own picker ticks, from one expression fed to
the label, the icon, the tick and the agent the session is created with:
reading the raw signal in one place and `resolve_agent` in another is how a
chip came to say `Build` on a server whose list has no `build` while the sheet
it opened ticked `Plan`.

The goose tab never had this problem and must not be "fixed": it ships
`currentValue` inside the `configOptions` of `session/new`, so its two chips
read real values on every build that sends one. The single exception in the app
is a fallback rather than a face it can wear — `mode.current_label()` answering
`None` puts the word `Mode` on the goose mode chip — and it stays because
hiding the chip would hide the picker with it: the mode is filtered out of the
settings sheet precisely because the chip is where it lives.

**A screen that configures a session is a composer, not a form.** The
new-session screen used to be a repository dropdown, a task box and a model
text field stacked in a card over a Start button — three labelled boxes to
scroll past to reach the one control. It is now the same composer the two chat
screens are: the page is the field, the parameters are pills under it, and the
field's placeholder is a sentence naming them, so the screen states its own
configuration where the reader is already looking. Two rows, not one, and the
split is by what a pill decides: the top row is what the session runs **on** and
outlives every turn (repository, base branch), the bottom row is what its first
turn runs **as** and can be changed again from the chat afterwards (model,
mode) — plus the attach button and send, which belong to the message. Nothing
spans the width and nothing is boxed: the composer drops its card there
(`.composer.bare`), because its field is not inside it and a border around six
capsules is a rectangle drawn for its own sake.

**A model is chosen, never defaulted into.** It is the one parameter that
decides what the work costs, how good it is, and — through privacy hard rule 1
— who gets to see the code, and "Model (optional)" meant most sessions ran on
whatever the manager happened to fall back to. Send is disabled until the pill
is filled, and an unfilled required pill carries the warning dot the Code list
already puts on a chat that is waiting on you (rule 8) — a disabled primary
action says something is missing, and the dot is what says which. The one case
a default is offered is the one where nothing else can be: `/config/providers`
is a route on a *chat's* `OpenCode` server, so a manager with no chat on it has
no catalogue to show, and what the picker offers then is "The server's default
model" as a real value with a reason under it — the same downgrade
`SettingRow::select` makes when there is nothing to choose between.

That row is offered only once the fetch has **settled** empty, never while it
is in flight, and the distinction is the whole rule rather than a detail of it.
The catalogue is fetched by the tap that opens the sheet, so on every first
open the list is empty and loading at the same instant — and offering the
escape hatch then would put the manager's default at the top of an otherwise
blank sheet on every single open, which is the path of least resistance this
paragraph exists to close. While it is in flight the sheet offers nothing and
says it is asking. The other empty list has no escape hatch at all and must
not grow one: a catalogue every model of which trains on its input is a dead
end for a repo that is not a public throwaway, and "the server's default"
there would be one of those same models with the rule not applied to it. The
copy states the way out (a throwaway repo, or a model on the server that does
not train) rather than offering a door that should not open.

**What is deliberately not there.** No **environment** pill: the reference has
cloud environments and this app has no such concept anywhere — not in the
manager, not in `repos.json`, not on a chat — so it would be chrome that
decides nothing. No **microphone**: voice is out of scope. No **thinking
effort**: a tier belongs to a model and `set_code_model` clears it on every
switch, so a tier picked before the model settles is a value the next tap
throws away; the chat's own settings chip takes it from the first turn on.

**The picker grows a filter where finding beats scrolling** — over eight
choices, which today is the branch list and the model catalogue and nothing
else. It sits at the bottom of the sheet, within reach and still while the list
moves under it, which is what makes the sheet a column around a scrolling list
rather than one scrollbox — and it is why `.modal-backdrop` now follows the
visual viewport the way `.app` does: no modal had held a focusable control
before, so nothing had noticed that a fixed backdrop puts its own contents
under the keyboard.

Moving the backdrop moved what a sheet's height is a percentage **of**, and
both caps had to follow it. `70vh` is the layout viewport, which iOS does not
shrink; measured at 375x874 with a 30-row branch list and the keyboard up, a
612px sheet inside a 538px backdrop put its own top at −74 and its title at
−49, off the top of the screen. `max-height: 70%` and `85%` measure against the
backdrop, so the sheet is bounded by the space it is actually in.

**A borrowed list says whose it is.** The mode picker on that screen asks a
container for its agents, and this repo may have none — so the app borrows one,
preferring a chat on the selected repo, then any that is already awake. A
repository can define agents of its own (`.opencode/agent/`), which makes a
borrowed list a good guess rather than an answer, so the sheet names the repo
it came from when that is not the one selected. The base-branch picker says the
same kind of thing about a different limit: the manager stops reading at 500
branches, and with a filter over a list that has been cut short, "Nothing
matches" about a branch that exists is a lie the reader has no way to catch.

**Attachments belong to the composer they were picked in**, and the new-session
screen is a composer without a chat. It gets a tray of its own
(`new_attachments`) rather than sharing the code chat's: `conversation_key`
decides which arriving picks are accepted, and one Vec behind two composers
meant a photo picked in a chat and left unsent was rendered in the new
session's tray and lifted into its first prompt — at a different repo. The two
halves have to be drawn by the same line.

**The chip block is one line and never more.** The send button lives outside
it (`.chip-row`) rather than inside, so it stays pinned to the trailing edge
whatever the chips do — putting the wrap on the row itself put the primary
action alone on line two, which is the one item that must never move.

The chips used to wrap, and the reversal is worth recording rather than
erasing. Wrapping was chosen so that a long model name would not be crushed; it
was rejected because a composer that grows a row under your thumb is worse than
a name you can tap to read in full. **The model name is the only elastic item
in the row** — everything else is rigid, so the deficit lands on it by
construction rather than by proportion, and the sheet the chip opens states the
model whole.

What one line costs, measured (Chromium, the longest catalogue name, real
effort tiers): at 402pt the name keeps 70–107px, at 375 it keeps 43–80, at 360
it drops to 28–65, and at 320 with a tier beside it there is nothing left and
the chip is an ellipsised tier and a chevron. In characters rather than pixels
— which is the honest unit for a name — 43px is five, and two of this app's
models share their first four, so at 375 the chip is telling you which family
the model is in and the sheet is where you read which one.

The crowded goose row — model, tier, mode and the context readout — is worse
than that paragraph reads and the number is worth writing down. It spends the
name entirely at **every** width the app meets when a tier is present: 0px at
320, 360 and 375, 6px at 390, 18px at 402 with `Max` and 9px with `Xhigh`. With
no tier it recovers 5px at 360 and 20 at 375. So on that row the model chip is
a chevron in an empty pill on an iPhone 12 mini, and a chevron beside two
letters of a tier at 375. That is the trade the row already makes deliberately
— at that moment the context warning is the most useful thing in it, and
`crowding()` only shows the readout from 75% of the window on — but it is a
trade, not a truncation, and the alternative on offer is the wrapping row this
one replaced. 320pt is a defensive width rather than a device; the narrowest
this app really meets is 360.

That row cannot be one line at 320pt by 7–35px however the space is divided, so
`.chip-row` is a sideways scroller in the same shape `.action-row` and
`.attach-tray` already use. It is a **valve, not a layout**: it engages in
exactly that one composition family, and `docs/measure-composer.js` fails on
any overflow anywhere else, so a fifth chip cannot quietly slide off the edge.
Without it those chips paint over the send button, which is the worse of the
two.

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

**The tier may not eat the name.** The name is what gives way when the chip
runs out of room — it is the long one, and the sheet the chip opens states it
in full — but the tier is the fact the label grew a second element to carry, so
it must never be what gives instead. The goose row is the tight one: its chips
and send button share 306px at 360pt, which leaves the label 120 and `Claude
Sonnet 5` wants 94 of them, so a tier that is pinned *and* unbounded spends the
whole difference and the chip comes back reading `Claude…`, which cannot tell
Opus from Sonnet. Three things hold the tier to about a fifth of the label
instead: `chip_effort` shortens the two long tiers either backend serves
(goose's `medium`, `OpenCode`'s `minimal`) on the way to the chip,
`.chip-effort` caps what arrives at five characters, and the token chip stopped
spending 20px on decimals it could not use — `128k/200k`, not `128.0k/200.0k`,
since a tenth of a thousand is a hundred tokens and no one is deciding anything
on it. `docs/measure-composer.js` fails if the tier takes more than 40px, or if
it is clipped away or ellipsised while the name still has width.

**"Last" is not "never", and the difference is a rendering bug.** Holding the
tier rigid (`flex-shrink: 0`) said "never" and produced the failure the
paragraph above is about, one element along: once the name reached zero the
tier was still asking for its full width inside a label narrower than that, and
the LABEL became the clipper — which `text-overflow` never paints on. A 32.5px
`Xhigh` with 14.7px of it painted is a chip stating a tier called `Xh`; `Max`
came out `Ma` at 320pt on every row. What orders the two now is the ratio of
their shrink factors — 1000 on the name against 1 on the tier — so the name
absorbs 99.9% of any deficit (measured: within 0.1px of what a rigid tier gave
it, at every width and every tier) and the tier gives only once the name is
frozen at nothing, on its own box, where the ellipsis is. Strict priority is
what was wanted; rigidity was a way of asking for it that had a last case.

The name has **no width floor**, and the `min-width: 6ch` that used to be one
is gone because it never was one. `.chip-label` clips, so once the row is one
line the label shrinks past 6ch and a 45px `.chip-model` box inside a 12px
label is cut mid-glyph by the *parent* — and `text-overflow` only ever paints
on the box that does its own clipping. Measured at 320pt: the box reported 45px
and 0–26px of it was painted, with no ellipsis at all. What replaced it is
three absolute things — the tier's 5ch cap, the chip's own `min-width: 44px`
(the padding, border, gap and one glyph it takes to draw itself, below which a
crushed pill paints its chevron outside itself), and a script assertion that
the name's own box is what clips it, so a cut always says it was a cut. Above
375pt the script also holds a non-crowded row to 30px of name — three
characters, two glyphs and the ellipsis. That is a floor against the name
vanishing rather than a promise of legibility, and it is deliberately below the
42.5px the arrangement actually delivers there: pinned to the measurement, the
assertion would be a record of today's layout instead of a statement of the
rule, which is that the chip must still be a name and not a chevron.
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

### 13. A list only reports what its backend can be asked

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

### 14. The app follows the system text size

Every size in `assets/main.css` is a `rem`, and `rem` means the root's
font-size. On iOS `assets/platform/ios.css` sets that root to
`font: -apple-system-body`, which resolves through `UIFont`'s preferred body
font — so the root's computed size **is** the content size category the reader
chose, and WebKit re-resolves it when they change it. One declaration moves the
whole scale.

**Every number written in this file is at a 16px root unless it says
otherwise.** That is what a browser gives by default, so it is what Android's
WebView and `dx serve --desktop` render at; iOS starts at 17 and goes to 53.

The three platforms, since the app builds for all of them from one file:

| | root | how |
| --- | --- | --- |
| iOS | 17–53px, live | `assets/platform/ios.css`, compiled in by `#[cfg(target_os = "ios")]` in `src/css.rs` |
| Android | 16px | that sheet is not compiled in; the WebView's `textZoom` is the platform answer and is not wired up yet |
| desktop | 16px | not compiled in — and this is the point of the `cfg`, because macOS is WKWebView too and resolves the *same keyword* to a flat 13px |

**Opting in is itself a layout change.** iOS body at Large is 17px against the
16 these values were authored at, so the app is 6.25% bigger before anyone
touches a slider: the review screen goes from 52 monospace columns to 48, and
the composer chip's content went 2px past a pill that used to be pinned at 32.
That is the cost, taken deliberately, rather than normalising it away with a
`--text-md: 0.941rem` that would permanently hand back 6% of what the system
was asked for.

**Text grows; tap targets do not, and neither do the glyphs inside them.** Six
rules in the stylesheet set a font-size in px and not one of them sets text —
every one sizes an SVG through `.icon { width: 1em }` inside a fixed circle or
square. They take `--icon-md` / `--icon-lg`, which are clamps: a chevron may
grow a little, but 53px of it in a 44px circle is the failure, not the fix.
Whether the *targets* should grow with the type is a genuine open question —
Apple's controls do — and nothing measured breaks if they do not, so it is
recorded here rather than defaulted into.

**A pinned height is the bug, not the clipping.** Twenty-six rules set a height
in px. Fifteen of them held text or a 1em glyph and became `min-height`, or a
`max(px, em)` square; the eleven that stayed pinned are a graphic on a graphic.
The reason it matters more than it sounds: a chip, a
swipe action and a floating action button set *no overflow at all*, so text too
big for them is not clipped — it is painted outside the pill, over whatever is
behind it, while every number the audit used to read stayed in range. That is
what the `SPILL` check in `docs/audit.js` exists for.

**An `em` on a form control needs `font-size: inherit` beside it, or it is not
an em at all.** `input`, `button`, `select` and `textarea` take a font from the
UA stylesheet — 13.333px in Chromium, a system control font in WebKit — and
that font is what `em` resolves against, whatever the paragraph around them is
set in. Two rules here are ems on a control (`.md input[type="checkbox"]`, the
markdown task-list box; `.attach-remove`, the × on an attachment chip) and both
declare `font-size: inherit` for that reason. The checkbox is the one that
proves it: without the declaration it rendered at 13.3px inside a 16px
paragraph — a 17% shrink on all three platforms — and stayed there at every
Dynamic Type size, which is the exact opposite of what a rule written in ems is
asking for. Nothing catches this: a shrink is not a spill, so no check in
`docs/audit.js` can see it.

**Where it stops: the monospace slabs.** See the Deviations section.

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
- **The three monospace slabs stop growing at 20px** (`--text-code`), while
  everything else follows Dynamic Type without a ceiling. Code is not prose,
  and a column count is part of its legibility. Measured at 402pt with a
  50-character probe inside `.diff-code`, uncapped: **52** columns at a 16px
  root, **48** at the iOS default of 17, **36** at xxxLarge, **20** at AX3 and
  **15** at AX5 — and at 15 every source line wraps three or four times with
  the 2ch hanging indent taking two of them. Capped, the same measurement
  reads 52 / 48 / 35 / 30 / 30: the floor is 30 columns from xxxLarge up. (The
  36 → 35 is `.diff-sign` becoming `max(15px, 2ch)`, which is half a column
  wider once the glyph in it is bigger than the cell was.) It is one decision
  applied to all three slabs rather than to the review screen alone, because
  the argument does not distinguish them — but **inline `.md code` is
  deliberately not capped**, because it sits in a sentence and a 20px word in
  the middle of 40px prose is a rendering fault.
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

**Check both ends of the text scale.**
`xcrun simctl ui booted content_size accessibility-extra-extra-extra-large`,
and `large` to put it back. Rule 14 makes the root font-size the reader's
choice, so a screen that only holds together at the default is a screen that
has not been checked.

Some things are easier to measure than to see, and `node docs/audit.js both`
checks them: contrast against the first opaque background behind each element,
icon contrast against 3:1 (icons carry no text, so the contrast walk skips
them), geometry (overflow, clipped text, square corners, undersized tap
targets, radius nesting), the scrim still being opaque where the title sits,
and rows that render nothing and so measure nothing. It exits non-zero on a
finding.

It walks the geometry **at four text sizes** — 16px (Android and the desktop
build), 17 (iOS at Large), 23 (xxxLarge, the largest non-accessibility size)
and 53 (AX5) — by setting the root font-size directly on each captured state.
That is the right simulation rather than a shortcut: Chromium cannot parse
`-apple-system-body` at all and leaves the root at 16px, and the whole design
of the opt-in is that the root's px *is* the body size, so stating it as a
number is the same claim in the only form this browser can hear. It needs no
re-capture, which is what makes it compatible with never hand-editing the
gallery. Contrast runs at the smallest size alone, because the 18.66px
large-text threshold makes every larger one strictly more permissive.

Three of its checks only mean something on that axis. **`CLIPPED-Y`** is the
vertical half of the clipped-text walk, with anything carrying a
`-webkit-line-clamp` exempted, because four `.session-*` elements clip
vertically on purpose. **`SPILL`** is ink outside a box that never clips —
which is the entire class of failure the other walks are blind to, since a chip
sets no overflow and so paints its text over the composer rather than cutting
it; `docs/measure-composer.js` has had that check for the composer for a while
and the audit did not have it at all. **`TITLE-TALLER`** is the bar's heading
leaving the bar, which the `SCRIM` check could not see while `.topbar` had a
pinned height: its bottom edge never moved, so the comparison could not go
wrong and reported clean with the title painted outside the glass.

Against the stylesheet as it stood before rule 14 landed, that pass reports
**844 findings** — 36 at 16px, 36 at 17, 84 at 23 and 688 at 53 — and it is
clean against the one in the tree. A check you cannot make fail is worth
nothing.

It also repeats every state with the server-supplied strings swapped for the
longest plausible value, because a captured state only shows the one string the
app happened to be holding. What belongs in that list is any text this app does
not choose: a model name, a session title, the command a permission ask quotes
— and a filename, which is the agent's to pick and sits opposite a fixed-width
control on the review screen's file head.

`node docs/measure-composer.js [width…]` is separate because the audit cannot
see what it looks for: text spilling out of a chip is an anonymous text node
with no box, and no chip sets `overflow-x`. It builds the composer rows the app
actually assembles — both chat tabs and the new-session screen's two — at every
width you give it, with the longest model names any server has offered and
every effort tier that can reach a chip.

The first thing it fails on is a chip block taking **more than one line**, which
is the rule the stylesheet states. It is measured by clustering each row's chips
on their vertical centres rather than their tops, because the attach button is
36px tall and the chips 32, so grouping by `top` reports two lines for a row
that is plainly one. It replaced the send-button-below-the-chips check, which a
`nowrap` row can no longer fail; the send button is still held to its own row's
centre, and judged against *that* row rather than every chip on the page —
otherwise the new-session screen's context row reads as a wrap on every clean
run.

Then: the send button leaving the screen, any chip's own children painting past
its pill (not just its label — a crushed pill pushes its chevron out through the
side, which is a real 5px failure at 320pt), the composer growing wider than the
screen, the tier being clipped away or ellipsised while the name still has
width, the tier taking more than the 40px it is allowed, and a label clipped
with no ellipsis — in the mode chip, in the model name, and in the tier. That
last one is what the removed `min-width: 6ch` floor was silently failing: a
parent narrower than the box inside it produces a hard cut mid-glyph and
`text-overflow` never paints, so the floor measured as present while nothing of
it was drawn. The tier had the same failure and it survived the floor's
removal, because the question was asked *only where the name still had width* —
and the label is narrowest, so the parent is likeliest to be the clipper,
exactly where the name has none. 360 compositions were rendering `Xh` and `Ma`
under a clean run. The hard-cut check is now asked of every composition
regardless, and it is what the name's suppression rule was moved off: a tier
that has given way may ellipsise once the name is at zero, but it may never be
cut by a box that cannot say so.

The width floor that remains is banded — 30px of name at 375pt and above, on a
row that is not carrying the context readout — because below 375 one line costs
the name everything and 320 is a defensive width rather than a device.

It runs the **same four text sizes the audit does**, and three of its constants
had to stop being pixels first, because at an accessibility size they failed on
layouts doing exactly what the stylesheet says. The tier's 40px allowance is an
assertion about a `5ch` cap, so at AX5 a correct tier measured ~100px against
it; the 30px name floor is three characters, which no name can be at 53px; and
the exemption that lets `.chip-row`'s valve engage was keyed to raw viewport
px. All three are now per-rem, and the valve's exemption is in **effective
width** — `width x 16 / root` — because what the chips divide is columns, and a
402pt phone at AX5 has the columns of a 121pt one. At a 16px root every one of
them is word for word the rule that was there.

**Larger text genuinely cannot hold the row in pills that all fit, and the
valve is what happens instead.** Measured at 402pt, this is where each row
first needs to scroll: the four-chip goose row at **AX1** (root 28, 230
effective points, 27px over), the two chat rows at **AX3** (root 40, 161
points, 12px over), and the new-session context row at **AX4** (root 47, 137
points, 31px over). Nothing overflows at xxxLarge at any width in the sweep.
What still holds at every size, and is still asserted: the block is **one
line**, send is on screen and centred on its own row, nothing paints outside a
pill, and every cut says it was one.

The row's overflow is `.chip-row`'s, not `.composer-row`'s, and it is now how far
the scroller would have to scroll. It must be 0 everywhere except the one family
that cannot fit — 320pt with four chips — so the valve cannot quietly absorb a
fifth chip.

And it fails on one thing no single width can show — a bigger phone rendering
*less* of the model name than a smaller one. That was the shape of failure a
wrapping row invited, because where a line breaks and what a chip gets
afterwards are two decisions that can stop agreeing; it read `Claude Son…` at
390pt and `Claude Sonnet 4.5` at 375pt while every number was in range at both.
One line makes it true by construction, so it is a guard against the wrap coming
back rather than the thing it was written to catch. The script takes a list of
widths and defaults to 320/360/375/390/393/402 — 390 and 393 are in it because
the old habit of running 360, 375 and 402 stepped straight over the two widths
where it was worst.

`node docs/measure-ptr.js both` and `node docs/measure-scroll-bottom.js both`
cover the two controls no screenshot will ever catch. Against a local mock a
refresh settles in about 120ms, so the pull indicator exists for two frames;
and the scroll-to-bottom button is only on screen while a transcript is
scrolled up, which is never a state the capture settles in. Both restate their
markup instead of reading the gallery — that is exactly the drift the gallery
exists to prevent, and it is accepted here only because the alternative is not
checking them at all, so keep them in step with the views. The scroll-to-bottom
fixture builds its composer with a `.chip-row`, and one of its cases fills that
row with everything it can carry at once. That case used to prove the composer
at its tallest — four chips wrapped, 34px the button had to move up by — and it
proves the opposite now: the row is one line whatever is in it, so the button
must sit at exactly the `top` the plain case puts it at, and the case asserts
that rather than silently becoming a second copy of the first. Both measure what
a screenshot would have shown: hidden when it should be, present and in the
right place when it should be, in both themes.

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

---

## The desktop shell

The same design system, worn a second way. `assets/main.css` is untouched by
it: everything below lives in `assets/desktop.css`, which `src/css.rs` embeds
only in a desktop binary.

**Platform and width are different axes.** The platform decides the
*affordances* and it decides them at compile time — the desktop shell has
always-visible row actions, no swipe tray and no pull-to-refresh; the phone
shell has all three, unchanged. Width decides one thing and one thing only:
how many columns are drawn. **A desktop window dragged narrow is a narrow
desktop app, not the phone.** Nothing about how a control behaves changes with
width. goose's own desktop app takes the same position — its single width
threshold collapses the sidebar and changes nothing else.

**The columns, and where the numbers come from.** Nav 212, list 330, and a
content column no narrower than 360, which is this file's own floor (rule 14's
note: 320 is a defensive width rather than a device) and the width
`docs/measure-composer.js` is gated at. The breakpoints are their sums rather
than round numbers:

| width      | columns                                              |
|------------|------------------------------------------------------|
| ≥ 902      | nav 212 · list 330 · detail                          |
| 572 – 901  | nav 212 · list **or** detail                         |
| < 572      | nav collapses to a 56px icon rail · list or detail   |

Below 902 the list and the detail share a column and whichever has something
in it wins — which is the arrangement goose's desktop app is in at every
width. The rail keeps every destination on screen and one click away; the
labels stay in the DOM at zero size, so each button keeps its name, and
`title` gives the pointer a copy of it.

**The window's floor is 480 × 560 points**, set with
`WindowBuilder::with_min_inner_size`. 480 is the rail plus 424 of content —
wider than the 402 frame every gallery state is audited in — and it is also
exactly what goose's own desktop app ships as its minimum. 560 is the nav's
intrinsic height (seven destinations at 48px, plus the wordmark and the
padding) with slack. A test in `src/shell/desktop.rs` checks the floor against
the breakpoints, because only one of the two is compiled.

**Nothing observes a resize.** The breakpoints are `@media` rules in a
stylesheet a phone binary does not contain, so pane count needs no Rust at
all. That is not tidiness — it is the synchronous-XHR rule above: a Rust
resize handler would be a blocking round trip per frame of a drag. The one
thing CSS cannot work out for itself is whether a detail is open, and that is
a fact about the app rather than the window, so the shell states it in a
`data-detail` attribute.

**Refresh is arrival, plus a chord.** There is no refresh control, which is
also goose's own desktop app's answer — `SessionListView.tsx` has none and does
not poll. Arriving at a destination re-fetches its list, and ⌘R (Ctrl+R
elsewhere) re-fetches whatever is mounted. Both go through the same
`viewport::refresh_named` the phone's pull gesture uses, so a list cannot be
refreshable one way and not another. The arrival half is the one a user meets;
⌘R is deliberately undocumented in the UI, and registering it as a real menu
item is the obvious next step for discoverability — it is not taken here
because a menu accelerator *consumes* the key on macOS, so it would have to
replace the JS listener rather than sit beside it, and that swap cannot be
verified without a desktop run.

**The row is the list's, the detail is beside it, and the row says so.** In
three columns the list and what it opened are on screen together, which is the
one arrangement where an unmarked list is confusing — the phone never had it.
The open row takes `--bg-tertiary`, which is the token `assets/main.css` already
proved reads as *selected* (see the note over `.drawer-item.active`), and
`ListRow` refuses to paint it at all on the phone.

### Deviations, desktop only

**The pane header does not float.** Rule 3 — "each carries its own glass,
nothing spans the width" — is the phone's identity and cannot survive three
columns: `.topbar` is positioned against `.app`, so three of them would stack
in one corner, and three glass pills abreast is three where the eye expects
one. On the desktop the bar is static with a rule under it. The controls keep
their shapes and lose their shadows.

**Row action buttons are 32px, not rule 9's 44.** 44 is the HIG's *touch*
number and a pointer is not a thumb. 32 is not a taste either: it is the floor
`docs/audit.js` enforces (SMALL-TAP fires under 32px in either axis), and it
is the size goose's own desktop rows use. The mockups' 26 would fail our own
gate. The nav's rows keep 48px — nothing is gained by shrinking them.

**Row actions are out of flow.** The icons are absolutely positioned in the
row's top-right, and only the *title line* reserves a gutter for them
(`:has()` counts the buttons, so a one-action row reserves one and a row with
none reserves nothing). Left in the row's flex flow they take their width off
the meta line and the quote as well: measured at a 1180-wide window, the quote
came out 141px with 220px of text clamped into it. The agreed mockup puts the
actions beside the title only and lets the quote span the card; this is that
arrangement reached without changing the markup, which is what keeps the
phone's DOM identical.

**The app's first `:hover` and `:focus-visible` rules live here.**
`assets/main.css` has none at all; it was written for a thumb. Hover changes
colour and shows a button's border — it never *reveals* a control and never
moves one. Two corollaries that are easy to get wrong and were:

- **Hover may not borrow the selected colour.** `--bg-tertiary` is what
  `.drawer-item.active` paints with, and main.css's own note says why ("at
  1.08:1 against the light page the selected destination was indistinguishable
  from the unselected ones"). Spending it on hover made two destinations look
  selected at once. Hover gets its own step, halfway between the pane and the
  selected fill.
- **The focus ring is an outline and nothing else.** `border-radius` is not an
  outline property: setting one in the `:focus-visible` rule squared off every
  control the moment it took focus — the FAB went from a 9999px pill to a 4px
  rectangle. WebKit and Chromium already draw the outline following the
  element's own radius, so there is nothing to add.

The ring itself is `--text-primary`, which is by construction the
highest-contrast colour against every surface in the file; inventing an accent
token is a decision this work had no mandate for.

### What is not covered yet

`docs/style-gallery.html` and `docs/audit.js` cover the **phone shell only**.
The gallery holds no desktop state, so nothing in `assets/desktop.css` is
audited — which is why `assets/desktop.css` is deliberately *not* in
`assets/features/`, since the audit links that whole directory into 402×874
phone frames and would measure every phone state against a layout no phone can
produce. Capturing desktop states works and is easy — the desktop build is the
default feature set, so `cargo run > /tmp/desktop.log` against
`cargo run -p mock-goose-server` prints the same `@@DOM@@` dumps a device does
— but `scripts/capture-gallery.py` writes the store wholesale from one log and
`window.__dumpKey` is the bare screen name, so a desktop `chats` would
overwrite the phone's. Two prerequisites before a desktop axis exists: the
script has to take several logs in one invocation, and the dump key has to
carry the shell.
