// Does the composer row survive a long, server-supplied model name?
//
//   node docs/measure-composer.js [width]
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
// overflow-x: hidden, which a chip never sets. And it renders at 402pt, which
// is the width where the damage is smallest.
const fs = require('fs');
const path = require('path');
const os = require('os');
const { chromium } = require('playwright');

const CSS = fs.readFileSync(path.join(__dirname, '..', 'assets', 'main.css'), 'utf8');
const WIDTH = parseInt(process.argv[2] || '402', 10);

// What .chip-effort's max-width comes to at --text-xs, plus a couple of
// pixels of font-metric slack. No tier may cost the name more than this.
const TIER_CAP = 40;

const icon = '<svg class="icon" viewBox="0 0 24 24"></svg>';
const settings = (label, effort) =>
  '<button class="composer-chip action"><span class="chip-label">'
  + `<span class="chip-model">${label}</span>`
  + (effort ? `<span class="chip-effort">${effort}</span>` : '')
  + `</span>${icon}</button>`;

// The two rows the app actually builds, not a synthetic worst case. The code
// row is the shorter of the two now: Diff and PR moved out to the action row
// above the composer, which is a sideways scroller and has its own budget.
const ROWS = {
  code: settings,
  goose: (label, effort) => settings(label, effort)
    + '<span class="composer-chip">128k/200k</span>',
};

const page = (row, label, effort) => `<!doctype html><html><head><meta charset="utf-8">
<style>${CSS}</style></head><body><div class="app"><footer class="composer">
<textarea class="input" rows="1" placeholder="Message the code agent…"></textarea>
<div class="composer-row">${ROWS[row](label, effort)}
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
  const p = await browser.newPage({ viewport: { width: WIDTH, height: 874 } });
  let bad = 0;
  for (const row of Object.keys(ROWS)) {
    console.log(`\n  ${row} composer @ ${WIDTH}pt`);
    for (const label of LABELS) {
      for (const { tier, capped } of EFFORTS) {
        const file = path.join(os.tmpdir(), `composer-${row}-${WIDTH}.html`);
        fs.writeFileSync(file, page(row, label, tier));
        await p.goto(`file://${file}`, { waitUntil: 'load' });
        const r = await p.evaluate(() => {
          const rowEl = document.querySelector('.composer-row');
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
          return {
            overflow: Math.round(rowEl.scrollWidth - rowEl.clientWidth),
            spill,
            lost: tierEl
              ? box.width < 1
                || box.right > clip.getBoundingClientRect().right + 0.5
              : false,
            tierWidth: tierEl ? Math.round(box.width * 10) / 10 : 0,
            tierCut: tierEl ? tierEl.scrollWidth > tierEl.clientWidth + 0.5 : false,
            nameWidth: model.clientWidth,
            nameCut: model.scrollWidth > model.clientWidth + 0.5,
            sendRight: Math.round(send.getBoundingClientRect().right),
            vw: document.documentElement.clientWidth,
          };
        });
        const problems = [];
        if (r.sendRight > r.vw) {
          problems.push(`send is ${r.sendRight - r.vw}px off the right edge`);
        }
        if (r.overflow > 0) problems.push(`the row overflows by ${r.overflow}px`);
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
        if (r.nameCut && r.nameWidth < 2 * r.tierWidth) {
          problems.push(`the name keeps ${r.nameWidth}px beside a ${r.tierWidth}px tier`);
        }
        if (problems.length) bad += problems.length;
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
  await browser.close();
  if (bad) {
    console.log(`\n${bad} composer problem${bad > 1 ? 's' : ''}.`);
    process.exit(1);
  }
  console.log('\nClean: send stays on screen, no chip spills, and the model name keeps the chip.');
})();
