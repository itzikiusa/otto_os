import { test, expect } from '@playwright/test';
import { mkdirSync } from 'node:fs';
import { expectNoHorizontalOverflow } from './helpers';

const routes = ['home', 'assistant', 'agents', 'history', 'run-with-otto', 'mission-control', 'swarm', 'loops', 'workflows', 'scheduled-tasks', 'personal-agents', 'git', 'proof', 'product', 'vault', 'canvas', 'design', 'design/brand', 'design/learned', 'browser', 'connections', 'database', 'brokers', 'api', 'aws', 'kubernetes', 'mcp', 'skills-eval', 'settings/plugins', 'insights', 'usage', 'walkthroughs', 'settings/appearance', 'settings/users', 'settings/assistant'];
const variants = [
  { name: 'native-light', theme: 'native', scheme: 'light', direction: 'ltr', width: 1440, height: 900 },
  { name: 'native-dark', theme: 'native', scheme: 'dark', direction: 'ltr', width: 1440, height: 900 },
  { name: 'warm-dark', theme: 'warm', scheme: 'dark', direction: 'ltr', width: 1440, height: 900 },
  { name: 'pro-dark', theme: 'pro-dark', scheme: 'dark', direction: 'ltr', width: 1440, height: 900 },
  { name: 'warm-light-phone', theme: 'warm', scheme: 'light', direction: 'ltr', width: 375, height: 812 },
  { name: 'phone-light', theme: 'native', scheme: 'light', direction: 'ltr', width: 375, height: 812 },
  { name: 'tablet-rtl', theme: 'native', scheme: 'dark', direction: 'rtl', width: 1024, height: 768 },
];
for (const variant of variants) {
  test(`UX page inventory — ${variant.name}`, async ({ page }, testInfo) => {
    test.setTimeout(240_000);
    await page.setViewportSize({ width: variant.width, height: variant.height });
    await page.addInitScript(({ theme, scheme, direction }) => {
      localStorage.setItem('otto_theme', theme);
      localStorage.setItem('otto_scheme', scheme);
      localStorage.setItem('otto_direction', direction);
    }, variant);
    const evidence = process.env.OTTO_UX_SCREENSHOTS;
    if (evidence) mkdirSync(evidence, { recursive: true });
    for (const route of routes) {
      await test.step(route, async () => {
        await page.goto(`/#/${route}`);
        await expect(page.locator('.shell')).toBeVisible();
        await expect(page.locator('.center').first()).toBeVisible();
        await page.waitForLoadState('networkidle', { timeout: 3_000 }).catch(() => {});
        await expectNoHorizontalOverflow(page);
        if (evidence) {
          // Capture after fonts settle; these snapshots support visual review,
          // while interaction/state behavior has focused regression specs.
          await page.evaluate(() => document.fonts.ready);
          await page.screenshot({ path: `${evidence}/${variant.name}-${route.replaceAll('/', '-')}.png` });
        }
      });
    }
    await testInfo.attach('reviewed-variants', { body: JSON.stringify({ variant, routes }), contentType: 'application/json' });
  });
}
