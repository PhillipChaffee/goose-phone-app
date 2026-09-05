// Check the rendered UI for the mistakes that are easier to measure than to see.
//
//   npm i -D playwright        (Chromium only; see PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD)
//   node docs/audit.js         [light|dark|both]
//   node docs/audit.js fonts   (macOS only — see the font block below)
//
// Every screen state in docs/gallery-states.json is rebuilt as a standalone
// document at every phone size below — the gallery's <iframe> gives clean
// style isolation, but this needs the states as top-level pages — and walked
// for:
//
//   geometry  overflow past the viewport, text clipped without an ellipsis or
//             spilling a box that does not clip at all, filled or
//             fully-bordered boxes left at radius 0, buttons under 32px, any
//             child rounded more than the parent clipping it, a bar title
//             taller than the bar, floating chrome that has outgrown the
//             gutter it floats on, the chrome band still opaque where that
//             title sits with the scroller's padding still clearing it, and
//             rows that render nothing and therefore measure nothing.
//   occlusion an interactive element painted over by a box that neither
//             contains it nor sits inside it, with nothing full-screen between
//             the two — because a modal is meant to cover the page and a
//             covered button is not. Four defects found in one week lived in
//             cells this file already walked and reported Clean on.
//   contrast  every element carrying its own text, composited against the
//             first opaque background behind it, against 4.5:1 (3:1 for large
//             or bold text); every icon, which carries no text of its own and
//             so is invisible to that walk, against 3:1; and every indicator
//             that paints a fill and no text — a status dot has neither text
//             nor an `.icon` class, so both of the others walked straight past
//             one at 2.03:1.
//
// Colour is walked through BOTH of the app's theme mechanisms: the media query
// and `data-theme` on the root, the second set to the opposite of the first so
// that the attribute's own block is rendered alone rather than under an
// identical one.
//
// Each state is also repeated with server-supplied text swapped for the
// longest plausible value, because a captured state only ever shows the one
// string the app happened to be holding — and the geometry walk is repeated at
// every iOS text size and at every phone size, because a captured state was
// also only ever rendered at one of each.
//
// Exits non-zero if anything is found, so it can gate a change.
//
// What it cannot check: anything that needs a real device. Safe-area insets
// are zero in a browser, so the floating chrome sits higher here than it does
// on a phone. Positions are what the simulator is for.
//
// Text metrics used to be on that list, and being on it was what let the same
// commit come out Clean here and 24 findings on ubuntu-latest: an
// approximation nobody gates on is a caveat, and this gates on it. The faces
// are pinned now, close to iOS's by a measurement that is itself runnable —
// the block below is the whole argument, including what it still cannot see.
//
// What it is structurally blind to, and what covers it instead: this walks
// whole screens at whole phone sizes, and the composer's chip row is decided
// by how many COLUMNS it has rather than by which phone it is on — so
// docs/measure-composer.js sweeps that one row across six widths at one
// height, including two (390, 393) that are three points apart and were where
// the model chip was at its worst. The two lists are not the same list and are
// not meant to be: they share the 320 floor and the 360/375/390/402 middle,
// and each holds one width the other does not (393 there, 440 here) for a
// reason stated where it is declared. The text spilling out of a chip that
// SPILL below now catches is the check that script had and this one did not.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { chromium } = require('playwright');

const STATES = path.join(__dirname, 'gallery-states.json');
// Every stylesheet the app embeds, in the order src/css.rs concatenates them:
// assets/shared.css is the design system and comes first, and a feature brings
// assets/features/<name>.css of its own. Linking shared.css alone would rebuild
// every feature screen unstyled and then measure the result, which passes.
const ASSETS = path.join(__dirname, '..', 'assets');
const FEATURE_CSS = path.join(ASSETS, 'features');
const STYLESHEETS = [
  path.join(ASSETS, 'shared.css'),
  ...(fs.existsSync(FEATURE_CSS)
    ? fs.readdirSync(FEATURE_CSS).filter((f) => f.endsWith('.css')).sort()
      .map((f) => path.join(FEATURE_CSS, f))
    : []),
];
// A sheet that is present but EMPTY is the same fault stated quietly, and this
// walk cannot see it from the inside: `<link>`ing a zero-byte file fires the
// load event, and `document.styleSheets` counts a link that 404s too, so
// neither the wait nor the count can tell a styled document from an unstyled
// one. Measured: empty assets/features/skills.css and the run still reports
// **Clean**, because the two selectors it carries are the whole difference
// between a styled `.skill-body` and an unstyled one. Empty assets/shared.css
// and it is the loud version of the same fault — 73664 findings about
// `<textarea>`'s default 182x21 box and buttons at radius 0, which read as a
// design regression rather than as a missing file. One stat each, at startup,
// so neither outcome has to be diagnosed from the findings.
// The sheets a DESKTOP state is measured against, on top of the list above, in
// the order src/app.rs emits them: STYLES, then SHELL, then PLATFORM.
//
// ORDER IS THE WHOLE CARE HERE. Both sides set `--chrome-h` and `--traffic-w`
// on the same `.app > .shell` selector — assets/desktop/ defaults them and
// macos.css raises them — so at equal specificity the later sheet wins.
// src/app.rs carries the write-up of getting it the other way round: the
// reservation was always zero and the nav toggle painted on top of the macOS
// close button at the rail width. Linked in the wrong order this would audit a
// window nobody ships and report it clean.
//
// THE SHELL HALF IS A DIRECTORY NOW, AND SORTED IS THE CASCADE. src/css.rs
// concatenates assets/desktop/'s thirteen region files in filename order, so
// this list must be that same order or the audit measures a cascade the app
// never emits — two rules of equal specificity, and whichever comes last wins.
// The prefixes are zero-padded precisely so that readdirSync().sort() here,
// the const list in src/css.rs and sorted() in scripts/capture-gallery.py
// cannot disagree. macos.css is appended AFTER that sort rather than being
// part of it, because "macos.css last" is now a property of a partly-computed
// list: PLATFORM comes last by src/app.rs's order, and a name-sorted directory
// would put it in the middle.
//
// And they are linked PER STATE rather than from the phone's own directory,
// which is what makes it structurally impossible for them to reach a phone
// frame. That is exactly the property `assets/features/` cannot give —
// src/css.rs keeps all of these out of it for that reason — and the cost of
// getting it wrong is measured rather than feared: copy them into
// assets/features/, so every phone frame gets them too, and
// `node docs/audit.js light` against a Clean tree reports 716 findings — 438
// OVERFLOW-X, 266 SPILL, 12 CLIPPED-X. A phone state laid out as three columns
// is not a phone state.
const SHELL_CSS = path.join(ASSETS, 'desktop');
const SHELL_REGIONS = fs.existsSync(SHELL_CSS)
  ? fs.readdirSync(SHELL_CSS).filter((f) => f.endsWith('.css')).sort()
  : [];
// A COMPUTED LIST CAN COME BACK EMPTY, and the per-file guard below cannot see
// it: an empty list has no missing file and no zero-byte file in it, so every
// desktop state would render against macos.css alone and the run would go on to
// report whatever that is. The zero-byte case was already the loud one (73664
// findings for an empty shared.css); this is the quiet one, and it is the failure
// the split newly makes possible.
if (SHELL_REGIONS.length === 0) {
  console.error(`${SHELL_CSS} has no .css in it — every desktop state would be measured with the whole shell sheet missing`);
  process.exit(1);
}
const DESKTOP_SHEETS = [
  ...SHELL_REGIONS.map((f) => path.join(SHELL_CSS, f)),
  path.join(ASSETS, 'platform', 'macos.css'),
];
for (const sheet of [...STYLESHEETS, ...DESKTOP_SHEETS]) {
  if (!fs.existsSync(sheet) || fs.statSync(sheet).size === 0) {
    console.error(`${sheet} is missing or empty — every screen it styles would be measured against the UA defaults`);
    process.exit(1);
  }
}

// The prefix src/shell/mod.rs puts on a desktop dump's key, and therefore the
// only thing in docs/gallery-states.json that says which shell drew a state.
// It is a fact about the capture rather than a convention this file invents —
// see scripts/capture-gallery.py, which partitions on the same string.
const DESKTOP_PREFIX = 'desktop-';

// ── what this run is supposed to cover ──────────────────────────────────
// THE DESKTOP HALF WAS OPT-IN, AND THAT IS THE LOUDEST FAILURE ON THIS LIST.
//
// Every desktop check in this file is reached by iterating states whose key
// starts with DESKTOP_PREFIX, so with no such key nothing desktop-shaped runs
// — no window sizes, no shell axis, no window bar, no `.traffic-slot`, none of
// it. Proved rather than argued: delete all 14 `desktop-` states from
// docs/gallery-states.json and `node docs/audit.js both` reports **Clean**,
// with a summary line that simply stops mentioning the desktop and reads as
// though there were nothing to mention. A capture run given only the phone's
// log produces exactly that file, and a gate that passes because half its
// input went missing is worse than no gate: it is a green tick over an
// untested shell.
//
// So the run states what it is FOR and fails when the gallery is not it.
//
// ASKED OF THE DRAWER, NOT OF THE KEYS. The obvious shape is "every
// `nav::DESTINATIONS` id has a `desktop-<id>` state", and it is the wrong one.
// Dump keys are not destination ids and src/nav.rs says so where it defines
// them: `Screen::Chat` dumps as `chat`, singular, because "the gallery and
// docs/audit.js have been keyed on these names since before this table
// existed". A prefix match happens to work today only because `desktop-chats`
// also exists, and it would fail the moment somebody renamed a screen — which
// is a check that fires for the wrong reason, the other half of the same
// disease this file is being treated for.
//
// What the capture DOES record, in markup, is which destination the nav had
// marked while the dump was taken: `src/shell/mod.rs` renders
// `class="drawer-item active" title="<label>"` for it and nothing else. So the
// question becomes "was this destination ever the one on screen when a desktop
// state was captured", which is the coverage claim actually wanted, is
// answered by the captured bytes, and survives any renaming of keys.
//
// BOTH SOURCES, because either alone has a hole. src/nav.rs is the truth about
// what the app has — a destination added and never captured leaves the gallery
// self-consistent and stale, and only the source says so. The states are the
// truth about what was captured. Reading the labels out of the source and
// looking for them in the markup is what makes a stale capture a failure
// rather than a smaller grid.
const NAV_RS = path.join(__dirname, '..', 'src', 'nav.rs');
// `id` then `label`, adjacent and each on its own line, which is how every row
// of the table is written. Matching on the pair rather than on `Destination {`
// is deliberate: that string is also `impl Destination {`, the struct's own
// definition and the return type of `nav::current`, so counting it says 9
// where the table has 7.
//
// A REGEX OVER RUST IS A GUESS, and this one is checked rather than trusted.
// Reordering the two fields is a free change in Rust and would leave this
// matching nothing at all — a coverage claim over the empty set, which is the
// exact disease being treated. So the fields are counted independently and the
// two numbers have to agree: 7 pairs against 14 fields on the table as it
// stands. `pub id:` and `pub label:` in the struct definition are not counted,
// because the leading `pub ` is not whitespace and neither is followed by a
// quote.
const DESTINATION = /\n\s*id:\s*"([^"]+)",\n\s*label:\s*"([^"]+)",/g;

// The destinations a plane OPENS on, which the desktop sidebar reaches by its
// segmented control rather than by a destination row, and the WORD that
// control paints. Both read out of `nav.rs` rather than listed here, so a
// third plane does not silently reintroduce the gap this replaced.
//
// They have to be paired through the plane VARIANT and not through the label,
// because the two words differ on purpose: the destination is "Chats" and the
// segment is "Chat" — the half, not the list.
const PLANE_PRIMARY = /Plane::(\w+)\s*=>\s*&([A-Z_]+),/g;
const PLANE_LABEL = /Self::(\w+)\s*=>\s*"([^"]+)",/g;
const DESTINATION_FIELD = /\n\s*(?:id|label):\s*"/g;
const coverage = (states) => {
  const gaps = [];
  const phone = states.filter((state) => !state.desktop);
  const desktop = states.filter((state) => state.desktop);
  if (!phone.length) {
    gaps.push('not one state is keyed for the PHONE, so the six phone sizes and'
      + ' the four text sizes walked no markup at all');
  }
  if (!desktop.length) {
    gaps.push(`not one state is keyed \`${DESKTOP_PREFIX}…\`, so the whole desktop grid —`
      + ' its window sizes, its shell axis and every check only a window bar can fail —'
      + ' did not run; re-capture with both logs'
      + ' (scripts/capture-gallery.py /tmp/applog.txt /tmp/desktop.log)');
    return gaps;
  }
  const nav = fs.readFileSync(NAV_RS, 'utf8');
  const dests = [...nav.matchAll(DESTINATION)];
  // variant -> the segment's word. `Plane::label`'s arms are the first
  // `Self::X => "…"` run in the file after the enum, and `Group::header`'s
  // return `Some(…)` so they do not match this shape.
  // FIRST arm per variant, not last. `Plane::label` and `Plane::icon` have the
  // identical `Self::X => "…"` shape and `icon` comes second, so building this
  // with `new Map(...)` over every match silently kept the ICON name — the
  // gate then looked for a segment reading "message" and reported both
  // primaries as never captured.
  const planeWord = new Map();
  for (const [, variant, word] of nav.matchAll(PLANE_LABEL)) {
    if (!planeWord.has(variant)) planeWord.set(variant, word);
  }
  // primary destination id -> the word its segment paints.
  const PLANE_PRIMARIES = new Map(
    [...nav.matchAll(PLANE_PRIMARY)].map(([, variant, konst]) => {
      const m = nav.match(new RegExp(`const ${konst}: Destination = Destination \\{\\n\\s*id: "([^"]+)"`));
      return m && planeWord.has(variant) ? [m[1], planeWord.get(variant)] : null;
    }).filter(Boolean),
  );
  if (!PLANE_PRIMARIES.size) {
    gaps.push(`${NAV_RS} names no plane primaries this file can read, so the`
      + ' segmented control cannot stand in for a destination row and every'
      + ' primary would report as uncovered');
  }
  const fields = (nav.match(DESTINATION_FIELD) || []).length;
  if (dests.length * 2 !== fields) {
    gaps.push(`${NAV_RS} holds ${fields} destination \`id\`/\`label\` fields but only`
      + ` ${dests.length} that this file can read as a pair — the loop below would be`
      + ' checking a coverage claim over the wrong set, or over none');
  }
  for (const [, id, label] of dests) {
    // TWO SHAPES, because the sidebar stopped being one flat list.
    //
    // A library destination is still a `.drawer-item` and is still marked when
    // it is the one on screen — those bytes are `src/shell/mod.rs`'s, unchanged.
    // A plane's PRIMARY is not: `chats` and `code` are reached by the segmented
    // control and their list IS the sidebar's body, so nothing paints a marked
    // destination row for them. Asking for one is asking for markup the shell
    // deliberately does not emit, and before this was widened the gate blocked
    // every run with two gaps that no capture could ever close.
    //
    // What stands in for it is the segment's own selected state, which is the
    // fact this gate is actually about: was this half of the app ever rendered
    // in a window. Substrings and not a parse, for the reason below.
    const asRow = `class="drawer-item active" title="${label}"`;
    const word = PLANE_PRIMARIES.get(id);
    // THE ACTIVE SEGMENT'S OWN WORD, not the word anywhere in the state.
    //
    // The first version of this asked whether the body held
    // `class="plane-seg active"` AND the word — and every state holds both,
    // because the switch always paints both halves and only one of them is
    // active. It could not fail: dropping `desktop-code-list` from the store
    // entirely left the run Clean, which is the shape of check this whole
    // repository keeps getting caught by. Measured, then fixed.
    //
    // So: slice from the active segment to the end of that button, and ask
    // whether the word is inside THAT.
    const activeSegmentSays = (body) => {
      const at = body.indexOf('class="plane-seg active"');
      if (at < 0) return null;
      const end = body.indexOf('</button>', at);
      return end < 0 ? null : body.slice(at, end);
    };
    const seen = desktop.some((state) => state.body.includes(asRow))
      || (word !== undefined
        && desktop.some((state) => (activeSegmentSays(state.body) || '').includes(word)));
    if (!seen) {
      gaps.push(`no desktop state was captured with ${id} open — the nav offers it and`
        + ' this grid has never rendered it in a window');
    }
  }
  // AND EVERY STATE MUST CARRY A TITLE, which is the opposite of what this
  // asked for until the band became total.
  //
  // It used to demand BOTH answers to "is anything open", because the band was
  // two arrangements: a title when a detail column had something in it, and
  // the lights, the toggle and a drag strip when it did not. Six of thirteen
  // states were the second kind — an empty middle in the one strip that is on
  // screen at every width.
  //
  // `crumb_parts` (src/shell/desktop/mod.rs) is total now: an open detail, a
  // home screen and a destination's own root all produce a crumb, so the band
  // names something on every screen and `assets/desktop/`'s
  // `:has(.chrome-title)` suppression of the pane's own heading is
  // unconditional. The old check cannot pass any longer — it asks for a state
  // the shell no longer has — and the useful question inverts with it: a state
  // WITHOUT a title is now the defect, because it means a screen the window
  // cannot name.
  const untitled = desktop.filter((state) => !state.body.includes('chrome-title'));
  if (untitled.length) {
    gaps.push(`${untitled.length} desktop state(s) render no .chrome-title `
      + `(${untitled.map((s) => s.label).join(', ')}) — the band is total now, so a`
      + " state with no title is a screen the window cannot name AND one whose pane"
      + ' heading is hidden by `:has(.chrome-title)` without anything replacing it');
  }
  // AND EVERY DESKTOP STATE MUST CARRY THE COLUMN THE SHELL AXIS COLLAPSES.
  //
  // `DESKTOP_SHELL`'s fourth cell shuts the inspector and INSP-SHUT checks
  // that the room came back. Both are silent on a state whose markup has no
  // `.insp` in it at all: `querySelector` returns null, the width is zero, the
  // claim passes, and the cell walks a frame that was already collapsed. That
  // is the same disease this whole function treats one level down — a gate
  // reporting Clean because its subject went missing rather than because it
  // was right.
  //
  // Asked of the store rather than of the layout because it is a question
  // about the CAPTURE. src/shell/desktop/mod.rs:1143-1150 renders the column
  // unconditionally and lets the sheet hide it, so a desktop state without one
  // is either markup that stopped being rendered or a store older than the
  // change that stopped rendering it — and neither is something a viewport
  // sweep can tell you.
  //
  // The class as a whole TOKEN, not as a substring: `.insp-toggle` lives in
  // the band and would answer for the column it opens.
  const noInsp = desktop.filter((state) => !/class="(?:[^"]*\s)?insp(?:\s[^"]*)?"/.test(state.body));
  if (noInsp.length) {
    gaps.push(`${noInsp.length} desktop state(s) render no .insp column `
      + `(${noInsp.map((s) => s.label).join(', ')}) — the shell axis's `
      + '`data-insp: closed` cell would collapse nothing there and INSP-SHUT would'
      + ' pass on an empty question');
  }
  // AND SOME STATE IN EACH SHELL MUST STILL CARRY A JUMP-TO-LATEST DISC.
  //
  // #250's axis below flips `body.away-from-bottom` so the disc is in the hit
  // test, and it walks the states that have one. A store with none left is the
  // same silence one level down: the axis runs over nothing, the summary goes
  // on naming it, and the only element in the app that is `pointer-events:
  // none` until JS says otherwise is unwalked again.
  //
  // PER SHELL, because the two discs are different rules. `assets/shared.css`
  // centres it on the reading column and `assets/desktop/80-measure.css` takes
  // it off that column and off `--z-chrome` (#209) — so a phone transcript in
  // the store says nothing about the rule the desktop actually ships.
  for (const [half, list] of [['phone', phone], ['desktop', desktop]]) {
    if (list.some((state) => stateIsAway(state.body))) continue;
    gaps.push(`no ${half} state carries a \`.scroll-bottom\`, so the`
      + ' away-from-bottom axis walked nothing in that half — the jump-to-latest'
      + ' disc is `pointer-events: none` until that class is on, which is the whole'
      + ' reason the OCCLUDED walk could not see it before #250');
  }
  return gaps;
};

