// Does the composer row survive a long, server-supplied model name?
//
//   node docs/measure-composer.js [width]
//
// The send button is the primary action and the only control that must always
// be reachable, so the failure this watches for is a chip holding the row open
// until send is pushed past the right edge. Three things get measured, because
// they fail differently: the row's own overflow (a chip that will not shrink),
// text spilling past its pill (a chip that shrank but whose label could not
// follow it), and a label clipped down to nothing (a chip that gave way so far
// it stopped saying anything).
//
// The third was added the day a mode chip joined the row: every layout still
// reported clean while `Auto` was rendering as a bare ellipsis, because
// flexbox shares a deficit in proportion to content width and a four-letter
// label has nothing to give. A row can be within its bounds and still be
// useless.
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
const settings = (label) =>
  `<button class="composer-chip action model"><span class="chip-label">${label}</span>${icon}</button>`;
// The mode chip. It does not shrink — its label is short and the sheet does
// not restate it — so it is the model chip beside it that absorbs the whole
// deficit, and the cap on this label is what stops a long mode name taking
// the row instead.
const mode = (label) =>
  `<button class="composer-chip action mode">${icon}<span class="chip-label">${label}</span></button>`;

// The two rows the app actually builds, not a synthetic worst case. Diff and
// PR used to be here and are not any more — they moved to the action row
// above the composer — so what this measures now is a mode chip in their
// place, with the longest mode label each backend really produces: goose's
// "Manual approval", and "General" out of OpenCode's built-in agents.
const ROWS = {
  code: (label) => settings(label) + mode('General'),
  goose: (label) => settings(label) + mode('Manual approval')
    + '<span class="composer-chip">100%</span>',
};

const page = (row, label) => `<!doctype html><html><head><meta charset="utf-8">
<style>${CSS}</style></head><body><div class="app"><footer class="composer">
<textarea class="input" rows="1" placeholder="Message the code agent…"></textarea>
<div class="composer-row">${ROWS[row](label)}
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

(async () => {
  const browser = await chromium.launch();
  const p = await browser.newPage({ viewport: { width: WIDTH, height: 874 } });
  let bad = 0;
  for (const row of Object.keys(ROWS)) {
    console.log(`\n  ${row} composer @ ${WIDTH}pt`);
    for (const label of LABELS) {
      const file = path.join(os.tmpdir(), `composer-${row}-${WIDTH}.html`);
      fs.writeFileSync(file, page(row, label));
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
        // The narrowest label that is actually being clipped. A label with
        // room to spare is not a stub however short its text is — "Auto" at
        // its natural 26px is fine, "Auto" clipped to 4px is not.
        const clipped = [...document.querySelectorAll('.chip-label')]
          .filter((l) => l.scrollWidth > l.clientWidth + 1)
          .map((l) => Math.round(l.clientWidth));
        return {
          overflow: Math.round(rowEl.scrollWidth - rowEl.clientWidth),
          spill,
          stub: clipped.length ? Math.min(...clipped) : null,
          sendRight: Math.round(send.getBoundingClientRect().right),
          vw: document.documentElement.clientWidth,
        };
      });
      const past = r.sendRight - r.vw;
      // Below this a clipped label is an ellipsis and at most one character,
      // which says no more than an empty chip would.
      const STUB = 16;
      const stubbed = r.stub !== null && r.stub < STUB;
      const ok = r.overflow <= 0 && r.spill <= 0 && past <= 0 && !stubbed;
      if (!ok) bad += 1;
      console.log(
        `    ${ok ? 'ok  ' : 'FAIL'} ${label.padEnd(38)}`
        + ` overflow=${r.overflow}  spill=${r.spill}`
        + `  narrowest=${r.stub === null ? 'full' : r.stub}`
        + `  send.right=${r.sendRight}/${r.vw}`,
      );
    }
  }
  await browser.close();
  if (bad) {
    console.log(`\n${bad} composer layout${bad > 1 ? 's' : ''} unusable.`);
    process.exit(1);
  }
  console.log('\nClean: send stays on screen, no chip spills, every chip still says something.');
})();
