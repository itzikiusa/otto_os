import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { openPage, expectNoHorizontalOverflow, expectContentHasHeight } from './helpers';

// Scheduled Tasks E2E. Drives the API against the isolated E2E daemon (the
// OTTO_E2E agent stub returns a deterministic markdown report for the
// `OTTO_TASK: scheduled_task` sentinel, so a manual run completes offline), then
// renders the page in the desktop browser. Desktop-browser project only.
//
// The full route → policy → auth → engine → repo → report-file stack is
// exercised by the manual run + report fetch (no live agent, no network).

const V1 = '/api/v1';

let base = '';
let wsA = '';
let wsB = '';
let taskId = '';

test.beforeAll(async () => {
  const a = await apiCtx();
  base = a.base;
  wsA = await seedWorkspace(a.ctx, base);
  wsB = await seedWorkspace(a.ctx, base);

  // Create an interval (hourly) agent_prompt task in workspace A.
  const r = await a.ctx.post(`${base}${V1}/workspaces/${wsA}/scheduled-tasks`, {
    data: {
      name: 'E2E Nightly Review',
      prompt: 'Review the tickets and produce a report.',
      schedule: { cadence: 'interval', every_min: 60 },
      destination: { type: 'none' },
      enabled: true,
    },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const task = await r.json();
  taskId = task.id;
  expect(task.name).toBe('E2E Nightly Review');
  expect(task.enabled).toBe(true);
  // next_run_at is computed on create for display.
  expect(task.next_run_at, 'next_run_at set on create').toBeTruthy();
  await a.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsA);
});

test.describe('scheduled tasks API (route → policy → engine → report)', () => {
  test('validation: sub-5-min interval, bad cron, bad sandbox/retries/tz rejected', async () => {
    const { ctx } = await apiCtx();
    const bad = async (data: any) =>
      (await ctx.post(`${base}${V1}/workspaces/${wsA}/scheduled-tasks`, { data })).status();
    expect(await bad({ name: 'too fast', schedule: { cadence: 'interval', every_min: 1 } })).toBe(400);
    expect(await bad({ name: 'bad cron', schedule: { cadence: 'cron', expr: '99 9 * * 1' } })).toBe(400);
    expect(await bad({ name: 'bad sandbox', sandbox: 'vm' })).toBe(400);
    expect(await bad({ name: 'too many retries', max_retries: 9 })).toBe(400);
    expect(await bad({ name: 'bad tz', timezone: 'Mars/Phobos' })).toBe(400);
    expect(await bad({ name: 'bad provider', provider: 'has spaces!' })).toBe(400);
    expect(await bad({ name: 'workflow no id', kind: 'workflow' })).toBe(400);
    await ctx.dispose();
  });

  test('v2: codex provider + cron + worktree + retries + proof persist', async () => {
    const { ctx } = await apiCtx();
    const r = await ctx.post(`${base}${V1}/workspaces/${wsA}/scheduled-tasks`, {
      data: {
        name: 'E2E v2 task',
        prompt: 'do it',
        provider: 'codex',
        schedule: { cadence: 'cron', expr: '0 9 * * 1' },
        timezone: 'Europe/London',
        sandbox: 'worktree',
        max_retries: 2,
        notify_on_change: true,
        attach_proof: true,
      },
    });
    expect(r.ok(), await r.text()).toBeTruthy();
    const t = await r.json();
    expect(t.provider).toBe('codex');
    expect(t.schedule.cadence).toBe('cron');
    expect(t.schedule.expr).toBe('0 9 * * 1');
    expect(t.timezone).toBe('Europe/London');
    expect(t.sandbox).toBe('worktree');
    expect(t.max_retries).toBe(2);
    expect(t.notify_on_change).toBe(true);
    expect(t.attach_proof).toBe(true);
    // A cron cadence still computes a next_run_at for display.
    expect(t.next_run_at, 'cron next_run_at set').toBeTruthy();
    await ctx.dispose();
  });

  test('list is workspace-scoped (A task absent from B)', async () => {
    const { ctx } = await apiCtx();
    const a = await (await ctx.get(`${base}${V1}/workspaces/${wsA}/scheduled-tasks`)).json();
    expect(a.some((t: any) => t.id === taskId)).toBe(true);
    const b = await (await ctx.get(`${base}${V1}/workspaces/${wsB}/scheduled-tasks`)).json();
    expect(b.some((t: any) => t.id === taskId), 'A task must not appear in B').toBe(false);
    await ctx.dispose();
  });

  test('run now → ok run with a summary, and a fetchable markdown report', async () => {
    const { ctx } = await apiCtx();
    // The OTTO_E2E stub is synchronous, so the manual run completes before the
    // POST returns; poll defensively all the same.
    const run = await (await ctx.post(`${base}${V1}/scheduled-tasks/${taskId}/run`, { data: {} })).json();
    expect(run.trigger).toBe('manual');

    let final = run;
    for (let i = 0; i < 20 && final.status === 'running'; i++) {
      await new Promise((res) => setTimeout(res, 250));
      const runs = await (await ctx.get(`${base}${V1}/scheduled-tasks/${taskId}/runs`)).json();
      final = runs.find((x: any) => x.id === run.id) ?? final;
    }
    expect(final.status, JSON.stringify(final)).toBe('ok');
    expect(final.summary).toContain('Reviewed');
    expect(final.report_rel).toBeTruthy();

    // The stored report is fetchable as markdown and carries the report shape.
    const rep = await ctx.get(`${base}${V1}/scheduled-tasks/runs/${run.id}/report`);
    expect(rep.ok(), await rep.text()).toBeTruthy();
    const md = await rep.text();
    expect(md).toContain('Processed-ticket follow-up review');
    expect(md).toContain('\n---\n');
    await ctx.dispose();
  });

  test('v2: attach_proof builds a proof pack; notify_on_change skips an unchanged re-run', async () => {
    const { ctx } = await apiCtx();
    const t = await (
      await ctx.post(`${base}${V1}/workspaces/${wsA}/scheduled-tasks`, {
        data: {
          name: 'E2E proof+notify',
          prompt: 'review',
          schedule: { cadence: 'interval', every_min: 60 },
          notify_on_change: true,
          attach_proof: true,
        },
      })
    ).json();

    const runOnce = async () => {
      const run = await (await ctx.post(`${base}${V1}/scheduled-tasks/${t.id}/run`, { data: {} })).json();
      let final = run;
      for (let i = 0; i < 20 && final.status === 'running'; i++) {
        await new Promise((res) => setTimeout(res, 250));
        const runs = await (await ctx.get(`${base}${V1}/scheduled-tasks/${t.id}/runs`)).json();
        final = runs.find((x: any) => x.id === run.id) ?? final;
      }
      return final;
    };

    const first = await runOnce();
    expect(first.status, JSON.stringify(first)).toBe('ok');
    expect(first.proof_pack_id, 'attach_proof → proof pack id').toBeTruthy();

    // The deterministic stub returns the same report, so the second run is
    // "unchanged" and delivery is skipped (notify_on_change).
    const second = await runOnce();
    expect(second.status).toBe('ok');
    expect(second.skipped_delivery, 'unchanged re-run skips delivery').toBe(true);
    await ctx.dispose();
  });

  test('report route 404s for an unknown run id', async () => {
    const { ctx } = await apiCtx();
    const r = await ctx.get(`${base}${V1}/scheduled-tasks/runs/does-not-exist/report`);
    expect(r.status()).toBe(404);
    await ctx.dispose();
  });

  test('presets include the ticket follow-up + recurring review/security scans', async () => {
    const { ctx } = await apiCtx();
    const presets = await (await ctx.get(`${base}${V1}/scheduled-tasks/presets`)).json();
    const ids = presets.map((p: any) => p.id);
    expect(ids).toContain('ticket-followup-review');
    expect(ids).toContain('weekly-security-scan');
    expect(ids).toContain('weekly-code-review');
    await ctx.dispose();
  });

  test('patch enabled, then delete', async () => {
    const { ctx } = await apiCtx();
    // Create a throwaway task to delete.
    const t = await (
      await ctx.post(`${base}${V1}/workspaces/${wsA}/scheduled-tasks`, {
        data: { name: 'E2E Disposable', schedule: { cadence: 'daily', at: '03:00' } },
      })
    ).json();
    let r = await ctx.patch(`${base}${V1}/scheduled-tasks/${t.id}`, { data: { enabled: false } });
    expect(r.ok()).toBeTruthy();
    expect((await r.json()).enabled).toBe(false);
    r = await ctx.delete(`${base}${V1}/scheduled-tasks/${t.id}`);
    expect(r.ok()).toBeTruthy();
    r = await ctx.get(`${base}${V1}/scheduled-tasks/${t.id}`);
    expect(r.status()).toBe(404);
    await ctx.dispose();
  });
});

test.describe('scheduled tasks UI', () => {
  test('page renders the seeded task; Run now produces a report row', async ({ page }) => {
    await openPage(page, 'scheduled-tasks');
    await expect(page.getByText('E2E Nightly Review').first()).toBeVisible({ timeout: 15_000 });
    await expectContentHasHeight(page);
    await expectNoHorizontalOverflow(page);

    // Self-contained (tests run in parallel): trigger a run from the UI. The
    // handler runs the task (OTTO_E2E stub → instant report) AND auto-expands the
    // run history, so the report summary appears without a separate "Runs" click.
    await page.getByText('E2E Nightly Review').first().scrollIntoViewIfNeeded();
    await page.getByRole('button', { name: 'Run now' }).first().click();
    await expect(page.getByText('Reviewed').first()).toBeVisible({ timeout: 15_000 });
  });

  test('create form exposes v2 controls (provider, cron, timezone, sandbox, toggles)', async ({ page }) => {
    await openPage(page, 'scheduled-tasks');
    await page.getByRole('button', { name: 'New task' }).click();
    // Provider select + the new toggles are present.
    await expect(page.getByText('Provider', { exact: true })).toBeVisible();
    await expect(page.getByText('Only notify on meaningful change')).toBeVisible();
    await expect(page.getByText('Attach a proof pack to each run')).toBeVisible();
    // Switching the cadence to Cron reveals the cron expression + timezone fields.
    await page.locator('select').filter({ hasText: 'Interval' }).selectOption('cron');
    await expect(page.getByText('Cron expression (5 fields)')).toBeVisible();
    await expect(page.getByText('Timezone', { exact: true })).toBeVisible();
    // Sandbox control is present for an agent task.
    await expect(page.getByText('Sandbox', { exact: true })).toBeVisible();
  });
});
