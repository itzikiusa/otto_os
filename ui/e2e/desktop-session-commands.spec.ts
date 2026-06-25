import { test, expect, type APIRequestContext } from '@playwright/test';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// Desktop BROWSER: ⌘I close commands + ⌘T New-Session keyboard navigation.
// Uses explicit session titles (theme-agnostic) so it never races the mobile
// session-names spec on the shared per-user active theme. Real keyboard + no
// drawer make the palette/dialog flows reliable at desktop width.
//
// Only meaningful on the desktop-browser project; self-skips on mobile/tablet.

let ctx: APIRequestContext;
let base: string;
let wsId = '';
const SHELLS = ['Zlatan', 'Pirlo', 'Buffon'];
const idByTitle: Record<string, string> = {};

async function freshWorkspaceWithShells(): Promise<void> {
  wsId = await seedWorkspace(ctx, base);
  for (const title of SHELLS) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
      data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
    });
    if (!r.ok()) throw new Error(`seed ${title} → ${r.status()} ${await r.text()}`);
    idByTitle[title] = (await r.json()).id as string;
  }
}

async function isArchived(id: string): Promise<boolean> {
  const r = await ctx.get(`${base}/api/v1/sessions/${id}`);
  if (!r.ok()) return true; // gone (killed) counts as closed
  return ((await r.json()) as { archived?: boolean }).archived === true;
}

async function sessionExists(id: string): Promise<boolean> {
  return (await ctx.get(`${base}/api/v1/sessions/${id}`)).ok();
}

/** Open the ⌘I plain-English palette and submit `text`. */
async function runOttoCommand(page: import('@playwright/test').Page, text: string): Promise<void> {
  await page.keyboard.press('Meta+i');
  const box = page.locator('.pal-english textarea');
  await expect(box).toBeVisible({ timeout: 10_000 });
  await box.fill(text);
  await page.keyboard.press('Meta+Enter');
  // The palette closes itself on a handled command.
  await expect(box).toBeHidden({ timeout: 10_000 });
}

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  await freshWorkspaceWithShells();
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
  await page.goto('/#/agents');
  // Wait until the seeded sessions are loaded into the store (names visible).
  await expect(page.getByText('Zlatan').first()).toBeVisible({ timeout: 20_000 });
});

test.afterEach(async () => {
  await ctx?.dispose();
});

test('⌘I: close a session by NAME, then close ALL of a provider', async ({ page }) => {
  // "close zlatan" → only that session is archived.
  await runOttoCommand(page, 'please close zlatan');
  await expect.poll(() => isArchived(idByTitle.Zlatan), { timeout: 15_000 }).toBe(true);
  // The others are untouched.
  expect(await isArchived(idByTitle.Pirlo)).toBe(false);
  expect(await isArchived(idByTitle.Buffon)).toBe(false);

  // "close all shell sessions" → every remaining shell is archived.
  await runOttoCommand(page, 'please close all shell sessions');
  await expect.poll(() => isArchived(idByTitle.Pirlo), { timeout: 15_000 }).toBe(true);
  await expect.poll(() => isArchived(idByTitle.Buffon), { timeout: 15_000 }).toBe(true);
});

test('⌘I: "delete <name>" removes the session permanently (not just archive)', async ({ page }) => {
  await runOttoCommand(page, 'please delete pirlo');
  // Gone entirely — a GET returns 404, unlike archive which keeps the row.
  await expect.poll(() => sessionExists(idByTitle.Pirlo), { timeout: 15_000 }).toBe(false);
  // The others are untouched.
  expect(await sessionExists(idByTitle.Zlatan)).toBe(true);
  expect(await sessionExists(idByTitle.Buffon)).toBe(true);
});

