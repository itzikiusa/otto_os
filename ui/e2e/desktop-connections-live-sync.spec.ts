import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// connections_changed on a real daemon: two windows on one isolated daemon.
// Page A creates, renames and deletes a connection through the UI; page B's
// list shows each change without a reload, and only because of the event —
// one list read per change, nothing in between (no polling).
test.use({ serviceWorkers: 'block' });
test.setTimeout(90_000);

let wsId = '';
const anchor = `live-sync-anchor-${Math.random().toString(36).slice(2, 8)}`;
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  wsId = await seedWorkspace(ctx, base);
  // One profile so the list (not its first-run empty state) is on screen.
  await ctx.post(`${base}/api/v1/workspaces/${wsId}/connections`, { data: { name: anchor, kind: 'mysql', params: { host: '127.0.0.1', port: 9 } } });
  await ctx.dispose();
});

async function openList(page: Page): Promise<void> {
  await page.addInitScript((w) => { localStorage.setItem('otto_workspace', w as string); localStorage.setItem('otto_firstrun_dismissed', '1'); }, wsId);
  await page.goto('/#/connections');
  await expect(page.locator('.conn-row', { hasText: anchor })).toBeVisible();
}

test('create, rename and delete in one window show up in the other without a reload or polling', async ({ browser }) => {
  const ctxB = await browser.newContext({ storageState: test.info().project.use.storageState as string });
  const a = await (await browser.newContext({ storageState: test.info().project.use.storageState as string })).newPage();
  const b = await ctxB.newPage();
  await openList(a);
  // Reads of the list in B, and the events that invalidate it that B received
  // (connections_changed, resource_access_changed)
  // (profiles are global: another spec on this daemon may change one too, so
  // the bar is "every read follows an event", not a fixed count).
  let listReads = 0;
  let events = 0;
  b.on('request', (r) => { if (new URL(r.url()).pathname.endsWith(`/workspaces/${wsId}/connections`) && r.method() === 'GET') listReads += 1; });
  b.on('websocket', (s) => s.on('framereceived', (f) => { const p = String(f.payload); if (p.includes('connections_changed') || p.includes('resource_access_changed')) events += 1; }));
  await openList(b);
  // From here on, every list read in B must follow an event.
  const base = { reads: listReads, events };
  const reads = () => listReads - base.reads;
  const evs = () => events - base.events;
  const name = `live-sync-${Math.random().toString(36).slice(2, 8)}`;
  const rowB = (n: string) => b.locator('.conn-row', { hasText: n });
  // Nothing happens while nothing changes (no polling).
  await b.waitForTimeout(3000); // ui-guards: allow — proves no list read without a change
  expect(reads()).toBeLessThanOrEqual(evs());

  // Create in A.
  await a.getByRole('button', { name: /New connection/ }).first().click();
  await a.getByRole('menuitem', { name: /^Database…/ }).click();
  await a.getByRole('dialog').getByRole('button', { name: 'MySQL', exact: true }).click();
  await a.getByLabel('Name').fill(name);
  await a.getByLabel('Host').fill('127.0.0.1');
  await a.getByRole('button', { name: 'Create connection' }).click();
  await expect(rowB(name)).toBeVisible();
  // A new database opens straight into its workbench in A; back to the list.
  await a.goto('/#/connections');
  await expect(a.locator('.conn-row', { hasText: name })).toBeVisible();
  expect(reads()).toBeGreaterThanOrEqual(1);
  expect(reads()).toBeLessThanOrEqual(evs());

  // Rename in A.
  // Keyboard, not a pointer: other specs on this daemon add connections, and a
  // row inserted above moves this one under the pointer between aim and click.
  await a.getByRole('button', { name: `Actions for ${name}`, exact: true }).focus();
  await a.keyboard.press('Enter');
  await a.getByRole('menuitem', { name: 'Edit', exact: true }).click();
  await a.getByLabel('Name').fill(`${name}-renamed`);
  await a.getByRole('button', { name: 'Save changes' }).click();
  await expect(rowB(`${name}-renamed`)).toBeVisible();
  expect(reads()).toBeGreaterThanOrEqual(2);
  expect(reads()).toBeLessThanOrEqual(evs());

  // Delete in A (the row goes when its Undo toast does).
  await a.getByRole('button', { name: `Actions for ${name}-renamed`, exact: true }).focus();
  await a.keyboard.press('Enter');
  await a.getByRole('menuitem', { name: 'Delete', exact: true }).click();
  // The delete lands when its Undo toast goes; dismiss it rather than wait
  // on a timer that a busy machine stretches.
  const toast = a.locator('.toast', { hasText: `Deleted “${name}-renamed”` });
  await toast.getByRole('button', { name: 'Dismiss' }).click();
  await expect(rowB(`${name}-renamed`)).toHaveCount(0, { timeout: 20_000 });
  expect(reads()).toBeGreaterThanOrEqual(3);
  await b.waitForTimeout(3000); // ui-guards: allow — proves no further read without a change
  expect(reads()).toBeLessThanOrEqual(evs());
  console.log(`B: ${reads()} list reads for ${evs()} connections_changed events (load: ${base.reads} reads)`);
  await ctxB.close();
});
