import { test, expect, type Page, type Route } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Workbench → Send to → Terminal / agent session must PASTE, never run.
// A file an agent wrote can carry `ESC[201~` (ends a bracketed paste early) and
// `\r` (Enter). The session list is answered by `page.route` with one agent
// session and one connection session (redis-cli); `POST /sessions/{id}/input`
// is captured, never forwarded — no PTY is involved.
// ─────────────────────────────────────────────────────────────────────────────

const suffix = Math.random().toString(36).slice(2, 8);
const ESC = '\u001b';
let workspaceId = '';
let base = '';
let token = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  base = c.base;
  token = c.token;
  workspaceId = await seedWorkspace(c.ctx, c.base);
  await c.ctx.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'Desktop flow');
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

function fakeSession(id: string, kind: 'agent' | 'connection', provider: string, title: string) {
  const now = new Date().toISOString();
  return {
    id, workspace_id: workspaceId, kind, provider, title, status: 'idle', cwd: '/tmp',
    provider_session_id: null, connection_id: kind === 'connection' ? 'conn-x' : null,
    created_by: 'u', created_at: now, last_active_at: now, archived: false, meta: {}, live: true,
  };
}

async function setup(page: Page, content: string): Promise<{ inputs: { id: string; body: any }[]; name: string }> {
  const agent = fakeSession(`paste-agent-${suffix}`, 'agent', 'claude', `Agent ${suffix}`);
  const conn = fakeSession(`paste-conn-${suffix}`, 'connection', 'redis', `Prod redis ${suffix}`);
  await page.route(/\/api\/v1\/workspaces\/[^/]+\/sessions(\?.*)?$/, (route: Route) =>
    route.request().method() === 'GET'
      ? route.fulfill({ json: route.request().url().includes(workspaceId) ? [agent, conn] : [] })
      : route.fallback(),
  );
  const inputs: { id: string; body: any }[] = [];
  await page.route(/\/api\/v1\/sessions\/paste-[^/]+\/input$/, (route: Route) => {
    const id = route.request().url().split('/sessions/')[1]!.split('/')[0]!;
    inputs.push({ id, body: route.request().postDataJSON() });
    return route.fulfill({ status: 200, body: '' });
  });
  const name = `paste-${suffix}-${Math.random().toString(36).slice(2, 6)}.sql`;
  const r = await page.request.fetch(`${base}/api/v1/workspaces/${workspaceId}/workbench/docs`, {
    method: 'POST',
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    data: JSON.stringify({ name, language: 'sql', content }),
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  await page.goto('/#/workbench');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  await page.getByTestId('wb-file-row').filter({ hasText: name }).first().click();
  await expect(page.getByTestId('wb-editor').locator('.cm-content')).toContainText('SELECT');
  return { inputs, name };
}

async function pickSession(page: Page, title: RegExp): Promise<void> {
  await page.getByTestId('wb-sendto').click();
  await page.getByRole('menuitem', { name: /Terminal \/ agent session/ }).click();
  const picker = page.getByTestId('wb-session-picker');
  await expect(picker).toBeVisible();
  await picker.getByRole('button', { name: title }).click();
}

test('an embedded paste-end sequence cannot break out of the paste', async ({ page }) => {
  const { inputs } = await setup(page, `SELECT 1;\n${ESC}[201~DROP TABLE users;\r\nSELECT 2;`);
  await pickSession(page, new RegExp(`Agent ${suffix}`));
  const dlg = page.locator('.sheet[role="dialog"]').last();
  await expect(dlg).toContainText('1 control character');
  await expect(dlg).toContainText('bracketed paste with no Enter');
  await dlg.getByRole('button', { name: 'Paste', exact: true }).click();
  await expect.poll(() => inputs.length).toBe(1);
  const { body } = inputs[0]!;
  expect(body.submit).toBe(false);
  const text: string = body.text;
  expect(text.startsWith(`${ESC}[200~`) && text.endsWith(`${ESC}[201~`)).toBe(true);
  const inner = text.slice(6, -6);
  expect(inner).not.toContain(ESC);
  expect(inner).not.toContain('\r');
  expect(inner).toBe('SELECT 1;\n[201~DROP TABLE users;\nSELECT 2;');
});

test('a multi-line file is refused for a connection session, with Copy offered', async ({ page }) => {
  const { inputs } = await setup(page, 'SELECT 1;\nSELECT 2;\n');
  await page.getByTestId('wb-sendto').click();
  await page.getByRole('menuitem', { name: /Terminal \/ agent session/ }).click();
  const picker = page.getByTestId('wb-session-picker');
  await expect(picker.getByRole('button', { name: new RegExp(`Prod redis ${suffix}`) })).toContainText('one line only — not this file');
  await picker.getByRole('button', { name: new RegExp(`Prod redis ${suffix}`) }).click();
  const toast = page.locator('.toast').filter({ hasText: 'connection sessions take one line' });
  await expect(toast).toBeVisible();
  await expect(toast.getByRole('button', { name: 'Copy' })).toBeVisible();
  await expect(page.locator('.sheet[role="dialog"]')).toHaveCount(0);
  expect(inputs.length).toBe(0);
});
