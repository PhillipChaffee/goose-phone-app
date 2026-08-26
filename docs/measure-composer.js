// Does the composer row survive a long, server-supplied model name?
//
//   node docs/measure-composer.js [width…] [--root=px…]
//
// The chip block is ONE LINE and never more. That is the first thing this
// checks, because it is the rule the stylesheet now states and because a
// wrapping row is not a number going out of range — it grows, so every other
// measurement here stays in bounds while the composer quietly gets taller
// under your thumb. The send button is the primary action and the only control
// that must always be reachable, so the second thing is a chip holding the row
// open until send is pushed past the right edge.
//
// The model name is the only elastic item in the row. Everything else —
// the attach button, the mode chip, the effort tier, the chevron, send —
// is rigid, so a deficit lands entirely on the name by construction rather
// than by proportion. Two failures follow from that and both are checked:
// the name must be cut by its OWN box, so `text-overflow` actually paints and
// what is cut says it was cut (a parent that clips it produces a hard cut
// mid-glyph with no ellipsis, which measured *cleaner* than a label with room
// to spare); and the tier must never be what gives while the name still has
// width, since the tier is the fact the label grew a second element to carry.
//
// docs/audit.js cannot see any of it. Its overflow walk visits elements, and
// spilling text is an anonymous text node; its clipped-text check requires
// overflow-x: hidden, which a chip never sets. That audit walks whole screens
// at six whole phone sizes now, and the two lists overlap on five of them —
// the 320 floor and the 360/375/390/402 middle — but they are not the same
// list and are not meant to be. It holds 440 because a phone that wide is a
// screen layout; these hold 393 because 390 and 393 are three points apart and
// this row's worst case is a column count rather than a phone.
//
// Two failures were invisible to this script as well, and both were on screen
// the whole time it reported Clean. A label that clips itself makes `spill`
// *negative*, so a hard cut with no ellipsis ("Manual ap") measured cleaner
// than a label with room to spare; and a crushed pill paints its chevron
// outside itself, which the spill walk could not see while it looked at
// `.chip-label` alone. Each needed its own question asked — does the thing
// being cut say that it was cut, and does anything at all leave the pill.
//
// One more failure is not a property of a single rendering at all: the model
// name was wider at 375pt than at 390, and every number was in range at both.
// That is why this takes a list of widths and runs them in one process. Run it
// at one width and it answers everything but that one. Under a one-line row
// it is true by construction — the deficit falls monotonically as the viewport
// grows and the name absorbs all of it — which makes it a cheap regression
// guard rather than dead weight.
//
// TEXT SIZE IS THE SECOND AXIS, and on this row it is almost the same axis as
// width: what the chips are dividing is not pixels but COLUMNS, and a 402pt
// phone at AX5 has the columns of a 121pt one at the browser default. So the
// numbers below are stated per rem rather than per pixel — three of them were
// px assertions about ch-based rules, and at an accessibility size they failed
// on layouts that were behaving exactly as the stylesheet says they should.
const fs = require('fs');
const path = require('path');
const os = require('os');
const { chromium } = require('playwright');

const CSS = fs.readFileSync(path.join(__dirname, '..', 'assets', 'main.css'), 'utf8');
// More than one width in a single run, because the failure below that no
// single width can see is a *comparison* between two of them. 390 and 393 are
// in the default list for that reason: they are the two the earlier list
// (280/320/360/375/402) stepped straight over, and they are where the model
// chip was at its worst.
const ARGS = process.argv.slice(2);
const WIDTHS = ARGS.filter((a) => !a.startsWith('--')).length
  ? ARGS.filter((a) => !a.startsWith('--')).map((w) => parseInt(w, 10))
  : [320, 360, 375, 390, 393, 402];

