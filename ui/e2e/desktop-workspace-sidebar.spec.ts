import { test, expect } from '@playwright/test';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openPage } from './helpers';
import { apiCtx } from './seed';

// All-workspaces sidebar view + workspace management:
//  - the Agents section's globe toggle lists sessions from OTHER workspaces,
//    grouped by workspace name; clicking a row switches workspace + opens it
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

    await openPage(page, 'agents');
    await page.evaluate(
      ([a]) => {
        localStorage.setItem('otto_rail_expanded', '1');
        localStorage.setItem('otto_workspace', a); // sit on workspace A
        localStorage.removeItem('otto_nav_all_ws'); // default = all workspaces ON
      },
      [wsAId],
    );
    await page.reload();
    await expect(page.locator('.navigator')).toBeVisible({ timeout: 15_000 });
  });

  test('sessions from other workspaces show grouped; click switches + opens', async ({ page }) => {
    const nav = page.locator('.navigator');

    // Group label with workspace B's name, with the session nested under it.
    const groupLabel = nav.locator('.ws-group-label', { hasText: WS_B });
    await expect(groupLabel).toBeVisible({ timeout: 15_000 });
    const row = nav.getByRole('button', { name: new RegExp(SESS_B) });
    await expect(row).toBeVisible();

    // Clicking the foreign row switches the workspace and focuses the session.
    await row.click();
    // Workspace B is now current (checkmark on its row) …
    await expect(
      nav.locator('.nav-item.active-ws', { hasText: WS_B }),
    ).toBeVisible({ timeout: 15_000 });
    // … and the session is the active tab (flat Agents list row is active).
    await expect(
      nav.locator('.nested-item.active', { hasText: SESS_B }),
    ).toBeVisible({ timeout: 15_000 });

    // Toggle OFF → foreign groups disappear and the choice persists.
    await nav.getByRole('button', { name: 'Toggle all-workspaces session list' }).click();
    await expect(nav.locator('.ws-group-label')).toHaveCount(0);
    expect(await page.evaluate(() => localStorage.getItem('otto_nav_all_ws'))).toBe('0');
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

    // Delete (archive) via the context menu → confirm dialog.
    await nav.locator('.nav-item', { hasText: `${WS_B} renamed` }).first()
      .click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Delete…' }).click();
    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    await expect(nav.locator('.nav-item', { hasText: `${WS_B} renamed` })).toHaveCount(0);
  });
});
