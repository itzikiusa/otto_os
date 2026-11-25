import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir, userInfo } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// The e2e runner gives the daemon a throwaway HOME: delve lives in the real one.
const homedir = (): string => userInfo().homedir;

// Agent debugging, the IDE side (dbg-port): the `otto.ui_ide_debug_*` handlers
// drive the IDE through `idePort`; here the test plays those handlers, calling
// the REAL port and the REAL uiControl store inside the page (Vite serves the
// same module instances the app uses), against a real debugger:
//
//   Go (delve): a testify-style suite behind `//go:build component` — the
//   agent names the suite METHOD; it runs as Runner/Method with the tag merged,
//   stops on its breakpoint and never on the other method's; the person sees
//   the red dot "Set by ‹agent›", the agent's narration and Proceed on the
//   driving bar (Proceed types "proceed" into the agent's session), and Stop
//   on the bar leaves the paused session to the person with the
//   "stopped driving" line.
//   Python (built-in bdb bridge, nothing to install): main target, a loop
//   breakpoint hit twice (stop_seq through onSnapshot), variables, evaluate
//   shown in the Debug console, clear-mine leaves the person's breakpoint.

const AGENT = { session_id: 'agent-e2e-dbg', title: 'Fix orders', provider: 'claude' };

const SUITE_GO = `package suite

import (
\t"reflect"
\t"strings"
\t"testing"
)

// Run runs every Test* method of s as a subtest named after it (as testify does).
func Run(t *testing.T, s interface{}) {
\tv := reflect.ValueOf(s)
\tfor i := 0; i < v.Type().NumMethod(); i++ {
\t\tname := v.Type().Method(i).Name
\t\tif !strings.HasPrefix(name, "Test") {
\t\t\tcontinue
\t\t}
\t\tm := v.Method(i)
\t\tt.Run(name, func(t *testing.T) { m.Call(nil) })
\t}
}
`;

const ORDERS_GO = `//go:build component

package orders

import (
\t"testing"

\t"example.com/shop/suite"
)

type OrdersSuite struct{}

func TestOrdersSuite(t *testing.T) {
\tsuite.Run(t, new(OrdersSuite))
}

func (s *OrdersSuite) TestCreate() {
\titems := 3
\ttotal := items * 7
\tprintln("CREATE", total)
}

func (s *OrdersSuite) TestRefund() {
\trefund := 1
\tprintln("REFUND", refund)
}
`;
const CREATE_LINE = 19; // total := items * 7
const REFUND_LINE = 24; // refund := 1

const PY = 'def total(items):\n    s = 0\n    for i in items:\n        s += i\n    return s\n\n\nif __name__ == "__main__":\n    print("TOTAL", total([1, 2, 3]))\n';

async function project(name: string, files: Record<string, string>): Promise<{ root: string; ws: string }> {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-agent-dbg-')));
  for (const [rel, src] of Object.entries(files)) {
    mkdirSync(join(root, rel, '..'), { recursive: true });
    writeFileSync(join(root, rel), src);
  }
  const { ctx, base } = await apiCtx();
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name, root_path: root } });
  expect(r.ok(), await r.text()).toBe(true);
  const ws = ((await r.json()) as { id: string }).id;
  const extraPath = [join(homedir(), 'go', 'bin'), '/opt/homebrew/bin', '/usr/local/bin'].filter(existsSync).join(':');
  const env = await ctx.put(`${base}/api/v1/ide/environments`, {
    data: { target: { kind: 'local', workspace_id: ws, root, connection_id: null }, extra_path: extraPath, env_vars: [] },
  });
  expect(env.ok(), await env.text()).toBe(true);
  await ctx.dispose();
  return { root, ws };
}

/** The IDE on `ws`, with the port + uiControl reachable as `window.__agentDbg`. */
async function openIde(page: Page, ws: string): Promise<void> {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
  await page.goto('/#/ide');
  await page.evaluate(async () => {
    const urls = ['/src/lib/uiCommands/idePort.ts', '/src/lib/stores/uiControl.svelte.ts'];
    const [portMod, ctlMod] = await Promise.all(urls.map((u) => import(/* @vite-ignore */ u)));
    const port = await portMod.idePort.get(new AbortController().signal, 30_000);
    const seen: unknown[] = [];
    port.onSnapshot((s: unknown) => seen.push(s));
    (window as unknown as { __agentDbg: unknown }).__agentDbg = { port, uiControl: ctlMod.uiControl, seen };
  });
}

