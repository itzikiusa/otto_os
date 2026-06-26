import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

// Review cancel — E2E against the isolated test daemon. Seeds a RUNNING review
// via the `__e2e` endpoint, cancels it through the real endpoint, and asserts the
// terminal `cancelled` status plus the 409-when-not-running guard. Daemon state
// is global → pin the whole file to one device project.

test.describe.configure({ mode: 'serial' });
test.beforeEach(({}, testInfo) => {
  test.skip(
    testInfo.project.name !== 'iphone-portrait',
    'review state is global to the daemon; run on a single project only',
  );
});

let ctx: APIRequestContext;
let base = '';
let ws = '';
let repoId = '';
const api = (p: string) => `${base}/api/v1${p}`;

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  ws = await seedWorkspace(ctx, base);
  ({ repoId } = await seedGitRepo(ctx, base, ws));
});

test.afterAll(async () => {
  await ctx.dispose();
});

test('cancel transitions a running review to cancelled, then 409s', async () => {
  const seed = await ctx.post(api(`/workspaces/${ws}/__e2e/review`), {
    data: { repo_id: repoId, pr_number: 0 },
  });
  expect(seed.ok()).toBeTruthy();
  const rev = await seed.json();
  expect(rev.status).toBe('running');

  const cancel = await ctx.post(api(`/reviews/${rev.id}/cancel`));
  expect(cancel.ok()).toBeTruthy();
  const cancelled = await cancel.json();
  expect(cancelled.status).toBe('cancelled');

  // A second cancel on a no-longer-running review is a 409 conflict.
  const second = await ctx.post(api(`/reviews/${rev.id}/cancel`));
  expect(second.status()).toBe(409);
});