// ── the faces this measures with, and why they are not the app's ────────
// THE GATE SHIPS ITS OWN FONTS. Every check below is a measurement of text,
// and until this block existed the text was laid out in whatever face the
// host machine happened to have — so the same commit was Clean here and 24
// findings on ubuntu-latest. Not a flake and not a difference of opinion
// about geometry: a difference of advance widths.
//
// What each environment actually resolved, measured with CDP's
// CSS.getPlatformFontsForNode against assets/shared.css's three stacks:
//
//   iOS            San Francisco / New York / SF Mono — the first entry of
//                  each stack, which is the whole design intent.
//   macOS Chromium `.SF NS` (matched at BlinkMacSystemFont, not at
//                  -apple-system) / **Charter** / **Menlo**. `ui-serif` is
//                  unmapped and the installed family is the hidden
//                  `.New York`, so the serif stack falls through two entries.
//   ubuntu-latest  Liberation Sans / Serif / Mono. `npx playwright install
//                  --with-deps` always installs its `tools` dependency group,
//                  and for ubuntu24.04 that group is where `fonts-liberation`
//                  comes from; fontconfig's metric aliases then answer `Arial`
//                  with Liberation Sans, and the serif and mono stacks reach
//                  their bare generic.
//
// Liberation Sans is ~5% wider than San Francisco for the strings on these
// screens and Liberation Serif ~8% narrower than New York, which is enough to
// decide a pass at 320x568 at root 53px. So the three tokens are repointed at
// three files in docs/fonts/ and every run everywhere lays out the same
// glyphs. The verdict is now a property of the commit.
//
// WHAT THIS COSTS, said plainly, because it is not free:
//
//   * These are not the faces anybody sees. No open licence ships San
//     Francisco or New York, and vendoring Apple's own files is not
//     redistributable — so the audit measures stand-ins and no run here,
//     on any machine, is a measurement of an iPhone's text.
//   * They are stand-ins CHOSEN by measurement, not by taste, and the
//     measurement is `node docs/audit.js fonts` — every string and every word
//     in gallery-states.json and in LONGEST, at all four roots, against the
//     real /System/Library/Fonts files. It runs on a Mac only, since only a
//     Mac has those files, and it fails if a median leaves ±5%. Today the
//     widest median is 4.3%: Inter runs 0.98-1.04 of San Francisco, Literata
//     0.97-1.04 of New York, and JetBrains Mono is 1.000 of SF Mono at every
//     size and every string, both being fixed-pitch and `size-adjust` closing
//     the one ratio between them. Liberation, which is what a Linux runner
//     was answering with, sits at 0.94 sans and 0.86 serif.
//   * A stand-in is close in the median and not in every word: the same run
//     reports single tokens 12% out either way, so this walk cannot
//     adjudicate the last few pixels of a text box. A design that FITS here
//     by a few pixels is not thereby proven to fit on a phone — which is a
//     claim about the design and not about the runner. `.fab` in
//     assets/shared.css grew a `max-width` for exactly that reason, and the
//     GUTTER check below states the rule it was relying on arithmetic for.
//   * Size-specific tracking and Core Text's shaping are unmeasured, here and
//     before. Optical sizing is NOT on that list any more, and it is why both
//     stand-ins are the `-standard-` builds rather than the `-wght-` ones
//     that are half the size: San Francisco tightens as it grows, and against
//     the wght-only Inter — no `opsz` axis, so no tightening — the same word
//     was 1.01x San Francisco at 16px and 1.15x at 53px. That 15% was
//     reporting spills at AX5 that no iPhone has. With the axis in, 1.00 and
//     1.04.
//   * Positions on a device are still what the simulator is for.
//
// And the pinning is CHECKED rather than assumed: the two guards below ask
// the browser which families it really used, and fail the run — not the
// check, the run — if a glyph reached a host font. A pin nobody verifies is
// how this problem comes back.
//
// THE VERTICAL METRICS ARE NOT THE STAND-IN'S. A face brings two things to a
// layout: how wide each glyph is, and how tall a line of it stands. The first
// is what a stand-in can only approximate. The second is a pair of numbers in
// the font's header, and `ascent-override` / `descent-override` /
// `line-gap-override` let this run state them — so every line box here is
// exactly as tall as the iOS face's, and the whole class of finding that is
// only about a taller stand-in never happens.
//
// Not a nicety: Literata is a 149% line box at `line-height: normal` against
// New York's 119%, and without these three lines the pinned run reported 96
// findings that were all one thing — `CLIPPED-Y h1.title.ellipsis scroll=83
// client=80` and `SPILL div.bubble-text content leaves the box by 2px`, at
// root 53px, in every state that has a bar title. Overridden, they are gone,
// and the SPILL and CLIPPED-Y checks go back to being about the design.
//
// Measured on this Mac with canvas TextMetrics against the real files, at
// 100px: SFNS.ttf (which `BlinkMacSystemFont` resolves to, byte-identical
// numbers) 97 up / 21 down / 118 normal; NewYork.ttf 95 / 24 / 119;
// SFNSMono.ttf 97 / 21 / 118. Ascent plus descent is the whole normal line
// box in all three, so the line gap is zero and is stated as zero rather than
// left to the stand-in.
const FONT_DIR = path.join(__dirname, 'fonts');
const FONTS = [
  {
    token: '--font-sans',
    family: 'Audit Sans',
    file: 'inter-latin-standard-normal.woff2',
    // Inter, OFL 1.1, from @fontsource-variable/inter 5.3.0. Drawn as a UI
    // face on the same brief San Francisco was, and the closer of the two
    // measured against it — the other being Liberation Sans, which is what
    // the runner had been answering with, at 0.94.
    // The face it stands for, and where a Mac keeps it: `node docs/audit.js
    // fonts` measures one against the other. Named here rather than in that
    // function so the claim sits beside the choice it justifies.
    standsFor: 'San Francisco',
    applePath: '/System/Library/Fonts/SFNS.ttf',
    metrics: { ascent: 97, descent: 21, lineGap: 0 },
  },
  {
    token: '--font-serif',
    family: 'Audit Serif',
    file: 'literata-latin-standard-normal.woff2',
    // Literata, OFL 1.1, from @fontsource-variable/literata 5.3.0. A screen
    // reading serif, which is what --font-serif is for; the tightest spread
    // against New York of the five tried (Literata, Source Serif 4, Noto
    // Serif, Charis SIL, Liberation Serif).
    standsFor: 'New York',
    applePath: '/System/Library/Fonts/NewYork.ttf',
    metrics: { ascent: 95, descent: 24, lineGap: 0 },
    // Literata runs wide against New York, and by more as it grows: medians
    // of 0.99 at a 16px root and 1.06 at 53. One scalar cannot fix a
    // distribution, but it can centre it — 0.977 is the geometric middle of
    // that range, and it takes the widest median deviation from 5.9% to 4.3%,
    // inside the +-5% the comparison asserts. It buys nothing about the
    // SPREAD, which stays what it is; see the cost list above.
    sizeAdjust: 0.977,
  },
  {
    token: '--font-mono',
    family: 'Audit Mono',
    file: 'jetbrains-mono-latin-wght-normal.woff2',
    // JetBrains Mono, OFL 1.1, from @fontsource-variable/jetbrains-mono
    // 5.3.0. Fixed-pitch against fixed-pitch, so this is the one of the three
    // where the horizontal match can be made EXACT rather than close: SF
    // Mono's advance is 0.61816em and JetBrains Mono's is 0.6em, one ratio
    // for every glyph at every size, so `size-adjust` closes it and the mono
    // slabs are measured at iOS's own column width. Nothing like it is
    // available for a proportional face, where the ratio is a distribution
    // and not a number — see the cost list above.
    standsFor: 'SF Mono',
    applePath: '/System/Library/Fonts/SFNSMono.ttf',
    metrics: { ascent: 97, descent: 21, lineGap: 0 },
    sizeAdjust: 61.8163 / 60,
  },
];
// The fourth face, which is not a design choice but an arithmetic one. The
// three above are LATIN subsets — 72, 110 and 40KB against the ~350KB of a
// full build — and this app puts one character outside that subset on screen:
// `⋯` (U+22EF), the "N unchanged lines" marker in the review screen
// (src/views/code.rs). Measured: not one of Inter, Literata, JetBrains Mono,
// Source Serif, Noto Serif, Charis SIL or Liberation has it even in its
// FULL build, so this is not a subsetting mistake to fix by shipping bigger
// files. Left alone it is one glyph resolved by the host — PingFang SC here,
// something else on a runner — which is the whole failure in miniature.
//
// So it is named as the next family after each of the three, and the run
// fails if any character reaches past it. 932 bytes: Google Fonts' css2 API
// will cut a face down to a given string, and this is Noto Sans Math holding
// ⋯ plus the two the inspector added below. Regenerate the same way if this
// list ever grows —
//   curl -H 'User-Agent: <a Chrome UA>' \
//     'https://fonts.googleapis.com/css2?family=Noto+Sans+Math&text=%E2%8B%AF%E2%9C%93%E2%86%92'
// — and follow the src: url() it answers with. Then CHECK the file rather
// than the CSS: see the measurement note below the list.
//
// A LIST, and the second entry is what made it one. The desktop's inspector
// prints a keyboard legend, so ⌘ (U+2318) and ⌥ (U+2325) are on screen — and
// no single free family carries all of the glyphs this app puts outside
// the Latin subsets. Measured, by regenerating and running: Noto Sans Math has
// ⋯ and neither key; Noto Sans Symbols 2 has both keys and not ⋯. So there are
// two faces, both named after each of the three above, and a glyph that
// reaches past BOTH still fails the run.
//
// The first face grew from one glyph to three, which is this check doing its
// job rather than a maintenance chore. The capture that put the code plane in
// the store connected for the first time, and the inspector's live subject
// draws two more characters: → (U+2192) between a branch and its base in
// `.insp-card-sub`, and ✓ (U+2713) as the reviewed tick in `.insp-file-tick`,
// both from `src/shell/desktop/inspector.rs`. Neither is in the Google `latin`
// subset — that range carries U+2191 and U+2193 but not U+2192, and no tick at
// all — so this stopped the run the moment the new states landed.
//
// WHICH FACE TOOK THEM WAS MEASURED, not assumed, and the obvious guess was
// wrong: Google's `css2` API answers `text=` with a `unicode-range` listing
// the codepoints you ASKED FOR whether or not the family has them, so a
// Symbols 2 subset requested with → comes back declaring U+2192 and renders
// it out of the host's Lucida Grande. `CSS.getPlatformFontsForNode` is what
// tells the two apart. Measured that way: Symbols 2 has ✓ and not →; Noto
// Sans (full) has neither, falling through to Zapf Dingbats; Noto Sans Math
// has BOTH, and already had ⋯. So the three live in one 932-byte face and the
// keys stay where they were.
const LEFTOVERS = [{
  family: 'Audit Leftovers',
  file: 'noto-sans-math-marks.woff2',
  standsFor: 'whatever the host would have chosen',
  // A glyph borrowed from a fourth face brings that face's ascent and descent
  // into the line box it lands in, so this one is overridden like the others.
  // San Francisco's numbers, which are also SF Mono's; the serif is 95/24 and
  // the two points of difference reach exactly one character on one screen.
  metrics: { ascent: 97, descent: 21, lineGap: 0 },
}, {
  // ⌘ and ⌥, for `inspector::CHORDS`. Same overrides, same reason.
  //   curl -H '<a Chrome UA>' \
  //     'https://fonts.googleapis.com/css2?family=Noto+Sans+Symbols+2&text=%E2%8C%98%E2%8C%A5'
  family: 'Audit Leftovers Keys',
  file: 'noto-sans-symbols2-keys.woff2',
  standsFor: 'whatever the host would have chosen',
  metrics: { ascent: 97, descent: 21, lineGap: 0 },
}];
for (const font of [...FONTS, ...LEFTOVERS]) {
  const file = path.join(FONT_DIR, font.file);
  if (!fs.existsSync(file) || fs.statSync(file).size === 0) {
    console.error(`${file} is missing or empty — without it this would measure the host's fonts and disagree with CI`);
    process.exit(1);
  }
  font.path = file;
}
// A data: URL rather than a relative one. The pages are built in a temp
// directory and a file:// document is an opaque origin, so a font fetched
// across to docs/fonts/ is a cross-origin font request and Chromium declines
// it — silently, which is the worst of the three outcomes. Inlined, there is
// nothing to decline.
//
// `font-weight: 100 900` because all three are variable fonts and the design
// uses four weights; declaring one weight would leave Chromium synthesising
// bold, which is a different advance width again.
// `size-adjust` scales the OVERRIDES too — measured: `size-adjust: 103.03%`
// with `ascent-override: 97%` reports a 100% ascent, not 97 — so the ratio is
// divided back out here. Stated once, in the one place that knows about both,
// rather than as three pre-divided numbers in the table above that would stop
// meaning "what SF Mono measures" the moment the ratio changed.
const face = (font) => {
  const adjust = font.sizeAdjust || 1;
  const pct = (v) => `${(v / adjust).toFixed(4)}%`;
  return `@font-face{font-family:"${font.family}";`
    + `src:url(data:font/woff2;base64,${fs.readFileSync(font.path).toString('base64')}) format("woff2");`
    + 'font-weight:100 900;font-style:normal;font-display:block;'
    + `ascent-override:${pct(font.metrics.ascent)};`
    + `descent-override:${pct(font.metrics.descent)};`
    + `line-gap-override:${pct(font.metrics.lineGap)};`
    + (font.sizeAdjust ? `size-adjust:${(font.sizeAdjust * 100).toFixed(4)}%;` : '')
    + '}';
};
// The stack each token is repointed at: the face, then the leftovers face, and
// nothing else. No generic at the end on purpose — `sans-serif` there would be
// the host quietly answering again, and the guards below would have nothing to
// catch because the browser would report a legitimate match.
// Single-quoted so the same string is legal both in a stylesheet and in a
// double-quoted style="" attribute, which is where the glyph guard puts it.
const STACK = (font) => [font, ...LEFTOVERS].map((f) => `'${f.family}'`).join(', ');
const FONT_CSS = [...FONTS, ...LEFTOVERS].map(face).join('\n')
  // Last sheet in the document, so this beats assets/shared.css's :root on
  // order at equal specificity — the same way the app's own later sheets do.
  + `\n:root{${FONTS.map((font) => `${font.token}:${STACK(font)};`).join('')}}`;

// A pin that is not checked is a pin that comes back. Two ways a host face can
// still get in, and one guard each:
//
//   the family  a rule that names a stack instead of a token — the tokens are
//               overridden, a literal `-apple-system` in a stylesheet is not.
//               familyLeaks() asks every element that lays out text of its own
//               what it computed, once per state, and anything outside the
//               four families above stops the run.
//   the glyph   a character none of the four covers falls through in the
//               ordinary way and takes its width with it. The corpus — every
//               character in gallery-states.json and in LONGEST — is rendered
//               once at startup in each stack and the BROWSER is asked which
//               platform faces it reached for; a non-custom face in that
//               answer is a host font, whatever its name.
//
// Both fail the run rather than adding a finding: a face that leaked makes
// every number in the run untrustworthy, so it is not one more thing to weigh
// against the others.
// PINNING THE FILE IS ONLY HALF OF IT: THE RASTERISER ROUNDS.
//
// The same woff2 does not produce the same advance widths on both machines.
// Chromium draws text through Skia, and on Linux Skia goes through FreeType
// with hinting on by default, which quantises a glyph's advance to fit the
// grid it hinted to; macOS has no equivalent step. Measured, with all four
// faces already pinned and the `.fab` bound deliberately taken back off:
// macOS reported 176 findings and `left=3`, `left=8`, `left=15`, and the same
// tree in mcr.microsoft.com/playwright:v1.62.1-noble reported 152 and
// `left=5`, `left=9`, `left=16` — the 15-versus-16 being a finding on one
// machine and silence on the other, which is precisely the failure this file
// set out to end, one layer further down than the font.
//
// `--font-render-hinting=none` turns that step off, and with it the two
// environments print the same bytes. It costs nothing this script measures:
// hinting is about where the ink lands on a pixel grid, and nothing here
// reads a pixel — the contrast walk reads computed colours, not a raster.
const LAUNCH = { args: ['--font-render-hinting=none'] };

