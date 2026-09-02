import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { openPage } from './helpers';

// Browser ⇄ agent embedding:
//  - the Browser page hosts an agent dock (attach/start a session, its
//    terminal, and an ask bar that submits page + marks + question);
//  - the agent-mode right panel's Browser tab carries a v1/v2 switch, and v2
//    embeds the Browser module with an ask bar targeting the active session.
//
// The reader-mode page fetch is mocked in the browser context (see
// desktop-browser-reader.spec.ts for why — the daemon's fetch is netguard-
// checked and never pointed at a local fixture). Session creation, marks, and
// the `/browser/ask` POST are REAL calls against the isolated test daemon;
// the ask target is a `shell` provider session (a `cat` PTY), so the block
// the daemon writes echoes back in its scrollback.

const FIXTURE_URL = 'https://example.invalid/agent-fixture';

test.describe.configure({ mode: 'serial' });

let ctx: APIRequestContext;
let base: string;
let wsId = '';
let shellId = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base);
  shellId = await seedShellSession(ctx, base, wsId);

  await page.route('**/browser/page?url=**', (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        url: FIXTURE_URL,
        title: 'Agent Fixture',
        markdown: 'Fixture body for the browser-agent E2E.',
        html: '<p>Fixture body for the browser-agent E2E.</p>',
        engine: 'mock',
        degraded: false,
      }),
    }),
  );

  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_right_open', '1');
  }, wsId);
});

test.afterEach(async () => {
  await ctx?.dispose();
});

async function openFixture(page: Page): Promise<void> {
  await page.getByPlaceholder('Enter URL').fill(FIXTURE_URL);
  await page.getByTitle('Go').click();
  await expect(page.locator('.reader h1')).toHaveText('Agent Fixture', { timeout: 15_000 });
}

async function markHeading(page: Page, note: string): Promise<void> {
  await page.getByRole('button', { name: 'Mark element' }).click();
  await page.locator('.reader h1').click();
  await page.getByPlaceholder('Add a note').fill(note);
  await page.getByRole('button', { name: 'Save mark' }).click();
  await expect(page.locator('.notes-rail')).toContainText(note);
}

test('browser page: dock starts detached, attaches a session, ask sends page + marks', async ({ page }) => {
  await page.goto('/#/browser');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });

  const dock = page.getByLabel('Browser agent');
  await expect(dock).toContainText('No session attached');
  // Detached → the ask bar is disabled with a hint, never a dead input.
  const ask = page.getByLabel('Ask the agent about this page');
  await expect(ask).toBeDisabled();
  await expect(ask).toHaveAttribute('placeholder', /Attach or start an agent/);

  // Attach the seeded shell session via the picker (the shared ctxMenu).
  await dock.getByRole('button', { name: 'Attach…' }).click();
  await page.getByRole('menuitem', { name: /E2E Shell/ }).click();
  await expect(dock).toContainText('E2E Shell');
  // Its terminal mounts inside the dock.
  await expect(dock.locator('.xterm')).toBeVisible({ timeout: 15_000 });

  // The binding survives a reload (persisted per workspace).
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByLabel('Browser agent')).toContainText('E2E Shell');

  await openFixture(page);
  await markHeading(page, 'this heading');

  // A fresh mark nudges the ask bar: focused, with the "just marked" hint,
  // and the marks chip counts it.
  await expect(ask).toBeFocused();
  await expect(ask).toHaveAttribute('placeholder', /element you just marked/);
  await expect(page.locator('.askbar .chip')).toContainText('1 mark');

  const askReq = page.waitForRequest(
    (req) => req.url().includes('/browser/ask') && req.method() === 'POST',
  );
  await ask.fill('what does the marked element do?');
  await ask.press('Enter');
  const req = await askReq;
  const body = req.postDataJSON() as { session_id: string; url: string; text: string; annotation_ids: string[] };
  expect(body.session_id).toBe(shellId);
  expect(body.url).toBe(FIXTURE_URL);
  expect(body.text).toBe('what does the marked element do?');
  expect(body.annotation_ids).toHaveLength(1);
  const resp = await req.response();
  expect(resp?.ok()).toBeTruthy();
  // Sent → cleared.
  await expect(ask).toHaveValue('');

  // The daemon wrote the fenced block + question into the session — the
  // shell echoes it back, so it's visible in the embedded terminal. Assert on
  // the TAIL of the block (the head scrolls out of xterm's rendered rows).
  await expect(dock.locator('.xterm')).toContainText('Question from user', { timeout: 15_000 });
  await expect(dock.locator('.xterm')).toContainText('what does the marked element do?');
  await expect(dock.locator('.xterm')).toContainText('Selector: h1');

  // Detach → back to the empty state; the session itself is untouched.
  await dock.getByRole('button', { name: 'Detach session' }).click();
  await expect(dock).toContainText('No session attached');
  const still = await ctx.get(`${base}/api/v1/sessions/${shellId}`);
  expect(still.ok()).toBeTruthy();
});