// The root font-size, in px, at each text size the app really runs at — the
// same list docs/audit.js walks, and for the same reason: 16 is what Android's
// WebView and the desktop build give, and 17/23/53 are iOS at Large, at
// xxxLarge and at AX5. The app opts into those through `-apple-system-body`
// (assets/platform/ios.css), which Chromium cannot parse; setting the root in
// px is that opt-in stated in the only form this browser can hear.
const ROOTS = ARGS.filter((a) => a.startsWith('--root=')).length
  ? ARGS.filter((a) => a.startsWith('--root=')).map((a) => parseInt(a.slice(7), 10))
  : [16, 17, 23, 53];

// Everything below is stated per rem — as a multiple of the root — because
// every rule it is asserting about is itself relative. Written as the pixel
// value each came to at a 16px root over the 16 it was measured against, so
// the number that was here is still visible in it.

// What .chip-effort's max-width comes to at --text-xs, plus a couple of pixels
// of font-metric slack. No tier may cost the name more than this.
//
// It is an assertion about a 5ch cap, so it was never really 40px: at AX5 a
// correct tier measures about 100px against a 40px constant, and this failed
// with "the tier takes 100px" on a layout doing exactly what the stylesheet
// says. Per rem it is the same claim at every size.
const TIER_CAP = (root) => (40 / 16) * root;

// The narrowest viewport a shipping phone gives this app. 320 is a defensive
// width, not a device, and it is the only one where any rule has to be
// relaxed — kept in the default list because a run that never sees the tight
// case is not a test of the tight case.
//
// Measured in EFFECTIVE width — the columns the row has, not the points — for
// the reason at the top of this file: a 402pt phone at AX5 is a 121pt phone
// as far as this row is concerned, which is well inside the defensive band.
const REAL_PHONE = 375;
const effective = (width, root) => (width * 16) / root;

// The width floor the stylesheet owes the model name on a real phone. Below
// this you are not reading a truncation, you are reading an absence.
//
// It is a floor against the name vanishing, not a promise of legibility, and
// the honest number is worth writing down rather than leaving to be inferred
// from the pixels: at --text-xs, 30px is THREE characters — two glyphs and the
// ellipsis. Two of this app's models share their first four, so a name at the
// floor does not tell them apart; what does is the sheet the chip opens, which
// states the model in full. The floor that the arrangement actually delivers
// on a non-crowded row at 375pt is 42.5px, or five characters. Raising the
// assertion to meet it would be pinning the gate to today's measurement rather
// than to the rule, which is that the chip must still be a name and not a
// chevron.
//
// Three characters is the rule; 30px was only ever what three characters
// measured at a 16px root, so the floor follows the text like the text does.
const NAME_FLOOR = (root) => (30 / 16) * root;

const icon = '<svg class="icon" viewBox="0 0 24 24"></svg>';
// The attach button. Fixed width, and first in the row, so it comes off the
// budget every other chip is dividing.
const attach = '<button class="composer-chip action attach" data-attach="code">'
  + `${icon}</button>`;
const settings = (label, effort) =>
  '<button class="composer-chip action"><span class="chip-label">'
  + `<span class="chip-model">${label}</span>`
  + (effort ? `<span class="chip-effort">${effort}</span>` : '')
  + `</span>${icon}</button>`;

// The mode chip. It does not shrink at all — its label is short, the sheet
// does not restate it, and the model name beside it is the one thing this row
// is allowed to give away — so the cap on this label is what stops a long mode
// name taking the row instead.
const mode = (label) =>
  `<button class="composer-chip action mode">${icon}<span class="chip-label">${label}</span></button>`;

// The new-session screen's two context pills. Both carry a server-supplied
// string in a real element, so both ellipsise on their own box.
const named = (kind, name) =>
  `<button class="composer-chip action ${kind}">${icon}`
  + `<span class="chip-label"><span class="chip-name">${name}</span></span></button>`;

const sendRow = (inner) =>
  `<div class="composer-row"><div class="chip-row">${inner}</div>`
  + `<button class="send">${icon}</button></div>`;