type Dbg = { state: string; stop_seq?: number; paused_line?: number | null; id: string };
const debugOf = (page: Page) =>
  page.evaluate(() => ((window as any).__agentDbg.port.debug ?? null) as Dbg | null);

async function waitPaused(page: Page, afterSeq: number): Promise<Dbg> {
  await expect.poll(async () => {
    const d = await debugOf(page);
    return d && d.state === 'paused' && (d.stop_seq ?? 0) > afterSeq ? 'paused' : d?.state ?? 'none';
  }, { timeout: 180_000 }).toBe('paused');
  return (await debugOf(page))!;
}

async function clickBreakpointGutter(page: Page, line: number): Promise<void> {
  const num = page.locator('.cm-editor:visible .cm-lineNumbers .cm-gutterElement').filter({ hasText: new RegExp(`^${line}$`) }).first();
  await num.scrollIntoViewIfNeeded();
  const g = await page.locator('.cm-editor:visible .cm-ide-bp-gutter').first().boundingBox();
  const n = await num.boundingBox();
  expect(g && n).toBeTruthy();
  await page.mouse.click(g!.x + g!.width / 2, n!.y + n!.height / 2);
}

test.describe.configure({ mode: 'serial' });

/** Run a shell UI command in this document as the daemon would (a `ui_command`
 *  frame) and read the result it posts back. */
async function runShellCommand(page: Page, id: string, args: Record<string, unknown>, timeout = 30_000): Promise<Record<string, any>> {
  const posted = page.waitForRequest((r) => r.method() === 'POST' && r.url().includes(`/ui/commands/${id}/result`), { timeout });
  await page.evaluate(async ({ id, args, agent }) => {
    const url = '/src/lib/uiCommands.ts';
    const { handleUiFrame } = await import(/* @vite-ignore */ url);
    handleUiFrame({ type: 'ui_command', id, session_id: agent.session_id, agent, command: 'open', args, deadline_ms: 30_000 });
  }, { id, args, agent: AGENT });
  return (await posted).postDataJSON() as Record<string, any>;
}

test('web: placement window refuses honestly; the daemon auto-open (fallback side) opens the side pane', async ({ page }) => {
  await page.route('**/api/v1/ui/commands/**', (r) => r.fulfill({ json: { ok: true } }));
  await page.addInitScript(() => localStorage.setItem('otto_rail_expanded', '0'));
  await page.goto('/#/home');
  await expect(page.getByTestId('agent-driving-bar')).toHaveCount(0);
  // The UI socket says hello first (a command before hello_ack is dropped).
  await expect.poll(async () => {
    const res = await runShellCommand(page, `probe-${Date.now()}`, { module: 'nope' }, 2_000).catch(() => null);
    return res?.ok === false ? 'ready' : 'waiting';
  }, { timeout: 30_000 }).toBe('ready');

  const refused = await runShellCommand(page, 'win-1', { module: 'ide', placement: 'window' });
  expect(refused.ok).toBe(false);
  expect(refused.error.message).toMatch(/needs the desktop app/);
  await expect(page.getByTestId('side-pane')).toHaveCount(0);

  const side = await runShellCommand(page, 'win-2', { module: 'ide', placement: 'window', fallback: 'side' });
  expect(side.ok, JSON.stringify(side)).toBe(true);
  expect(side.result.pane).toBe('side');
  await expect(page.getByTestId('side-pane')).toBeVisible();
});

