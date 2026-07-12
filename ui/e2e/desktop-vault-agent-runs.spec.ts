import { expect, test } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { openPage } from './helpers';

// Vault docs-agent RUN PERSISTENCE + embedded-only sessions — desktop, against
// the ISOLATED daemon (OTTO_E2E=1 stubs agent turns; runs complete instantly).
// API half: completed docs runs + refine turns land in the persisted runs list
// (kind/note_path), and GET-by-id survives the live registry's terminal
// eviction (DB fallback). UI half: a run is still visible after a FULL PAGE
// RELOAD (the "switching tabs loses the run" complaint), history rows select
// into the detail view, and vault-docs sessions never appear in the sidebar
// Agents group while ordinary sessions do.

let workspaceId = '';
let vaultId = 0;

test.describe.configure({ mode: 'serial' });

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  ({ vaultId } = await seedVaultDir(ctx, base, workspaceId));
  await ctx.dispose();
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript(
    ({ wsId, vId }) => {
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem(`otto_vault_last:${wsId}`, String(vId));
      localStorage.setItem('otto_rail_expanded', '0');
    },
    { wsId: workspaceId, vId: vaultId },
  );
});

test('api: docs runs + refine turns land in the persisted runs list', async () => {
  const { ctx, base } = await apiCtx();
  const v1 = `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}`;

  // Docs run → poll to terminal.
  const run = await (
    await ctx.post(`${v1}/docs-agents/run`, {
      data: { prompt: 'RUNS-LIST docs run', agents: [{ provider: 'claude' }] },
    })
  ).json();
  expect(run.kind).toBe('docs');
  let final = run;
  for (let i = 0; i < 60 && ['running', 'summarizing'].includes(final.state); i++) {
    await new Promise((r) => setTimeout(r, 250));
    final = await (await ctx.get(`${base}/api/v1/vault/docs-agents/runs/${run.id}`)).json();
  }
  expect(final.state).toBe('done');

  // GET-by-id keeps working after the terminal eviction (durable-row fallback).
  const fromDb = await (
    await ctx.get(`${base}/api/v1/vault/docs-agents/runs/${run.id}`)
  ).json();
  expect(fromDb.state).toBe('done');
  expect(fromDb.kind).toBe('docs');

  // A refine turn is recorded as a kind:"refine" run with the note path.
  const refine = await (
    await ctx.post(`${v1}/docs-agents/refine`, {
      data: { path: 'runbooks/deploy.md', prompt: 'RUNS-LIST refine turn' },
    })
  ).json();
  expect(refine.session_id).toBeTruthy();

  // The persisted list has both, newest-first, with kind + note_path.
  await expect
    .poll(
      async () => {
        const runs = await (await ctx.get(`${v1}/docs-agents/runs`)).json();
        return {
          docs: runs.some(
            (r: { id: string; kind: string; state: string }) =>
              r.id === run.id && r.kind === 'docs' && r.state === 'done',
          ),
          refine: runs.some(
            (r: { kind: string; note_path: string; state: string; written: string[] }) =>
              r.kind === 'refine' &&
              r.note_path === 'runbooks/deploy.md' &&
              r.state === 'done' &&
              r.written.includes('runbooks/deploy.md'),
          ),
        };
      },
      { timeout: 10_000 },
    )
    .toEqual({ docs: true, refine: true });

  // limit=1 caps the page.
  const capped = await (await ctx.get(`${v1}/docs-agents/runs?limit=1`)).json();
  expect(capped).toHaveLength(1);
  await ctx.dispose();
});

test('ui: a run survives a full page reload as history and reopens', async ({ page }) => {
  await openPage(page, 'vault');
  await page.locator('button[title^="Docs agent"]').click();

  const panel = page.locator('.docs-agents');
  await expect(panel).toBeVisible({ timeout: 15_000 });
  await panel.locator('textarea').fill('RELOAD-SURVIVOR run');
  await panel.getByRole('button', { name: /^Run$/i }).click();

  // Run view appears and reaches a terminal state (stubbed → fast).
  await expect(panel.locator('.agent-card').first()).toBeVisible({ timeout: 15_000 });
  await expect(panel.locator('.run-head .pill')).toContainText(/done|error/i, {
    timeout: 30_000,
  });

  // The run is in the Runs section too (server-persisted list).
  await expect(panel.locator('.runs-section')).toContainText('RELOAD-SURVIVOR run');

  // FULL reload — in-memory UI state is gone; the PERSISTED VIEW brings the
  // docs-agents center back automatically (no toggle click — clicking now
  // would collapse the restored view), and the server list restores the run.
  await page.reload();
  await openPage(page, 'vault');
  const panel2 = page.locator('.docs-agents');
  await expect(panel2).toBeVisible({ timeout: 15_000 });
  await expect(panel2.locator('.runs-section')).toContainText('RELOAD-SURVIVOR run', {
    timeout: 15_000,
  });

  // Selecting the history row opens the detail view with its agent rows.
  await panel2.locator('.run-row', { hasText: 'RELOAD-SURVIVOR run' }).first().click();
  await expect(panel2.locator('.agent-card').first()).toBeVisible();
  await expect(panel2.locator('.run-head .kind-chip')).toHaveText('docs');
});

