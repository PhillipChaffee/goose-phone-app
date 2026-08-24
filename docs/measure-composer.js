// Does the composer row survive a long, server-supplied model name?
//
//   node docs/measure-composer.js [width]
//
// The send button is the primary action and the only control that must always
// be reachable, so the failure this watches for is a chip holding the row open
// until send is pushed past the right edge. Two things get measured, because
// they fail differently: the row's own overflow (a chip that will not shrink)
// and text spilling past its pill (a chip that shrank but whose label could
// not follow it).
//
// docs/audit.js cannot see either. Its overflow walk visits elements, and
// spilling text is an anonymous text node; its clipped-text check requires
// overflow-x: hidden, which a chip never sets. And it renders at 402pt, which
// is the width where the damage is smallest.
const fs = require('fs');
const path = require('path');
const os = require('os');
const { chromium } = require('playwright');

const CSS = fs.readFileSync(path.join(__dirname, '..', 'assets', 'main.css'), 'utf8');
const WIDTH = parseInt(process.argv[2] || '402', 10);

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
    + '<span class="composer-chip">128.0k/200.0k</span>',
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

// The chip states the thinking effort after the model, so every label is
// measured with and without one. The tier is the part that must survive: it is
// pinned while the name ellipsises, so a name long enough to fill the chip
// must not push it out of the pill.
const EFFORTS = ['', 'Max'];

(async () => {
  const browser = await chromium.launch();
  const p = await browser.newPage({ viewport: { width: WIDTH, height: 874 } });
  let bad = 0;
  for (const row of Object.keys(ROWS)) {
    console.log(`\n  ${row} composer @ ${WIDTH}pt`);
    for (const label of LABELS) {
      for (const effort of EFFORTS) {
        const file = path.join(os.tmpdir(), `composer-${row}-${WIDTH}.html`);
        fs.writeFileSync(file, page(row, label, effort));
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
          const tier = document.querySelector('.chip-effort');
          const clip = document.querySelector('.chip-label');
          const box = tier && tier.getBoundingClientRect();
          return {
            overflow: Math.round(rowEl.scrollWidth - rowEl.clientWidth),
            spill,
            lost: tier
              ? box.width < 1
                || box.right > clip.getBoundingClientRect().right + 0.5
              : false,
            sendRight: Math.round(send.getBoundingClientRect().right),
            vw: document.documentElement.clientWidth,
          };
        });
        const past = r.sendRight - r.vw;
        const ok = r.overflow <= 0 && r.spill <= 0 && past <= 0 && !r.lost;
        if (!ok) bad += 1;
        console.log(
          `    ${ok ? 'ok  ' : 'FAIL'} ${`${label}${effort ? ` + ${effort}` : ''}`.padEnd(44)}`
          + ` overflow=${r.overflow}  spill=${r.spill}`
          + `  send.right=${r.sendRight}/${r.vw}${r.lost ? '  <- the effort tier was clipped away' : ''}`,
        );
      }
    }
  }
  await browser.close();
  if (bad) {
    console.log(`\n${bad} composer layout${bad > 1 ? 's' : ''} overflow.`);
    process.exit(1);
  }
  console.log('\nClean: the send button stays on screen and no chip spills its text.');
})();
