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
//   contrast  every element carrying its own text, composited against the
//             first opaque background behind it, against 4.5:1 (3:1 for large
//             or bold text) — and every icon, which carries no text of its
//             own and so is invisible to that walk, against 3:1.
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
// assets/main.css is the design system and comes first, and a feature brings
// assets/features/<name>.css of its own. Linking main.css alone would rebuild
// every feature screen unstyled and then measure the result, which passes.
const ASSETS = path.join(__dirname, '..', 'assets');
const FEATURE_CSS = path.join(ASSETS, 'features');
const STYLESHEETS = [
  path.join(ASSETS, 'main.css'),
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
// between a styled `.skill-body` and an unstyled one. Empty assets/main.css
// and it is the loud version of the same fault — 73664 findings about
// `<textarea>`'s default 182x21 box and buttons at radius 0, which read as a
// design regression rather than as a missing file. One stat each, at startup,
// so neither outcome has to be diagnosed from the findings.
for (const sheet of STYLESHEETS) {
  if (!fs.existsSync(sheet) || fs.statSync(sheet).size === 0) {
    console.error(`${sheet} is missing or empty — every screen it styles would be measured against the UA defaults`);
    process.exit(1);
  }
}

// ── the faces this measures with, and why they are not the app's ────────
// THE GATE SHIPS ITS OWN FONTS. Every check below is a measurement of text,
// and until this block existed the text was laid out in whatever face the
// host machine happened to have — so the same commit was Clean here and 24
// findings on ubuntu-latest. Not a flake and not a difference of opinion
// about geometry: a difference of advance widths.
//
// What each environment actually resolved, measured with CDP's
// CSS.getPlatformFontsForNode against assets/main.css's three stacks:
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
//     assets/main.css grew a `max-width` for exactly that reason, and the
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
// fails if any character reaches past it. 796 bytes: Google Fonts' css2 API
// will cut a face down to a given string, and this is Noto Sans Math holding
// that one glyph. Regenerate the same way if this list ever grows —
//   curl -H 'User-Agent: <a Chrome UA>' \
//     'https://fonts.googleapis.com/css2?family=Noto+Sans+Math&text=%E2%8B%AF'
// — and follow the src: url() it answers with.
const LEFTOVERS = {
  family: 'Audit Leftovers',
  file: 'noto-sans-math-U22EF.woff2',
  standsFor: 'whatever the host would have chosen',
  // A glyph borrowed from a fourth face brings that face's ascent and descent
  // into the line box it lands in, so this one is overridden like the others.
  // San Francisco's numbers, which are also SF Mono's; the serif is 95/24 and
  // the two points of difference reach exactly one character on one screen.
  metrics: { ascent: 97, descent: 21, lineGap: 0 },
};
for (const font of [...FONTS, LEFTOVERS]) {
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
const STACK = (font) => `'${font.family}', '${LEFTOVERS.family}'`;
const FONT_CSS = [...FONTS, LEFTOVERS].map(face).join('\n')
  // Last sheet in the document, so this beats assets/main.css's :root on
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

const PINNED = [...FONTS, LEFTOVERS].map((font) => font.family);
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

// CONTRAST runs at the reference size alone, so that flag is not a label: it
// is the entire scope of one of this script's two walks. Drop the key while
// rewriting the list and the colour walk stops running while the summary goes
// on saying it found nothing — measured, with `.session-title, .setting-value,
// .banner { color: #bbbbbb }` appended to main.css: 158 real contrast failures
// reported as Clean. Asserted rather than defaulted, because "whichever one
// happens to be first" is not a reference size.
const REFERENCE = SIZES.filter((size) => size.reference);
if (REFERENCE.length !== 1) {
  console.error(`exactly one SIZES entry must carry \`reference\` — it is where CONTRAST runs; found ${REFERENCE.length}`);
  process.exit(1);
}

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
    swap: Object.fromEntries(hits),
  }];
});

