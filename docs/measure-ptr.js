// Is the pull-to-refresh indicator where it should be, in each of its states?
//
//   node docs/measure-ptr.js [light|dark|both]
//
// The gesture itself is verified on a device — it is touch, and nothing here
// can stand in for that. What this covers is the half that a device run cannot
// show you: against a local mock the fetch settles in ~120ms, so the indicator
// exists for two frames and no screenshot will ever catch it. Every visual
// property of a control you cannot photograph has to be measured instead.
//
// The markup is built by src/viewport.rs rather than by a view, so it is
// restated here. That is the drift risk this repo otherwise avoids by
// generating the gallery — keep the two in step, and if the element grows a
// part, add it here.
const fs = require('fs');
const path = require('path');
const os = require('os');
const { chromium } = require('playwright');

const CSS = fs.readFileSync(path.join(__dirname, '..', 'assets', 'main.css'), 'utf8');

const SPINNER = '<div class="ptr"><svg viewBox="0 0 24 24" fill="none"'
  + ' stroke="currentColor" stroke-width="2" stroke-linecap="round"'
  + ' stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-2.64-6.36M21 3v6h-6"></path>'
  + '</svg></div>';

const page = (cls, y) => `<!doctype html><html><head><meta charset="utf-8">
<style>${CSS}</style><style>:root{--safe-top:62px;--safe-bottom:34px}
.ptr{${y === null ? '' : `--ptr-y:${y}px;`}}</style></head>
<body><div class="app"><header class="topbar"><h1 class="title">Code</h1></header>
<main class="scroll" data-refresh="code"><p>content</p></main></div>
${SPINNER.replace('class="ptr"', `class="${cls}"`)}</body></html>`;

// Every state the gesture puts it through, with the pull distance that goes
// with it. THRESHOLD and MAX come from src/viewport.rs.
const STATES = [
  { cls: 'ptr', y: null, label: 'at rest', visible: false },
  { cls: 'ptr on', y: 30, label: 'pulling, not yet armed', visible: true },
  { cls: 'ptr on armed', y: 64, label: 'armed at the threshold', visible: true },
  { cls: 'ptr on armed', y: 96, label: 'pulled to the maximum', visible: true },
  { cls: 'ptr run on', y: 64, label: 'refreshing', visible: true },
];

(async () => {
  const themes = (process.argv[2] || 'both') === 'both' ? ['light', 'dark'] : [process.argv[2]];
  const browser = await chromium.launch();
  let bad = 0;
  for (const theme of themes) {
    const p = await browser.newPage({ viewport: { width: 402, height: 874 } });
    await p.emulateMedia({ colorScheme: theme });
    console.log(`\n  ${theme}`);
    for (const state of STATES) {
      const file = path.join(os.tmpdir(), `ptr-${theme}-${state.cls.replace(/ /g, '_')}-${state.y}.html`);
      fs.writeFileSync(file, page(state.cls, state.y));
      await p.goto(`file://${file}`, { waitUntil: 'load' });
      const r = await p.evaluate(() => {
        const el = document.querySelector('.ptr');
        const bar = document.querySelector('.topbar');
        const cs = getComputedStyle(el);
        const box = el.getBoundingClientRect();
        return {
          opacity: parseFloat(cs.opacity),
          top: Math.round(box.top),
          bottom: Math.round(box.bottom),
          left: Math.round(box.left),
          width: Math.round(box.width),
          centred: Math.abs((box.left + box.right) / 2 - document.documentElement.clientWidth / 2) < 1,
          barBottom: Math.round(bar.getBoundingClientRect().bottom),
          vh: document.documentElement.clientHeight,
        };
      });
      const shown = r.opacity > 0.01;
      const problems = [];
      if (shown !== state.visible) {
        problems.push(shown ? 'visible when it should not be' : 'invisible when it should be shown');
      }
      if (state.visible) {
        if (!r.centred) problems.push('not horizontally centred');
        // Off the top of the screen is the same as not being there.
        if (r.bottom <= 0) problems.push(`entirely above the viewport (bottom ${r.bottom})`);
        // And it must not sail down into the list it is refreshing.
        if (r.top > r.barBottom) problems.push(`below the bar (top ${r.top} > ${r.barBottom})`);
      }
      if (problems.length) bad += problems.length;
      console.log(
        `    ${problems.length ? 'FAIL' : 'ok  '} ${state.label.padEnd(24)}`
        + ` opacity=${r.opacity} top=${r.top} bottom=${r.bottom} w=${r.width}`
        + (problems.length ? `  <- ${problems.join('; ')}` : ''),
      );
    }
    await p.close();
  }
  await browser.close();
  if (bad) {
    console.log(`\n${bad} problem${bad > 1 ? 's' : ''} with the pull indicator.`);
    process.exit(1);
  }
  console.log('\nClean: the indicator is hidden at rest and reachable in every pulled state.');
})();
