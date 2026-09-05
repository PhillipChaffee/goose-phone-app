# Design guide

What this app is trying to look like, and the rules that get it there. If you
are about to change `assets/shared.css`, read this first — most of the values in
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

The app ships no font file and never will. `docs/audit.js` does ship three, for
the length of a run and for one reason — a browser that has never heard of
San Francisco cannot be asked whether a box fits. See *The audit brings its own
fonts*, at the end of this file.

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
as an outstanding request on a live socket, and dropping the connection takes
the round with it — **measured**, not read: goose 1.46.0, sessions
`20260827_3` and `20260827_4`, replayed the user's message and nothing else
(`docs/permission-durability.md` section 0). A goose session cannot be sitting
blocked while the app is away. Nothing to poll, and nothing to catch up on.

> This paragraph used to give a mechanism — "the server resolves it as a
> transport error and the turn unwinds with it" — and that mechanism was
> falsified. There is no declined tool and no transcript record of any kind;
> the round is discarded whole. The *conclusion* survived the measurement and
> the reason for it did not, which is the failure mode a rule like this has:
> right for a wrong reason, and then defended on the wrong reason when it
> changes.

So a **live** ask gets nothing in the Chats list — not a placeholder, not a
greyed dot. Anything there would be a lie or a duplicate.

**A dead one is a different claim, and it does get a row.** An ask whose
answer never reached the agent is not waiting for you and cannot be
answered: the round it belonged to is already gone, and the session comes
back named after work that has no trace of having happened. That is neither a
lie nor a duplicate — nothing else on any screen says it — so it is reported
in the two registers rule 8 gives it, and no others. A dot on the tile
(`.session-tile.attention`, the same badge the Code list draws) and one
sentence in the panel under the row, in the past tense: *"An answer never
reached goose. That round was discarded."* Not "waiting on you", which is the
one thing that is definitely not true, and **no buttons**, because there is
nothing to press. The row opens the chat, where the loss appears again at the
tail of the transcript with a Dismiss.

Amber for both, deliberately, and it is the argument `.session-ask` already
makes: a lost answer is the same fact as the question it belonged to, so it
is the same colour. Red would rank it with a failed request, and nothing
failed — the work was thrown away.

### 14. The app follows the system text size

Every size in `assets/shared.css` is a `rem`, and `rem` means the root's
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
  dx build --platform ios
