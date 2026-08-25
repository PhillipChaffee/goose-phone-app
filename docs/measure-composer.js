// Does the composer row survive a long, server-supplied model name?
//
//   node docs/measure-composer.js [width…]
//
// The send button is the primary action and the only control that must always
// be reachable, so the first failure this watches for is a chip holding the
// row open until send is pushed past the right edge. Two things get measured,
// because they fail differently: the row's own overflow (a chip that will not
// shrink) and text spilling past its pill (a chip that shrank but whose label
// could not follow it).
//
// The second failure is inside the settings chip, between the two facts it
// carries. The tier is pinned and the model name ellipsises, so a tier wide
// enough to matter is spent entirely out of the name — and on the goose row,
// which shares 306px at 360pt with the token chip and send, "Claude Sonnet 5"
// beside a six-letter tier came out as "Claude…". A chip that cannot tell
// Opus from Sonnet has stopped doing the job it was there for, so the tier is
// held to a fixed small budget (`chip_effort` shortens the long ones,
// .chip-effort caps what is left) and the name gets everything else. That is
// what the tier rules below check, and they only mean anything if every tier
// the app can actually render is tried: the old list was `Max` alone, which
// is the cheapest of them and the one case where nothing goes wrong.
//
// docs/audit.js cannot see any of it. Its overflow walk visits elements, and
// spilling text is an anonymous text node; its clipped-text check requires
// overflow-x: hidden, which a chip never sets. And it renders at 402pt alone,
// which is the width where the damage is smallest.
//
// Three more failures were invisible to this script as well, and all of them
// were on screen the whole time it reported Clean. A wrapping row does not
// overflow — it grows — so `overflow` stayed 0 while the send button, being
// the row's last flex item, sat alone on a line of its own under the chips.
// A label that clips itself makes `spill` *negative*, so a hard cut with
// no ellipsis ("Manual ap") measured cleaner than a label with room to spare.
// And the third is not a property of one rendering at all: the model name was
// wider at 375pt than at 390, and every number was in range at both. None of
// them is a number that goes out of range; each needed its own question asked
// — where is send relative to the chips, does the thing being cut say that it
// was cut, and does a bigger phone ever show less.
//
// That last question is why this takes a list of widths and runs them in one
// process. Run it at one width and it answers everything but that one.
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
const WIDTHS = process.argv.length > 2
  ? process.argv.slice(2).map((w) => parseInt(w, 10))
  : [320, 360, 375, 390, 393, 402];

// What .chip-effort's max-width comes to at --text-xs, plus a couple of
// pixels of font-metric slack. No tier may cost the name more than this.
const TIER_CAP = 40;

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

// The mode chip. It does not shrink — its label is short and the sheet does
// not restate it — so it is the model chip beside it that absorbs the whole
// deficit, and the cap on this label is what stops a long mode name taking
// the row instead.
const mode = (label) =>
  `<button class="composer-chip action mode">${icon}<span class="chip-label">${label}</span></button>`;

// The two rows the app actually builds, not a synthetic worst case. Diff and
// PR used to be here and are not any more — they moved to the action row
// above the composer, which is a sideways scroller with its own budget — so
// what shares the row now is a mode chip, carrying the longest label each
// backend really produces: goose's "Manual approval", and "General" out of
// OpenCode's built-in agents.
//
// The goose row is the crowded one, and it is crowded by four things at once:
// a model name, a tier, a mode and the context readout. That combination
// exists on neither branch this came from, which is exactly why it is the one
// worth measuring.
// `crowded` rows carry the context readout as well. The app only shows it
// near the end of the window (`crowding` in src/views/chat.rs), because four
// chips do not fit a 360pt row — so measuring it as though it were always
// there would be measuring a composer the app does not build. It is measured
// separately, and held to a looser bar: at that moment the warning IS the
// most useful thing in the row, and the model name giving way further is the
// trade being made deliberately. What must still hold is that nothing
// overflows and the name keeps the floor the stylesheet gives it.
const ROWS = {
  code: { build: (label, effort) => attach + settings(label, effort) + mode('General') },
  goose: { build: (label, effort) => attach + settings(label, effort) + mode('Manual approval') },
  'goose near the limit': {
    crowded: true,
    build: (label, effort) => attach + settings(label, effort) + mode('Manual approval')
      + '<span class="composer-chip warn">96%</span>',
  },
};

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

