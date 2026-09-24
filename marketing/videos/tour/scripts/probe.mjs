// Dev helper: screenshot routes against a running `stack-up.mjs` stack.
//   node scripts/probe.mjs home assistant agents ...   (hash routes)
// Shots land in .cache/probe/<route>.png (1920×1080 @1x).
import { chromium } from 'playwright';
import { mkdirSync, readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { storageState, UI } from './lib/stack.mjs';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const { token } = JSON.parse(readFileSync(join(tour, '.cache/stack.json'), 'utf8'));
const out = join(tour, '.cache/probe');
mkdirSync(out, { recursive: true });
const theme = process.env.THEME ?? 'dark';
const browser = await chromium.launch();
const ctx = await browser.newContext({
  viewport: { width: Number(process.env.VW ?? 1600), height: Number(process.env.VH ?? 900) }, deviceScaleFactor: Number(process.env.DPR ?? 1),
  colorScheme: theme,
  storageState: storageState(token),
});
const page = await ctx.newPage();
page.on('pageerror', (e) => console.log('[pageerror]', e.message));
for (const route of process.argv.slice(2)) {
  await page.goto(`${UI}/#/${route}`);
  await page.waitForTimeout(Number(process.env.WAIT ?? 2500));
  const name = route.replace(/[^a-z0-9-]+/gi, '_') || 'root';
  await page.screenshot({ path: join(out, `${name}.png`) });
  console.log('shot', name);
}
await browser.close();
