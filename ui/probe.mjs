// Reproduce terminal copy on the REAL self-signed LAN origin, through Chrome's
// certificate interstitial (NOT --ignore-certificate-errors, which would make
// Chrome trust the origin and hide the very condition under test).
import { chromium } from '@playwright/test';

const ORIGIN = process.env.OTTO_ORIGIN;
const step = (m) => console.error(`[step] ${m}`);

const browser = await chromium.launch({ headless: false });
const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
const page = await ctx.newPage();
page.setDefaultTimeout(20000);

step('goto (expect interstitial)');
// The interstitial renders a moment AFTER goto rejects, so retry rather than
// checking once.
for (let i = 0; i < 6; i++) {
  await page.goto(ORIGIN, { waitUntil: 'domcontentloaded' }).catch(() => {});
  await page.waitForTimeout(1200);
  if (page.url().startsWith(ORIGIN)) break;
  if (await page.locator('#details-button').count()) {
    step(`clicking through the cert interstitial (try ${i + 1})`);
    await page.click('#details-button').catch(() => {});
    await page.waitForTimeout(400);
    await page.click('#proceed-link').catch(() => {});
    await page.waitForTimeout(3000);
    if (page.url().startsWith(ORIGIN)) break;
  }
}
step(`url=${page.url()}`);

if (await page.locator('#login-user').count()) {
  step('logging in');
  await page.fill('#login-user', 'root');
  await page.fill('#login-pass', process.env.OTTO_PW || '');
  await page.click('button[type=submit]');
  await page.waitForTimeout(6000);
}
step(`after login url=${page.url()}`);

step('opening a session');
await page.evaluate((sid) => { window.location.hash = `#/agents/${sid}`; },
  process.env.OTTO_SID);
await page.waitForTimeout(5000);
try {
  await page.waitForSelector('.xterm-rows', { timeout: 30000 });
} catch {
  console.error('NO TERMINAL. body:', (await page.locator('body').innerText()).slice(0, 300));
  await browser.close();
  process.exit(1);
}
await page.waitForTimeout(2500);

const env = await page.evaluate(() => ({
  bundle: [...document.querySelectorAll('script[src]')]
    .map((s) => s.src.split('/').pop()).find((s) => s.startsWith('index-')),
  secureContext: window.isSecureContext,
  hasClipboard: !!navigator.clipboard,
  renderer: document.querySelector('.xterm-screen canvas') ? 'canvas/webgl' : 'DOM',
}));

step('drag-selecting');
const box = await page.locator('.xterm-screen').boundingBox();
await page.mouse.move(box.x + 10, box.y + 30);
await page.mouse.down();
await page.mouse.move(box.x + box.width - 60, box.y + 160, { steps: 30 });
await page.mouse.up();
await page.waitForTimeout(600);

const afterDrag = await page.evaluate(() => {
  const ta = document.querySelector('.xterm-helper-textarea');
  return {
    mirrorLen: (ta?.value || '').length,
    mirrorHead: (ta?.value || '').slice(0, 50),
    domSelLen: String(document.getSelection() || '').length,
    activeEl: document.activeElement?.className || document.activeElement?.tagName,
    docFocused: document.hasFocus(),
  };
});

step('pressing the copy chord');
await page.evaluate(() => {
  window.__copied = null;
  document.addEventListener('copy', (e) => {
    window.__copied = e.clipboardData?.getData('text/plain') ?? '';
  });
});
await page.keyboard.press('ControlOrMeta+c');
await page.waitForTimeout(900);

const afterCopy = await page.evaluate(async () => {
  const r = { copyEventFired: window.__copied !== null,
              copyEventText: (window.__copied || '').slice(0, 50) };
  try { r.readback = (await navigator.clipboard.readText()).slice(0, 50); }
  catch (e) { r.readErr = `${e.name}: ${e.message}`; }
  try { await navigator.clipboard.writeText('OTTO_WRITE_PROBE'); r.writeOk = true; }
  catch (e) { r.writeErr = `${e.name}: ${e.message}`; }
  return r;
});

console.log(JSON.stringify({ env, afterDrag, afterCopy }, null, 2));
await browser.close();
