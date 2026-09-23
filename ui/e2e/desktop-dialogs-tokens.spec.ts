import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage } from './helpers';

// Design-token + dialog correctness (UI redesign, tokens layer).
//
//  - Native confirm()/prompt() are silent no-ops in the Tauri WKWebView, so a
//    "Delete?" guarded by one never deleted anything in the desktop app. The
//    call sites now use the in-app `confirmer` dialog: prove it shows, that
//    Cancel keeps the item, that the confirm button performs the action, and
//    that no native dialog is ever raised.
//  - Light mode used to inherit dark-only fallbacks for tokens that did not
//    exist (--fg, --surface-3, --danger…): the Usage "Cost Attribution"
//    heading was near-white on grey and backup errors were not red. Assert
//    real WCAG contrast from computed styles, in both schemes.
//  - Icon buttons in the Browser address bar squashed their svg to ~7px.

const V1 = '/api/v1';
let wsId = '';

/** WCAG contrast of `sel`'s text against its composited background. */
async function contrastOf(page: Page, sel: string): Promise<number> {
  return page.evaluate((selector) => {
    type C = [number, number, number, number];
    const parse = (s: string): C | null => {
      let m = /rgba?\(([^)]+)\)/.exec(s);
      if (m) {
        const p = m[1].split(/[\s,/]+/).filter(Boolean).map(Number);
        return [p[0] / 255, p[1] / 255, p[2] / 255, p[3] ?? 1];
      }
      m = /color\(srgb ([^)]+)\)/.exec(s);
      if (m) {
        const p = m[1].split(/[\s/]+/).filter(Boolean).map(Number);
        return [p[0], p[1], p[2], p[3] ?? 1];
      }
      return null;
    };
    const el = document.querySelector(selector) as HTMLElement;
    const chain: HTMLElement[] = [];
    for (let n: HTMLElement | null = el; n; n = n.parentElement) chain.unshift(n);
    let bg = [1, 1, 1];
    for (const n of chain) {
      const c = parse(getComputedStyle(n).backgroundColor);
      if (c && c[3] > 0) bg = bg.map((v, i) => c[i] * c[3] + v * (1 - c[3]));
    }
    const fg = parse(getComputedStyle(el).color)!;
    const f = [0, 1, 2].map((i) => fg[i] * fg[3] + bg[i] * (1 - fg[3]));
    const lum = (c: number[]) => {
      const l = c.map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
      return 0.2126 * l[0] + 0.7152 * l[1] + 0.0722 * l[2];
    };
    const a = lum(f);
    const b = lum(bg);
    return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
  }, sel);
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  await ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  // Any native dialog is a regression: fail loudly instead of auto-dismissing.
  page.on('dialog', (d) => {
    throw new Error(`native ${d.type()}() raised: ${d.message()}`);
  });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
  }, wsId);
});