test('ui: refine turns appear in the runs history with the note path', async ({ page }) => {
  await openPage(page, 'vault');
  await page.locator('button[title^="Docs agent"]').click();
  const panel = page.locator('.docs-agents');
  await expect(panel).toBeVisible({ timeout: 15_000 });
  // The refine turn recorded by the API test above is in the list.
  const refineRow = panel.locator('.run-row', { hasText: 'runbooks/deploy.md' }).first();
  await expect(refineRow).toBeVisible();
  await expect(refineRow.locator('.kind-chip')).toHaveText('refine');
});

test('ui: review history survives reload and exposes nested retry controls', async ({ page }) => {
  const now = new Date().toISOString();
  const baseRun = {
    ws_id: workspaceId,
    vault_id: vaultId,
    kind: 'docs',
    target_dir: '',
    note_path: '',
    agents: [],
    summarizer: {
      provider: 'claude',
      model: null,
      state: 'skipped',
      session_id: null,
      error: null,
    },
    written: ['services/orders.md'],
    started_at: now,
    finished_at: now,
  };
  const exhausted = {
    ...baseRun,
    id: 'review-history-exhausted',
    prompt: 'REVIEW-HISTORY exhausted',
    state: 'done_with_findings',
    error: null,
    review: {
      state: 'exhausted',
      max_iterations: 3,
      current_iteration: 3,
      outcome: 'findings_remain',
      reviewers: [],
      rounds: [
        {
          iteration: 3,
          state: 'exhausted',
          reviewers: [
            {
              index: 0,
              provider: 'claude',
              model: null,
              skill: 'vault-data-review',
              focus: 'Transactions and indexes',
              state: 'done',
              session_id: null,
              findings: [
                {
                  severity: 'blocking',
                  category: 'data',
                  summary: 'Transaction boundary is still undocumented',
                  evidence: [
                    { repo_path: 'src/store.rs', line: 88, doc_path: 'data/orders.md', section: 'Writes' },
                  ],
                  missed_item: 'Database transaction boundary',
                  required_fix: 'Explain the atomic write set and rollback behavior.',
                },
              ],
              error: null,
            },
          ],
          revision: {
            state: 'done',
            session_id: null,
            changed_paths: ['data/orders.md', 'coverage.md'],
            error: null,
          },
        },
      ],
    },
  };
  const reviewerStuck = {
    ...baseRun,
    id: 'review-history-reviewer',
    prompt: 'REVIEW-HISTORY reviewer retry',
    state: 'reviewing',
    error: null,
    finished_at: null,
    review: {
      state: 'reviewing',
      max_iterations: 3,
      current_iteration: 1,
      outcome: null,
      reviewers: [],
      rounds: [
        {
          iteration: 1,
          state: 'reviewing',
          reviewers: [
            {
              index: 0,
              provider: 'claude',
              model: null,
              skill: 'vault-evidence-review',
              focus: null,
              state: 'running',
              session_id: null,
              findings: [],
              error: null,
            },
          ],
          revision: {
            state: 'skipped',
            session_id: null,
            changed_paths: [],
            error: null,
          },
        },
      ],
    },
  };
  const revisionStuck = {
    ...baseRun,
    id: 'review-history-revision',
    prompt: 'REVIEW-HISTORY revision retry',
    state: 'revising',
    error: null,
    finished_at: null,
    review: {
      state: 'revising',
      max_iterations: 3,
      current_iteration: 1,
      outcome: null,
      reviewers: [],
      rounds: [
        {
          iteration: 1,
          state: 'revising',
          reviewers: [
            {
              index: 0,
              provider: 'claude',
              model: null,
              skill: 'vault-evidence-review',
              focus: null,
              state: 'done',
              session_id: null,
              findings: [],
              error: null,
            },
          ],
          revision: {
            state: 'running',
            session_id: null,
            changed_paths: [],
            error: null,
          },
        },
      ],
    },
  };
  await page.addInitScript(
    ({ reviewerRun, revisionRun, exhaustedRun }) => {
      const realFetch = window.fetch.bind(window);
      const state = { reviewerRetries: 0, revisionRetries: 0 };
      (window as unknown as { __vaultReviewHistoryE2E: typeof state }).__vaultReviewHistoryE2E =
        state;
      window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(
          typeof input === 'string' || input instanceof URL ? String(input) : input.url,
          location.href,
        );
        const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
        const path = url.pathname;
        if (path.endsWith('/docs-agents/runs') && method === 'GET') {
          return Response.json([reviewerRun, revisionRun, exhaustedRun]);
        }
        if (path.includes('/vault/docs-agents/runs/review-history-')) {
          if (path.endsWith('/review/rounds/1/reviewers/0/retry')) {
            state.reviewerRetries += 1;
            return new Response(null, { status: 202 });
          }
          if (path.endsWith('/review/rounds/1/revision/retry')) {
            state.revisionRetries += 1;
            return new Response(null, { status: 202 });
          }
          if (path.endsWith(exhaustedRun.id)) return Response.json(exhaustedRun);
          if (path.endsWith(revisionRun.id)) return Response.json(revisionRun);
          return Response.json(reviewerRun);
        }
        return realFetch(input, init);
      };
    },
    { reviewerRun: reviewerStuck, revisionRun: revisionStuck, exhaustedRun: exhausted },
  );

  await openPage(page, 'vault');
  await page.locator('button[title^="Docs agent"]').click();
  const panel = page.locator('.docs-agents');
  await expect(panel.locator('.runs-section')).toContainText('REVIEW-HISTORY exhausted', {
    timeout: 15_000,
  });
  await page.reload();
  await openPage(page, 'vault');
  const reloaded = page.locator('.docs-agents');
  await expect(reloaded.locator('.runs-section')).toContainText('REVIEW-HISTORY exhausted', {
    timeout: 15_000,
  });
  await reloaded.locator('.run-row', { hasText: 'REVIEW-HISTORY exhausted' }).click();
  await expect(reloaded.locator('.review-outcome')).toContainText('Review limit reached');
  await expect(reloaded).toContainText('Transaction boundary is still undocumented');
  await expect(reloaded).toContainText('data/orders.md');
  await expect(reloaded).toContainText('coverage.md');

  await reloaded.locator('.run-row', { hasText: 'REVIEW-HISTORY reviewer retry' }).click();
  await reloaded.locator('.reviewer-card').getByRole('button', { name: 'Retry reviewer' }).click();
  await reloaded.locator('.run-row', { hasText: 'REVIEW-HISTORY revision retry' }).click();
  await reloaded.locator('.revision-card').getByRole('button', { name: 'Retry revision' }).click();
  const retries = () =>
    page.evaluate(
      () =>
        (window as unknown as {
          __vaultReviewHistoryE2E: { reviewerRetries: number; revisionRetries: number };
        }).__vaultReviewHistoryE2E,
    );
  await expect.poll(async () => (await retries()).reviewerRetries).toBe(1);
  await expect.poll(async () => (await retries()).revisionRetries).toBe(1);
});

