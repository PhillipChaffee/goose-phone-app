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
// Two halves. The first is placement: the button hangs out of a zero-height
// slot above the composer, so that it follows the composer as the draft grows
// it and as the shell tracks the visual viewport with the keyboard up —
// anything anchored to the bottom of the screen ends up behind one or the
// other. Those cases are the compositions in CASES.
//
// The second is when it is there at all, which is the half a static
// composition cannot show, because it is a class that appears and disappears
// as a transcript is read. STEPS drives the real scripts through a real
// scroller: streaming, reading back, the keyboard, a growing draft, a tap.
//
// The markup is built by src/views/mod.rs (`ScrollToBottom`) and restated here
// the way measure-ptr.js restates the pull indicator's. Keep the two in step;
// if the button grows a part, add it here. The *scripts* are not restated —
// they are read out of src/viewport.rs and run as they ship, because a copy
// of a script is a copy that can drift while still passing.
const fs = require('fs');
const path = require('path');
const os = require('os');
const { chromium } = require('playwright');

const CSS = fs.readFileSync(path.join(__dirname, '..', 'assets', 'main.css'), 'utf8');
const VIEWPORT = fs.readFileSync(path.join(__dirname, '..', 'src', 'viewport.rs'), 'utf8');

const die = (why) => {
  console.log(`\n${why}`);
  process.exit(1);
};

// The listener is a raw string; the two scroller scripts are `format!` over
// escaped braces. Both are read back through the escaping Rust applies: a
// backslash swallows the newline and the indent after it, `{{` and `}}` are
// one brace each, and `{id}` is the scroller being addressed. Two markers per
// script rather than one, because the prose around them contains quotes of
// its own and the first `"` after a function's name is usually in a comment.
const script = (item, opens, id) => {
  const at = VIEWPORT.indexOf(item);
  if (at < 0) return die(`${item} is no longer in src/viewport.rs`);
  const from = VIEWPORT.indexOf(opens, at);
  if (from < 0) return die(`${item} no longer builds its script with ${opens}`);
  const open = VIEWPORT.indexOf('"', from);
  const close = VIEWPORT.indexOf('"', open + 1);
  if (open < 0 || close < 0) return die(`could not read the string after ${item}`);
  const out = VIEWPORT.slice(open + 1, close)
    .replace(/\\\n\s*/g, '')
    .replace(/\{\{/g, '{')
    .replace(/\}\}/g, '}')
    .replace(/\{id\}/g, id);
  try {
    new Function(out); // eslint-disable-line no-new-func
  } catch (e) {
    return die(`the script at ${item} does not parse: ${e.message}`);
  }
  return out;
};

const LISTENER = script('const TRANSCRIPT_BOTTOM', '&str = r', '');
const PIN = script('fn pin_script', 'format!(', 'chat-scroll');
const JUMP = script('fn jump_script', 'format!(', 'chat-scroll');
// The slack the listener allows itself, so this measures against the app's
// own idea of "at the bottom" rather than a second one that could differ.
const NEAR = (() => {
  const m = LISTENER.match(/const NEAR = (\d+);/);
  return m ? parseInt(m[1], 10) : die('the listener no longer names a NEAR');
})();

const ICON = '<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor"'
  + ' stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">'
  + '<path d="M12 4v16M5 13l7 7 7-7"></path></svg>';

const ATTACH = '<button class="composer-chip action attach">' + ICON + '</button>';

const CHIP = '<button class="composer-chip action model"><span class="chip-label">'
  + '<span class="chip-model">Claude Opus 5</span>'
  + '<span class="chip-effort">Max</span></span>' + ICON + '</button>';

const MODE = '<button class="composer-chip action mode">' + ICON
  + '<span class="chip-label">Manual approval</span></button>';

// Everything the composer can be carrying at once, which is what puts the
// chips on two lines: the app only shows the context readout near the end of
// the window (`crowding` in src/views/chat.rs), so this is the tallest
// composer the button ever has to clear.
const CROWDED = ATTACH + CHIP + MODE + '<span class="composer-chip warn">96%</span>';