test('delete scheduled task: in-app confirm, Cancel keeps it, Delete removes it', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const r = await ctx.post(`${base}${V1}/workspaces/${wsId}/scheduled-tasks`, {
    data: { name: 'E2E confirm target', prompt: 'noop', schedule: { cadence: 'interval', every_min: 60 } },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const task = await r.json();

  await openPage(page, 'scheduled-tasks');
  const row = page.locator('li.task', { hasText: 'E2E confirm target' });
  await expect(row).toBeVisible({ timeout: 15_000 });

  // Cancel: the dialog closes and the task is still there.
  await row.getByRole('button', { name: 'Delete' }).click();
  const dlg = page.getByRole('dialog', { name: 'Delete scheduled task' });
  await expect(dlg).toBeVisible();
  await expect(dlg).toContainText('Delete scheduled task "E2E confirm target"?');
  await dlg.getByRole('button', { name: 'Cancel' }).click();
  await expect(dlg).toBeHidden();
  await expect(row).toBeVisible();

  // Confirm: the destructive button (danger-styled) performs the delete.
  await row.getByRole('button', { name: 'Delete' }).click();
  const confirmBtn = dlg.getByRole('button', { name: 'Delete', exact: true });
  await expect(confirmBtn).toHaveClass(/danger/);
  // White label on the solid danger fill stays readable (>= 4.5:1).
  expect(await contrastOf(page, '.sheet[role="dialog"] footer .btn.danger')).toBeGreaterThanOrEqual(4.5);
  await confirmBtn.click();
  await expect(dlg).toBeHidden();
  await expect(row).toHaveCount(0, { timeout: 15_000 });

  const list = await (await ctx.get(`${base}${V1}/workspaces/${wsId}/scheduled-tasks`)).json();
  expect((list as { id: string }[]).some((t) => t.id === task.id), 'task deleted server-side').toBe(false);
  await ctx.dispose();
});

for (const scheme of ['light', 'dark'] as const) {
  test(`${scheme}: backup error text is danger-coloured and readable`, async ({ page }) => {
    await page.addInitScript((s) => localStorage.setItem('otto_scheme', s as string), scheme);
    await openPage(page, 'settings/backup');
    const input = page.locator('.full-backup input[type="file"]');
    await input.setInputFiles({ name: 'not-an-archive.json', mimeType: 'application/json', buffer: Buffer.from('{"x":1}') });
    const err = page.locator('.full-backup .error');
    await expect(err).toContainText('Choose an Otto data archive');
    // It resolves to the --danger token (it used to inherit body text colour).
    const [color, danger, body] = await err.evaluate((el) => [
      getComputedStyle(el).color,
      getComputedStyle(document.documentElement).getPropertyValue('--danger').trim(),
      getComputedStyle(document.body).color,
    ]);
    expect(danger, '--danger is defined').not.toBe('');
    expect(color, 'error text is not plain body text').not.toBe(body);
    const probe = await page.evaluate((c) => {
      const d = document.createElement('div');
      d.style.color = c;
      document.body.append(d);
      const v = getComputedStyle(d).color;
      d.remove();
      return v;
    }, danger);
    expect(color).toBe(probe);
    expect(await contrastOf(page, '.full-backup .error')).toBeGreaterThanOrEqual(4.5);
  });
}

test('light: Usage "Cost Attribution" heading + Group-by select are readable', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  let available = false;
  for (let i = 0; i < 60 && !available; i++) {
    const r = await ctx.get(`${base}${V1}/usage/status`);
    available = r.ok() && Boolean((await r.json()).available);
    if (!available) await new Promise((res) => setTimeout(res, 500));
  }
  await ctx.dispose();
  test.skip(!available, 'no clickhouse binary on this host — the attribution panel never renders');

  test.setTimeout(240_000);
  await page.addInitScript(() => localStorage.setItem('otto_scheme', 'light'));
  // Not openPage(): the Usage page polls, so "networkidle" can take minutes on
  // a loaded host.
  await page.goto('/#/usage');
  const title = page.locator('.attr-title');
  await expect(title).toHaveText('Cost Attribution', { timeout: 30_000 });
  expect(await contrastOf(page, '.attr-title')).toBeGreaterThanOrEqual(4.5);
  // The select was a solid black box (#0d1117 fallback) in light mode.
  expect(await contrastOf(page, '#attr-dim-select')).toBeGreaterThanOrEqual(4.5);
  const selectBg = await page
    .locator('#attr-dim-select')
    .evaluate((el) => getComputedStyle(el).backgroundColor);
  expect(selectBg).not.toBe('rgb(13, 17, 23)');
});

test('browser address bar: icon buttons keep full-size icons', async ({ page }) => {
  await openPage(page, 'browser');
  const go = page.locator('button.btn[title="Go"]').first();
  await expect(go).toBeVisible({ timeout: 15_000 });
  const box = await go.locator('svg').boundingBox();
  expect(box, 'Go icon rendered').not.toBeNull();
  // 14px icon; the old layout squashed it to ~7-8px under the 11px padding.
  expect(box!.width).toBeGreaterThanOrEqual(13);
});
