import { test, expect } from '@playwright/test';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openPage } from './helpers';
import { apiCtx } from './seed';

// All-workspaces sidebar view + workspace management:
//  - the Agents inbox's "All workspaces" scope lists sessions from OTHER
//    workspaces (each row names its workspace); clicking one switches + opens it
//  - workspace rows gain Rename… / Change folder… / Delete… context actions.
// Desktop-only: the expanded Navigator is the desktop surface.

// Unique suffix so parallel runs / leftovers never collide on names.
const RUN = Math.random().toString(36).slice(2, 7);
const WS_A = `AlphaWs ${RUN}`;
const WS_B = `BetaWs ${RUN}`;
const SESS_B = `BetaSess ${RUN}`;

let wsAId = '';
let wsBId = '';

test.describe('all-workspaces sidebar + workspace management', () => {
  test.beforeEach(async ({ page }, testInfo) => {
    test.skip(
      testInfo.project.name !== 'desktop-browser',
      'expanded Navigator is the desktop surface',
    );

    // Seed: two workspaces; a shell session in B. The UI will sit on A, so B's
    // session may only appear via the all-workspaces view.
    const { ctx, base } = await apiCtx();
    const mk = async (name: string) => {
      const r = await ctx.post(`${base}/api/v1/workspaces`, {
        data: { name, root_path: mkdtempSync(join(tmpdir(), 'otto-wsnav-')) },
      });
      expect(r.ok(), `create workspace → ${r.status()}`).toBeTruthy();
      return (await r.json()).id as string;
    };
    wsAId = wsAId || (await mk(WS_A));
    wsBId = wsBId || (await mk(WS_B));
    const have = await (
      await ctx.get(`${base}/api/v1/workspaces/${wsBId}/sessions`)
    ).json();
    if (!have.some((s: { title: string }) => s.title === SESS_B)) {
      const r = await ctx.post(`${base}/api/v1/workspaces/${wsBId}/sessions`, {
        data: { kind: 'agent', provider: 'shell', title: SESS_B, meta: {} },
      });
      expect(r.ok(), `create session → ${r.status()}`).toBeTruthy();
    }

    // Before the first load: a boot still resolving its workspace must not
    // write its own choice over A's.
    await page.addInitScript(
      ([a]) => {
        if (sessionStorage.getItem('e2e_seeded')) return;
        sessionStorage.setItem('e2e_seeded', '1');
        localStorage.setItem('otto_rail_expanded', '1');
        localStorage.setItem('otto_agents_fold_nav', '0'); // the Workspaces rows live in the expanded navigator
        localStorage.setItem('otto_workspace', a); // sit on workspace A
        localStorage.removeItem('otto_nav_all_ws'); // default = all workspaces ON
      },
      [wsAId],
    );
    await openPage(page, 'agents');
    await expect(page.locator('.navigator')).toBeVisible({ timeout: 15_000 });
  });

  test('sessions from other workspaces show in the inbox; click switches + opens', async ({ page }) => {
    const nav = page.locator('.navigator');
    const inbox = page.getByTestId('agents-inbox');

    // The row names workspace B on its first line.
    const row = inbox.locator('[data-testid^="agents-row-"] .tr-main', { hasText: SESS_B });
    await expect(row).toBeVisible({ timeout: 15_000 });
    await expect(row).toContainText(WS_B);

    // Clicking the foreign row switches the workspace and focuses the session.
    await row.click();
    // Workspace B is now current (checkmark on its row) …
    await expect(
      nav.locator('.nav-item.active-ws', { hasText: WS_B }),
    ).toBeVisible({ timeout: 15_000 });
    // … and the session is the active row.
    await expect(inbox.locator('[data-testid^="agents-row-"].selected', { hasText: SESS_B })).toBeVisible({ timeout: 15_000 });

    // "This workspace" → workspace A's rows only, and the choice persists.
    await page.getByTestId('agents-sort-toggle').click();
    await page.locator('.ctx-item', { hasText: 'This workspace' }).click();
    await page.getByRole('button', { name: /Current workspace/ }).click();
    await page.locator('.ctx-item', { hasText: WS_A }).first().click();
    await expect(inbox.locator('[data-testid^="agents-row-"]', { hasText: SESS_B })).toHaveCount(0);
    expect(await page.evaluate(() => localStorage.getItem('otto_nav_all_ws'))).toBe('0');
  });

  test('right-click a workspace under Agents → new session there', async ({ page }) => {
    // The current-workspace chip offers it, for the workspace it names.
    await page.getByTestId('agents-current-ws').click({ button: 'right' });
    await expect(page.getByRole('menuitem', { name: `New session in “${WS_A}”…` })).toBeVisible();
    await expect(page.getByRole('menuitem', { name: 'Copy folder path' })).toBeVisible();
    await page.keyboard.press('Escape');

    // An other workspace's row: switch there and open the New session sheet.
    const row = page.locator('[data-testid^="agents-row-"] .tr-main', { hasText: SESS_B });
    await expect(row).toBeVisible({ timeout: 15_000 });
    await row.click({ button: 'right' });
    await page.getByRole('menuitem', { name: new RegExp(`^New session in “${WS_B}`) }).click();
    await expect(page.getByRole('dialog', { name: 'New session' })).toBeVisible({ timeout: 15_000 });
    await expect(page.getByTestId('agents-current-ws')).toContainText(WS_B);
  });

  test('workspace context menu: rename and delete', async ({ page }) => {
    const nav = page.locator('.navigator');
    const bRow = nav.locator('.nav-item', { hasText: WS_B }).first();
    await expect(bRow).toBeVisible({ timeout: 15_000 });

    // Rename via the context menu → prompt dialog.
    await bRow.click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Rename…' }).click();
    const input = page.locator('.cf-input');
    await expect(input).toBeVisible();
    await input.fill(`${WS_B} renamed`);
    await page.getByRole('button', { name: 'Rename' }).click();
    await expect(nav.locator('.nav-item', { hasText: `${WS_B} renamed` })).toBeVisible();

    // Remove (archive — the folder is untouched) via the context menu → confirm dialog.
    await nav.locator('.nav-item', { hasText: `${WS_B} renamed` }).first()
      .click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Remove workspace…' }).click();
    await page.getByRole('button', { name: 'Remove', exact: true }).click();
    await expect(nav.locator('.nav-item', { hasText: `${WS_B} renamed` })).toHaveCount(0);
  });
});
