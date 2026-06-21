import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Per-device SESSION ISOLATION (opt-in, client-side only). Same daemon, same
// user — but a device can choose to show only the sessions it started. We pin a
// known client id into localStorage (otto_client_id) so the test is
// deterministic, then seed two sessions:
//   - one stamped with THIS device's client id  → always visible
//   - one stamped with a DIFFERENT device id     → hidden when isolation is on
// Default OFF → both show. Toggle ON → only the this-device session shows.
// Run with --workers=1.

const THIS_DEVICE = 'e2e-this-device';
const OTHER_DEVICE = 'other-device';
const MINE_TITLE = 'E2E Mine (this device)';
const OTHER_TITLE = 'E2E Theirs (other device)';

let workspaceId = '';

/** Seed an agent session with an explicit meta (so we control meta.client_id). */
async function seedAgentSession(
  ctx: APIRequestContext,
  base: string,
  wsId: string,
  title: string,
  meta: Record<string, unknown>,
): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta },
  });
  if (!r.ok()) throw new Error(`seed session → ${r.status()} ${await r.text()}`);
  const s = await r.json();
  return s.id as string;
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  // One session from THIS device, one from a different device — both on the
  // daemon, both in the same workspace, same user.
  await seedAgentSession(ctx, base, workspaceId, MINE_TITLE, {
    origin: 'manual',
    client_id: THIS_DEVICE,
  });
  await seedAgentSession(ctx, base, workspaceId, OTHER_TITLE, {
    origin: 'manual',
    client_id: OTHER_DEVICE,
  });
  await ctx.dispose();
});

// Pin the active workspace + this browser's client id deterministically before
// any page script runs. `isolation` is parameterized per test.
function pin(isolation: boolean) {
  return async ({ page }: { page: import('@playwright/test').Page }) => {
    await page.addInitScript(
      ([wsId, clientId, iso]) => {
        localStorage.setItem('otto_workspace', wsId as string);
        localStorage.setItem('otto_client_id', clientId as string);
        localStorage.setItem('otto_session_isolation', iso as string);
      },
      [workspaceId, THIS_DEVICE, isolation ? '1' : '0'] as const,
    );
  };
}

/** Open the Navigator drawer on phone viewports (it's behind a hamburger there);
 *  on wider profiles the Navigator is already inline. Then return the locator for
 *  the agent-session rows. */
async function openSessionList(page: import('@playwright/test').Page) {
  await page.goto('/#/agents');
  const openNav = page.getByRole('button', { name: 'Open navigator' });
  if (await openNav.isVisible().catch(() => false)) {
    await openNav.click();
  }
  const list = page.locator('.nested-row .nav-item.nested-item');
  // The Agents group is the default and should render at least the rows we seeded.
  await expect.poll(() => list.count(), { timeout: 30_000 }).toBeGreaterThan(0);
  return list;
}

/** Dismiss the Navigator drawer if it's open (phone). The drawer is a modal
 *  overlay (backdrop intercepts pointer events), so it must be closed before
 *  interacting with another page. No-op on inline (wider) profiles. */
async function closeNavDrawer(page: import('@playwright/test').Page) {
  const drawer = page.locator('.drawer.left');
  if (await drawer.isVisible().catch(() => false)) {
    // The explicit ✕ is the most reliable affordance (the backdrop sliver behind
    // a wide drawer is intercepted by the panel and Escape can race the route).
    await page.getByRole('button', { name: 'Close Navigator' }).click();
    await expect(page.locator('.drawer-backdrop')).toHaveCount(0);
  }
}

test('isolation OFF: both this-device and other-device sessions show', async ({ page }) => {
  await pin(false)({ page });
  const list = await openSessionList(page);

  await expect(page.getByText(MINE_TITLE)).toBeVisible();
  await expect(page.getByText(OTHER_TITLE)).toBeVisible();

  // Exactly the two seeded sessions (the seeded workspace is otherwise empty).
  await expect.poll(() => list.count()).toBe(2);
});

test('isolation ON: only the this-device session shows', async ({ page }) => {
  await pin(true)({ page });
  const list = await openSessionList(page);

  await expect(page.getByText(MINE_TITLE)).toBeVisible();
  await expect(page.getByText(OTHER_TITLE)).toHaveCount(0);

  // The other-device session is filtered out client-side; only ours remains.
  await expect.poll(() => list.count()).toBe(1);
});

test('toggle in Settings flips isolation live (other-device session disappears)', async ({
  page,
}) => {
  // Start OFF so both are visible, then flip the Settings toggle and confirm the
  // other-device session vanishes without a manual reload.
  await pin(false)({ page });
  const list = await openSessionList(page);
  await expect.poll(() => list.count()).toBe(2);
  await closeNavDrawer(page);

  // Go to Appearance settings and turn isolation on.
  await page.goto('/#/settings/appearance');
  const toggle = page
    .locator('label.switch-row', { hasText: 'Isolate sessions to this device' })
    .locator('input[type="checkbox"]');
  await expect(toggle).toBeVisible({ timeout: 15_000 });
  await toggle.check();
  // Persisted to localStorage for this device.
  await expect
    .poll(() => page.evaluate(() => localStorage.getItem('otto_session_isolation')))
    .toBe('1');

  // Back to the agents list — only the this-device session should remain.
  const list2 = await openSessionList(page);
  await expect(page.getByText(MINE_TITLE)).toBeVisible();
  await expect(page.getByText(OTHER_TITLE)).toHaveCount(0);
  await expect.poll(() => list2.count()).toBe(1);
});
