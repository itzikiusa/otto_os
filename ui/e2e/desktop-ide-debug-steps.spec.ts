import { test, expect, type Page, type Route } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { bootPolyglotIde, daemonCall, type PolyglotWorkspaceSeed } from './ide-polyglot-fixtures';

// The Debug All step line in the debugger panel (rc.6): the live step, a chip per
// other step, Skip to next step under ⋯, the run's summary with Debug on a failed
// test, and a chip that scrolls the console to its step. The debug-session routes
// are answered here (the daemon side is covered by the live .NET / Zig suites).

let seed: PolyglotWorkspaceSeed;
const ID = 'dbg_steps_e2e';

type Phase = 'running' | 'done';

function snapshot(root: string, phase: Phase): Record<string, unknown> {
  const proj = (name: string): string => `${root}/tests/${name}/${name}.csproj`;
  const done = phase === 'done';
  const steps = [
    { label: 'Core.Tests (net10.0)', state: 'passed', summary: '3 passed', exit_code: 0, program: proj('Core.Tests'), test_framework: 'net10.0' },
    { label: 'Api.Tests (net10.0)', state: done ? 'skipped' : 'running', summary: done ? 'skipped' : undefined, program: proj('Api.Tests'), test_framework: 'net10.0' },
    { label: 'Api.Tests (net11.0)', state: done ? 'failed' : 'pending', summary: done ? '1 of 2 failed' : undefined, exit_code: done ? 1 : undefined, failed_tests: done ? ['FailsOnlyOnNet11'] : [], program: proj('Api.Tests'), test_framework: 'net11.0' },
    { label: 'Mtp.Tests (net10.0)', state: done ? 'passed' : 'pending', summary: done ? '1 passed' : undefined, program: proj('Mtp.Tests'), test_framework: 'net10.0' },
  ];
  const lines = ['Reading test projects…\n', '── Step 1/4 · Core.Tests (net10.0) ──\n', '  Passed AddsSmallNumbers [2 ms]\n', '── Step 2/4 · Api.Tests (net10.0) ──\n'];
  if (done) lines.push('── Step 3/4 · Api.Tests (net11.0) ──\n', '  Failed FailsOnlyOnNet11 [143 ms]\n', '── Step 4/4 · Mtp.Tests (net10.0) ──\n', '4 steps · 2 passed · 1 failed · 1 skipped\n');
  return {
    id: ID,
    revision: done ? 9 : 5,
    title: 'Debug all tests · 3 test projects',
    language: 'csharp',
    adapter: 'netcoredbg',
    state: done ? 'terminated' : 'paused',
    stopped_reason: done ? null : 'breakpoint',
    active_thread_id: done ? null : 1,
    active_frame_id: done ? null : 1,
    paused_file: done ? null : `${root}/src/Calc/Calc.cs`,
    paused_line: done ? null : 7,
    threads: done ? [] : [{ id: 1, name: 'testhost' }],
    stack_frames: done ? [] : [{ id: 1, name: 'Calc.Calculator.Add', path: `${root}/src/Calc/Calc.cs`, line: 7, column: 1 }],
    scopes: [],
    inline_values: [],
    breakpoints: [],
    output: lines.map((text, i) => ({ category: 'console', text, seq: i + 1 })),
    target_root: root,
    steps,
    current_step: done ? null : 1,
  };
}

async function mockSteps(page: Page, root: string, calls: { action?: string; start?: Record<string, unknown> }[]): Promise<void> {
  let phase: Phase = 'running';
  await page.route(/\/api\/v1\/ide\/debug\/sessions(\/[^?]*)?(\?.*)?$/, async (route: Route) => {
    const req = route.request();
    const path = new URL(req.url()).pathname.replace(/^.*\/ide\/debug\/sessions/, '');
    if (path === '' && req.method() === 'GET') return route.fulfill({ json: { sessions: phase === 'running' ? [snapshot(root, phase)] : [] } });
    if (path === '' && req.method() === 'POST') {
      // A failed test's Debug names its test; any other start is the run itself.
      const body = req.postDataJSON() as { launch?: { test_name?: string } };
      if (!body.launch?.test_name) return route.fulfill({ json: { session: snapshot(root, phase) } });
      calls.push({ start: body as Record<string, unknown> });
      return route.fulfill({ json: { session: { ...snapshot(root, 'done'), id: 'dbg_single', title: 'Debug FailsOnlyOnNet11 (netcoredbg)', steps: [], current_step: null } } });
    }
    if (path === `/${ID}/events`) return route.abort();
    if (path === `/${ID}/command`) {
      const action = (req.postDataJSON() as { action: string }).action;
      calls.push({ action });
      if (action === 'skip_step') phase = 'done';
      return route.fulfill({ json: { session: snapshot(root, phase) } });
    }
    if (path === `/${ID}`) return route.fulfill({ json: { session: snapshot(root, phase) } });
    return route.continue();
  });
}

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'debug-steps'), { recursive: true });
  await page.screenshot({ path: join(dir, 'debug-steps', name) });
}