const page = (row, label, effort, names) => `<!doctype html><html><head><meta charset="utf-8">
<style>${CSS}</style></head><body><div class="app"><footer class="composer">
${names && names.length ? tray(names) : ''}
<textarea class="input" rows="1" placeholder="Message the code agent…"></textarea>
<div class="composer-row"><div class="chip-row">${ROWS[row].build(label, effort)}</div>
  <button class="send">${icon}</button>
</div></footer></div></body></html>`;

// The last two are real: OpenCode's own catalogue, and what the goose client
// falls back to when the agent's model id is not in its own options list.
const LABELS = [
  'Model',
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
  // nameWidth for every composition, keyed by everything except the width, so
  // the same composition can be compared across widths afterwards.
  const namesByWidth = new Map();
  for (const WIDTH of WIDTHS) {
  const p = await browser.newPage({ viewport: { width: WIDTH, height: 874 } });
  for (const row of Object.keys(ROWS)) {
    console.log(`\n  ${row} composer @ ${WIDTH}pt`);
    for (const label of LABELS) {
      for (const { tier, capped } of EFFORTS) {
       for (const names of TRAYS) {
        const file = path.join(os.tmpdir(), `composer-${row}-${WIDTH}.html`);
        fs.writeFileSync(file, page(row, label, tier, names));
        await p.goto(`file://${file}`, { waitUntil: 'load' });
        const r = await p.evaluate(() => {
          const rowEl = document.querySelector('.composer-row');
          // .chip-row is the box that can overflow now: .composer-row holds a
          // chip block that shrinks to fit and a send button that does not, so
          // its scrollWidth equals its clientWidth by construction and asking
          // it is asking a question with one possible answer. The composer
          // that offers no chips at all writes no .chip-row, and for that one
          // the row is still the right box.
          const chipRow = document.querySelector('.chip-row') || rowEl;
          const send = document.querySelector('.send');
          const chips = [...document.querySelectorAll('.composer-chip')];
          const spill = Math.max(0, ...chips.map((c) => {
            const right = c.getBoundingClientRect().right;
            const label = c.querySelector('.chip-label');
            // A label clips itself, so its own border box is the painted
            // extent — a Range would report the pre-ellipsis width and cry
            // wolf. A chip whose text is a bare node has nothing clipping it,
            // and an anonymous text node has no box, so that one needs the
            // Range.
            if (label) {
              return Math.round(label.getBoundingClientRect().right - right);
            }
            const range = document.createRange();
            range.selectNodeContents(c);
            return Math.round(range.getBoundingClientRect().right - right);
          }));
          // The tier is the fact the chip grew a second element to carry, and
          // the label clips: a name long enough to fill the chip must push the
          // ellipsis, not push the tier out through the side of the pill.
          const tierEl = document.querySelector('.chip-effort');
          const model = document.querySelector('.chip-model');
          const clip = document.querySelector('.chip-label');
          const box = tierEl && tierEl.getBoundingClientRect();
          // Where send sits relative to the block of chips, and whether the
          // one label that is a bare text node says it has been cut.
          const chipBoxes = chips.map((c) => c.getBoundingClientRect());
          const blockTop = Math.min(...chipBoxes.map((b) => b.top));
          const blockBottom = Math.max(...chipBoxes.map((b) => b.bottom));
          const sendBox = send.getBoundingClientRect();
          const modeLabel = document.querySelector('.composer-chip.mode .chip-label');
          return {
            overflow: Math.round(chipRow.scrollWidth - chipRow.clientWidth),
            spill,
            lost: tierEl
              ? box.width < 1
                || box.right > clip.getBoundingClientRect().right + 0.5
              : false,
            tierWidth: tierEl ? Math.round(box.width * 10) / 10 : 0,
            tierCut: tierEl ? tierEl.scrollWidth > tierEl.clientWidth + 0.5 : false,
            nameWidth: model.clientWidth,
            nameCut: model.scrollWidth > model.clientWidth + 0.5,
            sendBelowChips: sendBox.top > blockBottom - 0.5,
            sendOffCentre: Math.abs((sendBox.top + sendBox.bottom) / 2
              - (blockTop + blockBottom) / 2) > 1,
            modeHardClip: modeLabel.scrollWidth > modeLabel.clientWidth + 0.5
              && getComputedStyle(modeLabel).textOverflow !== 'ellipsis',
            sendRight: Math.round(send.getBoundingClientRect().right),
            composerWidth: Math.round(document.querySelector('.composer').getBoundingClientRect().width),
            vw: document.documentElement.clientWidth,
          };
        });
        const problems = [];
        if (r.sendRight > r.vw) {
          problems.push(`send is ${r.sendRight - r.vw}px off the right edge`);
        }
        if (r.overflow > 0) problems.push(`the chip block overflows by ${r.overflow}px`);
        if (r.sendBelowChips) problems.push('the send button wrapped below the chips');
        if (r.sendOffCentre) problems.push('the send button is not centred on the chip block');
        if (r.modeHardClip) problems.push('the mode label is clipped with no ellipsis');
        // An attachment tray full of long file names would stretch the
        // composer itself if it did not scroll sideways.
        if (r.composerWidth > r.vw) {
          problems.push(`the composer is ${r.composerWidth - r.vw}px wider than the screen`);
        }
        if (r.spill > 0) problems.push(`text spills ${r.spill}px past its pill`);
        if (r.lost) problems.push('the effort tier was clipped away');
        if (r.tierWidth > TIER_CAP) {
          problems.push(`the tier takes ${r.tierWidth}px, over the ${TIER_CAP}px it is allowed`);
        }
        if (r.tierCut && !capped) problems.push('the tier is ellipsised');
        // Where the name has had to give way, it must still be holding the
        // greater part of the label. This is the one that fails when a tier
        // is long: the name went to 50px beside a 45px tier, which is
        // "Claude…" next to "Medium".
        // `capped` is a tier neither backend serves, present only to prove
        // the stylesheet clips one. Its width is the cap, not a real tier's,
        // so holding the name to a ratio against it measures the fixture.
        if (!ROWS[row].crowded && !capped && r.nameCut && r.nameWidth < 2 * r.tierWidth) {
          problems.push(`the name keeps ${r.nameWidth}px beside a ${r.tierWidth}px tier`);
        }
        // The floor the stylesheet promises (.chip-model min-width), which
        // holds even in the crowded row. Below this a model name is not
        // truncated, it is absent — and two of this app's models share their
        // first four characters.
        if (r.nameWidth < 30) {
          problems.push(`the name is down to ${r.nameWidth}px, which identifies nothing`);
        }
        if (problems.length) bad += problems.length;
        const key = `${row} | ${label}${tier ? ` + ${tier}` : ''} | tray ${names.length}`;
        if (!namesByWidth.has(key)) namesByWidth.set(key, []);
        namesByWidth.get(key).push([WIDTH, r.nameWidth]);
        console.log(
          `    ${problems.length ? 'FAIL' : 'ok  '} ${`${label}${tier ? ` + ${tier}` : ''}`.padEnd(50)}`
          + ` overflow=${r.overflow}  spill=${r.spill}`
          + `  name=${r.nameWidth}${r.nameCut ? '…' : ''}  tier=${r.tierWidth}`
          + `  send.right=${r.sendRight}/${r.vw}`
          + (problems.length ? `  <- ${problems.join('; ')}` : ''),
        );
       }
      }
    }
  }
  await p.close();
  }
  await browser.close();

  // A bigger phone must never show less of the model name than a smaller one.
  // Nothing above can see this: every number is inside its range at every
  // width taken on its own, and the fault is that two of the widths disagree.
  // It is the shape of failure a wrapping row invites, because where the line
  // breaks and how much the chips get are two different decisions and they can
  // stop agreeing — a chip that just fits on one line takes what is left of
  // that line, while the same chip one point narrower gets a line to itself
  // and takes all of it. Measured at 390pt against 375pt, that read "Claude
  // Son…" on the bigger phone and "Claude Sonnet 4.5" on the smaller one.
  if (WIDTHS.length > 1) {
    for (const [key, seen] of namesByWidth) {
      const ordered = [...seen].sort((a, b) => a[0] - b[0]);
      for (let i = 1; i < ordered.length; i += 1) {
        const [wNarrow, nNarrow] = ordered[i - 1];
        const [wWide, nWide] = ordered[i];
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
    `\nClean at ${WIDTHS.join('/')}pt: send stays on screen, no chip spills, the model name`
    + ' keeps the chip, and it never shrinks as the phone grows.',
  );
})();
