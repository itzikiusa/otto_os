import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';
import { openPage, expectNoHorizontalOverflow, expectContentHasHeight } from './helpers';

// Run with Otto E2E. Drives the full pipeline against the isolated E2E daemon:
// the OTTO_E2E agent stub returns a deterministic reply (no live CLI), and the
// engine commits a deterministic note so proof/diff/PR-draft have real material;
// the review stage short-circuits to 0 findings under OTTO_E2E. So a single_agent
// run advances route → policy → engine → source → context → worktree → agent →
// proof → review → awaiting_approval entirely offline, then approve → drafting_pr
// → completed with a PR draft. Desktop-browser project only for the UI part.

const V1 = '/api/v1';

let base = '';
let wsA = '';
let wsB = '';
let repoId = '';
let seededRunId = '';

const TERMINAL = ['completed', 'failed', 'rejected', 'cancelled'];

async function pollUntil(
  ctx: any,
  runId: string,
  wanted: (s: string) => boolean,
  tries = 80,
): Promise<any> {
  let run: any = null;
  for (let i = 0; i < tries; i++) {
    run = await (await ctx.get(`${base}${V1}/runs/${runId}`)).json();
    if (wanted(run.status)) return run;
    await new Promise((r) => setTimeout(r, 250));
  }
  return run;
}