test('Go: the agent debugs a suite method behind a build tag; Proceed and Stop on the driving bar', async ({ page }) => {
  test.skip(!existsSync('/opt/homebrew/bin/go') || !existsSync(join(homedir(), 'go', 'bin', 'dlv')), 'needs go + dlv');
  test.setTimeout(300_000);
  const { root, ws } = await project('Agent debug Go', {
    'go.mod': 'module example.com/shop\n\ngo 1.22\n',
    'suite/suite.go': SUITE_GO,
    'orders/orders_component_test.go': ORDERS_GO,
  });
  const proceeds: unknown[] = [];
  await page.route(`**/api/v1/sessions/${AGENT.session_id}/message`, async (r) => {
    proceeds.push(r.request().postDataJSON());
    await r.fulfill({ json: { session_id: AGENT.session_id, delivered: true } });
  });
  await page.route(`**/api/v1/sessions/${AGENT.session_id}/ui-control`, (r) => r.fulfill({ json: { id: AGENT.session_id, meta: {} } }));
  await page.addInitScript(() => localStorage.setItem('otto_scheme', 'dark'));
  await openIde(page, ws);

  // ── start: open project, breakpoints, resolve, launch (what ide_debug_start does) ──
  const resolved = await page.evaluate(async ({ agent, root }) => {
    const { port, uiControl } = (window as any).__agentDbg;
    uiControl.begin('c-start', agent, 'ide_debug_start', 'ide');
    await port.openProject(agent, root);
    const bps = await port.setAgentBreakpoints(agent, [
      { file: 'orders/orders_component_test.go', line: 19 },
      { file: 'orders/orders_component_test.go', line: 24 },
    ]);
    const plan = await port.resolveTarget({ kind: 'test', file: 'orders/orders_component_test.go', name: 'TestCreate' });
    const { debug_id } = await port.launch(agent, plan);
    return { command: plan.command, bps: bps.length, debug_id };
  }, { agent: AGENT, root });
  expect(resolved.bps).toBe(2);
  expect(resolved.command).toBe('go test -tags=component -run ^TestOrdersSuite$/^TestCreate$ ./orders');

  const stop1 = await waitPaused(page, 0);
  expect(stop1.paused_line).toBe(CREATE_LINE);
  await expect(page.locator('.cm-ide-paused-line').first()).toContainText('total := items * 7');
  const panel = page.getByTestId('ide-debugger-panel');
  await expect(panel).toContainText('items');
  await expect(page.locator('.cm-ide-bp-marker[title*="Set by Claude · Fix orders"]')).toHaveCount(2);

  // ── the agent explains and ends its turn: narration + Proceed on the bar ──
  await page.evaluate(({ agent }) => {
    const { port, uiControl } = (window as any).__agentDbg;
    port.say(agent, 'Stopped in TestCreate: items is 3, so total becomes 21');
    uiControl.end('c-start', true);
  }, { agent: AGENT });
  const bar = page.getByTestId('agent-driving-bar');
  await expect(bar.getByTestId('agent-driving-say')).toHaveText('Stopped in TestCreate: items is 3, so total becomes 21');
  const proceed = bar.getByTestId('agent-driving-proceed');
  await expect(proceed).toBeVisible();
  await page.screenshot({ path: test.info().outputPath('agent-debug-bar-dark.png') });
  await proceed.click();
  await expect.poll(() => proceeds.length).toBe(1);
  expect(proceeds[0]).toEqual({ text: 'proceed' });
  await expect(proceed).toBeHidden();

  // ── Stop on the bar: the agent's control ends; the paused session is the person's ──
  await bar.getByTestId('agent-driving-stop').click();
  await expect(bar).toBeHidden();
  const left = page.getByTestId('ide-debug-agent-stopped');
  await expect(left).toContainText('Claude · Fix orders stopped driving — this debug session is yours');
  expect((await debugOf(page))?.state).toBe('paused');
  await page.screenshot({ path: test.info().outputPath('agent-debug-stopped-driving-dark.png') });

  // The person resumes: only TestCreate ran (-run Runner/Method), so the
  // TestRefund breakpoint is never hit and the session ends.
  await panel.getByRole('button', { name: /^Resume program/ }).click();
  await expect.poll(async () => (await debugOf(page))?.state, { timeout: 60_000 }).toBe('terminated');
  const pausedLines = await page.evaluate(() =>
    ((window as any).__agentDbg.seen as Dbg[]).filter((s) => s.state === 'paused').map((s) => s.paused_line));
  expect(pausedLines.length).toBeGreaterThan(0);
  expect(pausedLines).not.toContain(REFUND_LINE);
  await expect(left).toBeHidden();

  // clear-mine: the agent's two breakpoints go, wherever they are.
  const removed = await page.evaluate(({ agent }) => (window as any).__agentDbg.port.clearAgentBreakpoints(agent, 'all'), { agent: AGENT });
  expect(removed).toBe(2);
  await expect(page.locator('.cm-ide-bp-marker')).toHaveCount(0);
});