test('browser page: "New agent" creates a session bound to the dock without leaving the page', async ({ page }) => {
  await page.goto('/#/browser');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  const dock = page.getByLabel('Browser agent');

  // The default provider is whatever the daemon reports; the test daemon's
  // shell provider is what a fresh install falls back to when no agent CLI
  // is installed, so this stays hermetic either way.
  const createReq = page.waitForRequest(
    (req) => req.url().endsWith(`/workspaces/${wsId}/sessions`) && req.method() === 'POST',
  );
  await dock.getByRole('button', { name: 'New agent' }).click();
  const req = await createReq;
  const body = req.postDataJSON() as { kind: string; title: string; meta: Record<string, unknown> };
  expect(body.kind).toBe('agent');
  expect(body.title).toBe('Browser agent');
  expect(body.meta.origin).toBe('browser');
  expect(body.meta.browser).toBe(true);
  const resp = await req.response();
  if (resp?.ok()) {
    await expect(dock).toContainText('Browser agent');
    // Still on the Browser page — quiet create never routes to the session.
    await expect(page).toHaveURL(/#\/browser/);
  } else {
    // No spawnable provider in this environment: surfaced as a toast, dock
    // stays detached rather than binding a phantom session.
    await expect(page.locator('.toast, [role="alert"]').first()).toBeVisible();
    await expect(dock).toContainText('No session attached');
  }
});

test('agent mode: right panel Browser tab has a v1/v2 switch and v2 embeds the module', async ({ page }) => {
  await openPage(page, 'agents');
  await page.getByRole('button', { name: /E2E Shell/ }).first().click();
  const panel = page.locator('.rpanel');
  await expect(panel).toBeVisible();
  await panel.getByRole('tab', { name: 'Browser', exact: true }).click();

  const group = panel.getByRole('group', { name: 'Browser version' });
  await expect(group).toBeVisible();
  // v1 is the default — the pre-existing per-session panel (its take-over
  // toolbar), no Browser-module chrome.
  await expect(group.getByRole('button', { name: 'v1' })).toHaveAttribute('aria-pressed', 'true');
  await expect(panel.getByPlaceholder('Search or enter URL…')).toBeVisible();
  await expect(panel.getByPlaceholder('Enter URL', { exact: true })).toHaveCount(0);

  await group.getByRole('button', { name: 'v2' }).click();
  await expect(group.getByRole('button', { name: 'v2' })).toHaveAttribute('aria-pressed', 'true');
  // v2: the Browser module's URL bar + an ask bar aimed at the active session.
  await expect(panel.getByPlaceholder('Enter URL', { exact: true })).toBeVisible();
  const ask = panel.getByLabel('Ask the agent about this page');
  await expect(ask).toBeVisible();
  // No page open yet → disabled with the "open a page" hint, not the
  // "attach a session" one (the session is the pane beside it).
  await expect(ask).toHaveAttribute('placeholder', /Open a page first/);
  // No embedded dock inside the panel — the session IS the main pane.
  await expect(panel.getByLabel('Browser agent')).toHaveCount(0);

  // The choice persists across a reload (the panel's active tab itself does
  // not — re-open Browser and the switch comes back on v2).
  await page.reload();
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  await page.getByRole('button', { name: /E2E Shell/ }).first().click();
  await page.locator('.rpanel').getByRole('tab', { name: 'Browser', exact: true }).click();
  await expect(page.locator('.rpanel').getByRole('button', { name: 'v2' })).toHaveAttribute('aria-pressed', 'true');

  // Back to v1 restores the original panel.
  await page.locator('.rpanel').getByRole('button', { name: 'v1' }).click();
  await expect(page.locator('.rpanel').getByPlaceholder('Search or enter URL…')).toBeVisible();
});