test('⌘I: "send to <name/session N> <msg>" delivers to the right session', async ({ page }) => {
  // Open Zlatan as pane #1 so a positional address ("session 1") resolves.
  await page.goto(`/#/agents/${idByTitle.Zlatan}`);
  await expect(page.locator('.term-host')).toBeVisible({ timeout: 30_000 });

  const posFile = join(tmpdir(), `otto-e2e-pos-${Date.now()}`);
  const nameFile = join(tmpdir(), `otto-e2e-name-${Date.now()}`);

  // "send to session 1 <cmd>" → the shell in pane 1 runs it (this is the exact
  // form the user reported as broken — it must NOT fall through to the AI).
  await runOttoCommand(page, `send to session 1 touch ${posFile}`);
  await expect.poll(() => existsSync(posFile), { timeout: 15_000 }).toBe(true);

  // "send to zlatan <cmd>" → addressed by name.
  await runOttoCommand(page, `send to zlatan touch ${nameFile}`);
  await expect.poll(() => existsSync(nameFile), { timeout: 15_000 }).toBe(true);
});

test('⌘T: arrow keys switch provider, Tab moves to the next field', async ({ page }) => {
  await page.keyboard.press('Meta+t');
  const group = page.locator('[role="radiogroup"]');
  await expect(group).toBeVisible({ timeout: 10_000 });

  // Focus the currently-selected provider radio.
  const checked = page.locator('[role="radio"][aria-checked="true"]');
  await checked.focus();
  const before = await checked.getAttribute('aria-label').catch(() => null);
  const beforeText = (await checked.textContent())?.trim();

  // ArrowRight moves the selection to a different provider.
  await page.keyboard.press('ArrowRight');
  const checkedAfter = page.locator('[role="radio"][aria-checked="true"]');
  await expect(checkedAfter).toHaveCount(1);
  const afterText = (await checkedAfter.textContent())?.trim();
  expect(afterText).not.toBe(beforeText);
  void before;

  // Tab leaves the radiogroup and lands on the Title field (roving tabindex).
  await page.keyboard.press('Tab');
  await expect(page.locator('#ns-title')).toBeFocused();

  await page.keyboard.press('Escape');
});

test('shell reconnect: an exited shell offers Reconnect and comes back live', async ({ page }) => {
  const id = idByTitle.Buffon;
  await page.goto(`/#/agents/${id}`);
  await expect(page.locator('.term-host')).toBeVisible({ timeout: 30_000 });

  // Make the shell exit, then wait for the exited overlay.
  await ctx.post(`${base}/api/v1/sessions/${id}/input`, { data: { text: 'exit', submit: true } });
  const overlay = page.locator('.term-overlay');
  await expect(overlay).toBeVisible({ timeout: 20_000 });

  // A plain shell isn't WS-resumable, so the overlay offers "Reconnect"
  // (respawn) rather than "Resume". Clicking it brings the session back live.
  const reconnect = page.getByRole('button', { name: 'Reconnect' });
  await expect(reconnect).toBeVisible({ timeout: 10_000 });
  await reconnect.click();

  await expect
    .poll(
      async () => {
        const r = await ctx.get(`${base}/api/v1/sessions/${id}`);
        return r.ok() ? ((await r.json()) as { status?: string }).status : 'gone';
      },
      { timeout: 20_000 },
    )
    .toMatch(/running|working|idle/);
  // Overlay clears once reconnected.
  await expect(overlay).toBeHidden({ timeout: 10_000 });
});

test('shell terminal fits its pane (no stale-narrow PTY width)', async ({ page }) => {
  await page.goto(`/#/agents/${idByTitle.Zlatan}`);
  const host = page.locator('.term-host');
  await expect(host).toBeVisible({ timeout: 30_000 });
  // After the connect-time fit, the grid must reflect the wide desktop pane —
  // the wrapping bug pinned shells to ~46 cols regardless of width.
  await expect
    .poll(() => host.getAttribute('data-cols').then((v: string | null) => Number(v ?? '0')), {
      timeout: 15_000,
    })
    .toBeGreaterThan(100);
});