test('ui: vault-docs sessions are hidden from the sidebar Agents group', async (
  { page },
  testInfo,
) => {
  // The E2E stub never creates real sessions for runs, so seed the two cases
  // directly: an embedded vault-docs session and an ordinary foreground one.
  const { ctx, base } = await apiCtx();
  const mk = (title: string, source?: string) =>
    ctx
      .post(`${base}/api/v1/workspaces/${workspaceId}/sessions`, {
        data: {
          kind: 'agent',
          provider: 'shell',
          title,
          cwd: '/tmp',
          meta: source ? { source } : { origin: 'manual' },
        },
      })
      .then((r) => r.json());
  const suffix = testInfo.project.name;
  const docsTitle = `VDOCS-EMBEDDED-${suffix}`;
  const reviewerTitle = `VDOCS-REVIEWER-EMBEDDED-${suffix}`;
  const foregroundTitle = `VISIBLE-FOREGROUND-${suffix}`;
  await mk(docsTitle, 'vault-docs');
  await mk(reviewerTitle, 'vault-docs-review');
  await mk(foregroundTitle);
  await ctx.dispose();

  // This test needs the EXPANDED navigator (the shared beforeEach collapses
  // it to the rail); init scripts run in order, so this later write wins.
  await page.addInitScript(() => localStorage.setItem('otto_rail_expanded', '1'));
  await openPage(page, 'vault');
  const nav = page.locator('nav.navigator');
  await expect(nav).toBeVisible({ timeout: 15_000 });
  // The ordinary session shows in the sidebar Agents list…
  await expect(nav.getByText(foregroundTitle, { exact: true })).toBeVisible({
    timeout: 15_000,
  });
  // …the vault-docs one never does (same session list, filtered by source).
  await expect(nav.getByText(docsTitle, { exact: true })).toHaveCount(0);
  await expect(nav.getByText(reviewerTitle, { exact: true })).toHaveCount(0);
});