const PINNED = [...FONTS, ...LEFTOVERS].map((font) => font.family);
const familyLeaks = (pinned) => [...new Set([...document.querySelectorAll('*')]
  .filter((el) => {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') return false;
    // Only elements that lay out text of their own: an <svg> inherits a
    // font-family it never uses, and reporting it would be noise.
    return [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
  })
  .filter((el) => {
    const fam = getComputedStyle(el).fontFamily;
    return !pinned.some((p) => fam.includes(p));
  })
  .map((el) => {
    const cls = typeof el.className === 'string' ? el.className.trim() : '';
    return `${el.tagName.toLowerCase()}${cls ? `.${cls.split(/\s+/).join('.')}` : ''}`
      + ` computes font-family: ${getComputedStyle(el).fontFamily}`;
  }))];

// Which platform faces the browser really reached for, asked of the browser
// rather than inferred from widths. CDP only reports the text nodes directly
// under the node it is given — a nested <span> is invisible to it, measured —
// so this is used on a scratch page whose text is a direct child, never on a
// captured state.
const platformFonts = async (page, selector) => {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send('DOM.enable');
  await cdp.send('CSS.enable');
  const { root } = await cdp.send('DOM.getDocument', { depth: 1 });
  const { nodeId } = await cdp.send('DOM.querySelector', { nodeId: root.nodeId, selector });
  const { fonts } = await cdp.send('CSS.getPlatformFontsForNode', { nodeId });
  await cdp.detach();
  return fonts;
};

// ── text scale ──────────────────────────────────────────────────────────
// The root font-size, in px, at each text size the app really runs at. Every
// --text-* and --lh-* token is a rem and rem means the root, so this one
// number moves the whole scale — which is the entire design of the Dynamic
// Type opt-in in assets/platform/ios.css.
//
//   16  what a browser gives by default, and therefore what Android's WebView
//       and the desktop dev build run at: the opt-in is iOS-only, so this is a
//       real shipping size, not a control.
//   17  iOS at Large, the default content size category. The app is 6.25%
//       bigger the moment it opts in, before anyone touches a slider.
//   23  iOS at xxxLarge — the largest NON-accessibility size, so the one a
//       reader reaches without going near the accessibility settings. It is
//       where --tool-gutter and the two-line bar title broke first.
//   53  iOS at AX5, the top of the scale.
//
// Set on the root directly rather than through `font: -apple-system-body`,
// which is what the app ships: Chromium cannot parse that keyword and leaves
// the root at 16px, and the fact the opt-in rests on is exactly that the
// root's px IS the body size — so stating it as a number is the same claim in
// the only form this browser can hear.
//
// GEOMETRY runs at every one of them; CONTRAST runs at the first alone.
// Contrast is not size-independent, but it is monotone: audit.js drops the
// required ratio from 4.5 to 3 above 18.66px, so the smallest scale is the
// binding case and the larger ones can only re-report what it found.
const SCALES = [16, 17, 23, 53];

// ── phone size ──────────────────────────────────────────────────────────
// A phone size the app runs at, ascending, on BOTH platforms it ships to —
// this app is iOS and Android from one codebase, and a list of iPhones alone
// would gate half of it. Height travels with width because a phone is
// not a column: the two failures this axis found are both "content is taller
// than the space it was given", and holding the height at the reference 874
// while narrowing invents a tall thin phone nobody owns and understates every
// one of them. Measured, before the fixes that came with this list: 375x874
// reported 24 findings and 375x667 — the size an SE 3rd gen actually is —
// reported 116, with two destination names painted on top of each other.
//
//   320x568  what a 4.7" phone (SE 2nd/3rd gen, 8) reports with Display Zoom
//            set to Larger Text, and the layout of the retired 4" phones.
//            Unverified here — no simulator, no device — so the reason it is
//            in this list is the one that does not depend on it:
//            docs/measure-composer.js already gates 320 as a defensive floor
//            ("a run that never sees the tight case is not a test of the tight
//            case"), and an audit that gave up at 375 would gate a narrower
//            band than the composer script does.
//   360x800  Android's modal size: WebView's CSS px is a dp, and 360dp is the
//            width most Android phones in the field report. It is the one size
//            here that is not an iPhone, and root 16px above is the text size
//            that goes with it (the Dynamic Type opt-in is iOS-only). Clean
//            today — but so is 390, and it is in this list for the same
//            reason: "between two clean sizes" is not a proof on this axis.
//   375x667  SE (3rd gen), sold new until 2025 — the narrowest size a phone
//            Apple supports gives at the default zoom, and where the drawer
//            failure below is 38px rather than the 2px a tall 375 shows.
//   390x844  12 / 13 / 14 / 16e — the size most iPhones in the field report.
//            It finds nothing today; it is here because the failure
//            docs/measure-composer.js was written for was a width rendering
//            LESS than a narrower one.
//   402x874  17 Pro / 16 Pro. The reference: what docs/style-gallery.html is
//            captured at and what every measurement in docs/design.md was made
//            against. CONTRAST runs here alone — see the walk below.
//   440x956  17 Pro Max / 16 Pro Max, the widest.
//
// 393x852 (14 Pro / 15 / 16, and Pixel) is deliberately absent: three points
// from 390 is a question about how many columns one elastic chip has, which is
// docs/measure-composer.js's subject — that script does sweep 393 — not a
// question about the geometry of 98 screens. 375x812 (13 mini) is absent
// because 375x667 is the same width and strictly harsher in the axis that
// turned out to matter. And the rest of the Android band is absent because it
// was measured rather than assumed: adding 393x852, 412x915 and 360x640 to
// this list reports Clean, so all three would cost half again the runtime to
// restate what the six below already say.
//
// What is still uncovered, and cannot be covered here: the keyboard-up
// viewport, which is a real height this app renders at and which no headless
// browser reports.
//
// A fixed list rather than argv, unlike measure-composer.js's widths: that
// script's are a sweep for a comparison, while these are a COVERAGE CLAIM
// about which phones the app is gated on, and a claim that narrows silently
// when someone passes an argument is not a claim.
const SIZES = [
  { width: 320, height: 568 },
  { width: 360, height: 800 },
  { width: 375, height: 667 },
  { width: 390, height: 844 },
  { width: 402, height: 874, reference: true },
  { width: 440, height: 956 },
];

// ── window size ─────────────────────────────────────────────────────────
// A window is not a phone, and this list is a weaker claim than the one above
// — deliberately, and it should be read as the weaker one. SIZES is a COVERAGE
// claim about devices: those are the phones this app is gated on and there are
// no others. A window can be any size at all, so there is no equivalent claim
// available. What this list is instead: EVERY BREAKPOINT STRADDLED ON BOTH
// SIDES, plus the floor, plus the size the app actually opens at, plus the one
// width where the list column stops growing.
//
//   480x560   MIN_INNER (src/shell/desktop/mod.rs) — the floor
//             `with_min_inner_size` refuses to let the window past. 560 is the
//             nav's own intrinsic height, which is the first thing to give.
//   627x700   the last pixel before the sidebar floats
//   628x700   OVERLAY = NAV + CONTENT_MIN. The PAIR is what makes this a
//             straddle rather than a sample: a breakpoint is exactly the place
//             where one number renders one layout and the next renders
//             another, and only both sides can say the two agree.
//   703x760   the last pixel with no inspector in any shell state
//   704x760   the first pixel at which a SHUT sidebar leaves room for one, so
//             this pair only means anything because DESKTOP_STATES already
//             walks nav-closed — a run that did not would never render it.
//   971x800   the last pixel of two columns
//   972x800   NAV + CONTENT_MIN + INSP: the first pixel of three.
//   1440x860  `with_inner_size` in src/main.rs — the window everyone opens.
//             THE REFERENCE: where CONTRAST runs. Measured on the running app
//             rather than read off the source (see DESKTOP_SCALES).
//   1600x1000 a width past every breakpoint, where the content column is the
//             only thing still growing.
//
// THE 571/572 AND 901/902 PAIRS ARE GONE, and their absence is the point: they
// straddled the three-column and two-column sums of a shell that had a list
// column, and both went out with it. A pair that straddles nothing measures
// one layout twice and reports it as agreement.
//
// Height travels with width for SIZES' reason. It is not a device height here
// — there is no such thing — so each is chosen to be plausible and to leave
// the nav its 560.
const DESKTOP_SIZES = [
  { width: 480, height: 560 },
  { width: 627, height: 700 },
  { width: 628, height: 700 },
  { width: 703, height: 760 },
  { width: 704, height: 760 },
  { width: 971, height: 800 },
  { width: 972, height: 800 },
  { width: 1440, height: 860, reference: true },
  { width: 1600, height: 1000 },
];

// ONE text size on the desktop, and it is measured rather than derived.
//
// The four-scale walk above is the Dynamic Type opt-in, and that opt-in is
// `font: -apple-system-body` in assets/platform/ios.css — a line no other
// build has. src/css.rs gives a macOS binary assets/platform/macos.css
// instead, which sets no font at all, so nothing moves the root off the web
// view's own default. Read off the running window with one `document::eval`
// during the capture that produced these states: `16px root; viewport
// 1180x820; devicePixelRatio 2` — that viewport being the window the app
// opened at THEN, before src/main.rs:108 raised it to 1440x860, and what keeps
// the read good across the move is exactly the sentence above: the root is the
// web view's default and does not travel with the window. If a desktop build
// ever opts into a system text size, this list grows and that stops being true.
const DESKTOP_SCALES = [16];

// The shell's own state, walked rather than captured: the nav's collapse, the
// inspector's, and whether the window is fullscreen.
//
// `data-nav` is a plain attribute on `.shell` that only assets/desktop/
// reads (src/shell/desktop.rs sets it; a test there checks the sheet acts on
// it), so flipping it here is a real reflow of a real rule and not a fiction.
// It is worth the second pass because the rail tier and the collapsed tier are
// where the window chrome gets crowded: the toggle, the traffic-light
// reservation and the nav card are all in the same corner, and the one
// regression this shell has already shipped — a toggle painted on top of the
// macOS close button — lived in exactly that cell.
//
// `data-insp` IS THE SAME KIND OF FACT, and this list said the opposite until
// the fourth cell went in: that it was "a fact about what the reader last
// asked for that has to be captured and must never be flipped". Both halves
// of that were wrong.
//
//   Flippable, because the column is not conditional markup. Rust renders the
//   inspector unconditionally and the SHEET hides it —
//   src/shell/desktop/mod.rs:1143-1150 says so in as many words — so every
//   captured state carries the whole third column in its bytes whatever the
//   attribute happened to say, and flipping it lays out markup that is really
//   there. Exactly `data-nav`'s situation, one column over.
//
//   And "captured" is the reason it MUST be flipped rather than the reason it
//   must not. `inspector_open` is `use_synced_storage` (src/state.rs:557), a
//   preference the app restores; `nav_open` is a plain `use_signal(|| true)`
//   (src/shell/desktop/mod.rs:686), open again at every launch. So the one
//   value this grid was taking from the store is the one an operator's own
//   machine decides, and a capture run on a machine whose inspector was shut
//   would have quietly turned every cell collapsed — the expanded arrangement
//   all 26 desktop states were written for gone from the grid, with the
//   summary line still claiming a shell axis. Every cell states all three
//   attributes now, so no cell's arrangement is a fact about the machine the
//   capture ran on.
//
// AND THE WINDOW'S OWN TWO STATES, in the same list rather than as a second
// product.
//
// `data-fullscreen` is the other attribute `src/shell/desktop.rs` writes onto
// `.shell`, and until this list existed nothing rendered a frame with it set:
// `assets/platform/macos.css`'s `[data-fullscreen="true"]` block — which takes
// the whole 76pt traffic-light indent and the 52pt band's padding back — was
// checked by nothing at all. It was also, for the whole life of the feature,
// REACHED by nothing: the flag came from a JS guess at `innerHeight` that never
// once matched a real fullscreen window. A rule no frame ever renders and no
// window ever triggers is indistinguishable from a rule that works.
//
// FOUR CELLS AND NOT EIGHT, chosen rather than multiplied — the same
// discipline the first three were chosen with, applied to the attribute that
// doubled the product. Measured on `desktop-chats` at rest, at the reference
// window, as nav / .pane-main / .insp in px:
//
//   nav open,   insp open     268 /  828 / 344   the captured arrangement
//   nav closed, insp open       0 / 1096 / 344
//   nav open,   insp closed   268 / 1172 /   0   mockup 31, NOT walked
//   nav closed, insp closed     0 / 1440 /   0   mockup 32, the cell added
//
// The added cell is mockup 32's, and it takes the pane to the whole window:
// 1440px of content column, against 1096 for the widest the three cells before
// it ever rendered. Every centred `--measure` and every gutter in
// assets/desktop/80-measure.css is at its most stretched there — 400px a side
// at the reference — and that geometry had never been laid out by anything.
//
// MOCKUP 31'S CELL IS THE ONE LEFT OUT, and the reason is measured rather than
// asserted. At six of the nine window sizes it renders a frame identical to
// the `nav open` cell already walked: assets/desktop/90-inspector.css's
// `@media (max-width: 971px) { .shell .insp { display: none } }` (:468-471)
// hides the inspector below 972 outright, and the one rule that brings it back
// in the 704..971 band (:474-477) needs the nav SHUT to match — so at 480,
// 627, 628, 703, 704 and 971, flipping `data-insp` under an open nav changes
// nothing at all. At the other three it differs from the added cell only in
// how wide the pane is (704/1172/1332 against 972/1440/1600), and nothing in
// assets/desktop/ couples the pane's CONTENTS to `data-nav`: the four rules
// that read it touch `.navpane`, `.navcard`, `.nav-toggle` and `.insp`, and
// nothing else. A pane at 1172 is a pane at a width, and the added cell
// already sweeps the pane from 480 to 1600 across the size axis. THAT STOPS
// BEING TRUE the moment a rule makes the pane's contents depend on `data-nav`
// as well as on its width — #136 proposes exactly that, a `--measure` term
// keyed on `data-nav` and `data-insp` — and when one lands, this paragraph is
// what to re-read rather than the cell count.
//
// Fullscreen x insp is redundant for the reason fullscreen x nav-closed
// already was: fullscreen changes exactly one thing, the band, which loses its
// indent and gains its padding, and the band is the same band whichever
// columns are under it — `grep -rn data-insp assets/` reaches one file and it
// is the inspector's own.
//
// Stated as a product this axis would be 8 cells for two new arrangements;
// stated as a list it is 4 — a third more desktop cells, and 28s against 29s
// locally for `node docs/audit.js both`, which is inside the spread between
// two runs of the same file. The phone half is 98 of the 124 states and it did
// not move.
const DESKTOP_SHELL = [
  { label: 'nav open, insp open', attrs: { 'data-nav': 'open', 'data-fullscreen': 'false', 'data-insp': 'open' } },
  { label: 'nav closed, insp open', attrs: { 'data-nav': 'closed', 'data-fullscreen': 'false', 'data-insp': 'open' } },
  { label: 'nav open, insp open, fullscreen', attrs: { 'data-nav': 'open', 'data-fullscreen': 'true', 'data-insp': 'open' } },
  { label: 'nav closed, insp closed', attrs: { 'data-nav': 'closed', 'data-fullscreen': 'false', 'data-insp': 'closed' } },
];

// EVERY attribute the shell writes onto `.shell`, which used to be more than
// the ones this file flips and is now exactly them.
// `src/shell/desktop/mod.rs:783-790` is the whole list and the only source for
// it: `data-nav`, `data-fullscreen`, `data-insp`.
//
// All three arrive from DESKTOP_SHELL above, and the check below still READS
// all three before writing any of them. `setAttribute` on an element that
// exists cannot fail, so "did my write land" is close to a tautology; the
// question that is not a tautology is whether the captured markup carries these
// attributes ON THIS ELEMENT in the first place. It answers two failures at
// once: an attribute the app renamed or dropped (the store re-captured, the
// sheet's `[data-insp="closed"]` rules now matching nothing ever), and an
// attribute the app MOVED to another element — which `data-fullscreen` has
// already done once in this feature's life, from `.app` to `.shell` in ece4857.
// A move is invisible to a walk that just writes its own copy onto `.shell`:
// the sheet reads the element the app writes, the audit measures the element it
// wrote itself, and the two stop being the same question without a word.
// Measured, on this tree: move `data-fullscreen` off `.shell` and onto `.app`
// in one captured state — the half-finished refactor, exactly — and
// `node docs/audit.js dark` reported **Clean** before this list existed.
//
// `data-detail` is deliberately NOT here. It was a fourth until the sidebar
// took the list column (`src/shell/desktop/mod.rs:1106-1111`): it existed only
// to tell the sheet which of two columns held content, there is one column now,
// and `:has(.chrome-title)` asks the markup directly instead. Naming it here
// would fail every desktop cell on an attribute the app is right not to write.
const SHELL_ATTRS = ['data-nav', 'data-fullscreen', 'data-insp'];

// Flipping `data-nav` starts a 200ms transition on `.navpane`'s flex-basis,
// width and padding, so a walk that read geometry straight afterwards would be
// measuring a column mid-slide. Everything this script measures is measured AT
// REST — that is already true of the capture, which records markup with no
// scroll offset — so the axis states it rather than waiting a quarter of a
// second per cell for the same answer. `!important` because a transition is
// declared on the element itself, and `all` because the card's deferred
// `visibility` is part of the same slide: at rest a closed nav is hidden, not
// hidden-in-200ms.
//
// EVERY FRAME NOW, AND THE PHONE'S HALF WAS BOUGHT WITH A REAL NUMBER. This
// said "desktop states only, since nothing flips an attribute on a phone frame"
// until the `data-theme` axis went in, and that axis flips one on every frame
// there is. Measured, with this sheet still desktop-only: **208 findings**, all
// of them the colour walk reading a `background-color` mid-transition —
// `.btn` declares `transition: background-color 120ms`, so setting the
// attribute repaints the text instantly and leaves the fill on its way, and
// `button.btn.primary` reports 1.00:1 black-on-black that no reader has ever
// seen. Linked into the phone's list as well: **Clean**, and the whole grid's
// existing numbers are unmoved, because everything this script measures was
// already being measured at rest.
//
// It also takes a second source of drift out of the fill walk below. `.dot.busy`
// and `.dot-anim` carry `animation`s that move `opacity`, the colour walks skip
// anything under 0.99, and which of those two things is true at the instant a
// walk runs was a property of the wall clock.
const AT_REST_CSS = '*, *::before, *::after { transition: none !important;'
  + ' animation: none !important; }';

// ── the scroller's own state, which is the fourth thing this walk drives ──
//
// `body.away-from-bottom` is the class src/viewport.rs adds when a transcript
// is scrolled up, and it is the whole of what makes the jump-to-latest disc
// exist: `assets/shared.css` gives `.scroll-bottom` `opacity: 0;
// pointer-events: none` until the class is on. A `pointer-events: none` box is
// not in the hit test, and the OCCLUDED walk says so in as many words where it
// skips one — so the disc was invisible to this file in every cell, at every
// size, in both shells, for the whole life of the check. #209 was found by a
// human hand-injecting the class for exactly that reason.
//
// AN AXIS AND NOT A CAPTURE, which is #250 and is the same argument
// `data-nav`, `data-insp` and `data-fullscreen` are already here on. Those
// three are states of the SHELL rather than states of the app; this is a state
// of a SCROLLER, and it is the same shape — one class, on one element, set by
// JS, describing where something is rather than what the app has.
// scripts/capture-gallery.py emits 1500ms after the last mutation and
// docs/design.md says every state is photographed at rest, so reaching this
// cell by capture means leaving a transcript scrolled up and still for a second
// and a half and hoping nothing scrolls it back. Driving it is possible; it is
// just a worse instrument than the one this file already has.
//
// ONLY THE STATES THAT HAVE A DISC, which is 14 of the 69 (9 phone, 5 desktop)
// and is measured rather than assumed — `stateIsAway` reads the markup for the
// button rather than naming the keys, so a capture that adds a transcript gets
// the cell for free. In a state with no `.scroll-bottom` the class changes
// nothing at all, and a second identical walk of the same frame is exactly the
// fiction the SHELL_ATTRS check below exists to stop this file telling.
//
// BOTH CELLS, and the reason is smaller than it looks. The disc's box moves 8px
// between them, so this is not a walk of the same frame twice — but everything
// else in the document is untouched, because the disc hangs out of a zero-height
// slot and takes no room in flow. Measured on this tree with the axis walking
// both: the `away` cell reports everything the resting cell does and one thing
// more, which is the disc.
//
// REPRODUCED, three ways, because an axis that is not doing anything is exactly
// what this one was added to end:
//
//   the axis   Cut AWAY_CELLS to `[false]` and the run fails on the ledger's
//              reverse check — "1 ledger entry names a defect this run did not
//              find: button.scroll-bottom >> summary". So the entry below is
//              this axis' own product and nothing else's.
//   the class  Delete `pointer-events: auto` from assets/shared.css's
//              `body.away-from-bottom .scroll-bottom` and the check below stops
//              the run on `chat` with "it is still `pointer-events: none` with
//              the class on".
//   the rest   Change the same sheet's resting `translate(-50%, 8px)` to 10px
//              and it stops on "the disc sits at translateY(10.00px) where the
//              sheet puts it at 8", which is the transition trap's own alarm
//              reading a real element rather than a constant.
const AWAY_CLASS = 'away-from-bottom';
const AWAY_CELLS = [false, true];
// Read off the markup, so the list cannot go stale against the store. Tokens
// rather than a substring: `scroll-bottom-slot` is a different element — the
// zero-height box the disc hangs out of, which every transcript has whether or
// not it has a disc — and a `contains` would claim the cell for states with no
// button in them.
const stateIsAway = (body) => [...body.matchAll(/class="([^"]*)"/g)]
  .some(([, value]) => value.split(/\s+/).includes('scroll-bottom'));

// THE TRAP #165 WROTE UP, ONE PROPERTY PAIR OVER, and the reason this returns a
// number instead of a boolean. Flipping a class starts every transition the
// sheet declares on it and `getComputedStyle` then answers with an interpolated
// value; `.scroll-bottom` transitions `opacity` AND `transform`, and #240 gave
// the desktop's its own `translateY(8px)` at rest, so this is precisely the
// pair that trap applies to. AT_REST_CSS is linked into both frame lists and
// should make the flip instantaneous — SHOULD, which is not a measurement. So
// the axis asks the disc where it actually is: at rest the sheet puts it 8px
// down, with the class on it puts it on 0, and anything strictly between the
// two is a transition this run caught in flight. `pointer-events` is asked in
// the same breath for the SHELL_ATTRS reason — it is the whole of what this
// cell buys, and a cell that buys nothing must say so rather than report Clean.
const AWAY_SETTLED = ({ cls, on }) => {
  document.body.classList.toggle(cls, on);
  const el = document.querySelector('.scroll-bottom');
  if (!el) return ['it renders no `.scroll-bottom` at this size'];
  const cs = getComputedStyle(el);
  const m = new DOMMatrixReadOnly(cs.transform === 'none' ? '' : cs.transform);
  const problems = [];
  const want = on ? 0 : 8;
  if (Math.abs(m.f - want) > 0.01) {
    problems.push(`the disc sits at translateY(${m.f.toFixed(2)}px) where the sheet`
      + ` puts it at ${want} — a transition read in flight, or a rule that has moved`);
  }
  if (on && cs.pointerEvents !== 'auto') {
    problems.push(`it is still \`pointer-events: ${cs.pointerEvents}\` with the class on,`
      + ' so it is not in the hit test and the OCCLUDED walk cannot see it');
  }
  return problems;
};

// CONTRAST runs at the reference size alone, so that flag is not a label: it
// is the entire scope of one of this script's two walks. Drop the key while
// rewriting the list and the colour walk stops running while the summary goes
// on saying it found nothing — measured, with `.session-title, .setting-value,
// .banner { color: #bbbbbb }` appended to shared.css: 158 real contrast failures
// reported as Clean. Asserted rather than defaulted, because "whichever one
// happens to be first" is not a reference size.
//
// ONE PER SHELL, and the desktop's is not optional either: assets/desktop/
// splits `--nav-fill` and `--shell-line` by theme on purpose, and
// docs/design.md records that the split came out of a measured 1.53:1 slab and
// a 1.13:1 selected pill. A colour walk that does not run at 1440x860 in both
// themes would not have caught the thing the split exists to fix.
const REFERENCE = { false: SIZES.filter((z) => z.reference), true: DESKTOP_SIZES.filter((z) => z.reference) };
for (const [desktop, found] of Object.entries(REFERENCE)) {
  if (found.length !== 1) {
    const which = desktop === 'true' ? 'DESKTOP_SIZES' : 'SIZES';
    console.error(`exactly one ${which} entry must carry \`reference\` — it is where CONTRAST runs; found ${found.length}`);
    process.exit(1);
  }
}

// ── the two ledgers ─────────────────────────────────────────────────────
// Two checks below went in against a tree that already fails them, which is
// what a check finding something means and is not a reason to soften it. What
// they found is written down here, exactly, so that everything they did NOT
// find is a failure from the day they landed rather than after somebody has
// triaged a wall of output.
//
// THESE LISTS MAY ONLY SHRINK, and that is enforced rather than asked for. The
// shape is `UNCAPTURED`'s in src/shell/desktop/mod.rs — a ledger of what is
// broken today, not a set of exemptions — and it fails in both directions: a
// pair that is not on the list fails the run, and an entry on the list that
// nothing produced ALSO fails the run, because an allowance for a defect that
// has been fixed is an allowance nobody can check. The answer to a new entry is
// a fix or an issue, never a line here.
//
// The reverse half is only asked of a `both` run — see the check at the end of
// this file. Occlusion is geometry and is identical in the two themes
// (measured: the same 16 pairs in each), but a fill's contrast is not, and
// `span.mark.idle` fails in dark alone. A single-theme run walks half the grid,
// so it may not judge a ledger written for all of it. CI runs `both`.

// ── the occlusion ledger ────────────────────────────────────────────────
// `layer >> target`: a box that is painted over an interactive element it does
// not contain and is not contained by. Every one of these is a real defect with
// an issue against it; the OCCLUDED walk below is what the check asks and #226
// is why it asks it that way.
//
//   aside.navpane  #215 and #212. The desktop sidebar becomes an overlay below
//                  628 and again in 704..971 while the inspector is open, and
//                  it has neither a scrim nor a dismissal — so it is not a
//                  panel you open and close, it is 268px permanently over the
//                  content column. 29 of these 33 pairs are that one panel, and
//                  the fix deletes all 29 at once. Measured: it fires at 480,
//                  627, 704 and 971 and at no other window size, which is
//                  exactly the two bands the sheet makes it an overlay in.
//
//                  IT GREW BY TWELVE AND NOT ONE OF THEM IS NEW, which is the
//                  distinction this list is worth nothing without. The Phase 3
//                  capture drove the Code plane connected for the first time
//                  and opened a review, a pull request and a code
//                  conversation, so the store now holds worktree rows, a diff
//                  file head, the code composer and its chips — controls that
//                  had never been in front of this walk. The panel over them
//                  is the same panel, in the same two bands, at the same four
//                  window sizes; what changed is how much of the app was
//                  standing behind it when the photograph was taken. Every one
//                  of the twelve dies with #215 alongside the thirteen that
//                  were already here, and that is the test of the claim: if a
//                  fix leaves any of them alive it was not this defect.
//
//                  AND BY FOUR MORE, for the same reason a second time. The
//                  capture that took the store to 21 desktop states was driven
//                  against a Code home that had just grown its own geometry
//                  (#76), a fifth tile (#77), pressable tiles (#203) and a
//                  model chip on the Chat home (#194) — so `.home-tile`,
//                  `.home-tile.live`, `.home-tile.urgent` and
//                  `.home-chip.picker` are controls that did not exist the
//                  last time this walk ran. Same panel, same bands, same
//                  window sizes. All 29 still die together.
//   button.fab     `--z-chrome` is 20 and the overlay sidebar is `z-index: 3`,
//                  so a floating button belonging to the content column paints
//                  over the sidebar's rows — the second half of #209, one
//                  element over. And on the PHONE, where there is no sidebar,
//                  the same button covers the bottom of the list it floats on:
//                  `chats` at 402x874 with a long title, the fab at 264,806
//                  121x48 over "Load more" at 16,800 370x40.
//   div.toast      A toast is `position: fixed; z-index: 60` and at AX5 it is
//                  440px tall on a 568px phone, so it reaches the bar: 7px of
//                  the 44px back chevron on `recipes-detail-toast` at 320x568.
//   button
//     .scroll-bottom
//                  #261, and it is the entry #250's axis was added to find.
//                  The jump-to-latest disc is `pointer-events: none` until
//                  `body.away-from-bottom` is on, so until that axis existed
//                  this walk skipped it in every cell — see where it does, in
//                  GEOMETRY. With the class driven it fires on the PHONE, on
//                  the tool card's output disclosure: `code-chat` at 375x667
//                  root 16 (disc 165.5,467 44x44 over `summary` 17,453.6
//                  341x28) and at 390x844 root 23 (disc 173,635.3 over
//                  `summary` 17,633.4 356x35, which is 33 of its 35 points).
//                  The DESKTOP produces nothing, which is #209 having been
//                  fixed: `assets/desktop/80-measure.css` took the disc off
//                  the reading column and off `--z-chrome`, and
//                  `assets/shared.css` still centres the phone's on it.
const OCCLUSION_KNOWN = [
  'aside.navpane >> button.action-chip',
  'aside.navpane >> button.home-chip.picker',
  'aside.navpane >> button.home-tile',
  'aside.navpane >> button.home-tile.live',
  'aside.navpane >> button.home-tile.urgent',
  'aside.navpane >> button.btn.danger-outline',
  'aside.navpane >> button.btn.primary',
  'aside.navpane >> button.btn.secondary',
  'aside.navpane >> button.btn.small.primary',
  'aside.navpane >> button.composer-chip.action.attach',
  'aside.navpane >> button.composer-chip.action.branch',
  'aside.navpane >> button.composer-chip.action.mode',
  'aside.navpane >> button.composer-chip.action.model',
  'aside.navpane >> button.composer-chip.action.model.needed',
  'aside.navpane >> button.composer-chip.action.repo',
  'aside.navpane >> button.home-starter',
  'aside.navpane >> button.icon-btn',
  'aside.navpane >> button.icon-btn.back',
  'aside.navpane >> button.recent',
  'aside.navpane >> button.row-action',
  'aside.navpane >> button.setting-row',
  'aside.navpane >> button.tree',
  'aside.navpane >> button.tree.awake',
  'aside.navpane >> button.tree.waiting',
  'aside.navpane >> input.field',
  'aside.navpane >> summary',
  'aside.navpane >> summary.diff-file-head',
  'aside.navpane >> textarea.compose-field',
  'aside.navpane >> textarea.input',
  'button.fab >> button.btn.secondary.grow',
  'button.fab >> button.drawer-item',
  'button.scroll-bottom >> summary',
  'div.toast >> button.icon-btn.back',
];

// ── the fill-contrast ledger ────────────────────────────────────────────
// Indicators that carry their whole meaning in a background colour and do not
// clear 3:1 against what is behind them. Both are #213's, named by the element
// rather than by the ratio because the ratio is a fact about which surface the
// mark landed on and there are several.
//
// `span.dot.off` HAS COME OFF THIS LIST, which is what a ledger that may only
// shrink is for. It was the worst entry on it — 1.85:1 in dark on the Code
// plane's pull rows, 2.03:1 on the desktop band in light, and it is the mark
// that says the app has lost its server. `assets/shared.css` now paints it
// --text-secondary instead of --text-tertiary and it measures 4.64:1 at its
// worst over the whole grid, so the walk stopped producing it and the reverse
// half of the check below would have failed on the entry had it stayed.
//
// The two that remain are the same token spent the same way, and each is a
// separate decision about a separate element rather than a leftover:
//
//   span.dot-anim   `src/views/chat.rs`'s thinking indicator, --text-tertiary
//                   at 6px: 2.20:1 in light on the page, 2.87:1 in dark.
//   span.mark.idle  `assets/desktop/30-sidebar-list.css`, and its worst
//                   surface is a SELECTED row: 2.52:1 in dark on #2a2f36,
//                   4.01:1 in light. Dark-only, which is why the reverse check
//                   below is asked of a `both` run and not of half a grid.
const FILL_KNOWN = [
  'span.dot-anim',
  'span.mark.idle',
];

// A finding the ledger already holds comes back marked instead of counted, so
// the run can also say which entries fired. The string is passed INTO both
// walks rather than written twice: `page.evaluate` serialises a function
// without its closure, so a constant either travels as an argument or is
// duplicated, and a duplicated protocol between two halves of one file is how
// the halves stop agreeing.
const LEDGER_MARK = 'LEDGER ';

// ── stress ──────────────────────────────────────────────────────────────
// A captured state only ever shows the one string the app happened to be
// holding. Some of those strings come from a server and can be much longer:
// a model name is whatever the provider called it, and a chip sized to its
// content pushed the send button 50px off the right edge before this check
// existed. So each captured state is repeated with the server-supplied text
// swapped for the longest plausible value — substituting into markup the app
// really produced, rather than hand-writing a copy of it, which is the same
// reason the gallery is generated.
// One unbreakable token, not a long sentence: a permission ask quotes the
// command the agent wants to run, and a fetch or an install one-liner carries
// a URL. A word with nowhere to break is the case that pushes a card wider
// than the phone rather than simply wrapping.
const LONGEST = {
  // The model name moved into .chip-model when the chip grew an effort tier
  // beside it; a state captured before that has it straight on .chip-label.
  // Both are named, and the swap only writes into whichever one is actually
  // holding the text — see below.
  '.chip-label': 'Qwen3 Coder 480B A35B Instruct',
  '.chip-model': 'Qwen3 Coder 480B A35B Instruct',
  // A filename is the agent's to choose, not this app's, and the review
  // screen's head has a fixed-width control on the other end of it. The
  // stylesheet's promise is that the directory is spent first and the name
  // ellipsises rather than painting over that control; both halves of that
  // are geometry, so this walk can see them. Written into every .diff-name in
  // the state, which stresses the root-level file — the one with no directory
  // to spend — alongside the ones that have one.
  '.diff-name': 'transcript_folding_and_permission_merge_regression.rs',
  '.session-title': 'Refactor the transcript folding so streamed parts land in order',
  '.session-ask-title': 'Approve or deny curl -sSL https://raw.githubusercontent.com/example/really-long-org-name/main/scripts/install.sh',
  '.topbar > .title': 'Refactor the transcript folding so streamed parts land in order',
  // A two-line title is a different geometry from a one-line one — it is the
  // `.titlegroup` that is centred and clipped, not the `h1` — and every
  // screen that uses one puts *server* text in it: an extension's package
  // name, a skill's name, a recipe's title. Stressing only `.topbar > .title`
  // left the shape that actually carries the long strings unchecked.
  '.titlegroup > .title': 'Refactor the transcript folding so streamed parts land in order',
  // The DESKTOP's window bar, and it is the one title in the app that shares
  // its line with something load-bearing. `.window-drag` is the only way a
  // window whose titlebar src/main.rs has hidden can be moved, so the promise
  // `.chrome-title` makes is that it shrinks and ellipsises rather than eating
  // the drag strip — at 480pt, the window's own floor, with the whole of the
  // traffic-light reservation and the connection badge already spent. Nothing
  // else on this list appears in a desktop state's chrome, so without these
  // two the band would only ever be measured holding whatever short string the
  // capture happened to catch.
  '.chrome-heading': 'Refactor the transcript folding so streamed parts land in order',
  '.chrome-sub': 'PhillipChaffee/goose-phone-app · goose/desktop-unified-top-bar',
  // Every settings-shaped row on every screen puts server text here — a model
  // name, an MCP command line, a cron sentence read back as English — and
  // none of it was being stressed.
  '.setting-value': 'npx -y @modelcontextprotocol/server-filesystem /srv/goose/workspaces/current',
  // The leaf, not the wrapper: .session-meta is a div of spans, and the swap
  // below refuses to write into anything with an element child — so keying
  // this on the wrapper would look like a stress case and test nothing.
  '.session-meta > span': 'Every weekday at 09:00 America/Los_Angeles · 20250823_140512_9f3ab2',
};

// The class a selector is worth looking for in the captured markup: the last
// *class* in it, since a leaf may be a bare tag (`.session-meta > span`) and
// `span` is in every state ever captured.
const anchorClass = (sel) => sel.split(/[\s>]+/).filter((part) => part.startsWith('.')).pop().slice(1);

const stressed = (states) => states.flatMap((state) => {
  const hits = Object.entries(LONGEST).filter(([sel]) => state.body.includes(anchorClass(sel)));
  if (!hits.length) return [];
  return [{
    label: `${state.label} (long text)`,
    body: state.body,
    // Carried, not re-derived. This builds a fresh object, so a flag left out
    // here would quietly send every stressed desktop state through the phone's
    // sheets and the phone's sizes — half the desktop grid, silently, with the
    // summary still counting it.
    desktop: state.desktop,
    swap: Object.fromEntries(hits),
  }];
});

// ── geometry ────────────────────────────────────────────────────────────
const GEOMETRY = ({ mark, ledger }) => {
  const out = [];
  const px = (v) => parseFloat(v) || 0;
  const vw = document.documentElement.clientWidth;
  const vh = document.documentElement.clientHeight;
  // The gutter every floating thing shares, read from the stylesheet rather
  // than written here twice — see GUTTER below.
  const edge = px(getComputedStyle(document.documentElement).getPropertyValue('--edge'));
  const corners = ['TopLeft', 'TopRight', 'BottomRight', 'BottomLeft'];
  const rad = (cs) => corners.map((c) => px(cs[`border${c}Radius`]));
  const name = (el) => {
    const cls = typeof el.className === 'string' ? el.className.trim() : '';
    return el.tagName.toLowerCase() + (cls ? `.${cls.split(/\s+/).join('.')}` : '');
  };
  // The nearest ancestor that clips, which is the thing whose corners an
  // element inside it actually takes.
  const clipper = (el) => {
    for (let p = el.parentElement; p; p = p.parentElement) {
      if (getComputedStyle(p).overflow !== 'visible') return p;
    }
    return null;
  };
  // Inside something that scrolls sideways on purpose. A diff row in the
  // review screen's no-wrap mode is wider than the phone because that is
  // what "scroll long lines instead of wrapping" means; it is not spilling
  // off the page, it is the content of a scrollport.
  //
  // "On purpose" is the whole difficulty. Every .scroll on every screen ends
  // up with a computed overflow-x of `auto` — the used-value rules coerce a
  // `visible` axis to `auto` when the other axis scrolls — so overflow-x
  // alone cannot tell the two apart, and taking it at face value would
  // silence this check everywhere. A region that states overflow-y: hidden
  // has said which axis it means.
  // Does this box span the REGION that lays it out, edge to edge, in either
  // axis — where a region is the viewport, or a box that is itself one by the
  // same test?
  //
  // The SQUARE check below has always exempted "a page or a panel", and while
  // the page WAS the viewport, `r.width >= vw` was the whole of that idea. A
  // shell of columns is the first thing here that has regions inside regions:
  // `.pane-detail` reaches from under the window's chrome band to the bottom
  // of the window and from the divider to the window's right edge, which is a
  // panel by any reading — docs/design.md says so outright, "the content
  // beside it is the plain canvas: no card, no border" — and it spans neither
  // axis of the viewport, because the nav is beside it and the chrome band is
  // above it. Measured on the first desktop run: 1552 SQUARE findings, every
  // one of them `.pane-list`, `.pane-detail` or `.topbar`, in a stylesheet
  // whose square corners are stated design.
  //
  // EDGE TO EDGE AND NOT PAST IT, which is what keeps this from being a
  // general softening. A `<ul>` of session cards is 816px tall inside a 704px
  // scrollport: it has OVERFLOWED its region rather than spanned it, so the
  // walk stops there and every card inside it goes on answering for its own
  // corners. And the chain is unbroken or it is nothing — one padded ancestor
  // anywhere between a box and the window and the box is not a panel.
  const spansRegion = (el) => {
    let r = el.getBoundingClientRect();
    for (let p = el.parentElement; p; p = p.parentElement) {
      if (Math.abs(r.width - p.clientWidth) > 0.5 && Math.abs(r.height - p.clientHeight) > 0.5) {
        return false;
      }
      r = p.getBoundingClientRect();
    }
    return true;
  };
  const inHorizontalScroller = (el) => {
    for (let p = el.parentElement; p; p = p.parentElement) {
      const ps = getComputedStyle(p);
      if (/auto|scroll/.test(ps.overflowX) && ps.overflowY === 'hidden'
          && p.scrollWidth > p.clientWidth + 1) return true;
      if (ps.overflow !== 'visible') return false;
    }
    return false;
  };

  for (const el of document.querySelectorAll('*')) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    const tag = el.tagName.toLowerCase();

    // Wholly outside the viewport is parked, not overflowing: the closed
    // drawer is translated fully off the left edge on purpose. Only something
    // that is partly visible can be said to spill.
    const parked = r.right <= 0.5 || r.left >= vw - 0.5;
    // An element INSIDE an <svg> is painted through that root's own viewport,
    // and `svg:root { overflow: hidden }` is a UA rule — so a <path> whose
    // geometry bbox reaches past the screen has not put a pixel there. It is
    // the exemption walk above that needs this said out loud: that walk stops
    // at the first ancestor which clips, and from inside an icon the first
    // such ancestor is always the icon's own root, so it can never reach the
    // scroller. The result was an icon legitimately scrolled off the end of
    // .action-row being exempt while the path drawing it was not — 20 findings
    // at 320pt, all one element, none of them a pixel anyone could see.
    //
    // Conditioned on the root actually clipping rather than on being inside an
    // <svg> at all, so an svg that states `overflow: visible` really does let
    // its children out and they are still reported. The root itself is never
    // exempted by this — `ownerSVGElement` is null for it — so it goes on
    // answering for itself.
    const insideClippingSvg = !!el.ownerSVGElement
      && getComputedStyle(el.ownerSVGElement).overflow !== 'visible';
    if (!parked && !insideClippingSvg
        && (r.right > vw + 0.5 || r.left < -0.5) && !inHorizontalScroller(el)) {
      out.push(`OVERFLOW-X   ${name(el)} left=${r.left.toFixed(0)} right=${r.right.toFixed(0)} vw=${vw}`);
    }

    // Floating chrome that has outgrown the gutter it floats on.
    //
    // OVERFLOW-X above is the LAST thing that goes wrong to a box pinned to
    // one side of the screen, not the first. `.fab` is `position: absolute;
    // right: var(--edge)` with `left: auto`, so its width is
    // clamp(min-content, viewport - gutter, max-content) — an expression with
    // no term for the gutter on the other side. It reaches x=0 long before it
    // reaches x=-1, and only the second of those was reportable: measured at
    // AX5 before the `max-width` this check argued for, the button sat at
    // left=0.00 on 320, 360, 375, 390 AND 402 — at 402x874, the reference
    // size, which this script had already called Clean. Whether it tipped
    // past zero was then decided by the advance width of one word, which is
    // how the same commit was green on a Mac and red on a Linux runner.
    //
    // So the rule is stated instead of being waited for. assets/shared.css:
    // "--edge is the gutter every floating thing shares". An out-of-flow box
    // whose containing block is the screen and whose inset on one side IS that
    // gutter has declared itself one of those things; if its other side is
    // nearer the screen edge than the gutter, it has stopped floating.
    //
    // Narrow on purpose, and each condition earns its place. The containing
    // block must span the viewport, or every badge absolutely positioned in
    // the corner of a card answers a question about the screen it was never
    // asked. One inset must equal --edge exactly, which is what separates
    // floating chrome from a drawer or a sheet — those are anchored flush to
    // an edge and mean it, and neither of their insets is 16.
    const cb = el.offsetParent;
    const cbWidth = cb ? cb.getBoundingClientRect().width : vw;
    if (!parked && /absolute|fixed/.test(cs.position) && cbWidth >= vw - 0.5) {
      const gaps = [r.left, vw - r.right];
      if (gaps.some((g) => Math.abs(g - edge) < 0.5) && Math.min(...gaps) < edge - 0.5) {
        out.push(`GUTTER       ${name(el)} left=${r.left.toFixed(0)} right=${(vw - r.right).toFixed(0)} from the edges, gutter=${edge}`);
      }
    }

    if (parked) continue;
    // A flex or grid container that overflows is overflowing BOXES, not text,
    // and every one of those boxes is visited by this same walk — so it is
    // checked for its own ellipsis on its own terms. Asking a flex container
    // for `text-overflow` is asking a question the property does not answer:
    // it only applies to inline content in a block container. The chip label
    // holding a model name and an effort tier is exactly this shape.
    const laysOutBoxes = cs.display.includes('flex') || cs.display.includes('grid');
    if (!laysOutBoxes
        && el.scrollWidth > el.clientWidth + 1
        && cs.overflowX === 'hidden'
        && cs.textOverflow !== 'ellipsis') {
      out.push(`CLIPPED-X    ${name(el)} scroll=${el.scrollWidth} client=${el.clientWidth}`);
    }

    // The vertical half, which is the axis growing text moves. There is no
    // `text-overflow` escape here — nothing draws an ellipsis at the foot of a
    // box — so a clipping box whose content is taller than it has simply cut
    // the bottom off something.
    //
    // The line clamp exemption is load-bearing and not a softening: four
    // .session-* elements are -webkit-box line clamps, which work BY clipping
    // vertically, and they say so in the stylesheet. Anything else that clips
    // this axis did not mean to.
    if (/hidden|clip/.test(cs.overflowY)
        && cs.webkitLineClamp === 'none'
        && el.scrollHeight > el.clientHeight + 1) {
      out.push(`CLIPPED-Y    ${name(el)} scroll=${el.scrollHeight} client=${el.clientHeight}`);
    }

    // Ink outside a box that never clips.
    //
    // This is the whole class of failure the two checks above cannot see. A
    // chip, a swipe action and a floating action button all pin a height and
    // set no overflow at all, so text too big for them is not clipped — it is
    // painted outside the pill, over whatever is behind it, and every number
    // the other walks read stays in range. docs/measure-composer.js has had
    // this check for the composer since the day a crushed pill pushed its
    // chevron through its own side; this is the same question asked of every
    // box on every screen.
    //
    // Only in-flow children count. An absolutely positioned child leaving its
    // parent is usually the point — the bar's centred title, the badge on a
    // tile, the button hanging out of the zero-height slot above the composer
    // — and a pseudo-element is not in `children` at all, which is why the
    // dots drawn by ::before and ::after do not have to be exempted here.
    if (cs.overflowX === 'visible' && cs.overflowY === 'visible') {
      let ink = null;
      const add = (b) => {
        if (b.width === 0 && b.height === 0) return;
        ink = ink
          ? {
            left: Math.min(ink.left, b.left),
            right: Math.max(ink.right, b.right),
            top: Math.min(ink.top, b.top),
            bottom: Math.max(ink.bottom, b.bottom),
          }
          : { left: b.left, right: b.right, top: b.top, bottom: b.bottom };
      };
      for (const kid of el.children) {
        const ks = getComputedStyle(kid);
        if (ks.position !== 'static' || ks.float !== 'none') continue;
        // `checkVisibility`, not a `display` test: a CLOSED <details> still
        // lays its <pre> out — Chromium hides it with `content-visibility`
        // rather than `display: none` — so it reports a real rect 80px below
        // a tool card that is showing nothing but its summary.
        if (!kid.checkVisibility()) continue;
        add(kid.getBoundingClientRect());
      }
      // A bare text node has no box, which is exactly how "Delete" paints
      // outside an 84px swipe action with nothing reporting it. A Range is
      // the only handle on it.
      //
      // Trimmed to the text itself. `white-space: pre-wrap` — which the diff
      // body is, so that a source line's own indentation survives — preserves
      // trailing spaces and lets them HANG past the end of the line, by
      // specification. A range over the whole node measures that hang as ink
      // and reports 7px of spill on every wrapped line of every diff.
      for (const node of el.childNodes) {
        if (node.nodeType !== 3) continue;
        const raw = node.textContent;
        const from = raw.length - raw.trimStart().length;
        const to = raw.trimEnd().length;
        if (from >= to) continue;
        const range = document.createRange();
        range.setStart(node, from);
        range.setEnd(node, to);
        add(range.getBoundingClientRect());
      }
      if (ink) {
        // `pre-wrap` hangs a space that lands at a soft wrap past the end of
        // the line — by specification, and invisibly, since it is a space.
        // The diff body is pre-wrap so that a source line's own indentation
        // survives, and every wrapped line of every diff reports one space of
        // ink outside its box. The inline axis is the only one that can hang;
        // the block axis, which is the one growing text moves, still counts.
        const hangs = /^pre-wrap|^preserve$/.test(cs.whiteSpace);
        const over = Math.max(
          r.top - ink.top, ink.bottom - r.bottom,
          ...(hangs ? [] : [r.left - ink.left, ink.right - r.right]),
        );
        if (over > 1) {
          out.push(`SPILL        ${name(el)} content leaves the box by ${over.toFixed(0)}px`);
        }
      }
    }

    // A surface is something with a fill or a box of borders. A lone
    // border-top is a rule, not a box, and is meant to be square.
    const filled = cs.backgroundColor !== 'rgba(0, 0, 0, 0)';
    const boxed = px(cs.borderTopWidth) > 0 && px(cs.borderLeftWidth) > 0 && px(cs.borderBottomWidth) > 0;
    // A surface that spans the whole viewport in either axis is a page or a
    // panel; square corners are correct for both. Either axis really does
    // mean either: the review screen's file bands run edge to edge so the
    // code gets the width, and a curve at a corner the screen edge already
    // cuts is a notch rather than a card. The width half of this sentence
    // used to be `&&`-ed with the height and so decided nothing.
    //
    // `spansRegion` below is the same sentence for a window with columns in
    // it, and it is asked LAST, once everything cheap has already said this
    // box might be a finding — it is a walk to the root and this loop visits
    // every element on the page. Kept as a separate disjunct rather than
    // folded into this line so that nothing a phone frame exempts today can
    // stop being exempt: this test is unchanged, and the other one only ever
    // adds.
    const fullScreen = r.width >= vw - 0.5 || r.height >= vh - 0.5;
    // Nor is a row a surface. Something that fills its clipping parent from
    // edge to edge already has that parent's corners — rounding it as well
    // is what rule 4 means by concentric, and doing it to each row of a diff
    // would notch every join between two consecutive rows.
    const clip = clipper(el);
    const flush = clip && Math.max(...rad(getComputedStyle(clip))) > 0
      && r.width + 0.5 >= clip.clientWidth;
    if ((filled || boxed) && !fullScreen && !flush && Math.max(...rad(cs)) === 0
        && r.width > 24 && r.height > 12 && tag !== 'html' && tag !== 'body'
        && !spansRegion(el)) {
      out.push(`SQUARE       ${name(el)} ${r.width.toFixed(0)}x${r.height.toFixed(0)}`);
    }
    if (tag === 'button' && (r.height < 32 || r.width < 32)) {
      out.push(`SMALL-TAP    ${name(el)} ${r.width.toFixed(0)}x${r.height.toFixed(0)}`);
    }
  }

  // The title is centred on the screen and the controls are not, so the only
  // thing keeping them apart is the width the title is allowed. Nothing
  // clips, nothing overflows the viewport and nothing reports an error — the
  // title simply runs underneath a button. Caught here because it is the sort
  // of thing that only appears when a control group changes width.
  //
  // EVERY bar, not the first one. A phone screen has exactly one `.topbar`, so
  // `querySelector` said everything there was to say and these numbers cannot
  // move on that shell. The desktop shell has a header per PANE — measured over
  // the captured states, seven of the fourteen have two — and the singular
  // query would have measured the list column's and never looked at the
  // detail's, which is the one holding a back chevron, an overflow control and
  // often a two-line title. A check that silently examines one of two is worse
  // than one that examines neither, because the summary line counts it as
  // covered.
  //
  // AND THE WINDOW'S OWN BAR IS ONE OF THEM. `.shell-chrome` holds the name of
  // whatever the desktop's detail column has open, and it is the one bar in
  // this app whose height is not the app's to choose: `--chrome-h` is 52px
  // measured off a real macOS window, because the traffic lights are painted
  // into it. So "the title is taller than the bar" is a sharper question there
  // than anywhere else — a heading that outgrows this one is painted over the
  // window's own controls — and it is the one bar the phone does not have, so
  // adding it costs the phone grid nothing.
  const bars = [...document.querySelectorAll('.topbar, .shell-chrome')];
  for (const bar of bars) {
    const heading = bar.querySelector(':scope > .title, :scope > .titlegroup, :scope > .chrome-title');
    // RENDERED, not merely present. `assets/desktop/` takes the detail
    // pane's own heading out with `display: none` once the window's bar is
    // carrying it, and a `display: none` box reports 0x0 at the origin — which
    // is "outside the bar" by arithmetic and inside it by every meaning the
    // check has. Measured: without this the desktop grid reports 392
    // TITLE-TALLER, every one of them `div.titlegroup 0..0`, and the phone
    // grid is untouched because nothing on the phone hides a heading.
    // `getClientRects()` and not the bounding box, because a box of zero
    // height is a real finding and an element with no boxes at all is not.
    if (heading && heading.getClientRects().length) {
      const h = heading.getBoundingClientRect();
      // The other axis, and the one the SCRIM check below is blind to. The
      // scrim is `--topbar-h` tall and the scroll padding is derived from the
      // same token, so both are sized for a bar of that height — but the bar
      // is the only one of the three that has a title inside it. While the
      // bar's height was pinned, its bottom edge never moved whatever the
      // title did, so SCRIM compared against a number that could not go wrong
      // and reported clean with the title painted outside the material that
      // makes it readable.
      const b = bar.getBoundingClientRect();
      if (h.top < b.top - 0.5 || h.bottom > b.bottom + 0.5) {
        out.push(`TITLE-TALLER ${name(heading)} ${h.top.toFixed(0)}..${h.bottom.toFixed(0)}`
          + ` outside the bar's ${b.top.toFixed(0)}..${b.bottom.toFixed(0)}`);
      }
      // `.nav-toggle` and `.conn-badge` are the window bar's two controls, the
      // same way `.icon-btn` and `.topbar-actions` are a pane header's.
      for (const group of bar.querySelectorAll(
        ':scope > .icon-btn, :scope > .topbar-actions, :scope > .nav-toggle, :scope > .conn-badge',
      )) {
        const g = group.getBoundingClientRect();
        const over = Math.min(h.right, g.right) - Math.max(h.left, g.left);
        if (over > 0.5) {
          out.push(`TITLE-COLLIDE ${name(heading)} overlaps ${name(group)} by ${over.toFixed(0)}px`);
        }
      }
    }
  }

  // THE DETAIL'S HEADING BELONGS TO THE WINDOW, AND SO DOES THE CONNECTION.
  //
  // This is the whole claim the window bar exists to make, and until now it
  // was made by nothing but three `display: none` rules in
  // `assets/desktop/`. Delete any of them and the app paints the open
  // thing's name in the band AND again in the pane a few pixels below it,
  // while every other check in this file stays green: nothing clips, nothing
  // overflows, nothing collides, because two headings in two different bars
  // are each placed perfectly inside their own.
  //
  // NOT "one title per window", which is the tempting reading and is wrong.
  // The LIST column keeps its own heading on purpose — "Skills" over the list
  // of skills — so that the list never moves at any width or in any state
  // (`src/shell/desktop.rs`). Stated that way the rule fires 144 times on a
  // correct build, once per size where both columns are up. What must not
  // double is the DETAIL's heading, because that is the one the band took.
  //
  // COUNTED AS RENDERED, which is the only reading that means anything here.
  // The pane still emits its heading — `nav::Detail` is the pane's own data
  // and Dioxus has no portal, so the markup cannot be moved, only hidden — so
  // "present in the DOM" counts two on a correct build and checks nothing.
  // `getClientRects()` for that, the same test the bar loop above uses.
  //
  // DESKTOP ONLY, decided by the DOM rather than by the state's key: the band
  // is the discriminator and it is also the thing under test. A phone has one
  // `.topbar` by construction and no second column to disagree with it.
  const chrome = document.querySelector('.shell-chrome');
  if (chrome) {
    const shown = (sel) => [...document.querySelectorAll(sel)]
      .filter((el) => el.getClientRects().length);
    const band = shown('.shell-chrome > .chrome-title');
    // A DESCENDANT COMBINATOR, DELIBERATELY UNLIKE THE RULE IT GUARDS.
    //
    // `assets/desktop/` hides the pane's copy with `.pane-detail .topbar >
    // .title`, and this check used to be written with the same `>`. That is
    // the one shape a guard must never take: a heading nested one element
    // deeper is un-hidden by the sheet and unseen by the check in the same
    // stroke, so the failure and the blindness arrive together. It is not a
    // hypothetical shape either — `views::chrome::TopBar` growing a wrapper
    // around its heading is an ordinary refactor, and `.titlegroup` is already
    // one such wrapper that the sheet had to be taught by hand.
    //
    // Reproduced, on the tree this comment ships in: wrap each of the seven
    // detail panes' headings in `<div class="titlewrap">`, broaden
    // assets/shared.css's `.topbar > .title` to `.topbar .title` and give the
    // wrapper `display: contents` — which is a refactor that changes not one
    // pixel of a correct build — and the app paints the open thing's name
    // twice, in the band and again in the pane. With `>` here: **Clean**.
    // With the descendant selector: **588 TITLE-DOUBLED** (7 states x 2 for
    // the long-text pass x 2 themes x 7 window sizes x 3 shell states).
    //
    // The cost of the wider net is that a `.titlegroup` and the `.title`
    // inside it are both matched, so a doubled two-line heading names two
    // elements in one finding. That is the finding being more specific, not
    // less: on a correct build both are hidden and neither has a box.
    const pane = shown('.pane-detail .topbar .title, .pane-detail .topbar .titlegroup');
    // Guarded on the band actually carrying one. With nothing open the band
    // paints no title at all — `src/shell/desktop.rs` renders it only for a
    // `Some(crumb)`, so an empty flex item never takes its gap — and then the
    // detail pane keeping its own heading is not a duplicate, it is the only
    // one there is.
    if (band.length && pane.length) {
      out.push(`TITLE-DOUBLED the band carries ${name(band[0])}`
        + ` "${(band[0].textContent || '').trim().slice(0, 32)}" and the detail pane still paints`
        + ` ${pane.map((el) => `${name(el)} "${(el.textContent || '').trim().slice(0, 32)}"`).join(' + ')}`);
    }
    // The connection is the window's, full stop — there is one socket, so a
    // second badge is a second answer to a question with one answer. No guard
    // needed: `assets/desktop/` hides every pane's copy unconditionally,
    // so more than one rendered is always wrong.
    //
    // Measured, on the shipping grid: neuter `.pane .topbar > .conn-badge`
    // and the run reports **948 CONN-DOUBLED**. The number recorded when this
    // check went in was 632, and it was honest at the time — it was taken on a
    // two-cell shell axis that the same commit replaced with three, and
    // 632 x 1.5 is 948. Recorded again here so the next person measures rather
    // than trusts.
    const badges = shown('.conn-badge');
    if (badges.length > 1) {
      out.push(`CONN-DOUBLED ${badges.length} connection badges rendered at once:`
        + ` ${badges.map((el) => name(el)).join(' + ')}`);
    }
    // AND THE OTHER SIDE OF EXACTLY ONE, WHICH IS THE SIDE NOTHING ASKED.
    //
    // `badges.length > 1` is half a claim. The window can lose its connection
    // indicator ENTIRELY and every gate stays green: nothing overflows,
    // nothing collides, nothing is doubled, and the contrast walk is a loop
    // over elements that are there. Measured, on this tree: broaden
    // `assets/desktop/`'s `.pane .topbar > .conn-badge` to a bare
    // `.conn-badge` — the ordinary way an over-eager selector is written, and
    // it hides the shell's copy along with the panes' — and every desktop
    // state loses its dot at every size in both themes while
    // `node docs/audit.js both` reports **Clean**. With this arm:
    // **1176 CONN-GONE** (14 states x 2 for the long-text pass x 2 themes x
    // 7 window sizes x 3 shell states — every desktop cell there is).
    //
    // `src/shell/desktop.rs` renders `views::ConnBadge` unconditionally, so
    // there is no state of the app in which none is correct. That is asserted
    // on the Rust side too, but a source-shaped test can only say the call
    // site is still written down; this says the dot is still painted.
    if (badges.length === 0) {
      out.push('CONN-GONE    the window bar paints no connection badge at all —'
        + ' there is one socket and nothing on screen says what it is doing');
    }

    // A SQUEEZED TITLE MUST OUTRANK EVERYTHING NEGOTIABLE BESIDE IT.
    //
    // Everything in this band is a flex item competing for one line, and the
    // competition only has a wrong answer at the narrow end. Measured at the
    // 480pt floor — `MIN_INNER`, the width the window cannot be dragged below —
    // the title was cut to 119px while `.conn-badge` spent 135px spelling out
    // an agent version: fifteen characters of the name of the thing you have
    // open, beside a fully written connection state, on a window with no room
    // for either. Nothing else in this file could see it. The band did not
    // overflow, nothing collided and nothing clipped, because a correctly
    // ellipsised title at any width is a perfectly well-behaved box.
    //
    // ASKED ONLY WHEN THE TITLE IS ACTUALLY CUT, which is what makes it a rule
    // about the squeeze rather than about the strings. A short title is 128px
    // because that is all it wants — narrower than the badge and entirely
    // correct — so comparing widths unconditionally would fire on every state
    // whose crumb happens to be one word. `scrollWidth > clientWidth` is the
    // difference between "took what it needed" and "was given less than it
    // asked for", and only the second is a question of priority.
    //
    // THE HEADING, NOT THE GROUP, and that was the whole of what went wrong
    // the first time. The check tested `.chrome-heading` for being cut and
    // then compared the badge against `.chrome-title`, which is the heading
    // AND the subtitle AND the gap between them — so the more of the line the
    // subtitle took, the safer the title looked. It was not a theoretical
    // hole: on the captured `desktop-scheduler-detail`, unstressed, at
    // 572x700, the group measured 211px while the heading inside it was cut
    // to 108 and `.conn-badge` held 135. Against the group: silent. Against
    // the heading: **16 TITLE-OUTBID** on a tree that was reported Clean —
    // 4 states (`desktop-scheduler-detail` and three long-text passes) x 2
    // themes x the one 572x700 window x 2 of the 3 shell states, the
    // fullscreen cell being wider by the 76px it hands back. The fix is in
    // `assets/desktop/`: `.chrome-sub` is `flex: 1 1 0` now, so the
    // qualifier takes what is left after the name rather than bidding against
    // it, and the run is Clean again.
    //
    // AND EVERY SIBLING, NOT THE BADGE. `assets/desktop/` drops
    // `.conn-label` at 571px and under, which takes the badge to an 18px dot —
    // so at the two narrowest window sizes on this grid the badge cannot
    // outbid anything and a badge-only check is dead exactly where the band is
    // most crowded. Proved with a regression confined to that tier — add
    // `.window-drag { flex: 1 0 300px }` INSIDE the `max-width: 571px` block,
    // which crushes the title at 480 and 571 and touches nothing above them:
    // badge-only, **Clean**; per sibling, **162 TITLE-OUTBID**, 138 of them
    // naming the drag strip and 24 naming a title left narrower than the
    // button beside it. So each item on the line answers for itself, against
    // the FLOOR the sheet gives it:
    //
    //   .window-drag   96px, and that one is load-bearing rather than
    //                  cosmetic — `flex: 1 0 96px` is the only thing keeping
    //                  a window whose titlebar src/main.rs hid draggable, and
    //                  DRAG-GONE below is what guards the floor itself. It is
    //                  also the item that legitimately holds all the slack, so
    //                  only its EXCESS is a bid.
    //   .nav-toggle    zero. It is a 32px control, so this reads as "the name
    //                  of the open thing is in less room than the button
    //                  beside it" — an absolute floor on the title, taken off
    //                  a real control rather than typed in as a number.
    //   .chrome-sub    zero. The subtitle qualifies the name; a qualified name
    //                  with no name left is worth nothing.
    //   .conn-badge    zero. The dot carries the whole state in colour — the
    //                  sheet says so where it hides the label at the rail.
    //
    // `.traffic-slot` is the one item deliberately NOT on that list. Its width
    // is the room AppKit paints the window's own controls in; it is not the
    // app's to spend and cannot be given back while the lights are there. The
    // one case where it should be zero and might not be is fullscreen, and
    // FULLSCREEN below owns that question rather than this one.
    // `+ 1`, AND THE PIXEL IS ROUNDING RATHER THAN SLACK. `scrollWidth` and
    // `clientWidth` are both integers over a layout that is not: the desktop
    // capture put "Tighten the composer's chip row" in the band, and measured
    // at 972, 704 and 627 with the sheets the audit itself links, the heading
    // is a 200.3px box holding 200.3px of text — `scrollWidth` ceils to 201,
    // `clientWidth` floors to 200, and a `+ 0.5` tolerance read that as a cut
    // title and reported the subtitle beside it 24 times over a band that was
    // fitting its text exactly. The one-pixel difference is the only value the
    // two integers can take when nothing is cut at all, and no fractional
    // `scrollWidth` exists to ask instead. What it gives up is a title cut by
    // exactly one device pixel; what it buys is that the check means the same
    // thing at every fractional width, which a band whose title is a server
    // string will land on constantly. The real squeeze it is written for is
    // not close to the boundary — the subtitle on the same states wants 284px
    // and is given 198.
    const bandTitle = band[0] && band[0].querySelector('.chrome-heading');
    if (bandTitle && bandTitle.getClientRects().length
      && bandTitle.scrollWidth > bandTitle.clientWidth + 1) {
      const t = bandTitle.getBoundingClientRect().width;
      for (const rival of chrome.querySelectorAll(
        ':scope > *:not(.chrome-title):not(.traffic-slot),'
        + ' :scope > .chrome-title > *:not(.chrome-heading)',
      )) {
        const floor = rival.classList.contains('window-drag') ? 96 : 0;
        const held = rival.getBoundingClientRect().width - floor;
        if (held > t + 0.5) {
          out.push(`TITLE-OUTBID ${name(rival)} holds ${held.toFixed(0)}px it could give back`
            + `${floor ? ` (${(held + floor).toFixed(0)}px, ${floor}px of it its own floor)` : ''}`
            + ` while the band's title is cut to ${t.toFixed(0)}px`);
        }
      }
    }
  }

  // FULLSCREEN GIVES THE ROOM BACK, OR THE THIRD SHELL CELL BOUGHT NOTHING.
  //
  // `DESKTOP_SHELL` grew a fullscreen cell so that
  // `assets/platform/macos.css`'s `[data-fullscreen="true"]` block would be
  // rendered by something — it never had been, and a rule no frame renders is
  // indistinguishable from a rule that works. But RENDERING a block is not
  // CHECKING it: measured on this tree, breaking the block's selector so that
  // fullscreen keeps the 76pt traffic-light reservation and the 0pt band
  // padding leaves `node docs/audit.js both` **Clean**. The cell was a 50%
  // growth of the desktop grid that could not fail.
  //
  // The commit that added it recorded 672 findings for a sabotage of the same
  // block — that number came from setting `--traffic-w: 900px`, which
  // physically overflows the window and is caught by OVERFLOW-X like any other
  // 900px box. It is not the regression this block has. The regression it has
  // is the one it already shipped once: the block going DEAD, which looks like
  // nothing at all.
  //
  // So the claim is stated in the two numbers the block actually sets, and
  // both are read off the layout rather than off the custom properties —
  // a `getPropertyValue('--traffic-w')` test would pass on a sheet that
  // declared the token and never spent it.
  //
  //   the reservation  In fullscreen macOS takes the lights away entirely, so
  //                    52 points of band with 76 points held empty at the
  //                    start of it is 76 points of nothing at the top of a
  //                    window someone just asked to be as large as possible.
  //   the toggle       With the lights gone there is nothing left to line the
  //                    toggle up with, so it centres in the band like it does
  //                    on every platform that never had lights. That is the
  //                    whole of `--chrome-pad: 10px`, and it is the half a
  //                    reservation check alone would miss.
  //
  // Measured, with the block's selector broken so nothing matches it:
  // **784 FULLSCREEN** — 392 of each (28 desktop states x 2 themes x 7 window
  // sizes x the 1 fullscreen cell). Break only `--traffic-w` and it is 392.
  //
  // Keyed on the attribute rather than on the state's key, for the reason the
  // band block above is: the attribute is what the sheet reads, and the walk
  // sets it exactly as `src/shell/desktop.rs` does. A phone frame has no
  // element carrying it, so this cannot reach one.
  const fullscreen = document.querySelector('.shell[data-fullscreen="true"]');
  if (fullscreen) {
    const fsSlot = fullscreen.querySelector('.traffic-slot');
    const reserved = fsSlot ? fsSlot.getBoundingClientRect().width : 0;
    if (reserved > 0.5) {
      out.push(`FULLSCREEN   ${reserved.toFixed(0)}px is still reserved for traffic lights`
        + ' that a fullscreen window does not have on screen');
    }
    const fsBand = fullscreen.querySelector('.shell-chrome');
    const fsToggle = fullscreen.querySelector('.shell-chrome > .nav-toggle');
    if (fsBand && fsToggle && fsToggle.getClientRects().length) {
      const b = fsBand.getBoundingClientRect();
      const t = fsToggle.getBoundingClientRect();
      // Half a pixel, the same tolerance every other edge comparison in this
      // file uses: the band is 52 and the toggle 32, so a centred toggle has
      // exactly 10 above and 10 below and there is no rounding to absorb.
      if (Math.abs((t.top - b.top) - (b.bottom - t.bottom)) > 0.5) {
        out.push(`FULLSCREEN   the nav toggle sits ${(t.top - b.top).toFixed(0)}px below the band's`
          + ` top and ${(b.bottom - t.bottom).toFixed(0)}px above its bottom — it is still aligned`
          + ' on traffic lights that are not there');
      }
    }
  }

  // AND THE COLLAPSE HAS TO COLLAPSE SOMETHING, OR THE FOURTH CELL BOUGHT
  // NOTHING — the block above's lesson, applied to the axis added after it,
  // and measured before this was written rather than after.
  //
  // `DESKTOP_SHELL` grew a `data-insp: closed` cell so that
  // `assets/desktop/90-inspector.css:12-14` — `.shell[data-insp="closed"] .insp
  // { display: none }`, the only rule in the tree keyed on that value — would
  // be rendered by something. RENDERING a rule is not CHECKING it, which is the
  // whole of the FULLSCREEN note above: measured on this tree with the cell in
  // and this check out, repointing that rule at a value nothing writes
  // (`[data-insp="shut"]`) so the third column stays open in the collapsed cell
  // leaves `node docs/audit.js both` **Clean** — 0 findings for a toggle that
  // has stopped toggling. Nothing else notices, and nothing else can: a column
  // that is still there is 344px of correctly laid out inspector, and every
  // check in this file is about a box being in the wrong place rather than
  // about a box being there at all.
  //
  // With this check in, the same sabotage is **156 findings** — 26 desktop
  // states x 2 themes x the three window sizes where the sheet is not already
  // hiding the column for width alone (972, 1440, 1600) x the one collapsed
  // cell. The three-cell sweep this axis replaced reports Clean on the same
  // broken sheet, which is the whole argument for both halves of this change
  // in one number.
  //
  // So the cell carries its own claim, in the one number that rule exists to
  // produce: with the inspector shut, the third column holds no room. Read off
  // the LAYOUT and not off `display`, because `visibility: hidden` and
  // `opacity: 0` are both collapses that keep their 344px — and the pane is
  // then the one thing on screen that has not moved, which is the version of
  // this bug somebody would ship while looking straight at it.
  //
  // What this cannot say, stated rather than implied: an inspector hidden in
  // EVERY cell passes here, because there is then nothing to give back. That
  // failure is the loud kind — a whole column missing from the window the app
  // opens — and the quiet half of it, the markup going away, is asked of the
  // store in `coverage` before a browser is launched.
  const shut = document.querySelector('.shell[data-insp="closed"]');
  if (shut) {
    const held = shut.querySelector('.insp');
    const room = held ? held.getBoundingClientRect().width : 0;
    if (room > 0.5) {
      out.push(`INSP-SHUT    the inspector still holds ${room.toFixed(0)}px of the window while`
        + ' `data-insp="closed"` — the column the toggle just shut is still on screen,'
        + ' and the pane never got the room back');
    }
  }

  // THE WINDOW HAS TO STAY DRAGGABLE.
  //
  // `src/main.rs` hides the macOS titlebar, which takes AppKit's own drag
  // region with it, so `.window-drag` is the ONLY thing left that can move the
  // window. It is a flex sibling of everything else in the band — the traffic
  // lights' reservation, the nav toggle, the title of whatever is open, the
  // connection badge — and every one of those is free to grow: a long enough
  // session name, a long enough agent version, a wider `--traffic-w`. Squeeze
  // this to nothing and the window is stuck where it is, with no error, no
  // clipping and nothing else on screen out of place. `assets/desktop/`
  // answers that with `flex: 1 0 96px`; this is what notices when something
  // takes that back.
  //
  // 44px rather than the sheet's own 96, deliberately: this is a floor on the
  // AFFORDANCE, not a restatement of the declaration. 44 is design rule 9's
  // number for a target a person has to hit.
  const drag = document.querySelector('.window-drag');
  if (drag) {
    const d = drag.getBoundingClientRect();
    if (d.width < 44) {
      out.push(`DRAG-GONE    .window-drag is ${d.width.toFixed(0)}px wide — the window cannot be moved`);
    }
  }

  // A control never sits at a tighter radius than the container clipping it,
  // and never at a looser one either — that is what pokes through the curve.
  for (const el of document.querySelectorAll('*')) {
    const p = el.parentElement;
    if (!p) continue;
    const ps = getComputedStyle(p);
    if (ps.overflow === 'visible') continue;
    const pr = Math.min(...rad(ps));
    const cr = Math.min(...rad(getComputedStyle(el)));
    if (pr > 0 && pr < 100 && cr < 100 && cr > pr + 0.6) {
      out.push(`RADIUS-NEST  ${name(el)} ${cr}px inside ${name(p)} ${pr}px`);
    }
  }

  // The room the WINDOW's own controls are painted in, and the one thing this
  // walk can say about them.
  //
  // On macOS `src/main.rs` hides the titlebar, so AppKit paints the traffic
  // lights straight onto the app's surface — at logical (9, 9), measured off a
  // real window, because tao's `with_traffic_light_inset` is a no-op under
  // wry. Nothing in the DOM draws them. `.traffic-slot` is an empty box the
  // shell renders to hold that room, sized by `--traffic-w` in
  // `assets/platform/macos.css`, and the rule is simply that nothing else may
  // be in it.
  //
  // THIS IS THE REGRESSION THAT SHIPPED. The nav toggle used to be absolutely
  // positioned against `.shell` and slid between the nav card's corner and the
  // window's; at the rail width both of those are the window's own corner, so
  // the button was painted on top of the close button — a control you cannot
  // press sitting exactly where you press to close the window. It was found by
  // eye on a device, because nothing rendered a desktop state and no check
  // asked this question.
  //
  // What it can and cannot say, stated rather than implied: a browser draws an
  // empty box where the lights are, so this can prove nothing overlaps the
  // RESERVATION and can prove nothing about whether the reservation is where
  // the lights actually land. The second half is a device check and stays one.
  //
  // Controls and fills only. An ancestor of the slot contains it by
  // construction, and a box with nothing in it that happens to overlap is not
  // something anyone can see; what makes this a failure is ink or a target.
  const slot = document.querySelector('.traffic-slot');
  if (slot) {
    const s = slot.getBoundingClientRect();
    if (s.width > 0.5 && s.height > 0.5) {
      for (const el of document.querySelectorAll('*')) {
        if (el === slot || el.contains(slot) || slot.contains(el)) continue;
        const cs = getComputedStyle(el);
        if (cs.display === 'none' || cs.visibility === 'hidden') continue;
        const paints = cs.backgroundColor !== 'rgba(0, 0, 0, 0)'
          || el.tagName.toLowerCase() === 'button'
          || [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
        if (!paints) continue;
        const r = el.getBoundingClientRect();
        // A sheet that covers the whole window is not a thing in the corner.
        // `.modal-backdrop` is `position: fixed` over everything by design and
        // reported 56 times on the first run; AppKit paints the lights over
        // the web view, so a dim behind them is not a control in their way.
        // The failure this is about is a box that lands IN that corner.
        if (r.width >= vw - 0.5 && r.height >= vh - 0.5) continue;
        const over = Math.min(
          Math.min(r.right, s.right) - Math.max(r.left, s.left),
          Math.min(r.bottom, s.bottom) - Math.max(r.top, s.top),
        );
        if (over > 0.5) {
          out.push(`CHROME-SLOT  ${name(el)} is inside the ${s.width.toFixed(0)}x${s.height.toFixed(0)}px`
            + ` the window's own controls are painted in, by ${over.toFixed(0)}px`);
        }
      }
    }
  }

  // The floating chrome is legible because an opaque scrim covers the band it
  // sits in, and the scroller's top padding keeps content out of that band. If
  // the two ever disagree, the title lands on whatever scrolled underneath it
  // and both stay readable — a collision, not a layering. The failure is
  // invisible at rest, because at rest there is nothing scrolled up yet.
  //
  // Geometry, not contrast, and it lives here for that reason: it compares two
  // measured edges, and the bar's is the one that moves. A title that wraps at
  // a narrow width makes the bar taller and the scrim stops covering it — a
  // failure the whole phone-size axis exists to reach, and one TITLE-TALLER
  // cannot see, because the heading is still inside the now-taller bar.
  //
  // AND IT IS ASKED ONLY WHERE THERE IS A SCRIM. `assets/desktop/` sets
  // `.app::before { display: none }`: on that shell the bar is `position:
  // static`, nothing scrolls under it, and there is no band of material to
  // check. Chromium still reports the SPECIFIED height for a `display: none`
  // pseudo-element, so both branches below go on computing against 84px of
  // something that paints nothing — measured on the captured `desktop-chats`
  // at 1180x820, the pseudo flips `block` to `none` while
  // `getComputedStyle(...).height` stays `84px`, and the first branch then
  // compares 84-24=60 solid against a bar ending at 116. Every desktop state
  // would report it, in both themes, at every size. `display` is the question
  // the check was always asking and it is `block` on the phone, so this cannot
  // move a phone number.
  const app = document.querySelector('.app');
  const scroller = document.querySelector('.scroll');
  const [bar] = bars;
  if (app && bar && getComputedStyle(app, '::before').display !== 'none') {
    const scrim = parseFloat(getComputedStyle(app, '::before').height) || 0;
    const fade = parseFloat(getComputedStyle(app).getPropertyValue('--scrim-fade')) || 0;
    const barBottom = bar.getBoundingClientRect().bottom;
    // The mask fades the scrim out over its last `--scrim-fade` pixels, so
    // only `scrim - fade` of it is at full tint. That solid part has to reach
    // past the bar: put the fade inside the bar's own band and the material
    // thins exactly where the title sits, which leaves a dark code block
    // scrolling under it legible straight through the serif. The icon buttons
    // never showed this because they carry glass of their own.
    if (scrim - fade < barBottom - 0.5) {
      out.push(`SCRIM        solid to ${(scrim - fade).toFixed(0)}px but the bar ends at ${barBottom.toFixed(0)}px — the fade crosses the title`);
    }
    if (scroller) {
      const pad = scroller.getBoundingClientRect().top + parseFloat(getComputedStyle(scroller).paddingTop);
      if (scrim - pad > 0.5) {
        out.push(`SCRIM        content starts ${(scrim - pad).toFixed(1)}px inside the ${scrim}px chrome scrim`);
      }
    }
  }

  // A row that renders nothing renders no line box, so it measures zero and
  // disappears. That is how every blank line in a diff silently vanished,
  // closing up the gaps the author put there. A height, so it belongs on the
  // size axis with the rest of the geometry.
  //
  // `checkVisibility`, not a `display` test, and it is the same trap the SPILL
  // walk above already carries a note about. `display` is not inherited, so
  // `getComputedStyle(el).display` inside a `display: none` SUBTREE returns
  // the element's own specified value — `flex`, `list-item` — and nothing
  // about the ancestor that is hiding it. Below the three-column breakpoint
  // `assets/desktop/` hides whichever of the list and the detail is not
  // the one you are looking at, and every row inside the hidden column then
  // measured zero and reported as an empty row. Measured: 384 findings on the
  // first desktop run, all `.session-item`, all of them a list the window is
  // too narrow to show. The message this check prints — "empty content
  // generates no line box" — was false for every one of them.
  for (const el of document.querySelectorAll('.diff-line, .setting-row, .drawer-item, .session-item')) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    if (!el.checkVisibility()) continue;
    if (el.getBoundingClientRect().height < 1) {
      const cls = typeof el.className === 'string' ? el.className.trim() : '';
      out.push(`COLLAPSED    .${cls.split(/\s+/).join('.')} has no height — empty content generates no line box`);
    }
  }

  // ONE BOX PAINTED OVER ANOTHER, WHICH IS NOT A QUESTION ANYTHING ABOVE PUTS.
  // #226 is the whole argument, including every number quoted below.
  //
  // Four defects found in one week live in cells this file already walks, in a
  // tree it already reports Clean on. #212 is a blank 268px panel over the
  // transcript, topmost in the hit test. #214 was the pane header over the
  // plane switch — the app's only way to change halves, unclickable, on a tree
  // where every one of the four gates passed. #215 is the overlay sidebar over
  // 252px of content. #209 is the jump-to-latest button over the last 52px of
  // the conversation. #215 states the gap in one sentence and it is the reason
  // this walk exists: "an element painted over another element is not one of
  // the questions the walk puts. Occlusion is the gap, not the widths."
  //
  // Nothing above can see it, and that is structural rather than accidental.
  // OVERFLOW-X is about leaving the viewport, CLIPPED-* about a box cutting its
  // own content, SPILL about ink outside a box, SQUARE and RADIUS-NEST about
  // corners, CONTRAST about a pair of colours. A covered control is correctly
  // laid out, correctly coloured and entirely inside its parent. It is simply
  // behind something.
  //
  // GETTING THE PREDICATE RIGHT IS THE WHOLE JOB, because a check that fires on
  // every modal is a check somebody turns off within a day — and it is not a
  // small effect. Measured on this tree with the one exemption below deleted:
  // **6,542 findings**, of which 6,192 are `div.modal-backdrop` and
  // `div.drawer-scrim.open` and `aside.drawer.open` covering the page they are
  // over on purpose. With it: **350**, every one of them a defect with an issue.
  // So the rule is stated three ways, and each is a claim rather than a filter:
  //
  //   AN INTERACTIVE TARGET. A paragraph behind a panel is a legibility
  //   question and this file has no vocabulary for it; a BUTTON behind a panel
  //   is a control that cannot be pressed, which is a fact `elementsFromPoint`
  //   can settle. Every one of the four defects above is that shape — #214's
  //   own commit message ends "only the hit test changes".
  //
  //   IT DOES NOT OWN THE TARGET. An element's own descendants paint over it by
  //   construction (the `<svg>` inside a button is above the button), and its
  //   ancestors are the box it lives in. Neither is an occlusion; both are
  //   containment, so both are exempt.
  //
  //   NOTHING FULL-SCREEN IS BETWEEN THEM. This is the one that separates a
  //   defect from a design. A modal SHOULD cover the page and a scrim SHOULD
  //   cover content — that is what they are for — and the property they share
  //   is that the page as a WHOLE has gone behind something, which is a
  //   statement to the reader rather than an accident. So the walk stops at the
  //   first box in the stack that spans the viewport in both axes: everything
  //   under it is deliberately not the subject. `.modal-backdrop`,
  //   `.drawer-scrim` and the phone's open drawer all pass out through that one
  //   sentence, with no class named anywhere.
  //
  // A STICKY HEADER OVER SCROLLED CONTENT IS CORRECT, and the answer to it is
  // the property this whole script already has rather than a fourth rule: every
  // state is measured AT REST. scripts/capture-gallery.py records markup with
  // no scroll offset, so at rest there is nothing scrolled up under a bar and
  // the scroller's own top padding is what keeps it that way — which SCRIM
  // above already checks. #214's bar, by contrast, covered the plane switch
  // with the page at its top, which is why it is a finding and a sticky header
  // is not.
  //
  // THE LAYER, NOT THE TOPMOST BOX. Naming whatever `elementsFromPoint` put
  // first fragments one cause into dozens: the overlay sidebar reported as
  // `span.nav-row-title`, `div.nav-sessions`, `div.navcard` and `p.nav-standing`
  // over the same three controls, 16 pairs of it, which is a wall of output for
  // one rule in one file. Measured: naming the topmost box gives **714**
  // findings and 61 distinct pairs; naming the layer gives 350 and 16, and the
  // 16 are three causes. The layer is defined rather than guessed — the
  // outermost ancestor of the blocker, stopping before the target's own box,
  // that is out of flow or claims a stacking order. That is precisely the
  // property that put it on top, and it names the element whose declaration is
  // the bug in all four defects: `.navpane` (absolute, z 3), `.fab` (absolute,
  // z 20), `.toast` (fixed, z 60) and #214's `.topbar` (STATIC, z 20 — a flex
  // item keeps its z-index, which is the whole of that bug and is why
  // `position` alone would have missed it).
  //
  // NINE POINTS, not one. A control covered down one side is the ordinary case
  // — the fab takes the right 59px of a 251px sidebar row, and its centre is
  // clear — so a centre-only test would report neither that nor #209. A 3x3
  // grid inset to 15% needs roughly a third of one axis to be covered before a
  // sample lands in it, which is a noise floor stated as geometry rather than
  // as a threshold nobody can derive.
  const INTERACTIVE = 'a[href], button, summary, input, select, textarea,'
    + ' [role="button"], [role="radio"], [role="tab"], [role="link"],'
    + ' [role="switch"], [role="checkbox"], [tabindex]:not([tabindex="-1"])';
  const overPage = (el) => {
    const b = el.getBoundingClientRect();
    return b.width >= vw - 0.5 && b.height >= vh - 0.5;
  };
  const covered = new Map();
  for (const el of document.querySelectorAll(INTERACTIVE)) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    if (!el.checkVisibility()) continue;
    // `pointer-events: none` and `disabled` both take the control out of the
    // hit test itself, so it is not IN the stack to be above or below anything
    // — that is a different fault and this walk would only ever report it as
    // "covered by everything". `.scroll-bottom` is the live example: it is
    // `opacity: 0; pointer-events: none` until src/viewport.rs adds
    // `body.away-from-bottom`, a class no capture contains — so for the whole
    // life of this check the disc was skipped here in every cell, which is why
    // #209 had to be found by hand. THE AXIS IS WHAT ANSWERS THAT, not a
    // softening of this line: AWAY_CLASS above drives the class the way the
    // shell attributes are driven, the skip below then stops applying, and the
    // run reports `button.scroll-bottom >> summary` (#261) on the phone.
    if (cs.pointerEvents === 'none' || el.disabled) continue;
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) continue;
    // Off the screen entirely is parked, OVERFLOW-X's word for it: the closed
    // drawer is translated off the left edge on purpose and everything in it
    // goes with it.
    if (r.right <= 0.5 || r.left >= vw - 0.5 || r.bottom <= 0.5 || r.top >= vh - 0.5) continue;
    for (const fy of [0.15, 0.5, 0.85]) {
      for (const fx of [0.15, 0.5, 0.85]) {
        const x = r.left + r.width * fx;
        const y = r.top + r.height * fy;
        if (x < 0 || y < 0 || x >= vw || y >= vh) continue;
        const stack = document.elementsFromPoint(x, y);
        const at = stack.indexOf(el);
        // Absent from its own stack is the STRONGEST form of this finding, not
        // a reason to skip: `elementsFromPoint` returns the ancestors of
        // whatever it hit, so a control that is not in the list at a point
        // inside its own box is a control something else answered for entirely.
        const above = at < 0 ? stack : stack.slice(0, at);
        if (above.some(overPage)) continue;
        const blocker = above.find((b) => !b.contains(el) && !el.contains(b));
        if (!blocker) continue;
        let layer = blocker;
        for (let p = blocker; p && !p.contains(el); p = p.parentElement) {
          const ps = getComputedStyle(p);
          if (/absolute|fixed|sticky/.test(ps.position) || ps.zIndex !== 'auto') layer = p;
        }
        const key = `${name(layer)} >> ${name(el)}`;
        covered.set(key, (covered.get(key) || 0) + 1);
      }
    }
  }
  for (const [key, n] of covered) {
    if (ledger.includes(key)) {
      out.push(`${mark}${key}`);
      continue;
    }
    const [layer] = key.split(' >> ');
    out.push(`OCCLUDED     ${key} at ${n} of 9 points — ${layer} is painted over a control`
      + ' it neither contains nor sits inside, with nothing full-screen between them');
  }
  return out;
};

