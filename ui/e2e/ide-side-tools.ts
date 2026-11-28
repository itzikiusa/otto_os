// Shared setup for the IDE tool-window list specs (rc6-side-b2): a throwaway
// workspace with a real git repo (two changes), and mocked debug / simulator
// endpoints so the Frames / Variables / Devices lists have rows without a real
// debugger or Xcode. A run with tests is put on the store (no runner needed).
import { expect, type Page, type Route } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync, appendFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { daemonCall, seedPolyglotFiles } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

let seed: { workspaceId: string; root: string } | null = null;
export async function toolsSeed(): Promise<{ workspaceId: string; root: string }> {
  if (seed) return seed;
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-side-b2-')));
  seedPolyglotFiles(root);
  const git = (...a: string[]) => execFileSync('git', a, { cwd: root, stdio: 'pipe', env: { ...process.env, GIT_CONFIG_NOSYSTEM: '1', HOME: root } });
  git('init', '-q');
  git('config', 'user.email', 'e2e@example.invalid');
  git('config', 'user.name', 'E2E');
  git('add', '-A');
  git('-c', 'commit.gpgsign=false', 'commit', '-q', '--no-verify', '--allow-empty', '-m', 'seed');
  git('add', '-A');
  git('-c', 'commit.gpgsign=false', 'commit', '-q', '--no-verify', '--allow-empty', '-m', 'files');
  appendFileSync(join(root, 'go.mod'), '\n// edited\n');
  writeFileSync(join(root, 'app', 'notes.txt'), 'new file\n');
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `Side B2 ${Date.now()}`, root_path: root });
    seed = { workspaceId: ws.id, root };
    return seed;
  } finally {
    await c.ctx.dispose();
  }
}

const DBG = 'dbg_side_b2';
export function debugSnapshot(root: string) {
  return {
    id: DBG, revision: 3, title: 'Debug main.go', language: 'go', adapter: 'dlv', state: 'paused', stop_seq: 1,
    stopped_reason: 'breakpoint', active_thread_id: 1, active_frame_id: 1,
    paused_file: `${root}/app/main.go`, paused_line: 13,
    threads: [{ id: 1, name: 'main' }, { id: 2, name: 'worker' }],
    stack_frames: [
      { id: 1, name: 'main.AppBootstrap.InitApp', path: `${root}/app/main.go`, line: 13, column: 1 },
      { id: 2, name: 'main.main', path: `${root}/app/main.go`, line: 38, column: 1 },
      { id: 3, name: 'runtime.main', path: '/usr/local/go/src/runtime/proc.go', line: 271, column: 1 },
    ],
    scopes: [
      { name: 'Locals', variables_reference: 11, expensive: false, variables: [
        { name: 'name', value: '"otto-ledger"', var_type: 'string', variables_reference: 0 },
        { name: 'ver', value: '"2.4.0"', var_type: 'string', variables_reference: 0 },
      ] },
      { name: 'Globals', variables_reference: 12, expensive: true, variables: [] },
    ],
    inline_values: [], breakpoints: [], output: [{ category: 'stdout', text: 'starting otto-ledger\n', seq: 1 }],
    target_root: root, supports_set_variable: true,
  };
}

const ready = (platform: 'ios' | 'android') => ({ platform, state: 'ready', code: 'ready', summary: 'Ready', tools: [], issues: [], setup_steps: [] });
const caps = { boot: true, shutdown: true, screenshot: true, stream: false, logs: true, rotate: true, appearance: true, open_url: true, port_forward: false, install_app: true };
export const DEVICES = {
  devices: [
    { id: 'ios-15', platform: 'ios', name: 'iPhone 15', state: 'booted', os_version: '17.5', form_factor: 'phone', orientation: 'portrait', appearance: 'light', is_physical: false, capabilities: caps },
    { id: 'ios-ipad', platform: 'ios', name: 'iPad Air', state: 'shutdown', os_version: '17.5', form_factor: 'tablet', orientation: 'portrait', appearance: 'light', is_physical: false, capabilities: caps },
    { id: 'and-pixel', platform: 'android', name: 'Pixel 7 API 34', state: 'shutdown', os_version: '14', form_factor: 'phone', orientation: 'portrait', is_physical: false, capabilities: { ...caps, port_forward: true } },
  ],
  status: { ios: ready('ios'), android: ready('android'), checked_at: new Date().toISOString() },
};

