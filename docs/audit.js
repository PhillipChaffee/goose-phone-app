// Check the rendered UI for the mistakes that are easier to measure than to see.
//
//   npm i -D playwright        (Chromium only; see PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD)
//   node docs/audit.js         [light|dark|both]
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
//             taller than the bar, the chrome band still opaque where that
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
// on a phone, and the font stack resolves to whatever is installed locally
// rather than to iOS's. Positions and text metrics are what the simulator is
// for.
//
// What it is structurally blind to, and what covers it instead: this walks
// whole screens at whole phone sizes, and the composer's chip row is decided
// by how many COLUMNS it has rather than by which phone it is on — so
// docs/measure-composer.js sweeps that one row across six widths at one
// height, including two (390, 393) that are three points apart and were where
// the model chip was at its worst. Both scripts now gate the same floor, 320
// effective points; that is what stops them drifting apart. The text spilling
// out of a chip that SPILL below now catches is the check that script had and
// this one did not.

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
// Every point size the app runs at, ascending. Height travels with width
// because a phone is not a column: the two failures this axis found are both
// "content is taller than the space it was given", and holding the height at
// the reference 874 while narrowing invents a tall thin phone nobody owns and
// understates every one of them. Measured, before the fixes that came with
// this list: 375x874 reported 24 findings and 375x667 — the size an SE 3rd gen
// actually is — reported 116, with two destination names painted on top of
// each other.
//
//   320x568  what a 4.7" phone (SE 2nd/3rd gen, 8) reports with Display Zoom
//            set to Larger Text, and the layout of the retired 4" phones.
//            Unverified here — no simulator, no device — so the reason it is
//            in this list is the one that does not depend on it:
//            docs/measure-composer.js already gates 320 as a defensive floor
//            ("a run that never sees the tight case is not a test of the tight
//            case"), and an audit that gave up at 375 would gate a narrower
//            band than the composer script does.
//   375x667  SE (3rd gen), sold new until 2025 — the narrowest size a phone
//            Apple supports gives at the default zoom, and where the drawer
//            failure below is 38px rather than the 2px a tall 375 shows.
//   390x844  12 / 13 / 14 / 16e — the size most phones in the field report.
//            It finds nothing today; it is here because the failure
//            docs/measure-composer.js was written for was a width rendering
//            LESS than a narrower one, so "between two clean sizes" is not a
//            proof of anything on this axis.
//   402x874  17 Pro / 16 Pro. The reference: what docs/style-gallery.html is
//            captured at and what every measurement in docs/design.md was made
//            against. CONTRAST runs here alone — see the walk below.
//   440x956  17 Pro Max / 16 Pro Max, the widest.
//
// 393x852 (14 Pro / 15 / 16) is deliberately absent: three points from 390 is
// a question about how many columns one elastic chip has, which is
// docs/measure-composer.js's subject, not a question about the geometry of 98
// screens. 375x812 (13 mini) is absent because 375x667 is the same width and
// strictly harsher in the axis that turned out to matter.
//
// A fixed list rather than argv, unlike measure-composer.js's widths: that
// script's are a sweep for a comparison, while these are a COVERAGE CLAIM
// about which phones the app is gated on, and a claim that narrows silently
// when someone passes an argument is not a claim.
const SIZES = [
  { width: 320, height: 568 },
  { width: 375, height: 667 },
  { width: 390, height: 844 },
  { width: 402, height: 874, reference: true },
  { width: 440, height: 956 },
];

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
    scroll: state.scroll,
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
  const states = Object.entries(JSON.parse(fs.readFileSync(STATES, 'utf8')))
    .map(([label, body]) => ({ label, body, scroll: '' }));
  if (states.length === 0) {
    console.error(`${STATES} is empty`);
    process.exit(1);
  }
  states.push(...stressed(states));

  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'ui-audit-'));
  const browser = await chromium.launch();
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
        + STYLESHEETS.map((href) => `<link rel="stylesheet" href="${href}">`).join('')
        + `</head><body>${state.body}</body></html>`);
      await page.goto(`file://${file}`, { waitUntil: 'load' });
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
      // 980 resizes and no extra page loads.
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
        // That is a property of the list, not of the walk. Reverse SIZES with
        // this line deleted and the run reports 300 findings against a true
        // 284; put it back and it reports 284 again. Measured at no cost —
        // the walk was going to force that layout a moment later anyway — so
        // it buys the result's independence from the order of a list somebody
        // will eventually reorder.
        //
        // Do not delete it as a no-op. Without it the walk still reports; it
        // reports the wrong size, and on the steps where the stale value is
        // the SMALLER one that is a finding which silently passes.
        await page.evaluate(() => {
          for (const el of document.querySelectorAll('*')) el.getBoundingClientRect();
        });
        if (state.scroll) {
          // Inside the size loop, not before it: a resize re-clamps scrollTop
          // against the new scrollHeight, so a state scrolled once at the top
          // would be measured un-scrolled at four of the five sizes.
          await page.evaluate((want) => {
            const el = document.querySelector('.scroll');
            if (el) el.scrollTop = want === 'bottom' ? el.scrollHeight : (parseInt(want, 10) || 0);
          }, state.scroll);
        }
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
  // coverage claim rots.
  const scope = `${states.length} states x ${themes.length} theme${themes.length > 1 ? 's' : ''}`
    + ` x ${SIZES.length} phone sizes (${SIZES.map((z) => `${z.width}x${z.height}`).join('/')})`
    + ` x ${SCALES.length} text sizes (${SCALES.join('/')}px)`;
  if (findings) {
    console.log(`\n${findings} finding${findings > 1 ? 's' : ''} across ${scope}.`);
    process.exit(1);
  }
  console.log(`Clean: ${scope}, no geometry or contrast findings.`);
})();