// ── contrast ────────────────────────────────────────────────────────────
const CONTRAST = ({ mark, ledger }) => {
  // color-mix() resolves to color(srgb r g b / a) with components in 0..1
  // while rgb() gives 0..255. Scaling the wrong one makes every glass bar
  // read as near-black and invents a screenful of failures that are not there.
  const parse = (c) => {
    const k = c.startsWith('color(') ? 255 : 1;
    const m = c.match(/[\d.]+/g).map(Number);
    return { r: m[0] * k, g: m[1] * k, b: m[2] * k, a: m.length > 3 ? m[3] : 1 };
  };
  const over = (fg, bg) => ({
    r: fg.r * fg.a + bg.r * (1 - fg.a),
    g: fg.g * fg.a + bg.g * (1 - fg.a),
    b: fg.b * fg.a + bg.b * (1 - fg.a),
    a: 1,
  });
  const lum = (c) => {
    const f = (v) => { const s = v / 255; return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4; };
    return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b);
  };
  const ratio = (a, b) => {
    const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
    return (hi + 0.05) / (lo + 0.05);
  };
  // Walk up compositing translucent layers until something opaque is reached.
  const backdrop = (el) => {
    let cur = el;
    let acc = null;
    while (cur) {
      const c = parse(getComputedStyle(cur).backgroundColor);
      if (c.a > 0) acc = acc ? over(acc, c) : c;
      if (acc && acc.a >= 0.999) return acc;
      cur = cur.parentElement;
    }
    return acc || { r: 255, g: 255, b: 255, a: 1 };
  };

  const out = [];
  for (const el of document.querySelectorAll('*')) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    // A faded control is exempt from the text threshold; it is checked by
    // eye against the "still visible" bar instead.
    if (parseFloat(cs.opacity) < 0.99) continue;
    const hasOwnText = [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim());
    if (!hasOwnText) continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;

    const bg = backdrop(el);
    let fg = parse(cs.color);
    // FULLY TRANSPARENT INK PAINTS NOTHING, and compositing it over the
    // backdrop below would report the backdrop against itself — 1.00:1 on a
    // glyph no reader can see, which is a number about the arithmetic rather
    // than about the screen. The case is real and it is deliberate:
    // `.insp-file-tick` is a 14px box with a ✓ inside it and `color:
    // transparent` until the file is marked read, so that one element is both
    // states of the checkbox instead of two shapes swapping places
    // (`assets/desktop/90-inspector.css`, and the rsx at
    // `src/shell/desktop/inspector.rs` says the same thing from the other
    // side). What this gives up is an ink someone made transparent BY
    // ACCIDENT, and the mitigation is that such a glyph draws nothing at all:
    // whatever meaning it was carrying is carried by the box around it, and
    // the box is a background, which the fill walk further down measures.
    // `< 1` still composites, because a 40%-opaque ink is ink.
    if (fg.a === 0) continue;
    if (fg.a < 1) fg = over(fg, bg);
    const size = parseFloat(cs.fontSize);
    const large = size >= 18.66 || (size >= 14 && (parseInt(cs.fontWeight, 10) || 400) >= 700);
    const need = large ? 3 : 4.5;
    const got = ratio(fg, bg);
    if (got < need) {
      const cls = typeof el.className === 'string' ? el.className.trim() : '';
      const id = el.tagName.toLowerCase() + (cls ? `.${cls.split(/\s+/).join('.')}` : '');
      out.push(`CONTRAST     ${got.toFixed(2)}:1 (need ${need}) ${id} @${size}px  "${el.textContent.trim().slice(0, 34)}"`);
    }
  }

  // Icons carry no text of their own, so the walk above skips every one of
  // them — and an icon is often the only thing distinguishing two otherwise
  // identical rows. A chevron at 2.20:1 was the entire difference between a
  // settings row you can open and one you cannot. Non-text indicators want
  // 3:1 (WCAG 1.4.11), the same bar the stylesheet sets itself.
  for (const el of document.querySelectorAll('.icon')) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    if (parseFloat(cs.opacity) < 0.99) continue;
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    // Painted with `stroke: currentColor`, so `color` is the ink.
    const bg = backdrop(el);
    let fg = parse(cs.color);
    if (fg.a < 1) fg = over(fg, bg);
    const got = ratio(fg, bg);
    if (got < 3) {
      const owner = el.parentElement;
      const cls = owner && typeof owner.className === 'string' ? owner.className.trim() : '';
      out.push(`ICON-CONTRAST ${got.toFixed(2)}:1 (need 3) icon in ${cls ? `.${cls.split(/\s+/).join('.')}` : owner?.tagName.toLowerCase()}`);
    }
  }

  // AND EVERY INDICATOR THAT CARRIES ITS MEANING IN A BACKGROUND, WHICH BOTH
  // WALKS ABOVE ARE STRUCTURALLY BLIND TO.
  //
  // #213: `.dot.off` is the mark that says the app has lost its server, and on
  // the desktop band in light it measures **2.03:1** — a 6x6 square at two
  // thirds of the bar a non-text indicator owes. It survived every gate in this
  // repository because neither existing walk can see a dot at all. The text
  // walk needs an element with its own text and `<span class="dot off"></span>`
  // is empty; the icon walk is `querySelectorAll('.icon')` reading
  // `getComputedStyle(el).color`, because it is written for `stroke:
  // currentColor` SVGs, and a dot has no `.icon` class and spends its whole
  // meaning on `background-color`. So every background-painted indicator in the
  // app was unmeasured, in both themes.
  //
  // 3:1, WCAG 1.4.11, which is the icon walk's number four lines up and the
  // number `assets/desktop/30-sidebar-list.css` restates in its own words as "a
  // non-text indicator owes 3:1". Not 4.5 — that is the text bar, and this is
  // not text.
  //
  // A SHAPE, NOT A CLASS LIST. `.dot, .mark, .dot-anim` would have caught
  // exactly the three it landed on and nothing added tomorrow, which is the
  // disease this file keeps being treated for. The shape is: it paints a fill,
  // it carries no text anywhere under it, it has no element children, and it
  // has no border of its own — so the fill is the whole of what there is to
  // see. Measured over the whole grid on the day it landed, that predicate
  // reported **64 findings across three element families and nothing else**:
  // `.dot.off`, `.mark.idle`, `.dot-anim`, which is #213's list exactly,
  // arrived at without naming one of them. `.dot.off` has since been fixed and
  // is off the ledger below; the sentence is kept in the past tense rather than
  // rewritten, because the claim being made is about the PREDICATE and the
  // evidence for it is what it found before anything was repaired.
  //
  // The two conditions that made it that clean, each stated as a claim:
  //
  //   NO BORDER. An element with an edge of its own is a bordered box, and this
  //   design draws its boxes with hairlines that are deliberately well under
  //   3:1 — whether a hairline is enough of an edge is a real question, and it
  //   is a DIFFERENT one, with its own threshold and its own findings. Without
  //   this line the walk reports `input.field` at 1.00:1 in every state that has
  //   a text box, 54 of the 188 findings, because a field's fill is the page's
  //   by design and its border is what makes it a field. A `.dot` given a
  //   border to quiet this check has become a bordered box and owes that other
  //   rule instead — which nothing in this file asks yet, and is worth knowing.
  //
  //   NOT THE WHOLE PAGE. A scrim is a fill with no text and no border, and a
  //   scrim that reached 3:1 would be a blackout: `.drawer-scrim.open` is a 40%
  //   black wash measuring 1.18:1 in dark and 2.85:1 in light, and both numbers
  //   are the design working. An indicator is a mark ON something; something
  //   the size of the viewport is the something.
  //
  // WHAT IT STILL CANNOT SEE, said rather than implied: a fill painted by
  // `background-image` — a gradient, a mask — computes `backgroundColor:
  // rgba(0, 0, 0, 0)` and is skipped here, and nothing else in this file looks
  // at one either.
  const nameOf = (el) => {
    const cls = typeof el.className === 'string' ? el.className.trim() : '';
    return el.tagName.toLowerCase() + (cls ? `.${cls.split(/\s+/).join('.')}` : '');
  };
  const vw = document.documentElement.clientWidth;
  const vh = document.documentElement.clientHeight;
  const marked = new Set();
  for (const el of document.querySelectorAll('*')) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    if (!el.checkVisibility()) continue;
    if (parseFloat(cs.opacity) < 0.99) continue;
    // Anything with text under it answers to the text walk, and anything with
    // an element child is a container rather than a mark.
    if (el.children.length || el.textContent.trim()) continue;
    // Inside an `<svg>` is the icon walk's subject, painted with `fill` and
    // `stroke` off `currentColor` rather than with a background.
    if (el.ownerSVGElement) continue;
    const r = el.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) continue;
    const own = parse(cs.backgroundColor);
    if (own.a === 0) continue;
    if (['Top', 'Right', 'Bottom', 'Left'].some((s) => parseFloat(cs[`border${s}Width`]) > 0)) continue;
    if (r.width >= vw - 0.5 && r.height >= vh - 0.5) continue;
    // The ground is what is behind the mark, so the walk starts at the PARENT:
    // `backdrop(el)` composites the element's own fill first and would measure
    // the dot against itself.
    const bg = el.parentElement ? backdrop(el.parentElement) : { r: 255, g: 255, b: 255, a: 1 };
    const got = ratio(own.a < 1 ? over(own, bg) : own, bg);
    if (got >= 3) continue;
    const id = nameOf(el);
    if (ledger.includes(id)) {
      marked.add(`${mark}${id}`);
      continue;
    }
    out.push(`FILL-CONTRAST ${got.toFixed(2)}:1 (need 3) ${id} ${r.width.toFixed(0)}x${r.height.toFixed(0)}`
      + ' — a mark with no text, no children and no edge, so its fill is the whole of it');
  }
  out.push(...marked);
  return out;
};