// The rows the app actually builds, not a synthetic worst case. Diff and PR
// used to be in the composer and are not any more — they moved to the action
// row above it, which is a sideways scroller with its own budget — so what
// shares the row now is a mode chip, carrying the longest label each backend
// really produces.
//
// The goose row is the crowded one, and it is crowded by four things at once:
// a model name, a tier, a mode and the context readout. `crowded` rows carry
// that readout; the app only shows it near the end of the window (`crowding`
// in src/views/chat.rs), because four chips do not fit a 320pt row — so
// measuring it as though it were always there would be measuring a composer
// the app does not build. It is measured separately and held to a looser bar:
// at that moment the warning IS the most useful thing in the row, and the
// model name giving way further is the trade being made deliberately. What
// must still hold is one line, nothing spilling, and the name being cut by its
// own box.
const ROWS = {
  // `Accept-edits` rather than `General`: `general` is a subagent and
  // `agent_choices` filters it out, so it is a label this chip can never
  // render. `accept-edits` is the longest one it really can.
  code: { build: (l, e) => sendRow(attach + settings(l, e) + mode('Accept-edits')) },
  goose: { build: (l, e) => sendRow(attach + settings(l, e) + mode('Manual approval')) },
  // A short mode label is the case the mode chip's rigidity exists for: with
  // any shrink at all, `Auto` came back as `Aut…`.
  'goose on auto': { build: (l, e) => sendRow(attach + settings(l, e) + mode('Auto')) },
  'goose near the limit': {
    crowded: true,
    build: (l, e) => sendRow(attach + settings(l, e) + mode('Manual approval')
      + '<span class="composer-chip warn">96%</span>'),
  },
  // The new-session composer: two rows, no card around them, and no field
  // inside them — its field is the page. The row it adds here is the CONTEXT
  // row, which carries two more strings the app does not choose (a repo name
  // and a branch name) and has no send button in it to be pushed off, which is
  // exactly why the checks below stopped assuming there is one chip row and
  // that send sits beside it.
  'new session': {
    bare: true,
    field: false,
    context: true,
    build: (l, e, ctx) =>
      `<div class="composer-row"><div class="chip-row">`
      + `${named('repo', ctx[0])}${named('branch', ctx[1])}</div></div>`
      + sendRow(attach + settings(l, e) + mode('Accept-edits')),
  },
};

// The longest pair the allowlist and GitHub really produce, and a short one
// for contrast. Only the new-session row iterates it.
const CONTEXT = [
  ['personal-ai-setup', 'main'],
  ['base-image-with-debugger', 'claude/github-issues-mr-lep2wr-issue-10'],
];

// A picked file's name is not the app's to choose. The first is what iOS calls
// a screenshot; the others are things someone actually had lying around.
const FILES = [
  'Screenshot 2026-08-24 at 09.41.17 — build failure on the tailnet box.png',
  'IMG_0042.jpg',
  'notes.md',
];

const trayChip = (name) =>
  '<div class="attach-chip" role="listitem">'
  + '<span class="attach-icon"></span>'
  + `<span class="attach-meta"><span class="attach-name">${name}</span>`
  + '<span class="attach-size">1.2 MB</span></span>'
  + `<button class="attach-remove">${icon}</button></div>`;

const tray = (names) =>
  `<div class="attach-tray" role="list">${names.map(trayChip).join('')}</div>`;

// Empty, one file, and the tray at its cap — three chips is where it has to
// start scrolling rather than stretching the composer.
const TRAYS = [[], [FILES[0]], FILES];

const page = (row, label, effort, names, context, root) => {
  const spec = ROWS[row];
  const hasField = spec.field !== false;
  return `<!doctype html><html><head><meta charset="utf-8">
<style>${CSS}</style>
<style>html{font-size:${root}px}</style></head><body><div class="app">
${hasField ? '' : '<main class="compose"><textarea class="compose-field"></textarea></main>'}
<footer class="composer${spec.bare ? ' bare' : ''}">
${names && names.length ? tray(names) : ''}
${hasField ? '<textarea class="input" rows="1" placeholder="Message the code agent…"></textarea>' : ''}
${spec.build(label, effort, context)}
</footer></div></body></html>`;
};

