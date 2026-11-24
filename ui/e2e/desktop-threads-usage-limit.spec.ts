import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// A usage limit is a state with a next step (Wave 2, W2), on REAL fake-codex
// turns: `limit` fails the turn with `codexErrorInfo: "usageLimitExceeded"`
// ("Try again later." — no reset time):
//   • the sidebar row says "Limited"; the composer has ONE line
//     "Codex limit reached · reset time unknown" offering only
//     "Continue with <other provider>", which arms the switch line;
//   • a resume armed through the API (as "Resume at reset" does) shows
//     "resumes at …" with Cancel on every client, live; at that time the
//     daemon sends one short continue and the turn completes.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let dir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  dir = mkdtempSync(join(tmpdir(), 'otto-e2e-usage-limit-'));
  writeFileSync(join(dir, 'README.md'), '# limits\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Limit WS', root_path: dir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (ctx && wsId) {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string }[];
    for (const s of rows) await ctx.post(`${base}/api/v1/runtime-sessions/${s.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (dir) rmSync(dir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
});

async function limited(page: Page, title: string) {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: dir, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  const id = (await r.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  await thread.getByTestId('runtime-composer').fill('limit');
  await thread.getByTestId('runtime-send').click();
  const line = thread.getByTestId('runtime-usage-limit');
  await expect(line).toBeVisible({ timeout: 20_000 });
  return { id, thread, line };
}

test('desktop: Limited, one composer line, Continue with another provider arms the switch', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  test.setTimeout(90_000);
  const { id, thread, line } = await limited(page, 'Limited codex');
  await expect(page.getByTestId(`thread-status-${id}`).first()).toHaveText('Limited');
  await expect(line).toContainText('Codex limit reached · reset time unknown');
  await expect(line.getByTestId('runtime-resume-at-reset')).toHaveCount(0); // no time → no resume
  await expect(thread.getByTestId('runtime-failed')).toHaveAttribute('data-cause', 'usage_limit');
  const sw = line.getByTestId('runtime-limit-switch');
  await expect(sw).toHaveText(/^Continue with /);
  await expect(sw).not.toHaveText(/Codex$/); // another provider, never its own
  await sw.click();
  await expect(thread.getByTestId('runtime-switch-notice')).toContainText('Next message continues on');
  await expect(thread.getByTestId('runtime-usage-limit')).toHaveCount(0); // the switch line is the next step now
});

test('resume at reset: armed shows "resumes at" with Cancel; at that time the thread continues once', async ({ page }) => {
  test.setTimeout(90_000);
  const { id, thread, line } = await limited(page, 'Resume codex');
  await expectNoHorizontalOverflow(page);
  // Armed far out and cancelled: Cancel clears it everywhere.
  const later = new Date(Date.now() + 3_600_000).toISOString();
  expect((await ctx.post(`${base}/api/v1/runtime-sessions/${id}/resume-at`, { data: { at: later } })).ok()).toBeTruthy();
  await expect(line).toHaveAttribute('data-armed', 'true', { timeout: 10_000 });
  await expect(line).toContainText('resumes at');
  await line.getByRole('button', { name: 'Cancel' }).click();
  await expect(line).not.toHaveAttribute('data-armed', 'true');
  // Armed for now + 2 s: the daemon sends the continue; the turn completes.
  const soon = new Date(Date.now() + 2_000).toISOString();
  expect((await ctx.post(`${base}/api/v1/runtime-sessions/${id}/resume-at`, { data: { at: soon } })).ok()).toBeTruthy();
  await expect(thread.getByText('echo: Your usage limit has reset', { exact: false }).last()).toBeVisible({ timeout: 20_000 });
  await expect(thread.getByTestId('runtime-usage-limit')).toHaveCount(0);
  const s = (await (await ctx.get(`${base}/api/v1/runtime-sessions/${id}`)).json()) as { meta?: Record<string, unknown> | null };
  expect(s.meta?.resume_at).toBeUndefined();
});