test.describe('Desktop IDE — Debug All steps in the debugger panel', () => {
  test.use({ viewport: { width: 1280, height: 800 } });

  test.beforeAll(async () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-steps-')));
    execFileSync('git', ['init', '-q'], { cwd: root });
    const c = await apiCtx();
    try {
      const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `IDE debug steps ${Date.now()}`, root_path: root });
      seed = { base: c.base, token: c.token, workspaceId: ws.id, workspaceRoot: root, target: { kind: 'local', workspace_id: ws.id, root, connection_id: null } };
    } finally {
      await c.ctx.dispose();
    }
  });

  test.afterAll(() => {
    if (seed?.workspaceRoot) rmSync(seed.workspaceRoot, { recursive: true, force: true });
  });

  test.beforeEach(async ({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop debugger panel');
  });

  for (const scheme of ['dark', 'light'] as const) {
    test(`step line, Skip, summary, failed-test Debug and chip → console (${scheme})`, async ({ page }) => {
      test.setTimeout(90_000);
      const calls: { action?: string; start?: Record<string, unknown> }[] = [];
      await mockSteps(page, seed.workspaceRoot, calls);
      await bootPolyglotIde(page, seed, scheme);
      await page.getByTestId('ide-debug-btn').click();
      const panel = page.getByTestId('ide-debugger-panel');
      const steps = panel.getByTestId('ide-debug-steps');
      await expect(steps).toBeVisible({ timeout: 20_000 });
      await expect(steps.getByTestId('ide-debug-step-now')).toContainText('Step 2 of 4 · Api.Tests (net10.0)');
      const chips = steps.getByTestId('ide-debug-step-chip');
      await expect(chips).toHaveCount(3);
      await expect(chips.first()).toContainText('Core.Tests (net10.0) 3 passed');
      await expect(chips.nth(1)).toBeDisabled();
      const fits = await steps.evaluate((el) => el.scrollWidth <= el.clientWidth);
      expect(fits, 'the step line does not scroll sideways').toBe(true);
      await shot(page, `running-${scheme}.png`);

      // ⋯ › Skip to next step sends skip_step; the run then ends with its summary.
      await panel.getByTestId('ide-debug-more').click();
      await page.getByRole('menuitem', { name: 'Skip to next step' }).click();
      await expect.poll(() => calls.some((c) => c.action === 'skip_step')).toBe(true);
      await expect(steps.getByTestId('ide-debug-step-now')).toContainText('4 steps · 2 passed · 1 failed · 1 skipped', { timeout: 10_000 });
      await expect(chips).toHaveCount(4);
      const debugFailed = steps.getByTestId('ide-debug-step-failed-debug');
      await expect(debugFailed).toHaveCount(1);
      await expect(steps.getByRole('listitem')).toContainText('FailsOnlyOnNet11');
      await shot(page, `done-${scheme}.png`);

      // A chip opens the console at its step's separator.
      await chips.nth(2).click();
      await expect(panel.getByTestId('ide-console-line').filter({ hasText: '── Step 3/4 · Api.Tests (net11.0) ──' })).toBeInViewport();

      // Debug on the failed test starts one session for it, on its framework.
      await debugFailed.click();
      await expect.poll(() => calls.find((c) => c.start)?.start ?? null).not.toBeNull();
      const start = calls.find((c) => c.start)!.start as { language: string; launch: Record<string, unknown> };
      expect(start.language).toBe('csharp');
      expect(start.launch).toMatchObject({
        program: `${seed.workspaceRoot}/tests/Api.Tests/Api.Tests.csproj`,
        test_name: 'FailsOnlyOnNet11',
        test_framework: 'net11.0',
        is_test: true,
      });
    });
  }
});
