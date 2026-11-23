import { test, expect, devices, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// The Product mode switcher (Create · Stories · Designs · Voice · Learnings) is
// one control across the four Product pages that show it (Create opens in its
// own focus view): moving between them must change only the
// highlighted segment. Its box (x, y, width, height) and segment order are
// compared across the four routes at a desktop, a tablet and a phone size.

const ROUTES = [
  { path: 'product', active: 'Stories' },
  { path: 'design', active: 'Designs' },
  { path: 'design/tts', active: 'Voice' },
  { path: 'product/learnings', active: 'Learnings' },
];
const shots = process.env.OTTO_E2E_SHOTS;

let wsId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base, 'Mode switcher');
  // A story makes the Stories header carry its crumbs and stage badge.
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/product/drafts`, { data: {} });
  expect(r.ok(), await r.text()).toBeTruthy();
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
});

async function switcherOn(page: Page, path: string, active: string) {
  await page.goto(`/#/${path}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const tabs = page.getByRole('tablist', { name: 'View' });
  await expect(tabs.getByRole('tab', { name: active })).toHaveAttribute('aria-selected', 'true', { timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  // Let the header's fit pass (a rAF after the last resize) settle.
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  const box = (await tabs.boundingBox())!;
  const order = await tabs.getByRole('tab').allTextContents();
  return { box, order: order.map((t) => t.trim()) };
}

function steadyAcross(label: string, use: Parameters<typeof test.use>[0]) {
  test.describe(label, () => {
    test.use(use);
    test(`the mode switcher holds its place on all four Product pages (${label})`, async ({ page }) => {
      test.setTimeout(120_000);
      const seen: { path: string; box: { x: number; y: number; width: number; height: number }; order: string[] }[] = [];
      for (const r of ROUTES) {
        seen.push({ path: r.path, ...(await switcherOn(page, r.path, r.active)) });
        if (shots && label === 'desktop') await page.screenshot({ path: `${shots}/${r.path.replace('/', '-')}.png` });
      }
      const [first] = seen;
      for (const s of seen.slice(1)) {
        expect(s.order, `segment order on #/${s.path}`).toEqual(first.order);
        for (const k of ['x', 'y', 'width', 'height'] as const) {
          expect.soft(Math.abs(s.box[k] - first.box[k]), `switcher ${k} on #/${s.path} vs #/${first.path} (${s.box[k]} vs ${first.box[k]})`).toBeLessThanOrEqual(1);
        }
      }
    });
  });
}

// The phone and tablet profiles run on this project's Chromium (a describe
// can't switch browser engines), with their viewport, touch and DPR.
const strip = ({ defaultBrowserType: _, ...d }: (typeof devices)[string]) => d;
steadyAcross('desktop', { viewport: { width: 1280, height: 800 } });
steadyAcross('tablet', strip(devices['iPad Pro 11']));
steadyAcross('phone', strip(devices['iPhone 14 Pro Max']));
