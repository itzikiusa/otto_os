import { expect, test } from '@playwright/test';
import { apiCtx, seedWorkspace, seedVaultDir } from './seed';
import { openPage } from './helpers';

// Vault docs agents — desktop, against the ISOLATED daemon (OTTO_E2E=1 stubs
// the agent sessions, so runs complete instantly with no real CLI spawns).
// API half: run lifecycle (multi-agent → summarizer → done), refine turn +
// session registry. UI half: the ✨ panel launches a run, per-agent rows show
// provider chips + status pills and reach a terminal state; the refine drawer
// mounts on an open note.

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

test('api: multi-agent run reaches done with a summarizer stage', async () => {
  const { ctx, base } = await apiCtx();
  const run = await (
    await ctx.post(
      `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}/docs-agents/run`,
      {
        data: {
          prompt: 'Document the auth flow',
          target_dir: 'services',
          agents: [{ provider: 'claude' }, { provider: 'claude', model: 'sonnet' }],
          summarizer: { provider: 'claude' },
        },
      },
    )
  ).json();
  expect(run.id).toBeTruthy();
  expect(run.agents).toHaveLength(2);
  expect(run.agents[1].model).toBe('sonnet');

  // Stubbed sessions complete instantly — poll to a terminal state.
  let final = run;
  for (let i = 0; i < 60 && ['running', 'summarizing'].includes(final.state); i++) {
    await new Promise((r) => setTimeout(r, 250));
    final = await (await ctx.get(`${base}/api/v1/vault/docs-agents/runs/${run.id}`)).json();
  }
  expect(final.state).toBe('done');
  expect(final.agents.every((a: { state: string }) => a.state === 'done')).toBe(true);
  expect(final.summarizer.state).toBe('done');
  expect(final.agents.every((a: { session_id: string | null }) => !!a.session_id)).toBe(true);
  await ctx.dispose();
});

test('api: single-agent run skips the summarizer; refine registers a session', async () => {
  const { ctx, base } = await apiCtx();
  const v1 = `${base}/api/v1/workspaces/${workspaceId}/vault/vaults/${vaultId}`;
  const run = await (
    await ctx.post(`${v1}/docs-agents/run`, {
      data: { prompt: 'Document deploys', agents: [{ provider: 'claude' }] },
    })
  ).json();
  let final = run;
  for (let i = 0; i < 60 && ['running', 'summarizing'].includes(final.state); i++) {
    await new Promise((r) => setTimeout(r, 250));
    final = await (await ctx.get(`${base}/api/v1/vault/docs-agents/runs/${run.id}`)).json();
  }
  expect(final.state).toBe('done');
  expect(final.summarizer.state).toBe('skipped');

  // Refine: long request returns a reply + session id; registry serves it back.
  const refine = await (
    await ctx.post(`${v1}/docs-agents/refine`, {
      data: { path: 'runbooks/deploy.md', prompt: 'Add a rollback section' },
    })
  ).json();
  expect(refine.session_id).toBeTruthy();
  const reg = await (
    await ctx.get(`${v1}/docs-agents/refine-session?path=runbooks%2Fdeploy.md`)
  ).json();
  expect(reg.session_id).toBe(refine.session_id);
  await ctx.dispose();
});

test('ui: docs-agents panel runs and shows per-agent rows', async ({ page }) => {
  await openPage(page, 'vault');
  await page.locator('button[title^="Docs agent"]').click();

  const panel = page.locator('.docs-agents');
  await expect(panel).toBeVisible({ timeout: 15_000 });
  await panel.locator('textarea').fill('Document the services in this vault');
  // Two agents: the form starts with one row — add another.
  await panel.getByRole('button', { name: /add agent/i }).click();
  await panel.getByRole('button', { name: /^Run$/i }).click();

  // Per-agent rows with provider chips appear and reach a terminal state.
  await expect(panel.locator('.agent-row, .agent-card').first()).toBeVisible({ timeout: 15_000 });
  await expect(panel).toContainText('claude');
  await expect(panel).toContainText(/done|error/i, { timeout: 30_000 });
});

