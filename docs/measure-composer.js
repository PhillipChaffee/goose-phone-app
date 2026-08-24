// Does the composer survive a long, server-supplied model name — and a file
// name, which is worse, because that one comes from a photo library?
//
//   node docs/measure-composer.js [width]
//
// The send button is the primary action and the only control that must always
// be reachable, so the failure this watches for is a chip holding the row open
// until send is pushed past the right edge. Three things get measured, because
// they fail differently: the row's own overflow (a chip that will not shrink),
// text spilling past its pill (a chip that shrank but whose label could not
// follow it), and the composer itself growing wider than the screen — which is
// what an attachment tray full of long names would do if it did not scroll.
//
// docs/audit.js cannot see the first two. Its overflow walk visits elements,
// and spilling text is an anonymous text node; its clipped-text check requires
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
  `<button class="composer-chip action"><span class="chip-label">${label}</span>${icon}</button>`;
const attach = '<button class="composer-chip action attach" data-attach="code">'
  + `${icon}</button>`;

// The two rows the app actually builds, not a synthetic worst case. Diff and
// PR are not in here: they moved out of the composer into .action-row above
// it, which scrolls sideways and so has no width budget to blow.
const ROWS = {
  code: (label) => attach + settings(label),
  goose: (label) => attach + settings(label)
    + '<span class="composer-chip">128.0k/200.0k</span>',
};

// A picked file's name is not the app's to choose. The last is what iOS calls
// a screenshot; the first is a document someone actually had lying around.
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

const page = (row, label, names) => `<!doctype html><html><head><meta charset="utf-8">
<style>${CSS}</style></head><body><div class="app"><footer class="composer">
${names.length ? tray(names) : ''}
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

// Empty, one file, and the tray at its cap — three chips is where it has to
// start scrolling rather than stretching the composer.
const TRAYS = [[], [FILES[0]], FILES];

const MEASURE = () => {
  const rowEl = document.querySelector('.composer-row');
  const composer = document.querySelector('.composer');
  const send = document.querySelector('.send');
  const chips = [...document.querySelectorAll('.composer-chip')];
  // How far the *painted* text reaches past its pill.
  //
  // The text, not the box: a name that is a flex item is blockified, so its
  // border box is whatever width the column gives it however long the string
  // is, and reading the box reports zero for text spilling right across the
  // screen. A Range measures the string. It is then clamped by the element's
  // own box when the element clips, because that is where the ellipsis cuts
  // it off — without the clamp a correctly-ellipsised label cries wolf.
  const past = (chip, label) => {
    const right = chip.getBoundingClientRect().right;
    const range = document.createRange();
    range.selectNodeContents(label || chip);
    let reach = range.getBoundingClientRect().right;
    if (label && getComputedStyle(label).overflowX !== 'visible') {
      reach = Math.min(reach, label.getBoundingClientRect().right);
    }
    return Math.round(reach - right);
  };
  const spill = Math.max(
    0,
    ...chips.map((c) => past(c, c.querySelector('.chip-label'))),
    // The tray scrolls, so a chip past the right edge is fine; a NAME past
    // its own chip is not, and no chip sets overflow-x for the audit to see.
    ...[...document.querySelectorAll('.attach-chip')]
      .map((c) => past(c, c.querySelector('.attach-name'))),
  );
  return {
    overflow: Math.round(rowEl.scrollWidth - rowEl.clientWidth),
    spill,
    sendRight: Math.round(send.getBoundingClientRect().right),
    composerRight: Math.round(composer.getBoundingClientRect().right),
    vw: document.documentElement.clientWidth,
  };
};

(async () => {
  const browser = await chromium.launch();
  const p = await browser.newPage({ viewport: { width: WIDTH, height: 874 } });
  let bad = 0;
  for (const row of Object.keys(ROWS)) {
    for (const names of TRAYS) {
      const held = names.length === 0 ? 'no attachments'
        : `${names.length} attachment${names.length > 1 ? 's' : ''}`;
      console.log(`\n  ${row} composer @ ${WIDTH}pt, ${held}`);
      for (const label of LABELS) {
        const file = path.join(os.tmpdir(), `composer-${row}-${WIDTH}-${names.length}.html`);
        fs.writeFileSync(file, page(row, label, names));
        await p.goto(`file://${file}`, { waitUntil: 'load' });
        const r = await p.evaluate(MEASURE);
        const sendPast = r.sendRight - r.vw;
        const composerPast = r.composerRight - r.vw;
        const ok = r.overflow <= 0 && r.spill <= 0 && sendPast <= 0 && composerPast <= 0;
        if (!ok) bad += 1;
        console.log(
          `    ${ok ? 'ok  ' : 'FAIL'} ${label.padEnd(38)}`
          + ` overflow=${r.overflow}  spill=${r.spill}`
          + `  send.right=${r.sendRight}/${r.vw}  composer.right=${r.composerRight}`,
        );
      }
    }
  }
  await browser.close();
  if (bad) {
    console.log(`\n${bad} composer layout${bad > 1 ? 's' : ''} overflow.`);
    process.exit(1);
  }
  console.log('\nClean: the send button stays on screen, the composer stays on screen,'
    + ' and no chip spills its text.');
})();