test.beforeAll(async () => {
  const a = await apiCtx();
  base = a.base;
  wsA = await seedWorkspace(a.ctx, base);
  wsB = await seedWorkspace(a.ctx, base);
  const repo = await seedGitRepo(a.ctx, base, wsA);
  repoId = repo.repoId;

  // Launch a single-agent run; it should drive itself to the approval gate.
  const r = await a.ctx.post(`${base}${V1}/workspaces/${wsA}/runs`, {
    data: {
      source_kind: 'channel',
      source_ref: 'e2e-seed',
      seed_text: 'Add a short note describing the project.',
      mode: 'single_agent',
      repo_id: repoId,
      title: 'E2E seeded run',
    },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  const run = await r.json();
  seededRunId = run.id;
  expect(run.status).toBe('queued');
  expect(run.source_kind).toBe('channel');
  await a.ctx.dispose();
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsA);
});

test.describe('run-with-otto API (route → policy → engine → proof → review → approval → PR draft)', () => {
  test('a single_agent run drives itself to the approval gate with evidence', async () => {
    const { ctx } = await apiCtx();
    const run = await pollUntil(ctx, seededRunId, (s) => s === 'awaiting_approval' || TERMINAL.includes(s));
    expect(run.status, `run ended in ${run.status}: ${run.error ?? ''}`).toBe('awaiting_approval');
    // Evidence assembled along the way.
    expect(run.repo_id).toBe(repoId);
    expect(run.branch, 'isolated branch provisioned').toContain('otto-run/');
    expect(run.proof_pack_id, 'proof pack assembled').toBeTruthy();
    expect(run.proof_status, 'proof status derived').toBeTruthy();
    expect(run.findings_blocking).toBe(0);

    // The stage timeline records the pipeline.
    const events = await (await ctx.get(`${base}${V1}/runs/${seededRunId}/events`)).json();
    const statuses = events.map((e: any) => e.status);
    for (const s of ['provisioning', 'executing', 'proving', 'reviewing', 'awaiting_approval']) {
      expect(statuses, `timeline includes ${s}`).toContain(s);
    }
    await ctx.dispose();
  });

  test('approve advances to a completed run with a PR draft', async () => {
    const { ctx } = await apiCtx();
    // Ensure it's at the gate first.
    await pollUntil(ctx, seededRunId, (s) => s === 'awaiting_approval' || TERMINAL.includes(s));
    const ar = await ctx.post(`${base}${V1}/runs/${seededRunId}/approve`, {
      data: { decision: 'approve' },
    });
    expect(ar.ok(), await ar.text()).toBeTruthy();
    const done = await pollUntil(ctx, seededRunId, (s) => TERMINAL.includes(s));
    expect(done.status, `run ended in ${done.status}: ${done.error ?? ''}`).toBe('completed');
    expect(done.approval_decision).toBe('approved');
    expect(done.pr_draft_json, 'a PR draft was produced').toBeTruthy();
    const draft = JSON.parse(done.pr_draft_json);
    expect(draft.title, 'draft has a title').toBeTruthy();
    expect(draft.source_branch).toContain('otto-run/');
    await ctx.dispose();
  });

  test('reject ends a fresh run as rejected', async () => {
    const { ctx } = await apiCtx();
    const launched = await (
      await ctx.post(`${base}${V1}/workspaces/${wsA}/runs`, {
        data: {
          source_kind: 'channel',
          source_ref: 'e2e-reject',
          seed_text: 'Another change.',
          mode: 'single_agent',
          repo_id: repoId,
        },
      })
    ).json();
    await pollUntil(ctx, launched.id, (s) => s === 'awaiting_approval' || TERMINAL.includes(s));
    const rr = await ctx.post(`${base}${V1}/runs/${launched.id}/approve`, {
      data: { decision: 'reject', note: 'not now' },
    });
    expect(rr.ok(), await rr.text()).toBeTruthy();
    const after = await (await ctx.get(`${base}${V1}/runs/${launched.id}`)).json();
    expect(after.status).toBe('rejected');
    expect(after.approval_decision).toBe('rejected');
    await ctx.dispose();
  });

  test('list is workspace-scoped (A run absent from B)', async () => {
    const { ctx } = await apiCtx();
    const a = await (await ctx.get(`${base}${V1}/workspaces/${wsA}/runs`)).json();
    expect(a.some((r: any) => r.id === seededRunId)).toBe(true);
    const b = await (await ctx.get(`${base}${V1}/workspaces/${wsB}/runs`)).json();
    expect(b.some((r: any) => r.id === seededRunId), 'A run must not appear in B').toBe(false);
    await ctx.dispose();
  });

  test('detect classifies a Jira key and a GitHub PR URL', async () => {
    const { ctx } = await apiCtx();
    const jira = await (
      await ctx.get(`${base}${V1}/workspaces/${wsA}/runs/detect?q=PROJ-123`)
    ).json();
    expect(jira.detected?.source_kind).toBe('jira');
    expect(jira.detected?.source_ref).toBe('PROJ-123');
    const pr = await (
      await ctx.get(
        `${base}${V1}/workspaces/${wsA}/runs/detect?q=${encodeURIComponent('https://github.com/acme/widgets/pull/7')}`,
      )
    ).json();
    expect(pr.detected?.source_kind).toBe('github_pr');
    await ctx.dispose();
  });

  test('get 404s for an unknown run id', async () => {
    const { ctx } = await apiCtx();
    const r = await ctx.get(`${base}${V1}/runs/does-not-exist`);
    expect(r.status()).toBe(404);
    await ctx.dispose();
  });

  test('cancel ends an active run as cancelled', async () => {
    const { ctx } = await apiCtx();
    const launched = await (
      await ctx.post(`${base}${V1}/workspaces/${wsA}/runs`, {
        data: {
          source_kind: 'channel',
          source_ref: 'e2e-cancel',
          seed_text: 'A change we will abandon.',
          mode: 'single_agent',
          repo_id: repoId,
        },
      })
    ).json();
    const cr = await ctx.post(`${base}${V1}/runs/${launched.id}/cancel`, { data: {} });
    expect(cr.ok(), await cr.text()).toBeTruthy();
    const after = await pollUntil(ctx, launched.id, (s) => s === 'cancelled');
    expect(after.status).toBe('cancelled');
    await ctx.dispose();
  });

  test('open-pr is gated before approval and on an unproven pack (409)', async () => {
    const { ctx } = await apiCtx();
    const launched = await (
      await ctx.post(`${base}${V1}/workspaces/${wsA}/runs`, {
        data: {
          source_kind: 'channel',
          source_ref: 'e2e-openpr',
          seed_text: 'A change to gate.',
          mode: 'single_agent',
          repo_id: repoId,
        },
      })
    ).json();
    await pollUntil(ctx, launched.id, (s) => s === 'awaiting_approval' || TERMINAL.includes(s));

    // Before approval → blocked ("run is not approved").
    const early = await ctx.post(`${base}${V1}/runs/${launched.id}/open-pr`, { data: {} });
    expect(early.ok(), 'open-pr must be blocked before approval').toBeFalsy();
    expect(await early.text()).toContain('not approved');

    // Approve → completed, then open-pr → 409 (the e2e change has a diff but no
    // passing test, so the proof pack is `partial`, not `passed`/`waived`).
    await ctx.post(`${base}${V1}/runs/${launched.id}/approve`, { data: { decision: 'approve' } });
    const done = await pollUntil(ctx, launched.id, (s) => TERMINAL.includes(s));
    expect(done.status, `run ended in ${done.status}: ${done.error ?? ''}`).toBe('completed');
    expect(['partial', 'missing', 'failed']).toContain(done.proof_status);
    const late = await ctx.post(`${base}${V1}/runs/${launched.id}/open-pr`, { data: {} });
    expect(late.status()).toBe(409);
    expect(await late.text()).toContain('proof pack is not passed/waived');
    await ctx.dispose();
  });

  test('a product-story source resolves and drives the full pipeline', async () => {
    const { ctx } = await apiCtx();
    // Mint a story via the offline-safe drafts endpoint; we only need its id.
    const draft = await (
      await ctx.post(`${base}${V1}/workspaces/${wsA}/product/drafts`, {
        data: { title: 'E2E story: tidy the README' },
      })
    ).json();
    const storyId = draft.story.id as string;

    const launched = await (
      await ctx.post(`${base}${V1}/workspaces/${wsA}/runs`, {
        data: { source_ref: `story:${storyId}`, mode: 'single_agent', repo_id: repoId },
      })
    ).json();
    expect(launched.source_kind).toBe('product_story');
    expect(launched.source_ref).toBe(storyId);

    const run = await pollUntil(ctx, launched.id, (s) => s === 'awaiting_approval' || TERMINAL.includes(s));
    expect(run.status, `run ended in ${run.status}: ${run.error ?? ''}`).toBe('awaiting_approval');
    expect(run.branch, 'isolated branch provisioned').toContain('otto-run/');
    expect(run.proof_pack_id, 'proof pack assembled').toBeTruthy();
    await ctx.dispose();
  });
});

test.describe('run-with-otto UI', () => {
  test('the launcher renders, detects a source, and lists the seeded run', async ({ page }) => {
    await openPage(page, 'run-with-otto');
    // The one-button launcher input + the seeded run row are present.
    await expect(page.getByText('E2E seeded run').first()).toBeVisible({ timeout: 15_000 });
    await expectContentHasHeight(page);
    await expectNoHorizontalOverflow(page);
  });
});
