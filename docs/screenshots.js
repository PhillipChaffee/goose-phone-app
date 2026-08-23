// Regenerate the README screenshots from the style gallery.
//
//   npm i -D playwright        (Chromium only; see PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD)
//   node docs/screenshots.js
//
// Each image is one gallery frame — a 390x844 viewport rendering the app's
// real markup against the real assets/main.css — captured at 2x. They are
// renders of the shipping stylesheet, not captures of a running build: the
// gallery reproduces the DOM the views emit, so the styling is exactly what
// ships while the content is fixture data. That trade is deliberate. It keeps
// the README truthful about how the app looks on a change-by-change basis,
// where device screenshots go stale silently and did (the previous set showed
// a palette and a navigation layout that no longer existed).
//
// If you have a device or a desktop build to hand, real captures are better
// still — drop them in docs/images/ with these names and delete nothing else.

const fs = require('fs');
const path = require('path');
const { chromium } = require('playwright');

const ROOT = path.join(__dirname, '..');
const GALLERY = 'file://' + path.join(__dirname, 'style-gallery.html');
const OUT = path.join(__dirname, 'images');
const CHROME = '/opt/pw-browsers/chromium';

// label substring -> output file. Themes are mixed on purpose: the README
// should show that the app has both.
const SHOTS = [
  { match: 'Chat (transcript)', theme: 'dark', file: 'chat.png' },
  { match: 'Permission modal', theme: 'dark', file: 'permission.png' },
  { match: 'Sessions (populated)', theme: 'dark', file: 'sessions.png' },
  { match: 'Settings (connected, top)', theme: 'light', file: 'settings.png' },
  { match: 'Code · Chat (running)', theme: 'light', file: 'code-chat.png' },
];

(async () => {
  if (!fs.existsSync(CHROME)) {
    console.error(`Chromium not found at ${CHROME}; set executablePath for your machine.`);
    process.exit(1);
  }
  fs.mkdirSync(OUT, { recursive: true });
  const browser = await chromium.launch({ executablePath: CHROME });

  for (const shot of SHOTS) {
    const page = await browser.newPage({
      viewport: { width: 1720, height: 1200 },
      deviceScaleFactor: 2,
    });
    await page.emulateMedia({ colorScheme: shot.theme });
    await page.goto(GALLERY);
    await page.waitForTimeout(600);

    const labels = await page.$$eval('iframe.gallery-frame', (frames) =>
      frames.map((f) => f.getAttribute('title') || ''),
    );
    const i = labels.findIndex((l) => l.includes(shot.match));
    if (i < 0) {
      console.error(`no frame matching ${JSON.stringify(shot.match)}; have:\n  ${labels.join('\n  ')}`);
      process.exit(1);
    }
    const frames = await page.$$('iframe.gallery-frame');
    const out = path.join(OUT, shot.file);
    await frames[i].screenshot({ path: out });
    const kb = (fs.statSync(out).size / 1024).toFixed(0);
    console.log(`${path.relative(ROOT, out)}  <- ${labels[i]}  [${shot.theme}, ${kb} KB]`);
    await page.close();
  }

  await browser.close();
})();