// The last two are real: OpenCode's own catalogue, and what the goose client
// falls back to when the agent's model id is not in its own options list.
// `Default` is the app's own last-resort face for a chat whose model nothing
// has been able to name — it replaced `Model`, which was the name of the
// control rather than a value, and this list documents itself as labels the
// app really builds.
const LABELS = [
  'Default',
  'Claude Sonnet 4.5',
  'Qwen3 Coder 480B A35B Instruct',
  'anthropic/claude-sonnet-4-5-20250929',
];

// Every tier that can reach a chip, which is not the same list as every tier
// a backend serves: src/views/session_settings.rs shortens goose's `medium`
// and OpenCode's `minimal` on the way here, because six letters on this side
// are six letters off the model name. Keep this in step with `chip_effort`.
// The last one is a word neither backend has — the case the CSS cap exists
// for, and the only one allowed to be clipped.
const EFFORTS = [
  { tier: '' },
  { tier: 'Max' },
  { tier: 'Min' },
  { tier: 'Low' },
  { tier: 'Med' },
  { tier: 'High' },
  { tier: 'Xhigh' },
  { tier: 'None' },
  { tier: 'Ultrathink', capped: true },
];

(async () => {
  const browser = await chromium.launch();
  let bad = 0;
  // A directory of this run's own, and a filename no navigation has used
  // before, which is docs/audit.js's arrangement and for a reason this script
  // learned the hard way. Every combination below used to write to one path
  // keyed on the row and the width — so a second invocation running at the
  // same time (two worktrees, or CI checking two branches) overwrote the file
  // the first was mid-navigation to, and Chromium measured a document that
  // was half of one page and half of another. It failed as
  // `TypeError: Cannot read properties of null (reading 'closest')` on
  // `document.querySelector('.send')`, on a tree that is clean: reproduced
  // exactly by running two copies at once, 1 of 2 processes dying. The
  // dangerous direction is the other one — the same mechanism can serve a
  // stale PASSING page in place of a regressed one, so a green run did not
  // establish what it claimed. A unique URL per measurement ends the whole
  // class, including the same-process cache hit that a reused name invites.
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'composer-'));
  let nth = 0;
  // nameWidth for every composition, keyed by everything except the width, so
  // the same composition can be compared across widths afterwards.
  const namesByWidth = new Map();
  for (const WIDTH of WIDTHS) {
  for (const ROOT of ROOTS) {
  const p = await browser.newPage({ viewport: { width: WIDTH, height: 874 } });
  for (const row of Object.keys(ROWS)) {
    console.log(`\n  ${row} composer @ ${WIDTH}pt, root ${ROOT}px`);
    for (const label of LABELS) {
      for (const { tier, capped } of EFFORTS) {
       for (const names of TRAYS) {
        for (const context of (ROWS[row].context ? CONTEXT : [null])) {
        nth += 1;
        const file = path.join(tmp, `composer-${row.replace(/\s+/g, '-')}-${WIDTH}-${ROOT}-${nth}.html`);
        fs.writeFileSync(file, page(row, label, tier, names, context, ROOT));
        await p.goto(`file://${file}`, { waitUntil: 'load' });
        const r = await p.evaluate(() => {
          const rowEl = document.querySelector('.composer-row');
          // Every chip row, because the new-session composer has two of them
          // and the one WITHOUT a send button is the one carrying two
          // server-supplied strings.
          const chipRows = [...document.querySelectorAll('.chip-row')];
          const boxes = chipRows.length ? chipRows : [rowEl];
          const send = document.querySelector('.send');
          const chips = [...document.querySelectorAll('.composer-chip')];

          // How many lines each chip block takes. Clustered by vertical
          // CENTRE with 4px of tolerance rather than by `top`: the attach
          // button is 36px tall and the chips 32, so grouping by top reports
          // two or three lines for a row that is plainly one.
          const lines = Math.max(...boxes.map((b) => {
            const mine = chips.filter((c) => b.contains(c));
            const centres = [];
            for (const c of mine) {
              const box = c.getBoundingClientRect();
              const mid = (box.top + box.bottom) / 2;
              if (!centres.some((y) => Math.abs(y - mid) <= 4)) centres.push(mid);
            }
            return centres.length || 1;
          }));

          // Anything at all painting outside its pill, not just the label:
          // a chip squeezed to its floor pushes its chevron or its leading
          // icon out through the side, and that is a real 5px failure at
          // 320pt. A chip whose content is a bare text node has no box, so
          // that one still needs the Range.
          const spill = Math.max(0, ...chips.map((c) => {
            const right = c.getBoundingClientRect().right;
            const kids = [...c.children];
            if (kids.length) {
              return Math.round(Math.max(...kids.map((k) => k.getBoundingClientRect().right)) - right);
            }
            const range = document.createRange();
            range.selectNodeContents(c);
            return Math.round(range.getBoundingClientRect().right - right);
          }));

          // The tier is the fact the chip grew a second element to carry, and
          // the label clips: a name long enough to fill the chip must push the
          // ellipsis, not push the tier out through the side of the pill.
          //
          // `tierHardCut` is the one that was on screen while this said Clean.
          // The tier is cut by the LABEL rather than by its own box, which
          // `text-overflow` never paints on, so the chip states a tier called
          // `Xh` or `Ma` — a wrong name, not a shortened one. It needs its own
          // question because neither of the others can see it: the tier's own
          // scrollWidth equals its clientWidth (it is not the clipper), and
          // `lost` was suppressed wherever the name had already reached zero,
          // which is exactly where this fires.
          const tierEl = document.querySelector('.chip-effort');
          const model = document.querySelector('.chip-model');
          const clip = model && model.parentElement;
          const box = tierEl && tierEl.getBoundingClientRect();
          const clipBox = clip && clip.getBoundingClientRect();
          // Send is judged against ITS OWN row. Measured against every chip on
          // the page, the context row above always reads as "send wrapped
          // below the chips" — the one failure this exists to catch, firing on
          // every clean run.
          const mine = send.closest('.composer-row');
          const sendChips = chips
            .filter((c) => mine.contains(c))
            .map((c) => c.getBoundingClientRect());
          const blockTop = Math.min(...sendChips.map((b) => b.top));
          const blockBottom = Math.max(...sendChips.map((b) => b.bottom));
          const sendBox = send.getBoundingClientRect();
          const modeLabel = document.querySelector('.composer-chip.mode .chip-label');
          return {
            // How far the scroller would have to scroll. The valve exists for
            // exactly one composition family; everywhere else this is 0.
            overflow: Math.round(Math.max(...boxes.map((b) => b.scrollWidth - b.clientWidth))),
            spill,
            lines,
            // Squeezed out of existence. Kept separate from the hard cut
            // below: nothing is painted wrongly here, there is simply no tier.
            lost: tierEl ? box.width < 1 : false,
            // Painted glyphs lying outside the box that is doing the
            // clipping. `box.width > 0.5` because a zero-width tier sitting
            // past a zero-width label is the label's own 5px gap, not ink.
            tierHardCut: tierEl
              ? box.width > 0.5 && box.right > clipBox.right + 0.5
              : false,
            tierWidth: tierEl ? Math.round(box.width * 10) / 10 : 0,
            // A pixel of tolerance, not half of one, because `scrollWidth`
            // and `clientWidth` are both rounded to integers and a text size
            // off the 16px grid puts these boxes on halves: at the iOS
            // default a 31.5px tier reports client 31 and scroll 32 and reads
            // as ellipsised while nothing at all is cut. One px is the same
            // tolerance docs/audit.js uses on the same pair of properties.
            tierCut: tierEl ? tierEl.scrollWidth > tierEl.clientWidth + 1 : false,
            nameWidth: model ? model.clientWidth : null,
            nameCut: model ? model.scrollWidth > model.clientWidth + 1 : false,
            // Every server-supplied name's own box must be what clips it, so
            // what is cut says it was cut. A parent narrower than the box
            // inside it produces a hard cut mid-glyph and `text-overflow`
            // never paints. .chip-name is in here as well as .chip-model
            // because the new-session row carries two of them — a repo and a
            // branch — and they are strings this app does not choose either.
            hardClip: [...document.querySelectorAll('.chip-model, .chip-name')]
              .filter((el) => el.getBoundingClientRect().right
                > el.parentElement.getBoundingClientRect().right + 0.5)
              .map((el) => el.className),
            sendOffCentre: Math.abs((sendBox.top + sendBox.bottom) / 2
              - (blockTop + blockBottom) / 2) > 1,
            modeHardClip: modeLabel
              ? modeLabel.scrollWidth > modeLabel.clientWidth + 1
                && getComputedStyle(modeLabel).textOverflow !== 'ellipsis'
              : false,
            sendRight: Math.round(sendBox.right),
            composerWidth: Math.round(document.querySelector('.composer').getBoundingClientRect().width),
            vw: document.documentElement.clientWidth,
          };
        });
        const problems = [];
        // The rule the stylesheet states, and the one that replaced
        // `sendBelowChips` — with nowrap, send can no longer be pushed to a
        // line of its own, but the chips could still take one if the wrap ever
        // came back.
        if (r.lines !== 1) problems.push(`the chips take ${r.lines} lines`);
        if (r.sendRight > r.vw) {
          problems.push(`send is ${r.sendRight - r.vw}px off the right edge`);
        }
        // The 320pt four-chip row is over budget by 7-35px however the space
        // is divided, and the scroller is what stands between those chips and
        // the send button. Above the width the arrangement was designed down
        // to, the row must not need to scroll at all, so a fifth chip cannot
        // quietly slide off the edge.
        //
        // In EFFECTIVE width, not raw points: the exemption is about how many
        // columns the chips have to divide, and text size takes columns away
        // exactly as a narrower phone does. At a 16px root this is word for
        // word the rule that was here — 320 is not less than 320, so no
        // non-crowded row in the default sweep is exempted, and the crowded
        // one is exempted at 320 alone.
        //
        // Where each row actually reaches for the valve, measured at 402pt:
        // the four-chip row at AX1 (root 28, 230 effective points, 27px over),
        // the two chat rows at AX3 (root 40, 161 points, 12px over), the
        // new-session context row at AX4 (root 47, 137 points, 31px over).
        // Nothing overflows at xxxLarge at any width in the sweep.
        const designedDownTo = ROWS[row].crowded ? 360 : 320;
        const mayScroll = effective(WIDTH, ROOT) < designedDownTo;
        if (r.overflow > 0 && !mayScroll) {
          problems.push(`the chip block overflows by ${r.overflow}px`);
        }
        if (r.sendOffCentre) problems.push('the send button is not centred on the chip block');
        if (r.modeHardClip) problems.push('the mode label is clipped with no ellipsis');
        if (r.hardClip.length) {
          problems.push(`cut by its parent, with no ellipsis: ${r.hardClip.join(', ')}`);
        }
        // An attachment tray full of long file names would stretch the
        // composer itself if it did not scroll sideways.
        if (r.composerWidth > r.vw) {
          problems.push(`the composer is ${r.composerWidth - r.vw}px wider than the screen`);
        }
        if (r.spill > 0) problems.push(`text spills ${r.spill}px past its pill`);
        // Whatever the tier has left, it must be the tier's own box that cuts
        // it — a cut that says it was one. This is asserted at every width and
        // in every composition, INCLUDING the ones where the name is already
        // at zero: that is where the label is narrowest, so it is where the
        // parent is likeliest to be the clipper, and suppressing it there is
        // how `Xh` and `Ma` shipped.
        if (r.tierHardCut) problems.push('the tier is cut by the label, with no ellipsis');
        // The tier must never be what gives while the name still has room —
        // but once the name is at zero the chip is a bare control and
        // everything in it is under water together.
        if (r.lost && r.nameWidth > 0) problems.push('the effort tier was clipped away');
        if (r.tierWidth > TIER_CAP(ROOT)) {
          problems.push(`the tier takes ${r.tierWidth}px,`
            + ` over the ${Math.round(TIER_CAP(ROOT))}px it is allowed`);
        }
        if (r.tierCut && !capped && r.nameWidth > 0) problems.push('the tier is ellipsised');
        // The floor the stylesheet owes on a real phone. Below 375 effective
        // points none is asserted: 320 is a defensive width where one line
        // costs the name everything, which is the trade docs/design.md
        // records — and at an accessibility text size every phone is inside
        // that band, because the columns are what ran out.
        if (effective(WIDTH, ROOT) >= REAL_PHONE && !ROWS[row].crowded && r.nameWidth !== null
            && r.nameWidth < NAME_FLOOR(ROOT)) {
          problems.push(`the name is down to ${r.nameWidth}px, which identifies nothing`);
        }
        if (problems.length) bad += problems.length;
        const ctxKey = context ? ` | ${context[0]}@${context[1]}` : '';
        // The root is part of the key, not part of what is compared: the
        // monotonicity check below is "a wider phone shows more of the name",
        // and a wider phone at a bigger text size legitimately shows less.
        const key = `${row} | ${label}${tier ? ` + ${tier}` : ''} | tray ${names.length}`
          + `${ctxKey} | root ${ROOT}`;
        if (!namesByWidth.has(key)) namesByWidth.set(key, []);
        namesByWidth.get(key).push([WIDTH, r.nameWidth]);
        console.log(
          `    ${problems.length ? 'FAIL' : 'ok  '} ${`${label}${tier ? ` + ${tier}` : ''}`.padEnd(50)}`
          + ` lines=${r.lines}  overflow=${r.overflow}  spill=${r.spill}`
          + `  name=${r.nameWidth}${r.nameCut ? '…' : ''}  tier=${r.tierWidth}`
          + `  send.right=${r.sendRight}/${r.vw}`
          + (problems.length ? `  <- ${problems.join('; ')}` : ''),
        );
        }
       }
      }
    }
  }
  await p.close();
  }
  }
  await browser.close();
  fs.rmSync(tmp, { recursive: true, force: true });

  // A bigger phone must never show less of the model name than a smaller one.
  // Nothing above can see this: every number is inside its range at every
  // width taken on its own, and the fault is that two of the widths disagree.
  // It was the shape of failure a wrapping row invited, because where the line
  // breaks and how much the chips get are two different decisions and they can
  // stop agreeing. One line makes it true by construction — the deficit falls
  // monotonically as the viewport grows and the name absorbs all of it — so
  // this is now a cheap guard against the wrap coming back rather than the
  // thing it was written to catch.
  if (WIDTHS.length > 1) {
    for (const [key, seen] of namesByWidth) {
      const ordered = [...seen].sort((a, b) => a[0] - b[0]);
      for (let i = 1; i < ordered.length; i += 1) {
        const [wNarrow, nNarrow] = ordered[i - 1];
        const [wWide, nWide] = ordered[i];
        if (nNarrow === null || nWide === null) continue;
        if (nWide < nNarrow - 0.5) {
          bad += 1;
          console.log(
            `\n  FAIL ${key}: the name is ${nWide}px at ${wWide}pt`
            + ` but ${nNarrow}px at ${wNarrow}pt — a wider phone shows less of it`,
          );
        }
      }
    }
  }

  if (bad) {
    console.log(`\n${bad} composer problem${bad > 1 ? 's' : ''}.`);
    process.exit(1);
  }
  console.log(
    `\nClean at ${WIDTHS.join('/')}pt x root ${ROOTS.join('/')}px: every chip block is one`
    + ' line, send stays on screen and centred on its own row, nothing spills a pill, a cut'
    + ' name and a cut mode both say they were cut, and the name never shrinks as the phone'
    + ' grows.',
  );
})();