// ── geometry ────────────────────────────────────────────────────────────
const GEOMETRY = () => {
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
    // So the rule is stated instead of being waited for. assets/main.css:
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
    const fullScreen = r.width >= vw - 0.5 || r.height >= vh - 0.5;
    // Nor is a row a surface. Something that fills its clipping parent from
    // edge to edge already has that parent's corners — rounding it as well
    // is what rule 4 means by concentric, and doing it to each row of a diff
    // would notch every join between two consecutive rows.
    const clip = clipper(el);
    const flush = clip && Math.max(...rad(getComputedStyle(clip))) > 0
      && r.width + 0.5 >= clip.clientWidth;
    if ((filled || boxed) && !fullScreen && !flush && Math.max(...rad(cs)) === 0
        && r.width > 24 && r.height > 12 && tag !== 'html' && tag !== 'body') {
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
  const bar = document.querySelector('.topbar');
  if (bar) {
    const heading = bar.querySelector(':scope > .title, :scope > .titlegroup');
    if (heading) {
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
      for (const group of bar.querySelectorAll(':scope > .icon-btn, :scope > .topbar-actions')) {
        const g = group.getBoundingClientRect();
        const over = Math.min(h.right, g.right) - Math.max(h.left, g.left);
        if (over > 0.5) {
          out.push(`TITLE-COLLIDE ${name(heading)} overlaps ${name(group)} by ${over.toFixed(0)}px`);
        }
      }
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
  const app = document.querySelector('.app');
  const scroller = document.querySelector('.scroll');
  if (app && bar) {
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
  for (const el of document.querySelectorAll('.diff-line, .setting-row, .drawer-item, .session-item')) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    if (el.getBoundingClientRect().height < 1) {
      const cls = typeof el.className === 'string' ? el.className.trim() : '';
      out.push(`COLLAPSED    .${cls.split(/\s+/).join('.')} has no height — empty content generates no line box`);
    }
  }
  return out;
};

// ── contrast ────────────────────────────────────────────────────────────
const CONTRAST = () => {
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
  const states = Object.entries(JSON.parse(fs.readFileSync(STATES, 'utf8')))
    .map(([label, body]) => ({ label, body }));
  if (states.length === 0) {
    console.error(`${STATES} is empty`);
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
  const sheets = [...STYLESHEETS, fontSheet];
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
        if (!used.isCustomFont) leaked.push(`${font.token} (${font.file} then ${LEFTOVERS.file}) fell through to the host's ${used.familyName} for ${used.glyphCount} glyph${used.glyphCount > 1 ? 's' : ''}`);
      }
    }
    await page.close();
    if (leaked.length) {
      console.error(`the measurement faces do not cover this app's text, so this run would measure the host's fonts and disagree with CI:\n  ${leaked.join('\n  ')}`);
      process.exit(1);
    }
  }

  let findings = 0;

  for (const theme of themes) {
    const page = await browser.newPage({
      viewport: { width: SIZES[0].width, height: SIZES[0].height },
    });
    await page.emulateMedia({ colorScheme: theme });

    for (const [i, state] of states.entries()) {
      const file = path.join(tmp, `state-${i}.html`);
      fs.writeFileSync(file,
        '<!doctype html><html lang="en"><head><meta charset="utf-8">'
        + sheets.map((href) => `<link rel="stylesheet" href="${href}">`).join('')
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
      for (const size of SIZES) {
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
        for (const [s, scale] of SCALES.entries()) {
          await page.evaluate((px) => {
            document.documentElement.style.fontSize = `${px}px`;
          }, scale);
          const issues = [...new Set([
            ...await page.evaluate(GEOMETRY),
            // Contrast at the smallest scale and the reference size only. It
            // is a walk over computed colours: the 18.66px large-text
            // threshold makes every larger scale strictly more permissive,
            // and a wider phone moves boxes rather than colours. Honest to
            // gate on the size only because the two checks that were in this
            // function and were NOT about colour — the scrim covering the bar,
            // and a row that measures nothing — have moved into GEOMETRY,
            // where they are walked at every size like the geometry they are.
            ...(s === 0 && size.reference ? await page.evaluate(CONTRAST) : []),
          ])];
          if (issues.length) {
            findings += issues.length;
            console.log(`\n${state.label}  [${theme}, ${size.width}x${size.height}, root ${scale}px]`);
            issues.forEach((str) => console.log(`  ${str}`));
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
  const scope = `${states.length} states x ${themes.length} theme${themes.length > 1 ? 's' : ''}`
    + ` x ${SIZES.length} phone sizes (${SIZES.map((z) => `${z.width}x${z.height}`).join('/')})`
    + ` x ${SCALES.length} text sizes (${SCALES.join('/')}px)`;
  const [ref] = REFERENCE;
  const contrastScope = `${ref.width}x${ref.height} at root ${SCALES[0]}px`;
  if (findings) {
    console.log(`\n${findings} finding${findings > 1 ? 's' : ''} across ${scope}`
      + ` (contrast at ${contrastScope} only).`);
    process.exit(1);
  }
  console.log(`Clean: no geometry findings across ${scope};`
    + ` no contrast findings at ${contrastScope}, which is where that walk runs.`);
})();
