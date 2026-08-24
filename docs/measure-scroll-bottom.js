// Is the scroll-to-bottom button hidden, shown and placed the way it claims?
//
//   node docs/measure-scroll-bottom.js [light|dark|both]
//
// Same problem as docs/measure-ptr.js, and the same answer. Whether this
// button is on screen is a class a JS scroll listener puts on <body>
// (src/viewport.rs), so it is never in a captured gallery state: the capture
// runs when the UI settles, and a transcript that has settled is at its
// bottom, which is exactly when the button is not there. A visual property
// nothing can photograph has to be measured instead.
//
// What is measured is placement rather than the gesture. The button hangs out
// of a zero-height slot above the composer, so that it follows the composer as
// the draft grows it and as the shell tracks the visual viewport with the
// keyboard up — anything anchored to the bottom of the screen ends up behind
// one or the other. Those three cases are the compositions below.
//
// The markup is built by src/views/mod.rs (`ScrollToBottom`) and restated here
// the way measure-ptr.js restates the pull indicator's. Keep the two in step;
// if the button grows a part, add it here.
const fs = require('fs');
const path = require('path');
const os = require('os');
const { chromium } = require('playwright');

const CSS = fs.readFileSync(path.join(__dirname, '..', 'assets', 'main.css'), 'utf8');

const ICON = '<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor"'
  + ' stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">'
  + '<path d="M12 4v16M5 13l7 7 7-7"></path></svg>';

const CHIP = '<button class="composer-chip action"><span class="chip-label">'
  + '<span class="chip-model">Claude Opus 5</span>'
  + '<span class="chip-effort">Max</span></span>' + ICON + '</button>';

const shell = (c) => `<div class="app">
<header class="topbar"><h1 class="title">Chat</h1></header>
<main class="scroll chat" id="chat-scroll"><p>content</p></main>
<div class="scroll-bottom-slot"><button class="scroll-bottom">${ICON}</button></div>
${c.actions ? '<div class="action-row"><button class="action-chip">Diff</button></div>' : ''}
<footer class="composer">
<textarea class="input" rows="1" style="height:${c.draft}px"></textarea>
<div class="composer-row">${CHIP}<button class="send">${ICON}</button></div>
</footer></div>`;

// --vv-height is what src/viewport.rs writes when the keyboard opens, and
// --safe-bottom goes to 0 with it: the home indicator is under the keyboard by
// then, so reserving for it would park the composer on a strip of nothing.
const page = (c, on) => `<!doctype html><html><head><meta charset="utf-8">
<style>${CSS}</style><style>:root{--safe-top:62px;--safe-bottom:${c.keyboard ? '0px' : '34px'};
${c.keyboard ? '--vv-height:520px;' : ''}}</style></head>
<body class="${on ? 'away-from-bottom' : ''}">${shell(c)}</body></html>`;

// The three shapes the button has to sit above, on both tabs.
const CASES = [
  { label: 'goose composer', draft: 24, actions: false, keyboard: false },
  { label: 'code action row', draft: 24, actions: true, keyboard: false },
  { label: 'a draft four lines tall', draft: 96, actions: false, keyboard: false },
  { label: 'the keyboard up', draft: 24, actions: false, keyboard: true },
];

(async () => {
  const themes = (process.argv[2] || 'both') === 'both' ? ['light', 'dark'] : [process.argv[2]];
  const browser = await chromium.launch();
  let bad = 0;
  for (const theme of themes) {
    const p = await browser.newPage({ viewport: { width: 402, height: 874 } });
    await p.emulateMedia({ colorScheme: theme });
    console.log(`\n  ${theme}`);
    for (const c of CASES) {
      for (const on of [false, true]) {
        const file = path.join(os.tmpdir(), `sb-${theme}-${c.label.replace(/ /g, '_')}-${on}.html`);
        fs.writeFileSync(file, page(c, on));
        await p.goto(`file://${file}`, { waitUntil: 'load' });
        const r = await p.evaluate(() => {
          const el = document.querySelector('.scroll-bottom');
          // Whatever the slot hangs above: the action row where there is one,
          // the composer where there is not.
          const next = document.querySelector('.action-row') || document.querySelector('.composer');
          const cs = getComputedStyle(el);
          const box = el.getBoundingClientRect();
          const below = next.getBoundingClientRect();
          return {
            opacity: parseFloat(cs.opacity),
            taps: cs.pointerEvents !== 'none',
            width: Math.round(box.width),
            height: Math.round(box.height),
            top: Math.round(box.top),
            bottom: Math.round(box.bottom),
            gap: Math.round(below.top - box.bottom),
            centred: Math.abs((box.left + box.right) / 2
              - document.documentElement.clientWidth / 2) < 1,
            shellBottom: Math.round(document.querySelector('.app').getBoundingClientRect().bottom),
          };
        });
        const shown = r.opacity > 0.01;
        const problems = [];
        if (shown !== on) {
          problems.push(shown ? 'visible without the class' : 'invisible with the class');
        }
        if (r.taps !== on) {
          problems.push(on ? 'not tappable' : 'tappable while hidden');
        }
        // The HIG's 44pt. A round button with no label has nothing else to aim at.
        if (r.width < 44 || r.height < 44) problems.push(`${r.width}x${r.height}, under 44pt`);
        if (!r.centred) problems.push('not horizontally centred');
        // Above what follows it, and close enough to read as belonging to it.
        if (r.gap < 0) problems.push(`overlaps what is below it by ${-r.gap}px`);
        if (r.gap > 24) problems.push(`${r.gap}px above the composer — adrift`);
        if (r.top < 0 || r.bottom > r.shellBottom) {
          problems.push(`outside the shell (${r.top}..${r.bottom} of ${r.shellBottom})`);
        }
        if (problems.length) bad += problems.length;
        console.log(
          `    ${problems.length ? 'FAIL' : 'ok  '} ${`${c.label}, ${on ? 'shown' : 'at rest'}`.padEnd(34)}`
          + ` opacity=${r.opacity} top=${r.top} bottom=${r.bottom} gap=${r.gap}`
          + (problems.length ? `  <- ${problems.join('; ')}` : ''),
        );
      }
    }
    await p.close();
  }
  await browser.close();
  if (bad) {
    console.log(`\n${bad} problem${bad > 1 ? 's' : ''} with the scroll-to-bottom button.`);
    process.exit(1);
  }
  console.log('\nClean: hidden at the bottom, reachable above the composer everywhere else.');
})();
