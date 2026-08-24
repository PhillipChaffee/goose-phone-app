// Check the rendered UI for the mistakes that are easier to measure than to see.
//
//   npm i -D playwright        (Chromium only; see PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD)
//   node docs/audit.js         [light|dark|both]
//
// Every screen state in docs/gallery-states.json is rebuilt as a standalone
// 402x874 document — the gallery's <iframe> gives clean style isolation, but
// this needs the states as top-level pages — and then walked for:
//
//   geometry  overflow past the viewport, text clipped without an ellipsis,
//             filled or fully-bordered boxes left at radius 0, buttons under
//             32px, and any child rounded more than the parent clipping it.
//   contrast  every element carrying its own text, composited against the
//             first opaque background behind it, against 4.5:1 (3:1 for large
//             or bold text) — and every icon, which carries no text of its
//             own and so is invisible to that walk, against 3:1.
//   scrim     the chrome band still opaque where the title sits, and the
//             scroller's padding still clearing it.
//   collapsed rows that render nothing and therefore measure nothing.
//
// Each state is also repeated with server-supplied text swapped for the
// longest plausible value, because a captured state only ever shows the one
// string the app happened to be holding.
//
// Exits non-zero if anything is found, so it can gate a change.
//
// What it cannot check: anything that needs a real device. Safe-area insets
// are zero in a browser, so the floating chrome sits higher here than it does
// on a phone, and the font stack resolves to whatever is installed locally
// rather than to iOS's. Positions and text metrics are what the simulator is
// for.
//
// What it is structurally blind to, and what covers it instead: text spilling
// out of a chip is an anonymous text node with no box to measure, and no chip
// sets overflow-x, so neither the overflow walk nor the clipped-text check can
// see it — docs/measure-composer.js measures that, at several widths.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { chromium } = require('playwright');

const STATES = path.join(__dirname, 'gallery-states.json');
const CSS = path.join(__dirname, '..', 'assets', 'main.css');

// ── stress ──────────────────────────────────────────────────────────────
// A captured state only ever shows the one string the app happened to be
// holding. Some of those strings come from a server and can be much longer:
// a model name is whatever the provider called it, and a chip sized to its
// content pushed the send button 50px off the right edge before this check
// existed. So each captured state is repeated with the server-supplied text
// swapped for the longest plausible value — substituting into markup the app
// really produced, rather than hand-writing a copy of it, which is the same
// reason the gallery is generated.
const LONGEST = {
  '.chip-label': 'Qwen3 Coder 480B A35B Instruct',
  '.session-title': 'Refactor the transcript folding so streamed parts land in order',
  '.topbar > .title': 'Refactor the transcript folding so streamed parts land in order',
  // Every settings-shaped row on every screen puts server text here — a model
  // name, an MCP command line, a cron sentence read back as English — and
  // none of it was being stressed.
  '.setting-value': 'npx -y @modelcontextprotocol/server-filesystem /srv/goose/workspaces/current',
  '.session-meta': 'Every weekday at 09:00 America/Los_Angeles · 20250823_140512_9f3ab2',
};

const stressed = (states) => states.flatMap((state) => {
  const hits = Object.entries(LONGEST).filter(([sel]) => state.body.includes(sel.split(' > ').pop().slice(1)));
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
    if (!parked && (r.right > vw + 0.5 || r.left < -0.5) && !inHorizontalScroller(el)) {
      out.push(`OVERFLOW-X   ${name(el)} left=${r.left.toFixed(0)} right=${r.right.toFixed(0)} vw=${vw}`);
    }
    if (parked) continue;
    if (el.scrollWidth > el.clientWidth + 1 && cs.overflowX === 'hidden' && cs.textOverflow !== 'ellipsis') {
      out.push(`CLIPPED-X    ${name(el)} scroll=${el.scrollWidth} client=${el.clientWidth}`);
    }

    // A surface is something with a fill or a box of borders. A lone
    // border-top is a rule, not a box, and is meant to be square.
    const filled = cs.backgroundColor !== 'rgba(0, 0, 0, 0)';
    const boxed = px(cs.borderTopWidth) > 0 && px(cs.borderLeftWidth) > 0 && px(cs.borderBottomWidth) > 0;
    // A surface that spans the whole viewport in either axis is a page or a
    // panel; square corners are correct for both.
    const fullScreen = (r.width >= vw - 0.5 && r.height >= vh - 0.5)
      || r.height >= vh - 0.5;
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

  // The floating chrome is legible because an opaque scrim covers the band it
  // sits in, and the scroller's top padding keeps content out of that band. If
  // the two ever disagree, the title lands on whatever scrolled underneath it
  // and both stay readable — a collision, not a layering. The failure is
  // invisible at rest, because at rest there is nothing scrolled up yet.
  const app = document.querySelector('.app');
  const bar = document.querySelector('.topbar');
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
  // closing up the gaps the author put there.
  for (const el of document.querySelectorAll('.diff-line, .setting-row, .drawer-item, .session-item')) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden') continue;
    if (el.getBoundingClientRect().height < 1) {
      const cls = typeof el.className === 'string' ? el.className.trim() : '';
      out.push(`COLLAPSED    .${cls.split(/\s+/).join('.')} has no height — empty content generates no line box`);
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
    const page = await browser.newPage({ viewport: { width: 402, height: 874 } });
    await page.emulateMedia({ colorScheme: theme });

    for (const [i, state] of states.entries()) {
      const file = path.join(tmp, `state-${i}.html`);
      fs.writeFileSync(file,
        '<!doctype html><html lang="en"><head><meta charset="utf-8">'
        + `<link rel="stylesheet" href="${CSS}">`
        + `</head><body>${state.body}</body></html>`);
      await page.goto(`file://${file}`, { waitUntil: 'load' });
      if (state.swap) {
        await page.evaluate((swap) => {
          for (const [sel, text] of Object.entries(swap)) {
            document.querySelectorAll(sel).forEach((el) => { el.textContent = text; });
          }
        }, state.swap);
      }
      if (state.scroll) {
        await page.evaluate((want) => {
          const el = document.querySelector('.scroll');
          if (el) el.scrollTop = want === 'bottom' ? el.scrollHeight : (parseInt(want, 10) || 0);
        }, state.scroll);
      }
      const issues = [...new Set([...await page.evaluate(GEOMETRY), ...await page.evaluate(CONTRAST)])];
      if (issues.length) {
        findings += issues.length;
        console.log(`\n${state.label}  [${theme}]`);
        issues.forEach((s) => console.log(`  ${s}`));
      }
    }
    await page.close();
  }

  await browser.close();
  fs.rmSync(tmp, { recursive: true, force: true });

  const scope = `${states.length} states x ${themes.length} theme${themes.length > 1 ? 's' : ''}`;
  if (findings) {
    console.log(`\n${findings} finding${findings > 1 ? 's' : ''} across ${scope}.`);
    process.exit(1);
  }
  console.log(`Clean: ${scope}, no geometry or contrast findings.`);
})();