// ── `node docs/audit.js fonts` ──────────────────────────────────────────
// How far the stand-ins are from the faces an iPhone renders, measured rather
// than asserted, over the text this app actually puts on screen.
//
// Only a Mac can run it, because only a Mac has the files: San Francisco, New
// York and SF Mono are Apple's, they ship with the OS and they may not be
// vendored into a repository. That is the whole reason the three faces in
// docs/fonts/ exist. So this is not part of the gate — CI could not run it —
// it is the evidence for the choice the gate depends on, kept runnable so the
// numbers in the comment at the top of this file cannot quietly stop being
// true.
//
// It fails if a median leaves ±5%. That bound is not a taste: at 320pt and AX5
// the tightest boxes in this design have single-digit percentages of slack, so
// a stand-in that is 8% out — which is exactly where Liberation Sans and
// Liberation Serif sit, and exactly why CI disagreed with this laptop — is a
// stand-in that decides findings by itself.
const MEDIAN_BOUND = 0.05;

const compareFonts = async (states) => {
  const missing = FONTS.filter((font) => !fs.existsSync(font.applePath));
  if (missing.length) {
    console.error(`this comparison needs the real faces, which only macOS has: ${missing.map((font) => `${font.standsFor} (${font.applePath})`).join(', ')} not found`);
    process.exit(1);
  }
  // Every run of text the app puts on screen, and every word in it — the word
  // is the unit that decides a min-content width, and a min-content width is
  // what decided the argument this whole block came out of.
  const strings = new Set(Object.values(LONGEST));
  for (const state of states) {
    for (const [, text] of state.body.matchAll(/>([^<>]+)</g)) {
      const t = text.replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').trim();
      if (t) strings.add(t);
    }
  }
  const words = new Set();
  for (const text of strings) for (const word of text.split(/\s+/)) if (word) words.add(word);

  const real = FONTS.map((font) => `@font-face{font-family:"${font.standsFor}";`
    + `src:url(data:font/ttf;base64,${fs.readFileSync(font.applePath).toString('base64')});`
    + 'font-weight:100 900;font-display:block;}').join('\n');
  const browser = await chromium.launch(LAUNCH);
  const page = await browser.newPage();
  await page.setContent(`<!doctype html><html><head><style>${FONT_CSS}\n${real}</style></head><body></body></html>`);

  const runs = await page.evaluate(async ({ pairs, strings, words, scales }) => {
    // A face nothing uses is a face the browser has not fetched, and canvas
    // would then measure the fallback and report a flat 1.000 — which reads
    // as a perfect match. Load each one by name first.
    for (const [stand, real] of pairs) {
      for (const family of [stand, real]) {
        try { await document.fonts.load(`100px "${family}"`, 'Hxg'); } catch { /* reported below */ }
      }
    }
    await document.fonts.ready;
    const span = document.createElement('span');
    span.style.cssText = 'position:absolute;white-space:pre;';
    document.body.appendChild(span);
    const width = (text, family, size, weight) => {
      span.style.font = `${weight} ${size}px "${family}"`;
      span.textContent = text;
      return span.getBoundingClientRect().width;
    };
    const out = [];
    for (const [stand, real] of pairs) {
      for (const [what, list] of [['strings', strings], ['words', words]]) {
        for (const { root, size, weight } of scales) {
          const ratios = list.map((text) => {
            const base = width(text, real, size, weight);
            return { text, r: base > 0 ? width(text, stand, size, weight) / base : 1 };
          }).filter((x) => Number.isFinite(x.r)).sort((a, b) => a.r - b.r);
          out.push({
            stand,
            real,
            what,
            root,
            size,
            weight,
            min: ratios[0],
            median: ratios[Math.floor(ratios.length / 2)],
            max: ratios[ratios.length - 1],
            n: ratios.length,
          });
        }
      }
    }
    return out;
  }, {
    pairs: FONTS.map((font) => [font.family, font.standsFor]),
    strings: [...strings],
    words: [...words],
    // The body size and the label size at each of the four roots this walks,
    // which is the range every measurement in the run happens inside.
    scales: SCALES.flatMap((root) => [
      { root, size: root, weight: 400 },
      { root, size: root * 0.875, weight: 600 },
    ]),
  });
  await browser.close();

  let worst = 0;
  for (const run of runs) {
    worst = Math.max(worst, Math.abs(run.median.r - 1));
    console.log(`${run.stand} vs ${run.real.padEnd(14)} ${run.what.padEnd(7)} root ${String(run.root).padStart(2)}px @${run.size.toFixed(2)}px/${run.weight}  `
      + `median ${run.median.r.toFixed(3)}  min ${run.min.r.toFixed(3)} ("${run.min.text.slice(0, 24)}")  max ${run.max.r.toFixed(3)} ("${run.max.text.slice(0, 24)}")  n=${run.n}`);
  }
  console.log(`\nWidest median deviation: ${(worst * 100).toFixed(1)}% (bound ${(MEDIAN_BOUND * 100).toFixed(0)}%).`);
  if (worst > MEDIAN_BOUND) {
    console.error('a stand-in has drifted past the bound this gate rests on — pick a closer face or move the bound and say why');
    process.exit(1);
  }
};