test('ui: optional reviewers submit exact config and show the live review round', async ({
  page,
}) => {
  const reviewRun = {
    id: 'review-ui-1',
    ws_id: workspaceId,
    vault_id: vaultId,
    kind: 'docs',
    prompt: 'Review-config payload',
    target_dir: '',
    note_path: '',
    state: 'reviewing',
    agents: [
      {
        index: 0,
        name: 'writer-1 · claude',
        provider: 'claude',
        model: null,
        state: 'done',
        session_id: null,
        error: null,
        drafts: [],
      },
    ],
    summarizer: {
      provider: 'claude',
      model: null,
      state: 'skipped',
      session_id: null,
      error: null,
    },
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
              skill: 'vault-docs-review',
              focus: null,
              state: 'done',
              session_id: null,
              findings: [],
              error: null,
            },
            {
              index: 1,
              provider: 'claude',
              model: 'sonnet',
              skill: 'vault-api-review',
              focus: 'Request and response bodies',
              state: 'done',
              session_id: null,
              findings: [
                {
                  severity: 'major',
                  category: 'api-contract',
                  summary: 'Create request body is missing',
                  evidence: [
                    { repo_path: 'src/routes/orders.rs', line: 42, doc_path: null, section: null },
                  ],
                  missed_item: 'CreateOrder request fields',
                  required_fix: 'Document every field and add a JSON example.',
                },
              ],
              error: null,
            },
          ],
          revision: { state: 'pending', session_id: null, changed_paths: [], error: null },
        },
      ],
    },
    written: ['services/orders.md'],
    error: null,
    started_at: new Date().toISOString(),
    finished_at: null,
  };
  const noReviewRun = {
    ...reviewRun,
    id: 'review-ui-no-review',
    prompt: 'No-review payload',
    state: 'done',
    review: {
      state: 'skipped',
      max_iterations: 3,
      current_iteration: 0,
      outcome: null,
      reviewers: [],
      rounds: [],
    },
    finished_at: new Date().toISOString(),
  };

  await page.addInitScript(
    ({ first, reviewed }) => {
      const realFetch = window.fetch.bind(window);
      const state = { submissions: [] as Record<string, unknown>[] };
      (window as unknown as { __vaultReviewE2E: typeof state }).__vaultReviewE2E = state;
      window.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(
          typeof input === 'string' || input instanceof URL ? String(input) : input.url,
          location.href,
        );
        const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
        if (url.pathname.endsWith('/docs-agents/run') && method === 'POST') {
          state.submissions.push(JSON.parse(String(init?.body ?? '{}')));
          return Response.json(state.submissions.length === 1 ? first : reviewed);
        }
        if (url.pathname.endsWith(`/vault/docs-agents/runs/${reviewed.id}`)) {
          return Response.json(reviewed);
        }
        return realFetch(input, init);
      };
    },
    { first: noReviewRun, reviewed: reviewRun },
  );

  await openPage(page, 'vault');
  await page.locator('button[title^="Docs agent"]').click();
  const panel = page.locator('.docs-agents');
  await expect(panel).toBeVisible({ timeout: 15_000 });
  await panel.locator('textarea').fill('No-review payload');
  await expect(panel.getByRole('checkbox', { name: 'Review outcomes' })).not.toBeChecked();
  await panel.getByRole('button', { name: /^Run$/i }).click();
  const submissions = () =>
    page.evaluate(
      () =>
        (window as unknown as { __vaultReviewE2E: { submissions: Record<string, unknown>[] } })
          .__vaultReviewE2E.submissions,
    );
  await expect.poll(async () => (await submissions()).length).toBe(1);
  expect((await submissions())[0]).not.toHaveProperty('review');
  await panel.getByRole('button', { name: 'New run' }).click();

  await panel.locator('textarea').fill('Review-config payload');
  await panel.getByRole('checkbox', { name: 'Review outcomes' }).check();
  await expect(panel.locator('.reviewer-config-row')).toHaveCount(1);
  await expect(panel.getByLabel('Maximum review iterations')).toHaveValue('3');

  await panel.getByRole('button', { name: /add reviewer/i }).click();
  const focused = panel.locator('.reviewer-config-row').nth(1);
  await focused.getByLabel('Review method').selectOption('vault-api-review');
  await focused.getByLabel('Reviewer model').fill('sonnet');
  await focused.getByLabel('Review focus').fill('Request and response bodies');
  await panel.getByRole('button', { name: /^Run$/i }).click();

  await expect.poll(async () => (await submissions()).length).toBe(2);
  expect((await submissions())[1]).toMatchObject({
    prompt: 'Review-config payload',
    review: {
      max_iterations: 3,
      reviewers: [
        { provider: 'claude', skill: 'vault-docs-review' },
        {
          provider: 'claude',
          model: 'sonnet',
          skill: 'vault-api-review',
          focus: 'Request and response bodies',
        },
      ],
    },
  });
  await expect(panel.locator('.review-progress')).toContainText('Review round 1 of 3');
  await expect(panel.locator('.reviewer-card')).toHaveCount(2);
  await expect(panel).toContainText('Create request body is missing');
  await expect(panel).toContainText('src/routes/orders.rs:42');
});

test('ui: refine drawer mounts on an open note', async ({ page }) => {
  await openPage(page, 'vault');
  const tree = page.locator('.tree');
  await tree.getByText('runbooks', { exact: true }).click();
  await tree.getByText('deploy', { exact: true }).click();
  await page.locator('button[title="Refine with AI"]').click();
  const drawer = page.locator('.refine-drawer');
  await expect(drawer).toBeVisible();
  await expect(drawer.locator('select')).toBeVisible();
  await drawer.locator('input[type="text"], input:not([type])').first().fill('Tighten the intro');
  await drawer.getByRole('button', { name: /send/i }).click();
  // Stubbed turn completes; the drawer stays open and the input re-enables.
  await expect(drawer.locator('input').first()).toBeEnabled({ timeout: 20_000 });
});
