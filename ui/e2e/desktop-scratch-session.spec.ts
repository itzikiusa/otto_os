import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openPage } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Workspace-less ("scratch") sessions (desktop-browser only).
//
// A session started with "No workspace" in the New Session sheet lives in the
// daemon's hidden, system-owned scratch workspace (`id: "scratch"`, root =
// the daemon $HOME): it shows under the sidebar's "No workspace" group, is
// listed by `GET /workspaces/scratch/sessions`, survives a reload, and can be
// archived like any other session. The scratch workspace itself never shows in
// `GET /workspaces` and rejects edits with 409.
// ─────────────────────────────────────────────────────────────────────────────

let ctx: APIRequestContext;
let base = '';
let wsA = '';
const TITLE = 'e2e scratch';

test.beforeAll(async () => {
  const a = await apiCtx();
  ctx = a.ctx;
  base = a.base;
  wsA = await seedWorkspace(ctx, base);
  // An existing session so the Agents page renders panes (not the first-run coach).
  await seedShellSession(ctx, base, wsA);
});

test.afterAll(async () => {
  await ctx?.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, wsA);
});

/** The sidebar "No workspace" group label + its nested list right after it. */
function scratchGroup(page: Page) {
  const label = page.locator('.navigator .ws-group-label', { hasText: 'No workspace' });
  return { label, rows: label.locator('xpath=following-sibling::*[1]').locator('.nested-item') };
}

async function scratchSessions(): Promise<{ id: string; title: string; workspace_id: string; archived: boolean }[]> {
  const r = await ctx.get(`${base}/api/v1/workspaces/scratch/sessions`);
  if (!r.ok()) throw new Error(`GET /workspaces/scratch/sessions → ${r.status()} ${await r.text()}`);
  return r.json();
}

test.describe('scratch sessions', () => {
  test('New Session → No workspace: home notice, sidebar group, API listing, reload, archive', async ({ page }) => {
    await openPage(page, 'agents');

    // Open the sheet via ⌘T; fall back to the TabBar + button if the shortcut
    // doesn't reach the app in this browser build.
    const dialog = page.locator('.sheet[role="dialog"][aria-label="New Session"]');
    await page.keyboard.press('Meta+t');
    if (!(await dialog.isVisible().catch(() => false))) {
      await page.getByTitle('New session (⌘T)').click();
    }
    await expect(dialog).toBeVisible();

    // The Workspace switch: current workspace vs no workspace.
    const noWs = dialog.getByRole('radio', { name: 'No workspace' });
    await expect(noWs).toHaveAttribute('aria-checked', 'false');
    await noWs.click();
    await expect(noWs).toHaveAttribute('aria-checked', 'true');

    // Scratch mode defaults the cwd to the daemon's home folder and says what
    // that means for trust/sandbox (the notice keys on the cwd being ~).
    const scratchWs = await (await ctx.get(`${base}/api/v1/workspaces/scratch`)).json();
    await expect(dialog.locator('#ns-cwd')).toHaveValue(scratchWs.root_path);
    await expect(dialog.locator('.home-notice')).toContainText('trusted for, and may write anywhere under, ~');

    // Pick the plain shell (no external CLI needed in the throwaway daemon).
    await dialog.locator('.provider-card', { hasText: 'shell' }).locator('.card-main').click();
    await dialog.locator('#ns-title').fill(TITLE);
    await dialog.getByRole('button', { name: 'Start Session' }).click();
    await expect(dialog).toHaveCount(0, { timeout: 10_000 });

    // Sidebar: the "No workspace" group lists it (not the flat Agents list).
    const group = scratchGroup(page);
    await expect(group.label).toBeVisible({ timeout: 10_000 });
    await expect(group.rows.filter({ hasText: TITLE })).toHaveCount(1);

    // API: it belongs to the hidden scratch workspace.
    const created = (await scratchSessions()).find((s) => s.title === TITLE);
    expect(created?.workspace_id).toBe('scratch');

    // Reload → still listed under the group (loaded beside the workspace's).
    await page.reload();
    await expect(scratchGroup(page).rows.filter({ hasText: TITLE })).toHaveCount(1, { timeout: 20_000 });

    // Archive via the row menu → leaves the group, lands in Archived.
    await scratchGroup(page).rows.filter({ hasText: TITLE }).first().click({ button: 'right' });
    await expect(page.locator('.ctx-menu')).toBeVisible();
    await page.locator('.ctx-item', { hasText: /^Archive$/ }).first().click();
    await expect(scratchGroup(page).rows.filter({ hasText: TITLE })).toHaveCount(0, { timeout: 10_000 });
    await expect.poll(
      async () => (await scratchSessions()).find((s) => s.id === created!.id)?.archived,
      { timeout: 15_000 },
    ).toBe(true);
    await page.getByRole('button', { name: /^Archived/ }).click();
    await expect(page.locator('.navigator .nested-item.archived', { hasText: TITLE })).toBeVisible();
  });

  test('the scratch workspace is hidden from GET /workspaces and system-owned (409 on edit)', async () => {
    const list = await (await ctx.get(`${base}/api/v1/workspaces`)).json();
    expect((list as { id: string }[]).some((w) => w.id === 'scratch')).toBe(false);

    const scratch = await ctx.get(`${base}/api/v1/workspaces/scratch`);
    expect(scratch.ok()).toBe(true);
    expect((await scratch.json()).id).toBe('scratch');

    const patch = await ctx.patch(`${base}/api/v1/workspaces/scratch`, { data: { name: 'renamed' } });
    expect(patch.status()).toBe(409);
    const del = await ctx.delete(`${base}/api/v1/workspaces/scratch`);
    expect(del.status()).toBe(409);
    const members = await ctx.put(`${base}/api/v1/workspaces/scratch/members`, { data: { members: [] } });
    expect(members.status()).toBe(409);
  });
});
