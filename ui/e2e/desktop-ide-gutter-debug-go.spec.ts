import { test, expect, type Page } from '@playwright/test';
import { existsSync, mkdirSync, mkdtempSync, realpathSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// Debugging a Go test must start delve (`dlv test`) and pause on the
// breakpoints inside the test. A user gutter-ran a test, set breakpoints and
// pressed Debug: the toolbar debugged the auto-selected `go run ./cmd/…`
// config instead and the breakpoints never hit. The fixture mirrors that repo:
// a module with a cmd/ main (the first auto config) and the test in orm/sql.

const TEST_GO = `package sql

import "testing"

func buildDelete(table string, id int) string {
\treturn "DELETE FROM " + table
}

func TestDeleteSuccess(t *testing.T) {
\ttable := "players"
\tid := 7
\tquery := buildDelete(table, id)
\tif query == "" {
\t\tt.Fatal("empty query")
\t}
\tt.Log(query)
}
`;
// Breakpoints: `id := 7` and `t.Log(query)`.
const BP1 = 11;
const BP2 = 16;

let ws = '';
let root = '';

test.beforeAll(async () => {
  root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-gutter-debug-')));
  const pkg = join(root, 'go_utilities', 'orm', 'sql');
  mkdirSync(pkg, { recursive: true });
  writeFileSync(join(root, 'go_utilities', 'go.mod'), 'module example.com/go_utilities\n\ngo 1.21\n');
  writeFileSync(join(pkg, 'query_builder_test.go'), TEST_GO);
  mkdirSync(join(root, 'go_utilities', 'cmd', 'tool'), { recursive: true });
  writeFileSync(join(root, 'go_utilities', 'cmd', 'tool', 'main.go'), 'package main\n\nfunc main() {}\n');
  const { ctx, base } = await apiCtx();
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Gutter debug', root_path: root } });
  expect(r.ok(), await r.text()).toBe(true);
  ws = ((await r.json()) as { id: string }).id;
  const extraPath = [join(homedir(), 'go', 'bin'), '/opt/homebrew/bin', '/usr/local/bin'].filter(existsSync).join(':');
  const env = await ctx.put(`${base}/api/v1/ide/environments`, {
    data: { target: { kind: 'local', workspace_id: ws, root, connection_id: null }, extra_path: extraPath, env_vars: [] },
  });
  expect(env.ok(), await env.text()).toBe(true);
  await ctx.dispose();
});

async function clickBreakpointGutter(page: Page, line: number): Promise<void> {
  const gutter = page.locator('.cm-ide-bp-gutter').first();
  const num = page.locator('.cm-lineNumbers .cm-gutterElement').filter({ hasText: new RegExp(`^${line}$`) }).first();
  const g = await gutter.boundingBox();
  const n = await num.boundingBox();
  expect(g && n).toBeTruthy();
  await page.mouse.click(g!.x + g!.width / 2, n!.y + n!.height / 2);
}

async function openTestWithBreakpoints(page: Page, runs: string[]): Promise<void> {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  page.on('request', (req) => {
    if (req.method() === 'POST' && /\/api\/v1\/ide\/runs(\?|$)/.test(req.url())) runs.push(req.url());
  });
  await page.goto('/#/ide');

  const tree = page.getByTestId('ide-project-tree');
  for (const seg of ['go_utilities', 'go_utilities/orm', 'go_utilities/orm/sql']) {
    const item = tree.locator(`button.tree-item[title="${seg}"], button.tree-item[title$="/${seg}"]`).first();
    await expect(item).toBeVisible({ timeout: 20_000 });
    if ((await item.getAttribute('aria-expanded')) !== 'true') await item.click();
  }
  await tree.locator('button.tree-item[title$="query_builder_test.go"]').first().dblclick();
  await expect(page.locator('.cm-editor .cm-content').first()).toContainText('func TestDeleteSuccess');

  await clickBreakpointGutter(page, BP1);
  await clickBreakpointGutter(page, BP2);
  await expect(page.locator('.cm-ide-bp-marker')).toHaveCount(2);
}

async function gutterMenu(page: Page, entry: RegExp): Promise<void> {
  const runBtn = page.locator('.cm-gutters .cm-ide-gutter-run-btn[data-gutter-line="9"]');
  await expect(runBtn).toBeVisible({ timeout: 20_000 });
  await runBtn.click();
  await page.locator('.ctx-menu').getByRole('menuitem', { name: entry }).click();
}

/** Starts the debug session with `start`, then checks it stops on both breakpoints and ends. */
async function expectPausesOnBothBreakpoints(page: Page, start: () => Promise<void>): Promise<void> {
  const started = page.waitForResponse(
    (r) => r.request().method() === 'POST' && /\/api\/v1\/ide\/debug\/sessions(\?|$)/.test(r.url()),
    { timeout: 20_000 },
  );
  await start();
  const resp = await started;
  expect(resp.ok(), await resp.text()).toBe(true);
  const session = ((await resp.json()) as { session: { id: string; launch?: { test_name?: string } } }).session;
  expect(session.launch?.test_name).toBe('^TestDeleteSuccess$');

  const panel = page.getByTestId('ide-debugger-panel');
  try {
    await expect(panel).toContainText(/Paused/i, { timeout: 90_000 });
    await expect(page.locator('.cm-ide-paused-line').first()).toContainText('id := 7');
    // Locals show the variable assigned on the previous line.
    await expect(panel).toContainText('table');

    await panel.getByRole('button', { name: /^Resume program/ }).click();
    await expect(page.locator('.cm-ide-paused-line').first()).toContainText('t.Log(query)', { timeout: 20_000 });
    await expect(panel).toContainText('query');

    await panel.getByRole('button', { name: /^Resume program/ }).click();
    await expect(panel).toContainText(/Terminated/i, { timeout: 30_000 });
  } finally {
    const { ctx, base } = await apiCtx();
    await ctx.post(`${base}/api/v1/ide/debug/sessions/${session.id}/command`, { data: { action: 'stop' } }).catch(() => {});
    await ctx.dispose();
  }
}

const haveDlv = existsSync(join(homedir(), 'go', 'bin', 'dlv')) || existsSync('/opt/homebrew/bin/dlv');

test('gutter Debug on a Go test starts delve and pauses on its breakpoints', async ({ page }) => {
  test.skip(!haveDlv, 'dlv is not installed');
  test.setTimeout(120_000);
  const runs: string[] = [];
  await openTestWithBreakpoints(page, runs);
  await expectPausesOnBothBreakpoints(page, () => gutterMenu(page, /^Debug 'Run TestDeleteSuccess'/));
  expect(runs, 'Debug must not start a plain run').toEqual([]);
});

test('after a gutter Run, the toolbar Debug debugs that test', async ({ page }) => {
  test.skip(!haveDlv, 'dlv is not installed');
  test.setTimeout(150_000);
  const runs: string[] = [];
  await openTestWithBreakpoints(page, runs);
  await gutterMenu(page, /^Run 'Run TestDeleteSuccess'$/);
  await expect.poll(() => runs.length).toBe(1);
  await expectPausesOnBothBreakpoints(page, () => page.getByTestId('ide-debug-btn').click());
  expect(runs).toHaveLength(1);
  await expect(page.getByTestId('ide-run-config-select')).toContainText('TestDeleteSuccess');
});
