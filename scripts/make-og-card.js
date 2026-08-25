// Regenerate docs/images/og-card.png — the social preview for docs/index.html.
//
//   node scripts/make-og-card.js
//
// A link to the project page unfurls on Discord, X, Mastodon and LinkedIn only
// if the page offers an og:image, and the size those scrapers want (1200x630)
// is not the shape of any screenshot in docs/images/ — they are 1206x2622 and
// 780x1690 phone frames. So the card is drawn here rather than cropped, in the
// same tokens as the page and the app: dark surface, serif headline, sans
// interface text, the connection dot from the top bar.
//
// It is generated rather than hand-made for the same reason the style gallery
// is: an artefact nobody can rebuild is an artefact that drifts. Change the
// copy below, re-run, commit the PNG.

const path = require('path');
const { chromium } = require('playwright');

const OUT = path.join(__dirname, '..', 'docs', 'images', 'og-card.png');

// The tokens are the dark half of assets/main.css, copied the same way
// docs/index.html copies them.
const CARD = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><style>
  * { box-sizing: border-box; margin: 0; }
  :root {
    --bg-primary: #22252a;
    --bg-secondary: #3f434b;
    --text-primary: #ffffff;
    --text-secondary: #b8b8b8;
    --bg-success: #a3d795;
    --border-primary: #3f434b;
    --font-sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
    --font-serif: ui-serif, "New York", Charter, Georgia, serif;
  }
  body {
    width: 1200px; height: 630px;
    display: flex; flex-direction: column; justify-content: center;
    gap: 26px;
    padding: 0 86px;
    background: var(--bg-primary);
    color: var(--text-primary);
    font-family: var(--font-sans);
    -webkit-font-smoothing: antialiased;
  }
  .eyebrow {
    display: inline-flex; align-items: center; gap: 12px; align-self: flex-start;
    padding: 8px 20px;
    border: 1px solid var(--border-primary); border-radius: 9999px;
    background: var(--bg-secondary);
    font-size: 22px; font-weight: 500; letter-spacing: 0.02em;
  }
  .dot { width: 13px; height: 13px; border-radius: 9999px; background: var(--bg-success); }
  h1 { font-family: var(--font-serif); font-size: 78px; font-weight: 600; line-height: 1.1; letter-spacing: -0.015em; }
  p { font-family: var(--font-serif); font-size: 33px; line-height: 1.4; color: var(--text-secondary); max-width: 40ch; }
  .rule { width: 96px; height: 4px; border-radius: 9999px; background: var(--bg-secondary); }
  .foot { display: flex; gap: 14px; flex-wrap: wrap; }
  .chip {
    padding: 8px 18px; border-radius: 9999px;
    background: var(--bg-secondary); color: var(--text-primary);
    font-size: 21px; font-weight: 500;
  }
</style></head><body>
  <span class="eyebrow"><span class="dot"></span>Goose Mobile</span>
  <h1>Your own AI agents,<br>on your phone</h1>
  <div class="rule"></div>
  <p>A Rust client for a goose server and for containerised code agents, over your own Tailscale network.</p>
  <div class="foot">
    <span class="chip">iOS</span>
    <span class="chip">Android</span>
    <span class="chip">Desktop</span>
    <span class="chip">Dioxus</span>
    <span class="chip">No hosted service</span>
  </div>
</body></html>`;

(async () => {
  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1200, height: 630 } });
  await page.setContent(CARD, { waitUntil: 'load' });
  await page.screenshot({ path: OUT });
  await browser.close();
  console.log(OUT);
})();