(async () => {
  const arg = process.argv[2] || 'both';
  const themes = arg === 'both' ? ['light', 'dark'] : [arg];

  // The states are captured out of the running app by
  // scripts/capture-gallery.py — the same data the gallery is built from, so
  // this audits markup the app actually produced rather than a transcription
  // of it.
  if (!fs.existsSync(STATES)) {
    console.error(`no ${STATES}; run scripts/capture-gallery.py first`);
    process.exit(1);
  }
  // Every state is measured at rest. scripts/capture-gallery.py captures a
  // screen at the top of its scroller and records markup alone, so there is no
  // offset to replay — a scrolled state would have to be captured as one, and
  // the machinery for replaying an offset here was carrying a comment about
  // behaviour no run could produce. What that leaves uncovered is named in
  // docs/design.md with the rest of this script's blind spots.
  // Which shell drew a state is in its key and nowhere else — see
  // DESKTOP_PREFIX. It decides three things at once: which stylesheets the
  // rebuilt page links, which viewport sizes it is walked at, and whether the
  // shell's own collapses are walked as a second axis.
  const states = Object.entries(JSON.parse(fs.readFileSync(STATES, 'utf8')))
    .map(([label, body]) => ({ label, body, desktop: label.startsWith(DESKTOP_PREFIX) }));
  if (states.length === 0) {
    console.error(`${STATES} is empty`);
    process.exit(1);
  }
  // Before a browser is launched, because this is a question about the input
  // rather than about the pixels — and because the answer this used to give
  // when the input was half missing was "Clean". See `coverage`.
  const gaps = coverage(states);
  if (gaps.length) {
    console.error(`${STATES} does not cover what this run claims to check:\n  ${gaps.join('\n  ')}`);
    process.exit(1);
  }
  states.push(...stressed(states));

  if (arg === 'fonts') {
    await compareFonts(states);
    return;
  }

  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'ui-audit-'));
  // One file, linked last by every page: the faces are ~140KB of base64 and
  // writing them into each of the 250 documents would be 35MB of temp churn
  // for a string that never changes.
  const fontSheet = path.join(tmp, 'audit-fonts.css');
  fs.writeFileSync(fontSheet, FONT_CSS);
  const atRestSheet = path.join(tmp, 'audit-at-rest.css');
  fs.writeFileSync(atRestSheet, AT_REST_CSS);
  // Per state, not hoisted: the desktop sheets are linked from what the STATE
  // is, so there is no arrangement of this script in which they reach a phone
  // frame. Font sheet last, because it only repoints `:root`'s three font
  // tokens and has to beat every sheet that names them.
  const sheetsFor = (state) => (state.desktop
    ? [...STYLESHEETS, ...DESKTOP_SHEETS, atRestSheet, fontSheet]
    : [...STYLESHEETS, atRestSheet, fontSheet]);
  const browser = await chromium.launch(LAUNCH);

  // Every character this run will lay out, taken from the markup itself
  // rather than from a guess about which ones matter — plus the ellipsis,
  // which no state contains and every line clamp draws.
  const corpus = [...new Set(['…', ...states.map((s) => s.body).join(''),
    ...Object.values(LONGEST).join('')])].join('');
  {
    const page = await browser.newPage();
    await page.setContent(`<!doctype html><html><head><style>${FONT_CSS}</style></head><body>`
      + FONTS.map((font, i) => `<div id="f${i}" style="font-family:${STACK(font)}">`
        + corpus.replace(/[&<>]/g, (c) => `&#${c.charCodeAt(0)};`) + '</div>').join('')
      + '</body></html>');
    await page.evaluate(() => document.fonts.ready);
    const leaked = [];
    for (const [i, font] of FONTS.entries()) {
      for (const used of await platformFonts(page, `#f${i}`)) {
        if (!used.isCustomFont) leaked.push(`${font.token} (${font.file} then ${LEFTOVERS.map((f) => f.file).join(' then ')}) fell through to the host's ${used.familyName} for ${used.glyphCount} glyph${used.glyphCount > 1 ? 's' : ''}`);
      }
    }
    await page.close();
    if (leaked.length) {
      console.error(`the measurement faces do not cover this app's text, so this run would measure the host's fonts and disagree with CI:\n  ${leaked.join('\n  ')}`);
      process.exit(1);
    }
  }

  let findings = 0;
  // Which ledger entries the run actually produced. A ledger of what is broken
  // is only a ledger while every line of it is still true, and the check at the
  // end of this file is what makes it one.
  const ledgerUsed = new Set();

  for (const theme of themes) {
    const page = await browser.newPage({
      viewport: { width: SIZES[0].width, height: SIZES[0].height },
    });
    // The theme is a parameter of the run; the motion preference is not, and
    // until this line it came from whatever the host had set. It reaches this
    // file's numbers through exactly one rule — assets/shared.css's
    // `@media (prefers-reduced-motion: reduce)` block puts `.scroll-bottom` at
    // its revealed position AT REST — so a developer with Reduce Motion on
    // would read the disc 8px from where CI reads it and the `away` axis below
    // would report a transition it did not catch. Pinned for the reason the
    // fonts are: a number that depends on the machine is not a number.
    await page.emulateMedia({ colorScheme: theme, reducedMotion: 'no-preference' });

    for (const [i, state] of states.entries()) {
      const file = path.join(tmp, `state-${i}.html`);
      fs.writeFileSync(file,
        '<!doctype html><html lang="en"><head><meta charset="utf-8">'
        + sheetsFor(state).map((href) => `<link rel="stylesheet" href="${href}">`).join('')
        + `</head><body>${state.body}</body></html>`);
      await page.goto(`file://${file}`, { waitUntil: 'load' });
      // `load` fires when the stylesheets have arrived, not when the faces
      // they name have been decoded — and the first walk after a navigation
      // is close enough to it to catch the fallback metrics. Measured before
      // this line went in: intermittent findings that moved between runs of
      // the same commit, which is the failure this whole block exists to end.
      await page.evaluate(() => document.fonts.ready);
      const leaks = await page.evaluate(familyLeaks, PINNED);
      if (leaks.length) {
        console.error(`${state.label} [${theme}] lays out text in a family this run did not pin, so its numbers are the host's:\n  ${leaks.join('\n  ')}`);
        process.exit(1);
      }
      if (state.swap) {
        await page.evaluate((swap) => {
          for (const [sel, text] of Object.entries(swap)) {
            document.querySelectorAll(sel).forEach((el) => {
              // Never into a wrapper. The longest string belongs in the
              // element that holds the text, and writing it onto a parent
              // deletes the parent's other children — stressing .chip-label
              // that way would take the effort tier out of the chip and audit
              // an arrangement the app does not build.
              if (el.firstElementChild) return;
              el.textContent = text;
            });
          }
        }, state.swap);
      }
      // One document, walked once per phone size and once per text size
      // within that. Both are set from here rather than written into the
      // document because the state is otherwise identical — same markup, same
      // swap — and a reflow is far cheaper than a navigation: this list costs
      // 1176 resizes and no extra page loads.
      //
      // Size outside scale rather than inside, so the flush below runs once
      // per size instead of once per size and scale.
      //
      // WHICH grid, from the state. A phone state walks phone sizes at four
      // text sizes; a desktop state walks window sizes at the one root a
      // desktop build has, and walks the shell's own collapses instead of a
      // text scale. Two products, not one — the summary line below states them
      // separately for that reason.
      const sizes = state.desktop ? DESKTOP_SIZES : SIZES;
      const scales = state.desktop ? DESKTOP_SCALES : SCALES;
      const navs = state.desktop ? DESKTOP_SHELL : [null];
      // The disc is the app's, not the shell's, so this axis is asked of both
      // grids — and of the states that have one, which is what `stateIsAway`
      // reads off the markup.
      const disc = stateIsAway(state.body);
      const aways = disc ? AWAY_CELLS : [false];
      for (const size of sizes) {
        await page.setViewportSize({ width: size.width, height: size.height });
        // A resize is a reflow like the font-size below, with one exception
        // that costs phantom findings. A closed <details> is a
        // content-visibility-locked subtree, and Chromium does not re-lay it
        // out when the viewport NARROWS: its cached rect keeps the previous
        // size's inline size until something inside it is measured again. The
        // walk is that something, so the first walk after a narrowing resize
        // reads the last size's numbers — the code card's <pre>, whose right
        // edge is 385 at 402pt, reported at 423 when 402 is stepped down from
        // 440, which is 21px past a viewport it is 17px inside. Touching
        // every rect once is what forces the relayout; reading
        // document.body.offsetHeight is not, because a page-level reflow is
        // precisely what a locked subtree skips.
        //
        // It does not bite in the order SIZES is actually in, because that
        // list only ever widens and the narrowing step back to the front of it
        // lands on a document that has just been navigated and never measured.
        // That is a property of the list, not of the walk. Against the
        // stylesheet in the tree, which this reports Clean: reverse SIZES with
        // this line deleted and the run reports 16 findings against a true
        // zero — the <pre> at 423 in a 402 viewport it is stale from 440, and
        // at 343 in a 320 it is stale from 360 — and put it back and the
        // reversed list is Clean again. Measured at no cost, because the walk
        // was going to force that layout a moment later anyway, so it buys the
        // result's independence from the order of a list somebody will
        // eventually reorder.
        //
        // Do not delete it as a no-op. Without it the walk still reports; it
        // reports the wrong size, and on the steps where the stale value is
        // the SMALLER one that is a finding which silently passes.
        await page.evaluate(() => {
          for (const el of document.querySelectorAll('*')) el.getBoundingClientRect();
        });
        // An inline style on <html> beats every stylesheet rule, which is what
        // makes it a simulation of the Dynamic Type opt-in rather than a rule
        // competing with one.
        for (const [s, scale] of scales.entries()) {
          await page.evaluate((px) => {
            document.documentElement.style.fontSize = `${px}px`;
          }, scale);
          for (const [n, nav] of navs.entries()) {
            // An attribute the sheet reads, set the way the shell sets it.
            // Nothing to restore between states, since every page here is
            // navigated fresh from the captured markup.
            if (nav) {
              const problems = await page.evaluate(({ attrs, all }) => {
                const shell = document.querySelector('.shell');
                if (!shell) return ['it has no `.shell` element at all'];
                // READ FIRST, then write. The captured values are the app's own
                // and they are the evidence; overwriting them and reading back
                // would only ever confirm this walk's own assignment.
                const before = Object.fromEntries(all.map((a) => [a, shell.getAttribute(a)]));
                // Set here for the same reason `data-nav` is, and for one of
                // its own each. The captured markup can only ever say "false"
                // for `data-fullscreen`, because a window being driven for a
                // capture is not a window in fullscreen; and it says whatever
                // the operator's own stored `inspector_open` said for
                // `data-insp`, which is a fact about a machine rather than
                // about this app.
                for (const [name, value] of Object.entries(attrs)) shell.setAttribute(name, value);
                return all
                  .filter((a) => before[a] === null)
                  .map((a) => `its \`.shell\` carries no \`${a}\``);
              }, { attrs: nav.attrs, all: SHELL_ATTRS });
              // AND IT SAYS SO WHEN IT COULD NOT, which `if (!shell) return;`
              // did not. Everything the shell axis buys is bought by these
              // attributes: with no `.shell` to put them on, the cells are
              // that many walks of one identical frame and the summary line
              // goes on advertising a shell axis. Measured: rename the
              // class consistently — in `src/shell/desktop.rs`'s markup and in
              // both desktop sheets, which is what an ordinary refactor does —
              // and the layout is untouched, the axis quietly stops existing
              // and `node docs/audit.js both` reports **Clean**. It is the
              // exact failure DESKTOP_SHELL was added to end, one level down.
              //
              // WIDENED FROM `.shell` ALONE to every attribute in SHELL_ATTRS,
              // because "the element is there" was only the first of the ways
              // this axis can quietly stop existing. The other is per
              // attribute, and it is silent in the same way: the sheet keys a
              // block off a name the app no longer writes on that element,
              // every cell renders the frame the capture already had, and the
              // grid reports Clean over an axis that walked one frame four
              // times. Reported and not measured — an axis that is not there
              // is a fact about the instrument, and a broken instrument does
              // not get to return a number.
              if (problems.length) {
                console.error(`${state.label} is keyed as a desktop state but ${problems.join('; ')}`
                  + `, so the shell axis (${DESKTOP_SHELL.map((c) => c.label).join(' / ')}) would`
                  + ` walk the same frame ${DESKTOP_SHELL.length} times — have they been renamed or moved in`
                  + ' src/shell/desktop/mod.rs without being renamed here, or is'
                  + ' docs/gallery-states.json older than that change?');
                process.exit(1);
              }
            }
            for (const away of aways) {
            // WHERE THE SCROLLER IS, which is #250 and is the fourth thing
            // this walk drives. See AWAY_CLASS above for why it is an axis
            // rather than a capture; the check here is the one the shell
            // attributes get, for the same reason — an axis that is not
            // there is a fact about the instrument, and a broken instrument
            // does not get to return a number.
            if (disc) {
              const unsettled = await page.evaluate(AWAY_SETTLED, { cls: AWAY_CLASS, on: away });
              if (unsettled.length) {
                console.error(`${state.label} carries a \`.scroll-bottom\` but ${unsettled.join('; ')}`
                  + ` — so the \`${away ? 'scrolled up' : 'at the bottom'}\` cell of the`
                  + ' away-from-bottom axis is not the frame it claims to be. Has'
                  + " assets/shared.css's `body.away-from-bottom .scroll-bottom` been"
                  + ' renamed, or src/viewport.rs stopped writing that class?');
                process.exit(1);
              }
            }
            // Contrast at the smallest scale and the reference size only. It
            // is a walk over computed colours: the 18.66px large-text
            // threshold makes every larger scale strictly more permissive,
            // and a wider phone moves boxes rather than colours. Honest to
            // gate on the size only because the two checks that were in this
            // function and were NOT about colour — the scrim covering the bar,
            // and a row that measures nothing — have moved into GEOMETRY,
            // where they are walked at every size like the geometry they are.
            //
            // And at the FIRST nav state, which is the captured one: a
            // closed nav is `visibility: hidden`, which the colour walk
            // skips element by element, so the shut column can only ever
            // report a subset of what the open one does.
            //
            // AND THROUGH BOTH THEME MECHANISMS, WHICH IS #165.
            //
            // `grep -c data-theme docs/audit.js` was 0 before this: the theme
            // came from `page.emulateMedia({ colorScheme })` and nothing else,
            // which drives `prefers-color-scheme` alone. CLAUDE.md states the
            // app's mechanism as "semantic tokens, light and dark, with
            // `data-theme` on the root element overriding the system
            // preference", and every palette in the tree is written twice for
            // it — `assets/shared.css:283` and `:336`,
            // `assets/desktop/00-tokens.css:715` and `:752`, plus the same pair
            // in 20-plane-switch.css and twice in 30-sidebar-list.css. The
            // second of each pair was rendered by nothing at all, in either
            // shell, and #123 had already found seven tokens missing from one
            // of them. A rule no frame ever renders is indistinguishable from a
            // rule that works.
            //
            // THE OPPOSITE OF THE EMULATED THEME, which is what makes this one
            // extra cell cover both holes instead of neither. Setting the
            // attribute to the theme already being emulated renders the media
            // block and the attribute block on top of each other, so a
            // difference between them is invisible; setting it to the opposite
            // renders the attribute block ALONE. Over the two theme passes that
            // is `data-theme="dark"` under a light system — the arm nothing has
            // ever rendered — and `data-theme="light"` under a dark one, which
            // is the app-says-light-while-the-system-says-dark path #165 also
            // names. Both are what a reader who has used the in-app override
            // actually sees.
            //
            // CONTRAST ONLY, and measured rather than assumed: with the whole
            // grid walked twice, once with the attribute set, GEOMETRY returns
            // a byte-identical finding set in every cell — 0 differences across
            // 124 states x 2 themes x every size, scale and shell cell. These
            // blocks are colour and nothing else, so the geometry axis would be
            // paying for a second product to re-report what it already said.
            let colour = [];
            if (s === 0 && n === 0 && size.reference) {
              colour = await page.evaluate(CONTRAST, { mark: LEDGER_MARK, ledger: FILL_KNOWN });
              const override = theme === 'light' ? 'dark' : 'light';
              await page.evaluate((v) => {
                document.documentElement.setAttribute('data-theme', v);
              }, override);
              colour = colour.concat(
                (await page.evaluate(CONTRAST, { mark: LEDGER_MARK, ledger: FILL_KNOWN }))
                  // A finding the ledger already holds comes back marked and is
                  // not a finding, so it must not be labelled as one — the mark
                  // is the whole string and the driver matches it exactly.
                  .map((z) => (z.startsWith(LEDGER_MARK) ? z : `${z}  [via data-theme="${override}"]`)),
              );
              // Removed rather than left: this page is reused for every size
              // after this one, and the reference size is not the last of them.
              await page.evaluate(() => document.documentElement.removeAttribute('data-theme'));
            }
            const walked = [...new Set([
              ...await page.evaluate(GEOMETRY, { mark: LEDGER_MARK, ledger: OCCLUSION_KNOWN }),
              ...colour,
            ])];
            const issues = walked.filter((str) => {
              if (!str.startsWith(LEDGER_MARK)) return true;
              ledgerUsed.add(str.slice(LEDGER_MARK.length));
              return false;
            });
            if (issues.length) {
              findings += issues.length;
              console.log(`\n${state.label}  [${theme}, ${size.width}x${size.height}, root ${scale}px`
                + `${nav ? `, ${nav.label}` : ''}${away ? ', scrolled up' : ''}]`);
              issues.forEach((str) => console.log(`  ${str}`));
            }
            }
          }
        }
      }
    }
    await page.close();
  }

  await browser.close();
  fs.rmSync(tmp, { recursive: true, force: true });

  // The sizes are named rather than counted: an unnamed count is exactly how a
  // coverage claim rots. The two walks are stated separately for the same
  // reason — GEOMETRY runs the whole product below, CONTRAST runs one cell of
  // it, and one sentence over both would claim 24x the colour coverage this
  // script has.
  //
  // AND THE TWO SHELLS ARE STATED SEPARATELY, for exactly that reason one
  // level up. They are different grids — six phone sizes at four text sizes
  // against nine window sizes at one, with the shell's own collapses in place
  // of the text axis — so a single sentence over both would claim four times the
  // scale coverage the desktop half has.
  const themeCount = `${themes.length} theme${themes.length > 1 ? 's' : ''}`;
  const count = (desktop) => states.filter((z) => !!z.desktop === desktop).length;
  // The away axis is not a multiplier over the whole half — it is asked of the
  // states that have a disc — so it is stated as the subset it is. A count
  // dressed up as a factor would claim twice the coverage this run has.
  const discs = (desktop) => states.filter((z) => !!z.desktop === desktop && stateIsAway(z.body)).length;
  const grid = (desktop) => {
    const sizes = desktop ? DESKTOP_SIZES : SIZES;
    const scales = desktop ? DESKTOP_SCALES : SCALES;
    const n = discs(desktop);
    return `${count(desktop)} ${desktop ? 'desktop' : 'phone'} states x ${themeCount}`
      + ` x ${sizes.length} ${desktop ? 'window' : 'phone'} sizes (${sizes.map((z) => `${z.width}x${z.height}`).join('/')})`
      + ` x ${scales.length} text size${scales.length > 1 ? 's' : ''} (${scales.join('/')}px)`
      + (desktop
        ? ` x ${DESKTOP_SHELL.length} shell states (`
          // ' / ' and not '/': every label names two collapses now and carries
          // a comma of its own, so the tighter separator ran them together.
          + `${DESKTOP_SHELL.map((c) => c.label).join(' / ')})`
        : '')
      + (n ? `, of which the ${n} that hold a jump-to-latest disc are walked in`
        + ' both of the scroller\'s states (at the bottom / `body.away-from-bottom`)' : '');
  };
  const shells = [false, ...(count(true) ? [true] : [])];
  const scope = shells.map(grid).join(', and ');
  const contrastScope = shells
    .map((desktop) => {
      const [ref] = REFERENCE[desktop];
      const scales = desktop ? DESKTOP_SCALES : SCALES;
      return `${ref.width}x${ref.height} at root ${scales[0]}px`;
    })
    .join(' and ');
  // WHICH MECHANISM THE COLOUR WALK MEASURED, stated for the reason every other
  // number in this sentence is: before #165 it said "2 themes" over one of two,
  // and a coverage claim that names only the half it happens to walk is how the
  // other half stays unwalked for the life of a feature.
  const themeScope = `\`prefers-color-scheme\` and, in the same cell with the`
    + ' attribute set to the OTHER theme, `data-theme` on the root';
  // THE OTHER DIRECTION OF THE LEDGERS, which is what makes them ledgers rather
  // than suppressions — the shape `UNCAPTURED` uses in
  // src/shell/desktop/mod.rs, for its reason: an allowance for a defect that
  // has since been fixed is a line nobody can check, and it quietly widens the
  // gate every time the thing it named comes back.
  //
  // A `both` RUN ONLY. Occlusion is geometry and produces the identical 16
  // pairs in either theme, but a fill's contrast does not — `span.mark.idle` is
  // 2.52:1 in dark and 4.01:1 in light, so it fires in one pass and not the
  // other. A single-theme run walks half the grid and may not judge a ledger
  // written for all of it; CI runs `both`, which is where this bites.
  if (arg === 'both') {
    const stale = [...OCCLUSION_KNOWN, ...FILL_KNOWN].filter((e) => !ledgerUsed.has(e));
    if (stale.length) {
      console.error(`${stale.length} ledger entr${stale.length > 1 ? 'ies name' : 'y names'} a defect this run did not find:`
        + `\n  ${stale.join('\n  ')}\n`
        + 'These lists may only shrink. If the defect is fixed, delete the'
        + ' line — so that the next time it comes back it is a finding and not'
        + ' a line in a list that has stopped being true. If a capture stopped'
        + ' rendering the screen it was on, that is the store having gone stale'
        + ' and the entry still cannot stay.');
      process.exit(1);
    }
  }
  if (findings) {
    console.log(`\n${findings} finding${findings > 1 ? 's' : ''} across ${scope}`
      + ` (contrast at ${contrastScope} only, through ${themeScope}).`);
    process.exit(1);
  }
  console.log(`Clean: no geometry findings across ${scope};`
    + ` no contrast findings at ${contrastScope}, which is where that walk runs,`
    + ` through ${themeScope}.`
    + `\nThe ledgers hold ${OCCLUSION_KNOWN.length} occlusion pair(s) and`
    + ` ${FILL_KNOWN.length} indicator(s) that DID fail and are known — see`
    + ' OCCLUSION_KNOWN and FILL_KNOWN, which may only shrink.');
})();