// .chip-row, not the chips loose in .composer-row: the wrap lives one level in
// so that the send button stays pinned to the trailing edge (assets/main.css).
// A fixture with the chips flat cannot produce a two-line composer at all,
// whatever is put in it, so the height this measures against would be a height
// the app never has.
const shell = (c) => `<div class="app">
<header class="topbar"><h1 class="title">Chat</h1></header>
<main class="scroll chat" id="chat-scroll">${c.transcript || '<p>content</p>'}</main>
<div class="scroll-bottom-slot"><button class="scroll-bottom">${ICON}</button></div>
${c.actions ? '<div class="action-row"><button class="action-chip">Diff</button></div>' : ''}
<footer class="composer">
<textarea class="input" rows="1" style="height:${c.draft}px"></textarea>
<div class="composer-row"><div class="chip-row">${c.chips || ATTACH + CHIP}</div>
<button class="send">${ICON}</button></div>
</footer></div>`;

// --vv-height is what src/viewport.rs writes when the keyboard opens, and
// --safe-bottom goes to 0 with it: the home indicator is under the keyboard by
// then, so reserving for it would park the composer on a strip of nothing.
const page = (c, on) => `<!doctype html><html><head><meta charset="utf-8">
<style>${CSS}</style><style>:root{--safe-top:62px;--safe-bottom:${c.keyboard ? '0px' : '34px'};
${c.keyboard ? '--vv-height:520px;' : ''}}</style></head>
<body class="${on ? 'away-from-bottom' : ''}">${shell(c)}</body></html>`;

// The shapes the button has to sit above, on both tabs. The last one is the
// composer at its tallest: four chips is a second line of them, which is 34px
// the button has to move up by and could not reach before .chip-row existed.
const CASES = [
  { label: 'goose composer', draft: 24, actions: false, keyboard: false },
  { label: 'code action row', draft: 24, actions: true, keyboard: false },
  { label: 'a draft four lines tall', draft: 96, actions: false, keyboard: false },
  { label: 'the keyboard up', draft: 24, actions: false, keyboard: true },
  {
    label: 'chips on two lines', draft: 24, actions: false, keyboard: false, chips: CROWDED,
  },
];

// A transcript long enough to read a good way back through.
const TRANSCRIPT = '<div class="msg agent"><p>A paragraph of reply.</p></div>'.repeat(80);

// What a reader does, and what happens to them. `pinned` is "still at the
// bottom", `away` is "not, and told so", `held` is "left exactly where they
// put themselves". Every step is also checked against the one rule the whole
// thing exists for: the button is on screen precisely when the transcript is
// not at its bottom.
//
// The two keyboard steps and the draft step are the ones that had no answer.
// They change the transcript's height rather than its scroll offset, which
// moves the bottom of the conversation away from a reader who has not
// scrolled at all — and fires no scroll event to say so.
const STEPS = [
  { label: 'at the bottom of a long transcript', pinned: true },
  { label: 'a streamed part lands', act: 'grow', pinned: true },
  { label: 'the reader scrolls up', act: 'up', away: true },
  { label: 'a part lands while they read back', act: 'grow', away: true, held: true },
  { label: 'the keyboard opens while they read', act: 'keyboard', away: true, held: true },
  { label: 'the keyboard closes', act: 'nokeyboard', away: true },
  { label: 'the reader returns to the bottom', act: 'down', pinned: true },
  { label: 'the keyboard opens under them', act: 'keyboard', pinned: true },
  { label: 'the draft grows to four lines', act: 'draft', pinned: true },
  { label: 'the keyboard closes again', act: 'nokeyboard', pinned: true },
  { label: 'the reader scrolls up again', act: 'up', away: true },
  { label: 'and taps the button', act: 'tap', pinned: true },
];

