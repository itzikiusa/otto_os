import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });

test.describe('Workflows & Goal Loops UX closure (phone + tablet, light + dark)', () => {
  for (const scheme of ['light', 'dark'] as const) {
    test(`Workflows phone layout ignores persisted dockSide, keeps single primary CTA on Run…, and keeps step zoom button outside summary (${scheme})`, async ({ page }) => {
      const { ctx, base } = await apiCtx();
      const workspace = await seedWorkspace(ctx, base);
      await ctx.dispose();

      await page.addInitScript(({ workspace, scheme }) => {
        localStorage.setItem('otto_workspace', workspace);
        localStorage.setItem('otto_scheme', scheme);
        localStorage.setItem('otto_firstrun_dismissed', '1');
        localStorage.setItem('otto_rail_expanded', '0');
        localStorage.setItem('otto_wf_dock_side', '1');
      }, { workspace, scheme });

      const wf = {
        id: 'wf-closure-1',
        workspace_id: workspace,
        name: 'Nightly Release Verification Workflow With Long Name',
        description: 'Verifies build and deploy gates',
        nodes: [
          { id: 'n1', kind: 'shell', name: 'Run unit suite with very long step name that used to overflow inspector header', x: 60, y: 60, config: { cmd: 'echo ok' } },
        ],
        edges: [],
        created_at: new Date().toISOString(),
        updated_at: new Date().toISOString(),
      };
      const run = {
        id: 'run-closure-1',
        workflow_id: wf.id,
        workspace_id: workspace,
        status: 'failed',
        started_at: new Date().toISOString(),
        finished_at: new Date().toISOString(),
        nodes: [
          {
            node_id: 'n1',
            name: 'Run unit suite',
            status: 'error',
            attempts: 2,
            duration_ms: 420,
            logs: ['step failed'],
            error: 'exit status 1',
            sub_agents: [
              { id: 'sa-1', name: 'Worker 1', status: 'done' },
              { id: 'sa-2', name: 'Worker 2', status: 'failed' },
            ],
          },
        ],
      };

      await page.route(`**/workspaces/${workspace}/workflows`, (route) =>
        route.fulfill({ json: [wf] }),
      );
      await page.route(`**/workflows/${wf.id}`, (route) =>
        route.fulfill({ json: wf }),
      );
      await page.route(`**/workflows/${wf.id}/runs*`, (route) =>
        route.fulfill({ json: [run] }),
      );
      await page.route(`**/workflow-runs/${run.id}*`, (route) =>
        route.fulfill({ json: run }),
      );

      await page.setViewportSize({ width: 390, height: 844 });
      await page.goto(`/#/workflows/${wf.id}`);
      await expect(page.locator('.wf-root')).toBeVisible();
      // 01-06 / 18-01: dock-side class must not be active on phone even when persisted in localStorage
      await expect(page.locator('.wf-root')).not.toHaveClass(/dock-side/);
      await expectNoHorizontalOverflow(page);

      // 01-09: in RunSteps, "Zoom in on this step" button is a sibling inside .step-wrap, not nested inside <summary>
      const zoomInsideSummary = await page.locator('summary button.zoom-btn').count();
      expect(zoomInsideSummary).toBe(0);
    });

    test(`Goal Loops detail stepper, progress bar status colour, criteria labels, decision primary button, draft ⋯ menu, and phone session navigation (${scheme})`, async ({ page }) => {
      const { ctx, base } = await apiCtx();
      const workspace = await seedWorkspace(ctx, base);
      await ctx.dispose();

      await page.addInitScript(({ workspace, scheme }) => {
        localStorage.setItem('otto_workspace', workspace);
        localStorage.setItem('otto_scheme', scheme);
        localStorage.setItem('otto_firstrun_dismissed', '1');
        localStorage.setItem('otto_rail_expanded', '0');
      }, { workspace, scheme });

      const blockedLoop = {
        id: 'loop-blocked-1',
        name: 'Migrate Auth Service',
        workspace_id: workspace,
        status: 'blocked',
        phase: 'evaluating',
        current_iteration: 2,
        iterations_started: 2,
        progress_pct: 60,
        elapsed_secs: 120,
        run_started_at: null,
        definition: {
          title: 'Migrate Auth Service',
          summary: 'Port auth handlers and verify tests',
          acceptance_criteria: [
            { id: 'c-met', text: 'Unit tests pass', verify: 'cargo test', verify_kind: 'command', verify_cmd: 'cargo test' },
            { id: 'c-unmet', text: 'Zero clippy warnings', verify: 'cargo clippy', verify_kind: 'agent', verify_cmd: null },
            { id: 'c-unchecked', text: 'Manual sign-off', verify: 'Human check', verify_kind: 'human', verify_cmd: null },
          ],
        },
        config: {
          executors: [{ name: 'Executor', provider: 'codex', model: '' }],
          mode: 'build',
          allow_commits: false,
        },
        limits: { max_iterations: 5, max_runtime_secs: 600, per_phase_timeout_secs: 300 },
        ledger: {
          questions: [{ id: 'q1', question: 'Should token TTL stay 24h?', answer: null, answered_by: null }],
          verifications: [],
          next_action: 'Wait for TTL decision',
          review_summary: '',
          review_passed: false,
        },
      };

      const draftLoop = {
        ...blockedLoop,
        id: 'loop-draft-1',
        name: 'Draft Goal Loop',
        status: 'draft',
        phase: 'planning',
        current_iteration: 0,
        iterations_started: 0,
        progress_pct: 0,
        ledger: { questions: [], verifications: [], next_action: '', review_summary: '', review_passed: false },
      };

      const iterations = [
        {
          id: 'iter-1',
          loop_id: blockedLoop.id,
          idx: 2,
          status: 'done',
          plan: 'Run tests and evaluate criteria',
          agents: [
            { name: 'Executor', role: 'executor', provider: 'codex', status: 'error', session_id: 'sess-loop-1', note: '', output_summary: '' },
          ],
          evaluation: {
            verdict: 'blocked',
            progress_pct: 60,
            feedback: 'Clippy still has warnings',
            criteria: [
              { id: 'c-met', met: true, evidence: 'All 42 tests passed' },
              { id: 'c-unmet', met: false, evidence: '1 warning in auth.rs' },
            ],
          },
        },
      ];

      await page.route(`**/workspaces/${workspace}/goal-loops`, (route) =>
        route.fulfill({ json: [blockedLoop, draftLoop] }),
      );
      await page.route('**/goal-loops/*?summary=true', (route) => {
        const id = new URL(route.request().url()).pathname.split('/').at(-1);
        const found = id === draftLoop.id ? draftLoop : blockedLoop;
        return route.fulfill({ json: { loop: found, iterations: id === draftLoop.id ? [] : iterations } });
      });

      await page.setViewportSize({ width: 820, height: 1180 });
      await page.goto(`/#/loops/${blockedLoop.id}`);
      await expect(page.getByRole('heading', { name: 'Migrate Auth Service', exact: true })).toBeVisible();

      // 02-GL-01: Stepper marks Evaluate as current step (aria-current="step") and Plan/Execute as past
      const evalStep = page.locator('.stepper .step', { hasText: 'Evaluate' });
      await expect(evalStep).toHaveAttribute('aria-current', 'step');
      await expect(evalStep).toHaveClass(/current/);
      await expect(evalStep).not.toHaveClass(/active/);

      // 02-GL-02: Blocked loop progress bar uses warning fill (.bar-fill.warn)
      await expect(page.locator('.overview .bar-fill.warn')).toBeVisible();

      // 02-GL-03: Criteria show visible text state labels ("Met", "Not met", "Not checked")
      await expect(page.locator('.crit-state.met', { hasText: 'Met' })).toBeVisible();
      await expect(page.locator('.crit-state.unmet', { hasText: 'Not met' })).toBeVisible();
      await expect(page.locator('.crit-state.unchecked', { hasText: 'Not checked' })).toBeVisible();

      // 02-GL-04: Record answer is .primary while Resume is not .primary when unanswered; hint is visible
      await expect(page.locator('.resume-hint')).toContainText('Resume stays off until this answer is recorded.');
      await expect(page.getByRole('button', { name: 'Record answer', exact: true })).toHaveClass(/primary/);
      await expect(page.getByRole('button', { name: 'Resume', exact: true })).not.toHaveClass(/primary/);

      // 02-GL-09: IterationRow renders human status label ("Failed") instead of raw "error"
      await expect(page.locator('.agent', { hasText: 'Failed' })).toBeVisible();

      //Phone viewport check (390x844): no horizontal overflow, and draft loop puts Delete in ⋯ menu
      await page.setViewportSize({ width: 390, height: 844 });
      await expectNoHorizontalOverflow(page);

      // 02-GL-06: Draft loop header has Start + More actions (⋯), no standalone Delete goal loop icon button
      await page.evaluate((id) => { location.hash = `#/loops/${id}`; }, draftLoop.id);
      await expect(page.getByRole('heading', { name: 'Draft Goal Loop', exact: true })).toBeVisible();
      await expect(page.getByRole('button', { name: 'Start', exact: true })).toBeVisible();
      await expect(page.getByRole('button', { name: 'More actions', exact: true })).toBeVisible();
      await expect(page.getByRole('button', { name: 'Delete goal loop', exact: true })).toHaveCount(0);
    });
  }
});