```

Nothing to pass: `Cargo.toml` picks the renderer from the target triple with
two target-conditional `[dependencies]` tables, so `--platform ios` is the
whole instruction. (`dx` does synthesise flags of its own underneath —
`--verbose` shows `features: ["desktop", "dioxus/mobile"]` on iOS — but both
are inert against this manifest: `desktop = []` enables nothing and
`dioxus/mobile` is what the target table already selected.)

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
gallery.

It walks a second axis too — **six phone sizes**, width *and* height:
320×568, 360×800, 375×667, 390×844, 402×874 and 440×956. Height travels with
width because a phone is not a column, and that is not a detail. Both failures
this axis found are "content taller than the space it was given", so holding
the height at the reference 874 while narrowing invents a tall thin phone
nobody owns and understates every one of them: at 375×874 the drawer reported
24 findings, and at 375×667 — the size an SE 3rd gen actually is — it reported
116. 402×874 stays as the **reference size**, because it is what the gallery is
captured at and what every measurement in this document was made against;
contrast runs there and at the smallest text size alone, since the 18.66px
large-text threshold makes every larger one strictly more permissive and a
wider phone moves boxes rather than colours. 320×568 is in the list because
`docs/measure-composer.js` already gates 320 as a defensive floor, and an audit
that gave up at 375 would gate a narrower band than the composer script does.
360×800 is in it because this app ships to Android as well, WebView's CSS px is
a dp, and 360dp is the width most Android phones report — a list of iPhones
alone would gate half the codebase. It is the only entry here that is not an
iPhone, and root 16px is the text size that goes with it, since the Dynamic
Type opt-in is iOS-only.

The rest of the Android band is absent because it was measured rather than
assumed: 393×852, 412×915 and 360×640 added to that list all report clean, and
all three together would cost half again the runtime to restate what the six
already say. 393×852 has a second reason — three points from 390 is a question
about how many columns one elastic chip has, which is `measure-composer.js`'s
subject, and that script does sweep 393. The two width lists are not the same
list and are not meant to be: they share the 320 floor and the 360/375/390/402
middle, and each holds one width the other does not (393 there, 440 here) for a
reason stated where it is declared.

Adding that axis turned the gate red with **308 findings**, every one of them
at AX5 and none of them at 390, 402 or 440 — 148 at 320×568, 116 at 375×667 and
24 at 360×800. They were six causes: five in the stylesheet, accounting for 288
of them, and one in the check itself. Both of the real ones had been on screen
the whole time the audit reported clean at one size.

The first was the drawer, and it is the more instructive. `.drawer-nav` was
given `flex: 1; min-height: 0; overflow-y: auto` precisely so the destinations
would scroll at accessibility sizes — but a scroller only helps if its content
keeps its size, and the rows are flex items of that column with the default
`flex-shrink: 1`. Worse, `min-height: 48px` on `.drawer-item` is an *explicit*
minimum, so it replaces the automatic content-based one and licenses flexbox to
crush the row below its own line box before the scroller ever engages. The fix
had been half-done and looked finished. Measured at 375×667 and AX5: every row
squeezed to 48px, a two-line "Scheduler" label occupying 389..513 inside a
427..475 box, painting 28px of real glyph across the "Skills" row above it —
two destination names on top of each other. `flex-shrink: 0` is the whole fix,
and it cannot regress the reference size because there is no crush there to
undo: at 402×874 the rows already measure their natural 62px and the nav's
scrollHeight equals its clientHeight. With it, the nav finally scrolls the way
its own comment says it should — 724px of content in a 497px port.

The second was four boxes where one word is wider than the column it was given,
at 320×568: `streamable_http` 277px in a 256px `.banner`, `reviewed` 152px in a
119px `.diff-progress-label`, `commands` 189px in a 153px `.choice-note`, and
`Credentials` — uppercase with 0.06em of tracking, which is what makes it the
worst of the four — 290px in a 254px card head, whose ink lands 3px past the
right edge of the *screen* and is saved only by `.app`'s own overflow. All four
are the second half of an idiom this stylesheet already states nine times (ten
counting the feature sheets; the tenth line a grep for it finds in `shared.css`
is a comment), and two of them already carried the first half (`min-width: 0`):
the flex minimum had been lowered without the `overflow-wrap: anywhere` that
lets the word actually break. `anywhere` rather than `break-word`, because only
`anywhere` also lowers the min-content size the box is being sized from.

The remaining 20 were the check being wrong, and are the one place this axis
argued for *narrowing* a walk. All 20 were the same `<path>`, inside an
`svg.icon`, inside an `.action-chip` scrolled off the end of `.action-row` —
which is a deliberate sideways scroller. The chip and the svg root are both
exempted correctly; the path was not, because `inHorizontalScroller` stops at
the first ancestor that clips and from inside an icon that is always the icon's
own root (`svg:root { overflow: hidden }` is a UA rule), so the walk could
never reach the scroller. A path's bbox is not a box on screen. The exemption
is conditioned on the root actually clipping rather than on being inside an
`<svg>` at all, so an svg that states `overflow: visible` still reports its
children; and the root is never exempted by it, so it goes on answering for
itself. Verified both ways: with the CSS fixed but the tightening absent the
residual is exactly those 20 findings and nothing else, and with `.action-row`
forced to `overflow: visible` the svg root, both chips and the stat count all
still fire.

One implementation note worth keeping, because it looks like a no-op. The size
loop resizes a live page rather than opening one per size — 196 navigations
stay constant and only reflows multiply — and it follows every resize by
touching every element's rect once. A closed `<details>` is a
`content-visibility`-locked subtree, and Chromium does not re-lay it out when
the viewport *narrows*: the first walk afterwards reads the previous size's
numbers. Reading `document.body.offsetHeight` does not clear it, because a
page-level reflow is precisely what a locked subtree skips. It does not bite in
the order the list is actually in, since that list only ever widens — but
against the stylesheet in the tree, which the audit reports clean, reverse the
list with that line deleted and the run reports **16 findings against a true
zero**: the code card's `<pre>` at 423 in a 402 viewport, stale from 440, and
at 343 in a 320, stale from 360. Put the line back and the reversed list is
clean again. It costs nothing measurable, because the walk was going to force
that layout a moment later anyway.

Two coverage claims are asserted rather than assumed, because both had already
been demonstrated to fail quietly. The contrast walk runs at one cell of the
grid — the reference size, the smallest text size — so the entry carrying that
flag *is* the scope of one of the two walks: delete the flag and, against a
deliberately broken stylesheet, 158 real contrast failures are reported as
`Clean`. It is now a startup error to have anything but exactly one, and the
summary line names the cell rather than folding contrast into the whole
product. Separately, every stylesheet is checked non-empty at startup, because
a zero-byte `<link>` fires the load event and `document.styleSheets` counts a
link that 404s — so nothing inside the page can tell a styled document from an
unstyled one. Emptying `assets/features/skills.css` and running the gate
reports **Clean**; emptying `assets/shared.css` reports 73664 findings about
`<textarea>`'s default 182×21 box and buttons at radius 0, which reads as a
design regression rather than as a missing file.

Three of its checks only mean something on the text-size axis. **`CLIPPED-Y`** is the
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

`SCRIM` and `COLLAPSED` are counted with the geometry rather than the contrast
walk, which is where they started. Both measure edges, not colours — `SCRIM`
compares the solid band against the bar's *measured* bottom, and `COLLAPSED` is
a height — so both belong on the phone-size axis, and `SCRIM` in particular is
width-sensitive in a way nothing else covers: a title that wraps at a narrow
width makes the bar taller and the scrim stops reaching it, which
`TITLE-TALLER` cannot see because the heading is still inside the now-taller
bar. Moving them is also what makes it honest to run the contrast walk at the
reference size alone; what is left in that function is provably colour-only.

Against the stylesheet as it stood before rule 14 landed (`22a91ca^`), that
pass reports **6424 findings** — 240 at 16px, 240 at 17, 600 at 23 and 5344 at
53 — and it is clean against the one in the tree. A check you cannot make fail
is worth nothing. Those are the numbers the whole grid gives, and the grid is
what has to be restated whenever it grows: this figure read 844 (36/36/84/688)
when it was written, measured at 402×874 alone and over the 43 states captured
then. Pinning today's pass back to 402×874 gives 980 (40/40/96/804) — the same
walk, six more captured screens. Re-measure it when either axis changes; a
calibration number is only worth its date.

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

**One store, two shells, and therefore one invocation.** The desktop build
prints the same dumps a device does — `cargo run > /tmp/desktop.log` against
`cargo run -p mock-goose-server` — and both shells write into the same JSON, so
a phone capture followed by a desktop capture would each delete the other's
states. Pass both logs at once instead:

```bash
scripts/capture-gallery.py /tmp/applog.txt /tmp/desktop.log
```

Which shell drew a state is in its **key**: `src/shell::DUMP_PREFIX` is
`"desktop-"` in a desktop binary and the empty string in a phone one, prefixed
onto the destination key in `src/app.rs`. It has to be a value handed to the
JS rather than something the JS works out, because `src/domdump.rs` is
installed by a hook that runs *above* the shell split and a browser cannot ask
which binary it is in. The phone's empty prefix is load-bearing rather than a
default — the 49 phone keys already in the store, `LABELS` in
`scripts/capture-gallery.py` and every phone finding the audit has ever
reported are all keyed on the bare name — so `src/shell/mod.rs` asserts it at
compile time on the targets it is about. That one string then does every job
that has to be done downstream: it keeps the two sets from colliding in the
store, it picks the frame and the stylesheets `docs/style-gallery.html` renders
a state in, and it is what `docs/audit.js` partitions on.

The state is not just the screen. A drawer, a settings sheet, a choice list, a
swiped-open row and a confirm dialog each get their own key, so they each get
audited — keying on the mounted view alone filed them all under the screen
behind them, the last dump won, and three branches' worth of new UI sat
outside everything the audit checked while it reported clean.

A capture **replaces** the gallery, and a capture may be **several logs**.
Keeping states you did not visit is how the old hand-written gallery went
stale: a screen captured on another branch survives every later run and nothing
ever says the markup no longer exists. That guarantee is the reason the two
shells go in one invocation rather than two runs — `--merge` is not the
workaround, because it exists precisely to *shout* that a carried-over key may
be stale, and using it routinely would turn the freshness guarantee into noise
nobody reads. It is there if you really do want to build the set up over
several runs, and it names every key it carries over. Do not hand-edit the
gallery; re-capture it.

**`--only <prefix>` is the third mode, and it is not `--merge` under another
name.** `scripts/capture-gallery.py --only desktop- /tmp/desktop.log` declares a
scope: inside the prefix the run is authoritative exactly as a full run is — a
state matching it that the app no longer emits is dropped and named — and
outside it every key is carried over untouched, counted rather than listed. So
the carried set is by construction the half of the store this capture said
nothing about, which is why it does not have to shout about it the way `--merge`
does. It exists because a desktop-only change otherwise needs a booted simulator
and a phone driven through 49 screens to say anything at all. The script rejects
a log holding states outside the prefix, so a mistyped one (`--only destkop-`)
fails rather than silently carrying the whole store over and calling it a
capture.

**What says the store has gone stale.** Nothing in a capture can: `docs/audit.js`
renders the states it is given and cannot know the app stopped emitting them,
which has already produced a Clean audit over a sidebar that no longer existed.
`every_class_the_desktop_shell_renders_is_in_the_captured_store`
(`src/shell/desktop/mod.rs`) is the thing that can — it reads the class names
out of the shell's own source and fails on one that no captured desktop state
contains. The direction matters and only one of the two is decidable: the source
is authoritative for what the app emits, so "the app renders `.tree-branch` and
the store has never seen it" is answerable, while "the store holds a class the
app has dropped" is exactly the question `src/selfscan.rs` records as
unanswerable from a photograph. Its `UNCAPTURED` list is the gap, and that list
**may only shrink**. It landed at 53 class names — nearly all of them the Code
plane and the inspector with a live subject, because `desktop-code-list` had
been captured against a gateway that was not connected — and the capture that
took the desktop store from 13 states to 20 drove that gateway connected and
left **four**. What is left is unreachable rather than undriven: `pane-empty`
and its two children belong to an arm of `empty_detail` that nothing can take
(Settings is the one `root: None` destination and its detail is
unconditional), and `insp-empty` needs a plane whose server URL is *unset*,
which a `dev_seed!` build does not have. The reasons are written on the list.

What the gallery still cannot tell you: safe-area insets are zero in a
browser, so the floating chrome sits higher than it does on a device.
Positions and material are what the simulator is for. Two more, both of them
heights: the **keyboard-up viewport**
is a real height this app renders at and no headless browser reports it, and
every state is captured and measured **at rest** — `scripts/capture-gallery.py`
records markup at the top of its scroller and no offset, so a screen scrolled
under the chrome is a state that would have to be captured as one. (Headless
Chromium *does* composite `backdrop-filter` — a controlled test measured a
stdev of 7.03 with the blur against 47.11 without, so a tint tuned as if the
blur were absent comes out flat on a device.)

### The audit brings its own fonts

**A gate that asks the host what a font is has no verdict.** For a while this
list said the font stack "resolves to whatever is installed locally rather
than to iOS's, so every text measurement is approximate" — and that was fine
while the audit walked 402pt alone and nothing turned on a few pixels. Adding
the 320pt column and the AX5 scale turned the caveat into a verdict: the same
commit came out **Clean** on a Mac and **24 findings** on `ubuntu-latest`, all
of them at 320x568 at root 53px.

Nobody was rendering the design's fonts. `-apple-system` / `ui-serif` /
`ui-monospace` are San Francisco, New York and SF Mono on iOS — rule 1 above —
but Chromium on macOS matches none of those first entries and lands on
`.SF NS`, **Charter** and **Menlo**, and a Linux runner lands on Liberation
Sans, Serif and Mono (Playwright's `--with-deps` installs `fonts-liberation`,
and fontconfig answers `Arial` with it). Liberation Sans is ~5% wider than San
Francisco and Liberation Serif ~8% narrower than New York, which is more than
enough to decide a 3px question.

So `docs/audit.js` ships three faces in `docs/fonts/` and repoints the three
tokens at them for the duration of the run: **Inter** for the sans,
**Literata** for the serif, **JetBrains Mono** for the mono, all OFL 1.1, plus
796 bytes of Noto Sans Math for the one character (`⋯`) that none of them has.
The header of that file is the full argument; the four things worth knowing
here:

- **They are stand-ins, chosen by measurement.** `node docs/audit.js fonts`
  compares them against the real `/System/Library/Fonts` files over every
  string this app puts on screen, at all four roots, and fails if a median
  leaves ±5%. It is macOS-only, because only macOS has the files — Apple's
  faces are not redistributable, which is the whole reason stand-ins exist.
- **Vertical metrics are not the stand-in's.** `ascent-override` and
  `descent-override` state San Francisco's and New York's own numbers, so
  every line box is exactly as tall as iOS's and the only thing being
  approximated is how wide a glyph is.
- **Optical sizing is why the files are the big ones.** San Francisco tightens
  as it grows; a wght-only Inter does not, and ran 1.15x SF at 53px against
  1.01x at 16px — reporting spills at AX5 that no iPhone has.
- **Pinning the file is only half of it.** Chromium on Linux hints through
  FreeType and quantises advances; macOS does not. With every face pinned and
  a bound taken off `.fab` on purpose, the two machines still disagreed — 176
  findings against 152, `left=15` against `left=16`. `--font-render-hinting=none`
  is what makes them print the same bytes, and it costs nothing here because
  nothing in this walk reads a pixel.

What is still unmeasured: size-specific tracking, Core Text's shaping, and the
last few pixels of any text box — a single word can be 12% out either way. So
a box that fits **by a few pixels** here is not proven to fit on a phone. That
is a fact about the design rather than about the runner, and the answer to it
is to stop having boxes that fit by a few pixels: see `.fab` below.

Two guards keep the pinning honest, and both fail the run rather than adding a
finding. One asks every element that lays out text what family it computed —
which is how `.diff-seen` was found rendering "Viewed" in **Arial** (the UA
stylesheet's `font` shorthand names a family, and overriding `font-size` left
it behind) and a bare `<code>` outside `.md` rendering in the generic
`monospace` (Courier on iOS, WenQuanYi Zen Hei Mono on a Linux runner, Menlo
here). Both are fixed in `assets/shared.css`. The other renders the whole corpus
and asks the browser which platform faces it reached for, so a character
outside the shipped subsets cannot quietly bring a host font with it.

### A pinned box needs two bounds, not one

`.fab` is `position: absolute; right: var(--edge)` with `left: auto`, so its
width is `clamp(min-content, viewport − gutter, max-content)` — an expression
with no term for the gutter on the other side. Measured at AX5 before this was
fixed: `left = 0.00` at 320, 360, 375, 390 **and 402** — the reference width —
a "floating" action button 110px tall spanning the screen with a 16px margin
on one side and none on the other. `--edge` calls itself "the gutter every
floating thing shares"; this was the floating thing that did not.

The audit could not say so, because a shrink-to-fit box only reports
`OVERFLOW-X` once its *min-content* passes the viewport, and that cleared 320pt
by 10px in San Francisco and missed by 3 in Liberation Sans. **That** is what
CI's `button.fab left=-3` was: a real defect, reported by the machine whose
font happened to be wide enough to tip it.

The fix is two lines and a consequence. `max-width: calc(100% - 2 *
var(--edge))` gives the pill its second bound; `overflow-wrap: anywhere` makes
that bound safe, since at 320pt the cap is 7px narrower than the word
"extension" is at AX5 and a min-content floor that cannot be met is text
painted outside the pill. The consequence is that the label takes three lines
at 320pt/AX5 instead of two, so `.scroll.has-fab` clears three: measured, the
list now clears the pill at every size and scale this gates on, by 14px at the
tightest.

And the rule is now stated rather than waited for — `GUTTER` in
`docs/audit.js` reports an out-of-flow box, positioned against a screen-wide
containing block, with one inset at `--edge` and the other side nearer the
screen edge than that. On the tree before the fix it reports 88 findings per
theme, at scales and widths where `OVERFLOW-X` was silent.

### The other eight findings were the font, and the thing behind them is not

CI's other complaint was `CLIPPED-X div.session-title scroll=80 client=76` on
the recipe list. Measured under the pinned faces, and under the real New York:
`scrollWidth == clientWidth` on every row. It was Liberation Serif — or rather
Liberation *Sans*, one element over, since `.session-age` shares the row and
"Aug 18" is 6.7px wider in it — and there is no clip to fix.

What is left when the font is taken out of it is worth writing down anyway.
`.session-age` is `flex-shrink: 0` inside a 210px `.session-head`, so at AX5 an
absolute date takes 118 of those 210 points and the title gets 84 — two lines
of about three characters. That is a real legibility problem at 320pt and a
smaller one at 402, no check reports it, and no font choice will: it is a
question about how much of a row a timestamp deserves, and it is open.

If you want numbers out of the real DOM rather than pixels, `document::eval`
reaches into the live WKWebView and can send `getBoundingClientRect` and
computed styles back to Rust — that is how the spacing in this design was
measured, and how the keyboard bug was finally diagnosed after two wrong
guesses. Give it ~1500ms: WebKit applies `env(safe-area-inset-*)` after first
paint, and an earlier read reports every bar 62px too high.

---

## The desktop shell

The same design system, worn a second way. `assets/shared.css` is untouched by
it: everything below lives in `assets/desktop/`, which `src/css.rs` embeds
only in a desktop binary.

**Fifteen files, and the sort is the cascade.** `assets/desktop/` is a
directory rather than one sheet — `00-tokens.css`, `10-sidebar-frame.css`,
`20-plane-switch.css`, `30-sidebar-list.css`, `40-home-chat.css`,
`50-band.css`, `55-panes.css`, `60-sidebar-extra.css`, `65-responsive.css`,
`70-overrides.css`, `80-measure.css`, `90-inspector.css`, `95-transcript.css`,
`97-home-code.css`, `98-home-sched.css` — because one 4278-line file is one
file every branch of a wide change appends to at once, and parallel appends had
already left it declaring `--insp-add`/`--insp-del` twice. The split was purely
mechanical: no rule was edited, reordered or reformatted, the parts are
contiguous over the original's 1–4278, each is brace-balanced on its own, and
`cat assets/desktop/*.css` reproduces the old file byte for byte. That
byte-identity is the whole safety argument — identical bytes are an identical
token stream and therefore an identical cascade — so a change that has to move
a rule between files is a *second* commit, never part of a split.

The numeric prefixes are zero-padded so that Rust's `concat!` order in
`src/css.rs`, `readdirSync().sort()` in `docs/audit.js` and `sorted()` in
`scripts/capture-gallery.py` all produce the same sequence: **the filename sort
IS the cascade order**, and a sheet that lands in a different slot in any one of
those three is a different window from the one that ships. Leave gaps when you
number a new region. `assets/platform/macos.css` is appended after the sort in
all three, never swept into it — `src/app.rs` emits STYLES, then SHELL, then
PLATFORM, and the platform sheet has to win the `--chrome-h`/`--traffic-w`
declarations it shares with `00-tokens.css` at equal specificity.

Two files are named for where they *are*, not where they belong:
`60-sidebar-extra.css` is sidebar rules that were appended after the band's,
and `98-home-sched.css` is home-screen rules that were appended after the Code
half's. Moving either would forfeit the byte-identity proof, so they stayed put.

**Platform and width are different axes.** The platform decides the
*affordances* and it decides them at compile time — the desktop shell has
always-visible row actions, no swipe tray and no pull-to-refresh; the phone
shell has all three, unchanged. Width decides one thing and one thing only:
how many columns are drawn. **A desktop window dragged narrow is a narrow
desktop app, not the phone.** Nothing about how a control behaves changes with
width. goose's own desktop app takes the same position — its single width
threshold collapses the sidebar and changes nothing else.

**The columns, and where the numbers come from.** Sidebar 268, inspector 344 —
both read out of the mockups' own CSS (`grid-template-columns: 268px
minmax(0,1fr) 344px`) — and a content column no narrower than 360, which is
this file's own floor (rule 14's note: 320 is a defensive width rather than a
device) and the width `docs/measure-composer.js` is gated at. The breakpoints
are their sums rather than round numbers:

| width      | columns                                                    |
|------------|------------------------------------------------------------|
| ≥ 972      | sidebar 268 · content · inspector 344                      |
| 704 – 971  | content · inspector, with the sidebar floating over it     |
| 628 – 971  | sidebar 268 · content                                      |
| < 628      | content, with the sidebar floating over it                 |

The window opens at **1440x860** and that is arithmetic rather than deference:
268 + 344 is 612 of chrome, so at the 1180 this used to open at the content
column would have been 568 — under the 640 `--measure` the pane is built
around, and the inspector would have shipped having quietly narrowed the thing
it comments on. 860 rather than the mockups' 900 because a 1440x900 display has
about 875 of usable height once the menu bar is out.

There is no icon rail. It worked while the sidebar held seven destination
labels and cannot survive it holding a session list — 14px of content box is
not a session title — so below the two-column width the sidebar floats over the
content at its full 268 instead. The sidebar *column* is 268; the card inside
it is that minus 8 points of breathing room on each side, so the panel itself
is 252.

**The nav is a floating card, and it collapses.** It is inset from the window
by 8 points on three sides, rounded at `--radius-xl` and outlined — the phone's
"chrome floats" grammar, and the same treatment goose's own desktop app gives
the same panel ("a rounded outlined card floating on it with breathing room",
`AppLayout.tsx`). The content beside it is the plain canvas: no card, no
border.

It starts **open on every launch** and nothing persists the choice. The toggle
is `⌘/` — goose's own chord for this (`toggleNavigation`) — or the button, which
is a sibling of the columns rather than a child of the nav, because a control
that vanishes with the panel it reopens is a one-way door. It slides between
the card's top corner and the window's. State lives in a `use_signal` local to
`AppShell` and reaches the sheet as `data-nav`; it is deliberately *not*
`ctx.drawer_open`, which on a phone means "a panel is covering the screen" and
carries two behaviours that are wrong for a pinned column — `render_group`
closes it on every navigation, and `views/scheduler.rs` stops polling while it
is open.

**The window's floor is 480 × 560 points**, set with
`WindowBuilder::with_min_inner_size`. 480 is the rail plus 424 of content —
wider than the 402 frame every gallery state is audited in — and it is also
exactly what goose's own desktop app ships as its minimum. 560 is the nav's
intrinsic height (seven destinations at 48px, plus the wordmark and the
padding) with slack. A test in `src/shell/desktop.rs` checks the floor against
the breakpoints, because only one of the two is compiled.

**Nothing observes a DOM resize.** The breakpoints are `@media` rules in a
stylesheet a phone binary does not contain, so pane count needs no Rust at
all. That is not tidiness — it is the synchronous-XHR rule above: a Rust
`onresize` handler would be a blocking round trip per frame of a drag. The one
thing CSS cannot work out for itself is whether a detail is open, and that is
a fact about the app rather than the window, so the shell states it in a
`data-detail` attribute.

This paragraph read "**Nothing observes a resize**" until `use_fullscreen`
landed, and the qualifier is the whole of the difference rather than a hedge.
`src/shell/desktop.rs` does now take a `tao` `Resized` — as a *trigger*, to
read `window().fullscreen()` back off the window, because tao publishes no
fullscreen event of its own. A `tao` event is not a DOM event: it is already in
this process's event loop and reaches the closure by a function call, so none
of the per-frame cost the rule is about is paid, and a `peek` guard means the
signal is written on the transition rather than on the frame. Nothing about the
column count travels that way, and nothing may start to.

**The window's own bar says what the window has open.** `.shell-chrome` is
the band the traffic lights are painted in, and it carries three things: the
nav toggle, the name of whatever the detail column is showing, and the
connection. `assets/desktop/` then takes that same heading back out of the
pane below it (`[data-detail="open"] .pane-detail .topbar > .title`), so there
is **one title per window** where there used to be one per column.

Only the detail's, and only when there is one. A LIST keeps its heading on the
canvas at every width, in every state, so the list column never moves — which
is also the reference's arrangement: its band is an empty drag strip and
`SessionActionsHeader.tsx` returns `null` when no session is open, while
`SettingsView`, `SessionListView` and the rest each keep a big `<h1>` inside
their own content column.

**Leading-aligned, and that is measured rather than preferred.** The reference
centres its band title on the window, and it can: it has no list column, so its
window centre *is* its content centre. Ours is not. At a 902pt window the
detail column's 40rem measure centres **271px** from the window's centre, and
with the nav shut at 1400 it is **210px** out — a centred title would name the
pane it is not over. Anchored to the toggle it starts at x=122 at every width,
in every theme, with the nav open or shut (measured on all seven window sizes).
That also removes a real motion: the detail title used to be padded by
`--gutter`, which is computed from the pane's own width, so collapsing the nav
at 1400 slid the heading 486 → 380, 106px sideways over 200ms.

Recorded and not built: giving `.shell-chrome` the same flex template as
`.shell-body` — a `var(--nav-w)` spacer, then a `clamp(330px, 30%, 460px)` one
— would put the title over the pane it names at every tier, in CSS, with no
Rust observing anything. It costs back the slide above. Reconsider it if
leading alignment reads wrong once somebody has lived with it.

**The title travels as data, because Dioxus has no portal.** Nothing rendered
inside a pane can be *moved* into the band, so `nav::Destination::detail`
returns a `Detail { crumb, view }` instead of a bare `Element`: one function
answers "what is open" and "what is it called" at once, and a screen therefore
cannot have a detail without a name. Each screen's derivation is a
`pub(crate) fn` beside its view that the view itself reads, so the pane and the
window cannot end up calling the same thing two different things. Five headers
in `views/code.rs` and `views/chat.rs` are hand-rolled rather than
`views::chrome::TopBar`, and that is exactly why the pane's copy is removed in
**CSS** and not with a Rust conditional: `.topbar > .title` reaches all
fourteen `TopBar` call sites and all six hand-rolled headers alike, where a
branch inside the component would have left five of the nine detail screens
painting a second title.

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
The open row takes `--bg-tertiary`, which is the token `assets/shared.css` already
proved reads as *selected* (see the note over `.drawer-item.active`), and
`ListRow` refuses to paint it at all on the phone.

### Deviations, desktop only

**The pane header does not float, and it draws no line.** Rule 3 — "each
carries its own glass, nothing spans the width" — is the phone's identity and
cannot survive three columns: `.topbar` is positioned against `.app`, so three
of them would stack in one corner, and three glass pills abreast is three where
the eye expects one. On the desktop the bar is static: the title sits on the
canvas in a 4rem band of air, and the controls keep their shapes and lose their
shadows.

The first draft put a rule under it, on rule 6's reasoning that anything in
normal flow separates with a hairline. That was a misreading of what the line
had to separate: the bar and the pane under it are the same colour and nothing
scrolls beneath it, so the rule separated nothing — while meeting the column
divider at a right angle once per pane. A window whose structure is drawn in
corners is exactly the "boxy" this system exists to avoid. goose's own desktop
app draws no such divider either. The one line the shell still draws is
between the two content columns, which are two different things.

**The connection keeps its words here — above the rail tier, and nowhere
else.** Rule 8 says "in a bar, the dot is *all* you get", and `assets/shared.css`
enforces it with `.topbar .conn-label { display: none }`. The reason that rule
gives is specific — "the agent name and version are ~107px of text that a
**centred** title cannot clear" — and it does not survive the move: nothing in
`.shell-chrome` is centred, and at a 1400pt window the strip this sits in was
1278px of nothing. So the window's bar shows `● goose-mock 1.47.0`, which is
what the reference paints in the same place (`EnvironmentBadge`,
`BaseChat.tsx`), and both panes give theirs up at every width.

That last part replaced a `@media (min-width: 902px)` that hid only the
*detail's* copy. The media query was correct about the count and wrong about
the consequence: dragging the window across 902 made the badge jump between
columns, so the state of the connection appeared to move because the layout
reflowed. One per window is now structural.

**And the premise inverts at the narrow end, so the rule does too.** "1278px
of nothing" is an argument about width, and at the 480pt floor the same strip
is at its 96px minimum and the badge is the widest thing in the bar. So
`@media (max-width: 571px)` — the rail tier, the same breakpoint the nav
collapses at — takes `.conn-label` back out, and the band is a dot again.
Nothing is lost by it: the dot carries the whole state in colour, and the label
was only ever the identification of a control. This is the phone's argument
arriving at the phone's answer, and it takes the phone's own rule rather than
inventing a second one.

Which of the two gives way when they cannot both fit is `assets/shared.css`'s
decision, not a new one. `.conn-label`'s own note says the badge holds its
name and "the title beside it shrinks first — it has min-width 0 and its own
ellipsis, and a truncated title is normal", so the badge is `flex: 0 0 auto` —
and that is exactly why the label has to go at the floor rather than be allowed
to shrink there. Measured at 480×560 with the longest string in the audit's
stress table, on `desktop-chat (long text)`, both sides of the change:

| | title group | `.conn-badge` | `.window-drag` |
|---|---|---|---|
| with the label (the shipped bug) | 119px, ellipsised out of 417 wanted | 135px | 96px |
| without it (today) | 236px | 18px | 96px |

The drag strip is at its 96px floor either way, so the window is no harder to
move; the 117px comes entirely off the badge. Nothing at 572pt and above moves
at all. Reproduce it by putting the rail rule back — append
`@media (max-width: 571px) { .shell-chrome > .conn-badge .conn-label { display:
inline } .shell-chrome > .conn-badge { padding-right: 12px } }` to
`assets/desktop/98-home-sched.css` and re-run the axis. The *last* file in the
sort, because the point of an append is that the later copy wins.

**And inside the title group the qualifier yields to the name.** Both were
`flex: 0 1 auto`, which takes a deficit off two items in proportion to what
each asked for — so the longer the subtitle, the more of the title it cost.
Measured on the captured `desktop-scheduler-detail` at 572: the group had
211px, the subtitle spent 95 of it on *Runs every day at 2:30 AM*, and the
heading was cut to 108 — narrower than the badge beside it, which is the one
arrangement the band is not allowed to reach. `.chrome-sub` is `flex: 1 1 0`
now: it bids for nothing and grows into whatever the heading did not want, so
the heading keeps its intrinsic width until there is none left. Same window
after: heading 173px, uncut, subtitle 30px, and at 1180 the group is still
exactly heading plus gap plus the subtitle's own width.

**The pane header keeps its controls and loses its title.** With the window's
bar naming the detail, the detail's own `.topbar` is a back chevron and
whatever the screen puts in `.topbar-actions`. Both stay in the pane on
purpose: at three columns the chevron closes the DETAIL while the list stays on
screen, and in a window-level bar that would read as a window-level Back —
and `.topbar-actions` is an `Element`, which is the one thing that cannot
travel. The bar keeps its 4rem, because that height exists so the two panes'
headers do not stair-step across the divider between them, and that is still
true of a header holding only a chevron.

**Nav colour is split by theme, and it is measured.** The nav card paints
`--bg-secondary` in light and `--bg-primary` in dark — i.e. in dark it paints
nothing at all and the outline does the separating. Both directions come from
the same measurement. One fill for both put a #3f434b panel on a #22252a page
(1.53:1, a pale slab bolted to a dark window) and dragged the selected
destination down with it: `--bg-tertiary` on that slab is 1.13:1, so the one
pill whose job is to say where you are was the faintest thing in the column.
Against the page it is 1.60:1 — which is where goose's own sidebar lands
(sampled: identical sidebar and canvas fills, a 1px border, a 1.58:1 selected
pill). Light has the opposite problem — #f4f6f7 on white is 1.09:1, a step you
have to look for — so it keeps its fill and the selected pill gains an inset
1px ring on top of it. **Light separates with fills, dark separates with
lines.** `--shell-line` follows the same split so the card's edge and the
column divider land at the same strength in both themes (1.44:1 and 1.42:1).

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
`assets/shared.css` has none at all; it was written for a thumb. Hover changes
colour and shows a button's border — it never *reveals* a control and never
moves one. Two corollaries that are easy to get wrong and were:

- **Hover may not borrow the selected colour.** `--bg-tertiary` is what
  `.drawer-item.active` paints with, and shared.css's own note says why ("at
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

### What the desktop axis covers

`docs/audit.js` walks **two grids**, and they are stated separately in its own
verdict line for the reason it states everything separately: an unnamed count
is exactly how a coverage claim rots. The phone's is 98 states × 2 themes × 6
phone sizes × 4 text sizes. The desktop's is **28 states × 2 themes × 7 window
sizes × 1 text size × 3 shell states**, and every one of those numbers is a
different claim from the phone's. (It read "2 nav states" until this pass, and
that was only ever true of the commit that landed the axis: `data-fullscreen`
became a third cell in the very next one and the count here did not follow.
`node docs/audit.js both` prints the live grid on its own verdict line — 2 × 7
× 3 is 42 desktop cells today — so a number written here that disagrees with
that line is this paragraph's error, not the script's.)

**And the desktop half is no longer optional.** Every desktop check is reached
by iterating states whose key starts with `desktop-`, so a gallery with none
runs none of them — measured, by deleting all 14 from
`docs/gallery-states.json`: **Clean**, with a summary line that simply stops
mentioning the desktop and reads as though there were nothing to mention. A
capture given only the phone's log produces exactly that file. `docs/audit.js`
now states what it is *for* and exits before launching a browser when the
gallery is not it: at least one state per shell, both answers to "is anything
open in the detail column", and every destination in `nav::DESTINATIONS`
captured at least once with the desktop nav marking it. That last claim is
asked of the **drawer** and not of the keys — `src/shell/mod.rs` writes
`class="drawer-item active" title="<label>"` for the destination on screen, so
the question is answered by the captured bytes rather than by a naming
convention `src/nav.rs` explicitly does not promise (`Screen::Chat` dumps as
`chat`, singular, and says why). Drop `desktop-code-list` and the run stops
with *no desktop state was captured with code open*.

**Nine window sizes, and the claim is deliberately weaker.** `SIZES` is a
coverage claim about *devices*: those are the phones this app is gated on and
there are no others. A window can be any size at all, so no equivalent claim is
available. What `DESKTOP_SIZES` is instead: **every breakpoint straddled on
both sides, plus the floor, plus the size the app opens at, plus a width past
every breakpoint** — 480×560 (`MIN_INNER`), 627 and 628 (the last pixel before
the sidebar floats, and `NAV + CONTENT_MIN`), 703 and 704 (the last pixel with
no inspector in any shell state, and the first at which a shut sidebar leaves
room for one), 971 and 972 (the two/three-column edge), **1440×860**
(`with_inner_size` at `src/main.rs:108`, and the reference size), and 1600×1000,
where the content column is the only thing still growing. Straddling is the
point: a breakpoint is precisely the number where one width renders one layout
and the next renders another, and only both sides can say the two agree.

**The 571/572 and 901/902 pairs are gone, and their absence is the point.**
They straddled the sums of a shell that still had a list column between the nav
and the detail, and both went out with it — as did `.conn-badge`'s own turn at
571 and the `clamp(330px, 30%, 460px)` that used to saturate at 1533. A pair
that straddles nothing measures one layout twice and reports the result as
agreement, which is the failure this list is shaped to avoid. The straddles
above are the sums the three-column grid actually has; `docs/audit.js`'s comment
beside `DESKTOP_SIZES` derives each one.

**One text size, and it was read off a window rather than derived.** The
four-scale walk is the Dynamic Type opt-in, and that opt-in is
`font: -apple-system-body` in `assets/platform/ios.css` — a line no other build
has. A macOS binary gets `assets/platform/macos.css`, which sets no font, so
nothing moves the root off the web view's default. Measured during the capture
with one `document::eval`: **16px root, devicePixelRatio 2**. That read was
taken in the 1180×820 the window opened at before the re-scale, which changes
nothing it was taken for: the root is the web view's own default and does not
move with the window.

**The shell's own state walks in place of the text axis.** `data-nav` and
`data-fullscreen` are plain attributes on `.shell` that only
`assets/desktop/` and `assets/platform/macos.css` read, so flipping them is
a real reflow of a real rule rather than a fiction — which is exactly what
separates them from `data-detail`, a fact about what the app has open that has
to be *captured* and must never be flipped. Three cells, chosen rather than
multiplied: **nav open**, **nav closed**, and **fullscreen**. The collapse
earns its pass because the rail and collapsed tiers are where the window chrome
gets crowded, and that is where this shell's one shipped regression lived.
Fullscreen earns its own because the band is the whole of what it changes — it
gives back the 76pt traffic-light indent and takes 10pt of padding — so
`closed x fullscreen` would measure the collapsed cell twice and nothing else.

That both attributes are *written by the render* is what makes this legitimate,
and it was not always true. `data-fullscreen` used to be set from JS, inferred
from `innerHeight >= screen.height - 2`, and that comparison never once matched
a real fullscreen window: the rule below was dead in every window that ever
ran, and no frame here rendered it either. Two independent gaps, each of which
made the other invisible. It is now read off `tao` on a `Resized` and written
onto `.shell` beside the other two — verified by driving a real window in and
out of fullscreen and watching the attribute follow.

**Rendering that block is not checking it, and for one release it was not.**
The sabotage originally recorded here — 672 findings for the fullscreen rule —
was measured by setting `--traffic-w: 900px`, which physically overflows the
window and is caught by `OVERFLOW-X` like any other 900px box. It is not the
regression this rule has. Re-measured on the shipping grid: set the rule's
`--traffic-w` back to `0px` and the run is **Clean**; break its selector so the
whole block goes dead — which is the regression it *already shipped once*, back
when the flag came from a JS guess that never matched — and the run is
**Clean** as well. The third cell was a 50% growth of the desktop grid that
could not fail. `FULLSCREEN` is the check that closes it, and it states the
block's two numbers as two questions: the reservation is gone (**392**, with
`--traffic-w` held at 76), and with the lights gone the toggle centres in the
band like it does on every platform that never had them (**392**, with
`--chrome-pad` held at 0). Both, for the dead block: **784**.

**The desktop sheets are linked per state, never from a directory.** This is
why `assets/desktop/` and `assets/platform/macos.css` are deliberately *not*
in `assets/features/`: the audit links that whole directory into every 402×874
phone frame. Building the `<link>` list from the state instead makes it
structurally impossible for a desktop rule to reach a phone frame. The cost of
getting it wrong is measured rather than feared — copy them into
`assets/features/` and `node docs/audit.js light` against a clean tree reports
**716 findings** (438 OVERFLOW-X, 266 SPILL, 12 CLIPPED-X). A phone state laid
out as three columns is not a phone state. Their **order** is the other half of
that care, in two parts. Between the two sides: `00-tokens.css` and
`macos.css` both set `--chrome-h` and `--traffic-w` on the same
`.app > .shell` selector, so at equal specificity the later sheet wins, and the
wrong order is the one that made the reservation always zero. An audit linking
them that way would measure a window nobody ships and report it clean. And
*within* `assets/desktop/`, the readdir sort has to reproduce `src/css.rs`'s
`concat!` exactly, which is what the zero-padded prefixes are for — the audit's
list is computed and the app's is written out, and they are only the same
cascade because both are the same sort.

#### The calibration, measured

A check you cannot make fail is worth nothing, so each of these was made to
fail on purpose and the number written down. All against a tree the axis calls
clean.

**And a number is only true of the grid it was taken on.** The rows marked *(2
cells)* below were measured before the fullscreen cell existed and have not
been re-run; every other row in both tables was re-measured on the shipping
three-cell grid for this pass, because a calibration nobody re-derives is a
calibration that quietly stops describing the gate. The rescaling is not a
factor you can apply in your head: `TITLE-TALLER` went 196 → **588**, three
times rather than one and a half, because the two extra cells reach frames the
first one did not.

**And the width grid has moved under both tables since.** Every "where" below
names the seven widths `DESKTOP_SIZES` walked when the row was taken — including
571, 572, 901, 902 and the 1180×820 that was then the reference cell — and the
list is nine now, straddling 627/628, 703/704 and 971/972 with the reference at
1440×860. The counts are left as measured rather than rescaled, because a number
nobody re-ran is not a number: read each row as *this sabotage was caught, and
here is roughly how loudly*, and re-derive it on today's grid before quoting it
at anything. The one claim that does survive the move unchanged is the shape of
each row — which check fired, and whether it fired everywhere or only at the
floor.

| put back | reported | where |
|---|---|---|
| the toggle absolutely positioned against `.shell` and centred in the nav column — **the regression that shipped** | 112 CHROME-SLOT *(2 cells)* | 480×560 and 571×700 **only**, never 572 and up |
| the same collision behind `[data-nav="closed"]` | 392 CHROME-SLOT *(2 cells)* | nav **closed** only, all seven widths |
| `--text-secondary: #bbbbbb` on `.app > .shell` | 361 CONTRAST + 156 ICON-CONTRAST | 1180×820, nav open — the reference cell |
| `border-radius: 0` on `.session-item` | 1804 SQUARE | every width |
| `min-height: 0; height: 0` on `.setting-row` | 196 COLLAPSED *(2 cells)* | every width, both nav states |
| `margin-top: -20px` on `.chrome-title` | 588 TITLE-TALLER (+ 588 SPILL) | every width, every shell state |
| `margin-left: -40px` on `.chrome-title` | 588 TITLE-COLLIDE, all `overlaps button.nav-toggle by 32px` | every width, every shell state |
| `flex: 1 1 0` on `.window-drag` (i.e. letting it shrink) | 180 DRAG-GONE, down to `0px wide` | 480 through 902, silent at 1180 and up |

How each row is reproduced: **append** the declarations to the sheet named and
re-run. Appending is what makes them faithful without a rewrite — every one of
these is at the same specificity as the rule it is fighting, so the later copy
wins, and a `git checkout` of the one file puts the tree back. Append to
`98-home-sched.css`, the last file in `assets/desktop/`'s sort: appending into
the middle of the directory puts the copy *before* the rule it is meant to
beat, and the row silently fails to reproduce.

| put back, appended to `assets/desktop/98-home-sched.css` | reported | where |
|---|---|---|
| `.nav-toggle { position: absolute; z-index: calc(var(--z-chrome) + 1); top: 20px; left: calc(var(--nav-w) - var(--shell-gap) - 44px); margin-top: 0 }` — **the regression that shipped** | **224 CHROME-SLOT** and 426 TITLE-COLLIDE (650) | the slot findings at 480×560 and 571×700 **only**, never 572 and up — but in **nav open and nav closed alike**, 28 states × 2 themes × 2 widths × 2 cells |
| the same collision behind `[data-nav="closed"]` — `.shell[data-nav="closed"] .nav-toggle { position: absolute; z-index: calc(var(--z-chrome) + 1); top: 14px; left: var(--shell-gap); margin-top: 0 }` | **392 CHROME-SLOT** | nav **closed** only, all seven widths |
| `--text-secondary: #bbbbbb` on `.app > .shell` | **361 CONTRAST** and 156 ICON-CONTRAST (517) | 1180×820, nav open, **light only** — the reference cell, and a value that is only wrong against a white page |
| `border-radius: 0` on `.session-item` | **1804 SQUARE** | all 42 desktop cells; 24 states |
| `min-height: 0; height: 0; padding: 0` on `.setting-row` | **588 COLLAPSED** | all 42 cells, 14 per cell, 10 states |
| `margin-top: -20px` on `.chrome-title` | **588 TITLE-TALLER** and 588 SPILL (1176) | all 42 cells, 14 per cell — the 14 states whose band carries a title |
| `margin-left: -40px` on `.chrome-title` | **588 TITLE-COLLIDE**, every one of them `div.chrome-title overlaps button.nav-toggle by 32px` | all 42 cells |
| `flex: 1 1 0` on `.window-drag` (i.e. letting it shrink) | **180 DRAG-GONE** — 164 reading `0px wide` and 16 reading `6px wide` | **five** of the seven widths: 480×560, 571×700, 572×700, 901×760 and 902×760. Clean at 1180×820 and 1600×1000 |

**Two rows changed shape rather than just size, and both are worth reading.**

*The `.setting-row` row had stopped reaching its check at all.* `min-height: 0;
height: 0` alone now reports **488 SPILL and 252 SMALL-TAP and zero
COLLAPSED** — because `assets/shared.css` sets `box-sizing: border-box` and
`.setting-row` keeps `padding: 8px 12px`, so a row asked for zero height still
measures 16 and `COLLAPSED`'s `height < 1` test never fires. The sabotage had
quietly become a demonstration of two *other* checks. `padding: 0` is what
takes the last 16px out and puts the row back on the check it was written for.
That is the failure mode this whole table exists to catch, one level up: a
sabotage that goes on producing findings looks exactly like a sabotage that
still proves what it claims.

*The `.window-drag` row was wrong in three ways at once* — 70 against a real
180, `26px wide` against a real `0px`/`6px`, and "480×560 and 571×700 **only**"
against five of the seven widths. The straddle claim went with it: this demo
does **not** fall silent at 572, it goes on firing through 901 and 902 and only
stops at 1180. So `flex: 1 0 96px` is doing work across most of the band's
range rather than only at the floor, which is a stronger statement about the
declaration than the one it replaces — but it is not the tidy echo of the first
row that the paragraph below used to call it.

The last three rows arrived with the window's own bar and each covers a way it
can go wrong. `TITLE-TALLER` and `TITLE-COLLIDE` are the existing pane-header
checks pointed at `.shell-chrome` as well, which costs the phone grid nothing
because the phone has no such bar; `DRAG-GONE` is new, and it is the one that
guards a control you cannot see. `src/main.rs` hides the macOS titlebar, which
takes AppKit's own drag region with it, so `.window-drag` is the only thing
left that can move the window — and it is a flex sibling of a title, a badge
and a reservation that are all free to grow. Squeezed to nothing the window is
simply stuck, with no clipping, no overflow and nothing else out of place. It
fires on the ordinarily-captured title rather than only the stressed one, so
`flex: 1 0 96px` is doing work today rather than insuring against a
hypothetical — and re-measured on the three-cell grid it reaches further than
it was first recorded as doing: 480 through 902, down to a strip **0px** wide,
falling silent only at 1180 and above.

**One check had to be taught what "not rendered" means.** `TITLE-TALLER`
compared a heading's box against its bar's, and `assets/desktop/` now takes
the detail pane's heading out with `display: none` — which reports 0×0 at the
origin, "outside the bar" by arithmetic and inside it by every meaning the
check has. Re-measured at `d716047` by dropping the guard from a scratch copy
of the script: **588 findings**, 336 `h1.title.ellipsis 0..0` and 252
`div.titlegroup 0..0`, all on desktop states and none on a phone one. (It read
392 when it was written, under the two-cell grid.) The guard is
`getClientRects().length` rather than a zero-size test, because a box of zero
*height* is a real finding and an element with no boxes at all is not.

The first row is the one that matters most, because it is not hypothetical: the
nav toggle used to be absolutely positioned against `.shell` and slid between
the nav card's corner and the window's. At the rail width both of those are the
window's own corner, so the button painted **on top of the macOS close
button** — a control you cannot press sitting exactly where you press to close
the window. It was found by eye on a device, because nothing rendered a desktop
state and no check asked the question. `CHROME-SLOT` asks it now, and the fact
that it fires at 480 and 571 and falls silent at 572 is the straddle earning
its place rather than sampling. What it does *not* do is distinguish nav open
from nav closed — `--nav-w` is set by the width tier and not by `data-nav`, so
the toggle lands in the lights' corner in both — which is why that row reads
224 and not the 112 recorded when the table was written.

None of those eight runs put a single finding on a phone state: checked by
partitioning every run's output on the state key, and non-`desktop-` cells came
to zero in all eight.

**Owed: three checks with no calibration row.** `TITLE-DOUBLED`,
`CONN-DOUBLED` and `TITLE-OUTBID` are in `docs/audit.js` today and none of them
appears in the table above. Their commits each name a number — 392
TITLE-DOUBLED and 632 CONN-DOUBLED with the three `display: none` rules
removed; 52 TITLE-OUTBID with the rail rule removed — and **none of those three
numbers has been re-derived here**, so none of them should be read as current:
every one was measured before the grid's third cell, and the TITLE-OUTBID
figure was measured against the same rail rule this document's own connection
paragraph now describes. Do not scale them; run them. Whoever next touches
those three checks owes this table three rows, derived the way the eight above
were — the sabotage stated as declarations that can be appended, the count, the
cells, and a line confirming the phone grid stayed at zero.

#### The four checks that were checking nothing

The table above is the honest half. The window bar also arrived with four
checks that could not be made to fail at all, and each was found by trying —
which is the only way any of them are ever found. All four are repaired and
re-calibrated below, on the shipping three-cell grid.

| put back | before the repair | after |
|---|---|---|
| the fullscreen block's selector broken, so a fullscreen window keeps the 76pt reservation and the 0pt band padding | **Clean** | 784 FULLSCREEN (392 for either half alone) |
| the detail pane's heading wrapped one element deeper, with `.topbar > .title` broadened to match and the wrapper given `display: contents` — a refactor that moves not one pixel | **Clean**, with the name painted twice | 588 TITLE-DOUBLED |
| `.pane .topbar > .conn-badge` broadened to a bare `.conn-badge`, which takes the shell's copy with it | **Clean**, with no connection indicator anywhere | 1176 CONN-GONE |
| `flex: 1 0 300px` on `.window-drag` inside the `max-width: 571px` block — a crush confined to the tier where the badge is only a dot | **Clean** | 162 TITLE-OUTBID (138 the drag strip, 24 the title narrower than the 32pt button beside it) |

`TITLE-OUTBID` is the one that was blind to its own defect rather than merely
to a neighbouring one. It tested `.chrome-heading` for being cut and then
compared the badge against the whole `.chrome-title` **group**, so the more of
the line the subtitle took, the safer the title looked — and the defect it was
written for was still on screen: at 572×700 on the captured
`desktop-scheduler-detail`, unstressed, the group held 211px while the heading
inside it was cut to 108 and the badge spent 135 on an agent version. Pointed
at the heading it reports **16** on a tree that had been Clean. The fix is in
`assets/desktop/50-band.css`: `.chrome-sub` is `flex: 1 1 0`, so the qualifier grows
into what the name did not want instead of bidding against it — heading 173px
and uncut at the same window, and nothing changed where there is room. A large
`flex-shrink` was tried first and rejected as asymptotic: at a factor of 100
the heading still gives up 1.2px of a 65px deficit, which is a cut title that
only a tolerance can hide.

It was also structurally dead at the two narrowest windows on the grid, which
is where the band is most crowded: `.conn-label` is hidden at 571 and under, so
the badge there is an 18px dot that cannot outbid anything, and a badge-only
check has nothing to say. Every item on the band's line answers for itself now,
each against the floor the sheet gives it — `.window-drag` its documented 96,
`.nav-toggle`, `.chrome-sub` and `.conn-badge` zero — with `.traffic-slot`
deliberately excluded, because that room is not the app's to spend and
`FULLSCREEN` owns the one case where it should be given back.

#### The blind spots, named

- **The traffic lights themselves.** AppKit paints them, not the app;
  `.traffic-slot` is an empty reservation and the numbers in
  `assets/platform/macos.css` are measured off a real window. A browser draws
  an empty box where the lights are, so `CHROME-SLOT` can prove **nothing
  overlaps the reservation** and can prove **nothing about whether the
  reservation is where the lights land**. The second half stays a device check,
  beside the safe-area caveat above.
- **`:hover` and `:focus-visible`.** The app's first ones live in this sheet
  and a captured static DOM has neither. Forcing them (`page.hover()`, or CDP
  `CSS.forcePseudoState`) is a real next axis and not this one.
- **Motion.** Flipping `data-nav` starts a 200ms transition on `.navpane`, and
  desktop frames link a generated sheet that turns transitions off. Everything
  here is measured **at rest**, which is already true of the capture; the slide
  itself is unmeasured.
- **Four of the nine detail screens are not on this grid at all.** `Code` needs
  an opencode server, and there is none on the capture machine, so
  `code-new`, `code-chat`, `code-diff` and `code-pulls` cannot be reached in a
  desktop run: the Code list renders "Set the code server URL and password in
  Settings, then come back" and has no way in. Their band titles
  (`views::code::{new_crumb, chat_crumb, diff_crumb, pulls_crumb}`) are
  therefore held by unit tests and the compiler and by nothing that renders
  them. The phone's four `code-*` states in the gallery are unaffected —
  they were captured on a device against a real one.
- **`GUTTER` is phone-only.** It needs an out-of-flow box whose containing
  block spans the viewport, and `.pane` is `position: relative` on purpose so
  a FAB floats over its own column rather than the window. No desktop FAB's
  containing block is ever the viewport, and `.pane-detail .fab` uses
  `--gutter` rather than `--edge` anyway. It reports nothing on this grid, and
  a check that reports nothing reads like a pass — so it is written down here
  as a gap instead.