// A ResizeObserver delivers before the next paint and a scroll event lands
// after the scroll that caused it; two frames and a tick clears both.
const settle = (p) => p.evaluate(() => new Promise((done) => {
  requestAnimationFrame(() => requestAnimationFrame(() => setTimeout(done, 32)));
}));

const look = (p) => p.evaluate(() => {
  const el = document.getElementById('chat-scroll');
  return {
    distance: Math.round(el.scrollHeight - el.scrollTop - el.clientHeight),
    top: Math.round(el.scrollTop),
    height: Math.round(el.clientHeight),
    shown: document.body.classList.contains('away-from-bottom'),
  };
});

const act = async (p, what) => {
  switch (what) {
    // Content arrives and the view pins, which is what src/views/chat.rs does
    // on every change to the transcript.
    case 'grow':
      await p.evaluate((part) => {
        document.getElementById('chat-scroll').insertAdjacentHTML('beforeend', part);
      }, '<div class="msg agent"><p>Another paragraph.</p></div>');
      await p.evaluate(PIN);
      break;
    case 'up':
      await p.evaluate(() => { document.getElementById('chat-scroll').scrollTop -= 500; });
      break;
    case 'down':
      await p.evaluate(() => {
        const el = document.getElementById('chat-scroll');
        el.scrollTop = el.scrollHeight;
      });
      break;
    // What src/viewport.rs writes onto the root when the visual viewport
    // shrinks. The shell follows it, and the transcript is what gives.
    case 'keyboard':
      await p.evaluate(() => {
        document.documentElement.style.setProperty('--vv-height', '520px');
      });
      break;
    case 'nokeyboard':
      await p.evaluate(() => {
        document.documentElement.style.removeProperty('--vv-height');
      });
      break;
    case 'draft':
      await p.evaluate(() => { document.querySelector('.input').style.height = '96px'; });
      break;
    case 'tap':
      await p.evaluate(JUMP);
      break;
    default:
      break;
  }
  await settle(p);
};

const readingBack = async (browser) => {
  const p = await browser.newPage({ viewport: { width: 402, height: 874 } });
  const file = path.join(os.tmpdir(), 'sb-reading-back.html');
  fs.writeFileSync(file, page({ draft: 24, transcript: TRANSCRIPT }, false));
  await p.goto(`file://${file}`, { waitUntil: 'load' });
  await p.evaluate(LISTENER);
  await p.evaluate(PIN);
  await settle(p);

  let bad = 0;
  let before = await look(p);
  console.log('\n  reading back');
  for (const step of STEPS) {
    await act(p, step.act);
    const now = await look(p);
    const problems = [];
    if (now.shown !== now.distance > NEAR) {
      problems.push(now.shown
        ? 'the button is on screen at the bottom of the transcript'
        : `the button is hidden ${now.distance}px above the bottom`);
    }
    if (step.pinned && now.distance > NEAR) {
      problems.push(`${now.distance}px of transcript is below the fold`);
    }
    if (step.away && now.distance <= NEAR) problems.push('the reader was taken to the bottom');
    if (step.held && now.top !== before.top) {
      problems.push(`the reader was moved ${now.top - before.top}px`);
    }
    if (problems.length) bad += problems.length;
    console.log(
      `    ${problems.length ? 'FAIL' : 'ok  '} ${step.label.padEnd(34)}`
      + ` distance=${now.distance} scroller=${now.height} button=${now.shown ? 'shown' : 'hidden'}`
      + (problems.length ? `  <- ${problems.join('; ')}` : ''),
    );
    before = now;
  }
  await p.close();
  return bad;
};

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
  bad += await readingBack(browser);
  await browser.close();
  if (bad) {
    console.log(`\n${bad} problem${bad > 1 ? 's' : ''} with the scroll-to-bottom button.`);
    process.exit(1);
  }
  console.log('\nClean: hidden at the bottom, reachable above the composer everywhere'
    + ' else, and it notices the keyboard.');
})();