for (const look of [{ name: 'light', key: 'otto_scheme', value: 'light' }, { name: 'rtl', key: 'otto_direction', value: 'rtl' }]) {
  test(`Python: two stops, variables, evaluate in the console, clear-mine keeps the person’s breakpoint (${look.name})`, async ({ page }) => {
    test.skip(!existsSync('/usr/bin/python3'), 'needs python3');
    test.setTimeout(180_000);
    const { root, ws } = await project(`Agent debug Python ${look.name}`, { 'app.py': PY });
    await page.addInitScript(({ key, value }) => localStorage.setItem(key, value), { key: look.key, value: look.value });
    await openIde(page, ws);
    // The person's own breakpoint, set in the gutter the usual way.
    await page.evaluate(() => (window as any).__agentDbg.port.readLines('app.py', 1, 1));
    await page.getByTestId('ide-project-tree').locator('button.tree-item[title$="app.py"]').first().dblclick();
    await expect(page.locator('.cm-editor:visible .cm-content').first()).toContainText('return s');
    await clickBreakpointGutter(page, 5);

    const first = await page.evaluate(async ({ agent, root }) => {
      const { port, uiControl } = (window as any).__agentDbg;
      uiControl.begin('p-start', agent, 'ide_debug_start', 'ide');
      await port.openProject(agent, root);
      await port.setAgentBreakpoints(agent, [{ file: 'app.py', line: 4 }]);
      const plan = await port.resolveTarget({ kind: 'main', file: 'app.py' });
      await port.launch(agent, plan);
      return plan.command;
    }, { agent: AGENT, root });
    expect(first).toBe('python app.py');
    const s1 = await waitPaused(page, 0);
    expect(s1.paused_line).toBe(4);

    const step = await page.evaluate(async ({ agent }) => {
      const { port, uiControl } = (window as any).__agentDbg;
      port.say(agent, 'First pass: s is 0, i is 1');
      uiControl.end('p-start', true);
      const snap = port.debug;
      const locals = snap.scopes[0].variables.map((v: { name: string }) => v.name);
      const ev = await port.evaluate('s + i');
      uiControl.begin('p-cont', agent, 'ide_debug_control', 'ide');
      await port.command('continue');
      return { locals, ev: ev.result };
    }, { agent: AGENT });
    expect(step.locals).toEqual(expect.arrayContaining(['i', 's']));
    expect(step.ev).toBe('1');
    const s2 = await waitPaused(page, s1.stop_seq ?? 0);
    expect(s2.paused_line).toBe(4);
    expect(s2.stop_seq).toBeGreaterThan(s1.stop_seq ?? 0);

    await page.evaluate(({ agent }) => {
      const { port, uiControl } = (window as any).__agentDbg;
      port.say(agent, 'Second pass: s is 1, i is 2');
      uiControl.end('p-cont', true);
    }, { agent: AGENT });
    const bar = page.getByTestId('agent-driving-bar');
    await expect(bar.getByTestId('agent-driving-say')).toHaveText('Second pass: s is 1, i is 2');
    await expect(bar.getByTestId('agent-driving-proceed')).toBeVisible();
    if (look.value === 'rtl') await expect.poll(() => page.evaluate(() => document.documentElement.dir)).toBe('rtl');
    await page.screenshot({ path: test.info().outputPath(`agent-debug-bar-${look.name}.png`) });
    // The evaluate the agent ran is in the person's Debug console.
    const panel = page.getByTestId('ide-debugger-panel');
    await expect(panel).toContainText('s + i');

    const removed = await page.evaluate(async ({ agent }) => {
      const { port } = (window as any).__agentDbg;
      const n = await port.clearAgentBreakpoints(agent, 'all');
      await port.command('stop');
      return n;
    }, { agent: AGENT });
    expect(removed).toBe(1);
    await expect(page.locator('.cm-ide-bp-marker')).toHaveCount(1);
    // A Threads agent: opening its (already open) project docks its thread beside the editor.
    await page.evaluate(({ agent, root }) => (window as any).__agentDbg.port.openProject({ ...agent, runtime: 'structured' }, root), { agent: AGENT, root });
    await expect(page.getByTestId('ide-agent-panel')).toBeVisible();
    await expect.poll(async () => (await debugOf(page))?.state, { timeout: 30_000 }).toBe('terminated');
    await expect(bar.getByTestId('agent-driving-proceed')).toBeHidden();

    // An agent never silently takes over an IDE showing another project: it asks.
    const other = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-agent-dbg-other-')));
    await page.evaluate(({ agent, other }) => {
      const w = window as any;
      w.__takeover = w.__agentDbg.port.openProject(agent, other).then(() => 'opened', (e: { code?: string }) => e.code ?? 'error');
    }, { agent: AGENT, other });
    const dialog = page.getByRole('dialog');
    await expect(dialog).toContainText('Claude · Fix orders wants to open');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    expect(await page.evaluate(() => (window as any).__takeover)).toBe('cancelled_by_user');
    expect(await page.evaluate(() => (window as any).__agentDbg.port.root)).toBe(root);
  });
}
