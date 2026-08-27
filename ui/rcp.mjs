// Probe terminal copy against the REAL self-signed LAN origin, going through
// Chrome's certificate interstitial rather than `ignoreHTTPSErrors`. That
// distinction is the whole point: --ignore-certificate-errors makes Chrome
// treat the origin as fully trusted, which is exactly the state we are NOT
// trying to test. Clicking "Proceed" leaves the origin de-privileged, the way
// it is for a real user.
import { chromium } from '@playwright/test';
import { readFileSync } from 'node:fs';

const ORIGIN = 'https://192.168.60.96:7701';
const DIR = '/private/tmp/claude-502/-Users-tech-ai-otto-os/c747733f-d9ef-43f7-ba8a-416a033cba35/scratchpad';
const token = readFileSync(`${DIR}/.tok`, 'utf8').trim();
const sid = readFileSync(`${DIR}/.sid`, 'utf8').trim();

const browser = await chromium.launch({ headless: false });
// NO ignoreHTTPSErrors — we want the interstitial and the de-privileged origin.
const ctx = await browser.newContext();
const page = await ctx.newPage();

await page.goto(ORIGIN, { waitUntil: 'domcontentloaded' }).catch((e) => {
  console.error('goto threw:', e.message.split('\n')[0]);
});
console.error('after goto  url=', page.url(), 'title=', await page.title());
// Click through "Your connection is not private". Headless Chrome still renders
// the interstitial; the Advanced button may need a moment to attach.
for (let i = 0; i < 3; i++) {
  if (!(await page.locator('#details-button').count())) break;
  await page.click('#details-button').catch(() => {});
  await page.waitForTimeout(300);
  await page.click('#proceed-link').catch(() => {});
  await page.waitForTimeout(1500);
}
console.error('after proceed url=', page.url(), 'title=', await page.title());
// Log in through the UI — the stored token alone doesn't satisfy the app's
// own auth bootstrap.
if (await page.getByLabel(/username/i).count()) {
  await page.getByLabel(/username/i).fill('root');
  await page.getByLabel(/password/i).fill(process.env.OTTO_PW || '');
  await page.getByRole('button', { name: /sign in/i }).click();
  await page.waitForTimeout(4000);
}
await page.evaluate((w) => localStorage.setItem('otto_workspace', w),
  '01KXWRZ5WJ7RFV49ZXND4E39DX');
await page.goto(`${ORIGIN}/#/agents/${sid}`, { waitUntil: 'domcontentloaded' });
await page.waitForTimeout(6000);

try {
  await page.waitForSelector('.xterm-rows', { timeout: 30000 });
} catch {
  console.error('no .xterm-rows. url=', page.url());
  console.error('body head:', (await page.locator('body').innerText()).slice(0, 400));
  await page.screenshot({ path: `${DIR}/remote-probe.png`, fullPage: false });
  console.error('screenshot ->', `${DIR}/remote-probe.png`);
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

// Drag-select across the terminal, the way a user does.
const box = await page.locator('.xterm-screen').boundingBox();
await page.mouse.move(box.x + 10, box.y + 24);
await page.mouse.down();
await page.mouse.move(box.x + box.width - 60, box.y + 140, { steps: 30 });
await page.mouse.up();
await page.waitForTimeout(400);

const afterDrag = await page.evaluate(() => {
  const ta = document.querySelector('.xterm-helper-textarea');
  return {
    mirrorLen: (ta?.value || '').length,
    mirrorHead: (ta?.value || '').slice(0, 40),
    domSelLen: String(document.getSelection() || '').length,
    activeEl: document.activeElement?.className || document.activeElement?.tagName,
  };
});

// Arm a copy-event listener, press the real chord, see what the browser does.
await page.evaluate(() => {
  window.__copied = null;
  document.addEventListener('copy', (e) => {
    window.__copied = e.clipboardData?.getData('text/plain') ?? '';
  });
});
await page.keyboard.press('ControlOrMeta+c');
await page.waitForTimeout(600);

const afterCopy = await page.evaluate(async () => {
  let readback = null;
  let readErr = null;
  try { readback = await navigator.clipboard.readText(); }
  catch (e) { readErr = `${e.name}: ${e.message}`; }
  let writeErr = null;
  try { await navigator.clipboard.writeText('OTTO_WRITE_PROBE'); }
  catch (e) { writeErr = `${e.name}: ${e.message}`; }
  return {
    copyEventFired: window.__copied !== null,
    copyEventText: (window.__copied || '').slice(0, 40),
    readback: (readback || '').slice(0, 40),
    readErr,
    writeErr,
  };
});

console.log(JSON.stringify({ env, afterDrag, afterCopy }, null, 2));
await browser.close();
