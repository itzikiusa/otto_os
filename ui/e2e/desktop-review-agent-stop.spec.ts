import { test, expect, type APIRequestContext } from '@playwright/test';
import { writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

// Review-engine reliability (design 2026-07-04) — per-agent stop + durable
// retry, E2E against the isolated test daemon. Seeds reviews via `__e2e`
// (agents/prompts/diff), then drives the REAL endpoints:
//   - stop a running agent → 202, row error/"stopped by user", siblings intact
//   - stop guards: done row → 409, summarizer row → 409, out-of-bounds → 4xx
//   - retry without a durable prompt (and no temp file) → 400
//   - retry WITH the DB prompt+diff → accepted, row flips to pending
//   - legacy pre-0100 path: temp prompt file only (no DB row) still retries
//   - UI: Stop button on a running row; after stop the row shows error + Retry
// Daemon state is global → pin the whole file to one device project (the
// desktop browser — the Stop/Retry row is a pointer-first affordance).

test.describe.configure({ mode: 'serial' });
test.beforeEach(({}, testInfo) => {
  test.skip(
    testInfo.project.name !== 'desktop-browser',
    'review state is global to the daemon; run on a single project only',
  );
});

let ctx: APIRequestContext;
let base = '';
let ws = '';
let repoId = '';
const api = (p: string) => `${base}/api/v1${p}`;

/** A seedable agent row (matches ReviewAgentState's required fields). */
function agentRow(name: string, status: string, over: Record<string, unknown> = {}) {
  return {
    name,
    provider: 'claude',
    model: '',
    status,
    note: '',
    comment_count: 0,
    session_id: null,
    findings: [],
    ...over,
  };
}

async function seedReview(over: Record<string, unknown> = {}): Promise<any> {
  const r = await ctx.post(api(`/workspaces/${ws}/__e2e/review`), {
    data: { repo_id: repoId, pr_number: 0, ...over },
  });
  expect(r.ok()).toBeTruthy();
  return r.json();
}

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

test('stop marks a running agent error/"stopped by user" without touching siblings', async () => {
  const rev = await seedReview({
    agents: [agentRow('Correctness', 'running'), agentRow('Security', 'running'), agentRow('Summarizer', 'pending')],
  });

  const stop = await ctx.post(api(`/reviews/${rev.id}/agents/0/stop`));
  expect(stop.status()).toBe(202);
  const updated = await stop.json();
  expect(updated.agents[0].status).toBe('error');
  expect(updated.agents[0].note).toBe('stopped by user');
  // Sibling agent and summarizer are untouched — the run keeps going.
  expect(updated.agents[1].status).toBe('running');
  expect(updated.agents[2].status).toBe('pending');
  expect(updated.status).toBe('running');
});

test('stop guards: non-running row 409, summarizer 409, out-of-bounds 400', async () => {
  const rev = await seedReview({
    agents: [agentRow('Correctness', 'done'), agentRow('Waiting', 'waiting'), agentRow('Summarizer', 'running')],
  });

  // done row → 409 (only running/waiting rows are stoppable)
  expect((await ctx.post(api(`/reviews/${rev.id}/agents/0/stop`))).status()).toBe(409);
  // waiting row IS stoppable
  expect((await ctx.post(api(`/reviews/${rev.id}/agents/1/stop`))).status()).toBe(202);
  // a stopped row is no longer stoppable (idempotent-guard, not idempotent-OK)
  expect((await ctx.post(api(`/reviews/${rev.id}/agents/1/stop`))).status()).toBe(409);
  // the trailing summarizer row is never stoppable, even while "running"
  expect((await ctx.post(api(`/reviews/${rev.id}/agents/2/stop`))).status()).toBe(409);
  // out-of-bounds index
  expect((await ctx.post(api(`/reviews/${rev.id}/agents/9/stop`))).status()).toBe(400);
});

test('retry without a durable prompt fails; with the DB prompt+diff it re-runs', async () => {
  // No prompts seeded and no $TMPDIR prompt file exists for this fresh id →
  // the legacy failure mode.
  const bare = await seedReview({
    agents: [agentRow('Correctness', 'error', { note: 'stopped by user' }), agentRow('Summarizer', 'pending')],
  });
  const noPrompt = await ctx.post(api(`/reviews/${bare.id}/agents/0/retry`));
  expect(noPrompt.status()).toBe(400);
  expect((await noPrompt.text()).toLowerCase()).toContain('no longer available');

  // Same shape but WITH durable rows (what a post-0100 dispatch persists):
  // retry must succeed purely from the DB — this is the reboot/temp-sweep case.
  // A nonexistent provider keeps the accepted retry from spawning a real CLI in
  // the test daemon (the background run fails to start, which is fine — the
  // durability contract under test is the prompt lookup, not the agent run).
  const durable = await seedReview({
    agents: [
      agentRow('Correctness', 'error', { note: 'stopped by user', provider: 'e2e-no-such-cli' }),
      agentRow('Summarizer', 'pending'),
    ],
    prompts: ['review the diff and write findings'],
    diff: 'diff --git a/x b/x\n+1\n',
  });
  const retry = await ctx.post(api(`/reviews/${durable.id}/agents/0/retry`));
  expect(retry.ok()).toBeTruthy();
  const after = await retry.json();
  // The handler resets the row to pending/"retrying…" before spawning.
  expect(after.agents[0].status).toBe('pending');
  expect(after.agents[0].note).toContain('retrying');
});

test('legacy pre-0100 retry: temp prompt file only (no DB row) still works', async ({}) => {
  // The daemon inherits this process's TMPDIR (global-setup spawns it with
  // process.env), so planting the legacy `otto-review-<id>-<index>.prompt`
  // file here exercises the DB-miss → temp-file fallback ordering for real.
  const legacy = await seedReview({
    agents: [
      agentRow('Correctness', 'error', { note: 'stuck', provider: 'e2e-no-such-cli' }),
      agentRow('Summarizer', 'pending'),
    ],
  });
  writeFileSync(join(tmpdir(), `otto-review-${legacy.id}-0.prompt`), 'legacy prompt text');

  const retry = await ctx.post(api(`/reviews/${legacy.id}/agents/0/retry`));
  expect(retry.ok()).toBeTruthy();
  const after = await retry.json();
  expect(after.agents[0].status).toBe('pending');
});

test('UI: Stop on a running agent row; after stop the row shows error + Retry', async ({ page }) => {
  const rev = await seedReview({
    agents: [agentRow('Correctness', 'running'), agentRow('Summarizer', 'pending')],
  });
  expect(rev.status).toBe('running');

  await page.addInitScript((w) => localStorage.setItem('otto_workspace', w as string), ws);
  await page.goto(`/#/git/${repoId}/review`);

  // The running local review is adopted by the panel; its agent row offers Stop.
  const agentCard = page.locator('.rp-agent', { hasText: 'Correctness' });
  await expect(agentCard).toBeVisible({ timeout: 30_000 });
  const stopBtn = agentCard.getByRole('button', { name: 'Stop', exact: true });
  await expect(stopBtn).toBeVisible();

  await stopBtn.click();

  // Row flips to error ("stopped by user") and stays one click from a re-run.
  await expect(agentCard.locator('.rp-status-pill')).toHaveText(/error/i, { timeout: 15_000 });
  await expect(agentCard.getByText('stopped by user')).toBeVisible();
  await expect(agentCard.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  await expect(agentCard.getByRole('button', { name: 'Stop', exact: true })).toHaveCount(0);
});
