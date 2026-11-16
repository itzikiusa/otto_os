import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

// Settings → Self-improvement: the every-workspace actions (Reject all
// suggestions, Turn off everywhere) and the calm “off” state with Turn on.
// Needs a daemon built from this tree (OTTO_E2E_BIN=<repo>/target/debug/ottod).

/** Pending edits have no create route (runs produce them): write rows. */
function seedPending(wsId: string, n: number): void {
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  const meta = JSON.parse(readFileSync(join(process.cwd(), 'e2e', `.auth-${slot}`, 'daemon.json'), 'utf8')) as { dataDir: string };
  const now = new Date().toISOString();
  const run = `e2e-run-${wsId}`;
  let sql = `INSERT INTO improvement_runs (id, workspace_id, trigger, status, started_at) VALUES ('${run}', '${wsId}', 'manual', 'done', '${now}');`;
  for (let i = 0; i < n; i++) {
    sql += `INSERT INTO improvement_edits (id, run_id, workspace_id, target, target_ref, target_path, kind, risk, status, rationale, after_content, created_at)
      VALUES ('e2e-${wsId}-${i}', '${run}', '${wsId}', 'memory', 'MEMORY.md', '/dev/null', 'add', 'low', 'pending', 'Seeded suggestion ${i + 1}', 'x', '${now}');`;
  }
  execFileSync('sqlite3', ['-cmd', '.timeout 5000', join(meta.dataDir, 'otto.db'), sql]);
}

async function open(page: Page, wsId: string): Promise<void> {
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('e2e_seeded')) return;
    sessionStorage.setItem('e2e_seeded', '1');
    localStorage.setItem('otto_workspace', id);
  }, wsId);
  await page.goto('/#/settings/self-improvement');
}

/** Run a header action whether it is inline or folded into ⋯. */
async function headerAction(page: Page, name: RegExp): Promise<void> {
  const inline = page.locator('.ph-actions').getByRole('button', { name });
  if (await inline.isVisible()) return inline.click();
  await page.getByRole('button', { name: /more actions/i }).click();
  await page.getByRole('menuitem', { name }).click();
}

test('reject all suggestions and turn off everywhere, then turn back on', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  const { ctx, base } = await apiCtx();
  const api = `${base}/api/v1`;
  try {
    const a = await seedWorkspace(ctx, base, 'SI bulk A');
    const b = await seedWorkspace(ctx, base, 'SI bulk B');
    for (const id of [a, b]) {
      const cfg = await (await ctx.get(`${api}/workspaces/${id}/self-improvement`)).json();
      const put = await ctx.put(`${api}/workspaces/${id}/self-improvement`, { data: { ...cfg, enabled: true } });
      expect(put.ok(), await put.text()).toBeTruthy();
    }
    seedPending(a, 2);
    seedPending(b, 1);
    const summary = async () => (await (await ctx.get(`${api}/self-improvement`)).json()) as { enabled: number; pending: number; pending_workspaces: number };
    const before = await summary();
    expect(before.pending).toBeGreaterThanOrEqual(3);

    await open(page, a);
    await expect(page.getByRole('heading', { name: /Pending approvals/ })).toBeVisible();

    // Cancel changes nothing; confirming rejects every pending suggestion.
    await headerAction(page, /Reject all suggestions/);
    const reject = page.getByRole('dialog', { name: 'Reject all suggestions' });
    await expect(reject).toContainText(`Reject ${before.pending} pending suggestions in ${before.pending_workspaces} workspaces?`);
    await expect(reject).toContainText('Edits already applied stay applied.');
    await reject.getByRole('button', { name: 'Cancel' }).click();
    expect((await summary()).pending).toBe(before.pending);
    await headerAction(page, /Reject all suggestions/);
    await page.getByRole('dialog', { name: 'Reject all suggestions' }).getByRole('button', { name: 'Reject all' }).click();
    await expect.poll(async () => (await summary()).pending).toBe(0);
    await expect(page.getByRole('heading', { name: /Pending approvals/ })).toHaveCount(0);

    // Turn off everywhere → this workspace shows the calm off state.
    await headerAction(page, /Turn off everywhere/);
    const off = page.getByRole('dialog', { name: 'Turn off self-improvement everywhere' });
    await expect(off).toContainText('Scheduled and live runs stop.');
    await off.getByRole('button', { name: 'Turn off' }).click();
    await expect.poll(async () => (await summary()).enabled).toBe(0);
    await expect(page.getByRole('heading', { name: 'Self-improvement is off' })).toBeVisible();
    await expect(page.locator('.ph-actions').getByRole('button', { name: 'Save' })).toHaveCount(0);

    // Turn on (per workspace) brings the settings back, for this one only.
    await page.getByRole('button', { name: 'Turn on' }).click();
    await expect(page.getByRole('heading', { name: 'Schedule' })).toBeVisible();
    await expect.poll(async () => (await (await ctx.get(`${api}/workspaces/${a}/self-improvement`)).json()).enabled).toBe(true);
    expect((await (await ctx.get(`${api}/workspaces/${b}/self-improvement`)).json()).enabled).toBe(false);
  } finally {
    await ctx.put(`${api}/self-improvement`, { data: { enabled: false } });
    await ctx.dispose();
  }
});