export async function mockToolEndpoints(page: Page, root: string): Promise<void> {
  const snap = debugSnapshot(root);
  await page.route(/\/api\/v1\/ide\/debug\/sessions(\/[^?]*)?(\?.*)?$/, async (route: Route) => {
    const path = new URL(route.request().url()).pathname.replace(/^.*\/ide\/debug\/sessions/, '');
    if (path === '' && route.request().method() === 'GET') return route.fulfill({ json: { sessions: [snap] } });
    if (path.endsWith('/events')) return route.abort();
    if (path === `/${DBG}`) return route.fulfill({ json: { session: snap } });
    return route.fulfill({ json: { session: snap } });
  });
  await page.route(/\/api\/v1\/simulators\/devices(\?.*)?$/, (route) => route.fulfill({ json: DEVICES }));
  await page.route(/\/api\/v1\/simulators\/status(\?.*)?$/, (route) => route.fulfill({ json: DEVICES.status }));
}

export async function bootToolsIde(page: Page, pane = 'files'): Promise<{ root: string }> {
  const s = await toolsSeed();
  await mockToolEndpoints(page, s.root);
  await page.addInitScript(
    ({ wsId, root, pane }) => {
      if (sessionStorage.getItem('__side_b2_init')) return;
      sessionStorage.setItem('__side_b2_init', '1');
      sessionStorage.setItem('otto.ide.stackPane', pane);
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_direction', 'ltr');
      localStorage.setItem('otto.ide.agent.provider', 'claude');
      localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-b2', name: 'otto-web', folders: [{ path: root, name: 'otto-web' }], updatedAt: Date.now() }]));
    },
    { wsId: s.workspaceId, root: s.root, pane },
  );
  await page.goto('/#/ide');
  await expect(page.getByTestId('ide-page')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('ide-project-tree')).toBeAttached({ timeout: 20_000 });
  return { root: s.root };
}

/** A finished run with two suites (one failed test) on the store, and its tool window open. */
export async function putRun(page: Page, root: string): Promise<void> {
  await page.evaluate(async (root) => {
    // @ts-expect-error Vite serves the module in the browser
    const { ide } = await import('/src/modules/ide/ideStore.svelte.ts');
    ide.activeRun = {
      id: 'run_side_b2', title: 'go test ./...', runner: 'go', status: 'failed', exit_code: 1, duration_ms: 1840,
      output: ['=== RUN   TestComputeBonus', '--- FAIL: TestComputeBonus (0.00s)', '    main_test.go:14: want 330, got 300', 'FAIL', 'ok  \totto-web/decimal\t0.02s'],
      tests: [
        { suite: 'otto-web/app', name: 'TestInitApp', status: 'passed', file: `${root}/app/main.go`, line: 13, duration_ms: 2 },
        { suite: 'otto-web/app', name: 'TestComputeBonus', status: 'failed', file: `${root}/app/main.go`, line: 24, duration_ms: 3, message: 'main_test.go:14: want 330, got 300' },
        { suite: 'otto-web/decimal', name: 'TestMarshal', status: 'passed', file: `${root}/decimal/decimal.go`, line: 3, duration_ms: 1 },
      ],
    };
  }, root);
}

/** Opens a bottom tool window (and, stacked, shows the Tools as the content). */
export async function openTool(page: Page, tab: 'run' | 'git' | 'debug' | 'devices'): Promise<void> {
  await page.evaluate(async (tab) => {
    // @ts-expect-error Vite serves the module in the browser
    const { ide } = await import('/src/modules/ide/ideStore.svelte.ts');
    // @ts-expect-error Vite serves the module in the browser
    const { ideLayout } = await import('/src/modules/ide/shell/ideLayout.svelte.ts');
    if (tab === 'debug') {
      const r = await fetch('/api/v1/ide/debug/sessions').then((x) => x.json()).catch(() => null);
      if (r?.sessions?.[0]) {
        ide.activeDebug = r.sessions[0];
        const vars: Record<number, unknown[]> = {};
        for (const sc of r.sessions[0].scopes) vars[sc.variables_reference] = sc.variables;
        ide.expandedVariables = vars;
      }
    }
    ide.openBottomTab(tab);
    if (ideLayout.stack) ideLayout.pane = 'tools';
  }, tab);
}

export async function shot(page: Page, name: string): Promise<void> {
  const out = process.env.OTTO_E2E_SHOTS;
  if (!out) return;
  mkdirSync(out, { recursive: true });
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
    await page.screenshot({ path: join(out, `${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}
