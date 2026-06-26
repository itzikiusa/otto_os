import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultNotes } from './seed';

// Vault embedder — E2E against the isolated test daemon. Asserts the embedder
// status defaults to the local stub, the live PUT switch + per-workspace reindex
// work, and the Vault UI surfaces the embedder pill. Daemon state is global →
// pin the whole file to one device project.

test.describe.configure({ mode: 'serial' });
test.beforeEach(({}, testInfo) => {
  test.skip(
    testInfo.project.name !== 'iphone-portrait',
    'embedder state is global to the daemon; run on a single project only',
  );
});

let ctx: APIRequestContext;
let base = '';
let ws = '';
const api = (p: string) => `${base}/api/v1${p}`;

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  ws = await seedWorkspace(ctx, base);
  await seedVaultNotes(ctx, base, ws);
});

test.afterAll(async () => {
  // Leave the daemon on the default stub embedder.
  await ctx.put(api('/memory/embedder'), { data: { provider: 'stub' } });
  await ctx.dispose();
});

test('embedder status defaults to the local stub; PUT + reindex work', async () => {
  const status = await (await ctx.get(api('/memory/embedder'))).json();
  expect(status.provider).toBe('stub');
  expect(status.active).toBe(true);
  expect(status.model).toBe('stub-v1');

  // Switching to the stub is idempotent and needs no key — exercises the PUT.
  const put = await ctx.put(api('/memory/embedder'), { data: { provider: 'stub' } });
  expect(put.ok()).toBeTruthy();

  // Reindex the seeded workspace under the active embedder (idempotent).
  const re = await ctx.post(api(`/workspaces/${ws}/memory/reindex`), { data: {} });
  expect(re.ok()).toBeTruthy();
  const body = await re.json();
  expect(typeof body.embedded).toBe('number');
});

test('vault surfaces the embedder status pill', async ({ page }) => {
  await page.addInitScript((w) => localStorage.setItem('otto_workspace', w as string), ws);
  await page.goto('/#/vault');
  // The pill only renders once GET /memory/embedder resolves, so being attached
  // proves the status path is wired end-to-end (the sidebar may be off-screen on
  // a phone viewport, hence attached rather than strictly visible).
  await expect(page.getByTestId('vault-embedder')).toBeAttached();
});
