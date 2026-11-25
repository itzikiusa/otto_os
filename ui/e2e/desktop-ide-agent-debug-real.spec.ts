import { test, expect, type APIRequestContext, type FrameLocator, type Locator, type Page } from '@playwright/test';
import { execFileSync, spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import * as readline from 'node:readline';
import { cpSync, existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, tempDir } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Agent debugging + DB Watch, for real (desktop-browser only). A stand-in
// agent (a `shell` agent session — no agent CLI anywhere) drives
// `otto.ui_ide_debug_*` through `ottod mcp-tools` with the session's own
// token, the daemon's UI bridge routes each command to this window, and the
// IDE runs the REAL debugger (delve) on a real Go service / component test
// against real MariaDB + Redis containers:
//
//   A. grant → start (confirm, ready_pattern, breakpoints tagged) → request
//      (confirm) → Stop at the handler, redacted → step: DB Watch shows the
//      added row, then the changed cell (panel + the agent's `db`) → Proceed
//      on the driving bar → next breakpoint → the response arrives → stop
//      (the agent's breakpoints go, the session ends).
//   C. the person steps → `user_moved`; Stop on the driving bar ends the
//      agent's control, the debug session stays usable.
//   D. refusals: denylisted env, non-loopback request, a folder outside the
//      session's, another agent's session; evaluate asks every time.
//   B. a `//go:build component` testify suite on its own database — a MariaDB
//      container the spec starts with Testcontainers' labels (what Otto
//      recognises a run's database by; the fixture carries no testcontainers-go):
//      start by test name + tag, first stop on the exact line, DB Watch armed
//      on the run's container with zero clicks, "Since last stop | Since start".
//
// Needs dlv and Go (OTTO_E2E_GO_DIR = the folder with dlv) and Docker; the Go
// modules come from the machine's module cache (GOMODCACHE, GOPROXY=off).
// ─────────────────────────────────────────────────────────────────────────────

const SLOT = process.env.OTTO_E2E_SLOT ?? '0';
const CLIENT_ID = 'e2e-agentdbg-device';
const GO_DIR = process.env.OTTO_E2E_GO_DIR ?? '';
const RUN = Math.random().toString(36).slice(2, 8);
const FIXTURE = join(process.cwd(), 'e2e', 'fixtures', 'go-orders');
const APP_PORT = Number(process.env.OTTO_E2E_APP_PORT ?? 5718);

const haveTools = (): boolean => {
  if (!GO_DIR || !existsSync(join(GO_DIR, 'dlv'))) return false;
  try {
    execFileSync('docker', ['info', '--format', '{{.ServerVersion}}'], { stdio: 'ignore', timeout: 15_000 });
    return true;
  } catch {
    return false;
  }
};

test.describe.configure({ mode: 'serial' });
test.use({ actionTimeout: 30_000 });

let ws = '';
let root = '';
let sessionId = '';
let sessionToken = '';
let otherId = '';
let otherToken = '';
let dsn = '';
let redisAddr = '';
const containers: string[] = [];
const timings: Record<string, number> = {};

// ── MCP stdio (as desktop-agent-ui-control.spec.ts) ─────────────────────────

class McpStdio {
  private proc: ChildProcessWithoutNullStreams;
  private pending = new Map<number, (msg: Record<string, unknown>) => void>();
  private nextId = 1;
  stderr = '';

  constructor(command: string, args: string[], env: NodeJS.ProcessEnv) {
    this.proc = spawn(command, args, { env, stdio: ['pipe', 'pipe', 'pipe'] });
    this.proc.stderr.on('data', (d: Buffer) => (this.stderr = (this.stderr + d.toString()).slice(-8000)));
    readline.createInterface({ input: this.proc.stdout }).on('line', (line) => {
      let msg: Record<string, unknown>;
      try {
        msg = JSON.parse(line.trim());
      } catch {
        return;
      }
      const resolve = typeof msg.id === 'number' ? this.pending.get(msg.id) : undefined;
      if (resolve) {
        this.pending.delete(msg.id as number);
        resolve(msg);
      }
    });
  }

  request(method: string, params?: unknown, timeoutMs = 20_000): Promise<Record<string, unknown>> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`mcp ${method} timed out\n${this.stderr}`));
      }, timeoutMs);
      this.pending.set(id, (msg) => {
        clearTimeout(timer);
        resolve(msg);
      });
      this.proc.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });
  }

  /** `tools/call` → the command's result object (or the error text). */
  async call(name: string, args: Record<string, unknown> = {}, timeoutMs = 150_000): Promise<{ isError: boolean; text: string; r: Record<string, any> }> {
    const resp = await this.request('tools/call', { name: `otto_ui_${name}`, arguments: args }, timeoutMs);
    if (resp.error) return { isError: true, text: JSON.stringify(resp.error), r: {} };
    const result = resp.result as { content?: { text?: string }[]; isError?: boolean };
    const text = result?.content?.[0]?.text ?? '';
    let json: any = null;
    try {
      json = JSON.parse(text);
    } catch {
      /* plain text */
    }
    // The governed envelope nests the command's own object under `content`.
    const r = json?.content ?? json?.result ?? json ?? {};
    return { isError: Boolean(result?.isError), text, r };
  }

  notify(method: string): void {
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method })}\n`);
  }

  kill(): void {
    this.proc.kill('SIGKILL');
  }
}

function daemonMeta(): { dataDir: string } {
  return JSON.parse(readFileSync(join(process.cwd(), 'e2e', `.auth-${SLOT}`, 'daemon.json'), 'utf8'));
}

async function startMcp(sid: string, token: string): Promise<McpStdio> {
  const { base } = await apiCtx();
  const env: NodeJS.ProcessEnv = {};
  for (const [k, v] of Object.entries(process.env)) if (!k.startsWith('OTTO_')) env[k] = v;
  const mcp = new McpStdio(process.env.OTTO_E2E_BIN!, ['mcp-tools'], {
    ...env, OTTO_SECRETS: 'file', OTTO_SECRETS_ALLOW_PLAINTEXT: '1',
    OTTO_MCP_BASE: base, OTTO_MCP_TOKEN: token, OTTO_SESSION_ID: sid, OTTO_WORKSPACE_ID: ws, OTTO_DATA_DIR: daemonMeta().dataDir,
  });
  await mcp.request('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'e2e-agentdbg', version: '1' } });
  mcp.notify('notifications/initialized');
  return mcp;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function postJson<T>(ctx: APIRequestContext, url: string, data: unknown, method: 'post' | 'put' = 'post'): Promise<T> {
  const r = await ctx[method](url, { data });
  expect(r.ok(), `${method} ${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return (await r.json()) as T;
}

/** The agent session's own MCP token (only in its PTY env). */
async function sessionTokenOf(ctx: APIRequestContext, base: string, sid: string): Promise<string> {
  const file = join(root, `.e2e-token-${sid}`);
  for (let a = 0; a < 3 && !existsSync(file); a++) {
    await ctx.post(`${base}/api/v1/sessions/${sid}/input`, { data: { text: `printf '%s' "$OTTO_MCP_TOKEN" > '${file}'`, submit: true } });
    for (let i = 0; i < 40 && !existsSync(file); i++) await sleep(250);
  }
  const tok = existsSync(file) ? readFileSync(file, 'utf8').trim() : '';
  expect(tok).not.toBe('');
  return tok;
}

function docker(args: string[], timeout = 60_000): string {
  return execFileSync('docker', args, { encoding: 'utf8', timeout }).trim();
}

function hostPort(id: string, port: string): string {
  return docker(['port', id, port]).split('\n')[0].split(':').pop()!;
}

/** The 1-based line of the first line containing `needle` in a fixture file. */
function lineOf(file: string, needle: string): number {
  const i = readFileSync(join(FIXTURE, file), 'utf8').split('\n').findIndex((l) => l.includes(needle));
  expect(i, `${needle} in ${file}`).toBeGreaterThanOrEqual(0);
  return i + 1;
}

// ── Seed ────────────────────────────────────────────────────────────────────

test.beforeAll(async ({}, info) => {
  if (info.project.name !== 'desktop-browser' || !haveTools()) return;
  test.setTimeout(240_000);
  const maria = docker(['run', '-d', '--rm', '--name', `otto-e2e-dbg-maria-${RUN}`, '-e', 'MARIADB_ROOT_PASSWORD=secret', '-e', 'MARIADB_DATABASE=shop', '-p', '127.0.0.1::3306', 'mariadb:11.8.9']);
  const redis = docker(['run', '-d', '--rm', '--name', `otto-e2e-dbg-redis-${RUN}`, '-p', '127.0.0.1::6379', 'redis:7-alpine']);
  containers.push(maria, redis);
  dsn = `root:secret@tcp(127.0.0.1:${hostPort(maria, '3306/tcp')})/shop`;
  redisAddr = `127.0.0.1:${hostPort(redis, '6379/tcp')}`;
  for (let i = 0; i < 120; i++) {
    try {
      docker(['exec', maria, 'mariadb', '-uroot', '-psecret', '-e', 'SELECT 1', 'shop'], 10_000);
      break;
    } catch {
      await sleep(500);
    }
  }

  const { ctx, base } = await apiCtx();
  root = tempDir('otto-agentdbg-');
  cpSync(FIXTURE, root, { recursive: true });
  ws = (await postJson<{ id: string }>(ctx, `${base}/api/v1/workspaces`, { name: `Agent debug ${RUN}`, root_path: root })).id;
  // The IDE finds dlv and the Go module cache through the project environment.
  const goEnv = ['GOMODCACHE', 'GOPROXY', 'GOFLAGS', 'GOCACHE', 'GOTOOLCHAIN']
    .filter((k) => process.env[k])
    .map((k) => ({ key: k, value: process.env[k] }));
  await postJson(ctx, `${base}/api/v1/ide/environments`, {
    target: { kind: 'local', workspace_id: ws, root, connection_id: null },
    extra_path: [GO_DIR, '/opt/homebrew/bin', '/usr/local/bin'].filter(existsSync).join(':'),
    env_vars: goEnv,
  }, 'put');
  for (const [title, set] of [['orders-agent', (id: string) => (sessionId = id)], ['other-agent', (id: string) => (otherId = id)]] as const) {
    const s = await postJson<{ id: string }>(ctx, `${base}/api/v1/workspaces/${ws}/sessions`, {
      kind: 'agent', provider: 'shell', title, cwd: root, meta: { client_id: CLIENT_ID },
    });
    set(s.id);
  }
  sessionToken = await sessionTokenOf(ctx, base, sessionId);
  otherToken = await sessionTokenOf(ctx, base, otherId);
  await ctx.dispose();
});

test.afterAll(() => {
  for (const c of containers) {
    try {
      docker(['rm', '-f', c]);
    } catch {
      /* gone */
    }
  }
  console.log(`[agent-debug timings] ${JSON.stringify(timings)}`);
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  test.skip(!haveTools(), 'needs dlv (OTTO_E2E_GO_DIR) and Docker');
  await page.addInitScript(([w, cid]) => {
    if (!sessionStorage.getItem('agentdbg-reset')) {
      sessionStorage.setItem('agentdbg-reset', '1');
      localStorage.removeItem('otto_side_pane');
    }
    localStorage.setItem('otto_workspace', w as string);
    localStorage.setItem('otto_client_id', cid as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, [ws, CLIENT_ID]);
});

const frame = (page: Page): FrameLocator => page.frameLocator('[data-testid="side-pane-frame"]');

/** The IDE: in the side pane beside the session (the web's stand-in for its own window). */
const ideIn = (page: Page) => frame(page);

/** Answer the next confirm the agent raised (in the IDE's document); returns its text. */
async function answer(page: Page, button: string | null, expectText?: RegExp): Promise<string> {
  const dlg = ideIn(page).getByRole('dialog').filter({ hasText: /wants to|Allow/ }).last();
  await expect(dlg).toBeVisible({ timeout: 60_000 });
  const text = (await dlg.innerText()).trim();
  if (expectText) expect(text).toMatch(expectText);
  if (button) await dlg.getByRole('button', { name: button, exact: true }).click();
  else await dlg.getByRole('button', { name: 'Cancel' }).click();
  await expect(dlg).toHaveCount(0);
  return text;
}

async function grant(page: Page, mcp: McpStdio, title: string): Promise<void> {
  const prompt = page.getByTestId('ui-control-request').filter({ hasText: title });
  const granted = (o: { text: string }) => o.text.includes('"ui_control_granted": true');
  // The daemon may hold the call for the person's answer or answer at once
  // ("they were asked" / "could not be asked" before this window said hello).
  let call = mcp.call('request_control', {});
  let state = 'waiting';
  await expect.poll(async () => {
    if (await prompt.isVisible()) return (state = 'shown');
    const done = await Promise.race([call, sleep(500).then(() => null)]);
    if (done && granted(done)) return (state = 'granted');
    if (done) call = mcp.call('request_control', {});
    return state;
  }, { timeout: 30_000 }).toMatch(/shown|granted/);
  if (state === 'granted') return;
  await prompt.getByRole('button', { name: 'Allow for this session', exact: true }).click();
  if (granted(await call)) return;
  expect(granted(await mcp.call('request_control', {}))).toBe(true);
}

type Out = { isError: boolean; text: string; r: Record<string, any> };

/** Run an agent call that raises a confirm: answer it, return the result. A
 *  cold IDE window can miss the daemon's ready wait ("… retry"): the agent
 *  retries, as a real one does. */
async function confirmed(page: Page, call: Promise<Out> | (() => Promise<Out>), button: string, expectText?: RegExp) {
  const make = typeof call === 'function' ? call : null;
  let pending = typeof call === 'function' ? call() : call;
  const dlg = ideIn(page).getByRole('dialog').filter({ hasText: /wants to|Allow/ }).last();
  for (let i = 0; make && i < 3; i++) {
    const first = await Promise.race([pending.then((o) => o), dlg.waitFor({ timeout: 60_000 }).then(() => null)]);
    if (!first) break;
    if (!/did not report ready in time/.test(first.text)) return { ...first, asked: '' };
    console.log('[retry] the IDE window was not ready yet');
    pending = make();
  }
  const asked = await answer(page, button, expectText);
  const out = await pending;
  expect(out.isError, out.text).toBeFalsy();
  return { ...out, asked };
}

const json = (v: unknown): string => JSON.stringify(v);

/** The IDE's Debug dock tab, visible (the stacked layout keeps it under Tools). */
async function showDebugDock(page: Page): Promise<void> {
  const ide = ideIn(page);
  const tools = ide.getByRole('tab', { name: 'Tools', exact: true }).or(ide.getByRole('button', { name: 'Tools', exact: true }));
  if (await tools.first().isVisible()) await tools.first().click();
  const panel = ide.getByTestId('ide-debugger-panel');
  if (!(await panel.isVisible())) await ide.getByRole('tab', { name: /^Debug/ }).first().click();
  await expect(panel).toBeVisible();
}

test('A+C+D: an agent debugs a Go service end to end — request, stops, DB rows, Proceed, take-over, refusals', async ({ page }) => {
  test.setTimeout(600_000);
  const bp1 = lineOf('main.go', 'id, err := insertOrder(');
  const bp2 = lineOf('main.go', 'if err := redisSetEx(');
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const mcp = await startMcp(sessionId, sessionToken);
  const other = await startMcp(otherId, otherToken);
  const proceeds: string[] = [];
  page.on('request', (r) => {
    if (r.method() === 'POST' && /\/sessions\/[^/]+\/message$/.test(r.url())) proceeds.push(`${r.url()} ${r.postData()}`);
  });
  try {
    await test.step('grant: the prompt beside the session → Allow', async () => {
      console.log(`[step] ${new Date().toISOString()} grant: the prompt beside the session → Allow`);
      await grant(page, mcp, 'orders-agent');
    });

    await test.step('D: a loader variable in env and a folder outside the session are refused before anything runs', async () => {
      console.log(`[step] ${new Date().toISOString()} D: a loader variable in env and a folder outside the session are refused before anything runs`);
      const env = await mcp.call('ide_debug_start', { target: { kind: 'main', file: 'main.go' }, env: { DYLD_INSERT_LIBRARIES: '/tmp/x.dylib' } });
      expect(env.isError, env.text).toBe(true);
      expect(env.text).toMatch(/DYLD_INSERT_LIBRARIES/);
      const outside = await mcp.call('ide_debug_start', { root: '/tmp', target: { kind: 'main', file: 'main.go' } });
      expect(outside.isError, outside.text).toBe(true);
      expect(outside.text).toMatch(/forbidden|folder/);
    });

    let debugId = '';
    await test.step('A: start → the attributed confirm → running once the service listens; breakpoints verified and tagged; DB Watch armed', async () => {
      console.log(`[step] ${new Date().toISOString()} A: start → the attributed confirm → running once the service listens; breakpoints verified and tagged; DB Watch armed`);
      const t0 = Date.now();
      const call = () => mcp.call('ide_debug_start', {
        target: { kind: 'main', file: 'main.go' },
        env: { ORDERS_DSN: dsn, ORDERS_REDIS: redisAddr, ORDERS_ADDR: `127.0.0.1:${APP_PORT}` },
        breakpoints: [{ file: 'main.go', line: bp1 }, { file: 'main.go', line: bp2 }],
        ready_pattern: 'orders listening on',
        db_watch: true,
        say: 'Starting the orders service to follow one POST /orders',
      });
      const { r, asked } = await confirmed(page, call, 'Debug', /orders-agent[\s\S]*ORDERS_DSN/);
      timings.start_to_ready_ms = Date.now() - t0;
      console.log(`[start confirm] ${asked}`);
      console.log(`[start] ${json(r).slice(0, 3000)}`);
      expect(r.state).toBe('running');
      expect(r.ready).toBe(true);
      debugId = r.debug_id ?? '';
      expect(r.breakpoints.length).toBe(2);
      expect(r.breakpoints.every((b: { verified: boolean }) => b.verified)).toBe(true);
      expect(r.db?.watching, json(r.db)).toBe(true);
      expect(r.db.objects).toContain('orders');
      // The person sees the agent's breakpoints in the gutter, marked as its own.
      await expect(ideIn(page).locator('.cm-editor:visible [title*="orders-agent"]').first()).toBeVisible({ timeout: 20_000 });
      await expect(ideIn(page).getByTestId('agent-driving-bar')).toContainText('orders-agent');
      // The start's own narration is on the bar while the service runs (C1-023).
      await expect(ideIn(page).getByTestId('agent-driving-say')).toContainText('Starting the orders service');
    });

    await test.step('D: a request to a host that is not this Mac is refused', async () => {
      console.log(`[step] ${new Date().toISOString()} D: a request to a host that is not this Mac is refused`);
      const far = await mcp.call('ide_debug_request', { url: 'http://example.com/orders' });
      expect(far.isError, far.text).toBe(true);
      expect(far.text).toMatch(/not on this Mac/);
    });

    await test.step('D: another agent session cannot drive this debug session', async () => {
      console.log(`[step] ${new Date().toISOString()} D: another agent session cannot drive this debug session`);
      // Its grant prompt shows beside ITS session: open it in the list, allow, come back.
      const list = page.locator('.shell');
      await list.getByText('other-agent', { exact: true }).first().click();
      await grant(page, other, 'other-agent');
      await list.getByText('orders-agent', { exact: true }).first().click();
      const out = await other.call('ide_debug_control', { action: 'status' });
      expect(out.isError, out.text).toBe(true);
      expect(out.text).toMatch(/forbidden|no debug session/i);
    });

    let stop: Record<string, any> = {};
    await test.step('A: request → confirm → the handler stops; the Stop is redacted', async () => {
      console.log(`[step] ${new Date().toISOString()} A: request → confirm → the handler stops; the Stop is redacted`);
      const t0 = Date.now();
      const call = mcp.call('ide_debug_request', { method: 'POST', url: `http://127.0.0.1:${APP_PORT}/orders`, headers: { 'Content-Type': 'application/json' }, body: '{"customer":"ava","amount":42}' });
      const { r } = await confirmed(page, call, 'Send', /POST http:\/\/127\.0\.0\.1/);
      timings.request_to_stop_ms = Date.now() - t0;
      console.log(`[request] ${json(r).slice(0, 4000)}`);
      expect(r.request.in_flight).toBe(true);
      stop = r.stop;
      expect(stop.state).toBe('paused');
      expect(stop.location.line).toBe(bp1);
      const text = json(stop);
      expect(text).not.toContain('hunter2-local');
      expect(text).not.toContain('secret@tcp');
      expect(text).not.toContain(':secret@');
      // …while the person sees the raw values in the IDE.
      await expect(ideIn(page).locator('.cm-editor:visible')).toContainText('password = "hunter2-local"', { timeout: 15_000 });
    });

    await test.step('D: evaluate asks every time with the exact expression; a decline runs nothing', async () => {
      console.log(`[step] ${new Date().toISOString()} D: evaluate asks every time with the exact expression; a decline runs nothing`);
      const first = mcp.call('ide_debug_evaluate', { expression: 'len(in.Customer)' });
      const { r } = await confirmed(page, first, 'Evaluate', /len\(in\.Customer\)/);
      expect(json(r)).toContain('3');
      const again = mcp.call('ide_debug_evaluate', { expression: 'len(in.Customer) + 1' });
      await answer(page, null, /len\(in\.Customer\) \+ 1/);
      const out = await again;
      expect(out.isError).toBe(true);
      expect(out.text).toMatch(/cancelled_by_user|declined/);
    });

    await test.step('A: stepping — DB Watch shows the added row, then the changed cell (panel + the agent’s db)', async () => {
      console.log(`[step] ${new Date().toISOString()} A: stepping — DB Watch shows the added row, then the changed cell (panel + the agent’s db)`);
      let added = false;
      let changed = false;
      for (let i = 0; i < 8 && !(added && changed); i++) {
        const t0 = Date.now();
        const out = await mcp.call('ide_debug_control', { action: 'step_over', wait_ms: 20_000 });
        timings[`step_${i}_ms`] = Date.now() - t0;
        expect(out.isError, out.text).toBeFalsy();
        const orders = (out.r.db?.objects ?? []).find((o: { object: string }) => o.object === 'orders');
        console.log(`[step ${i}] line ${out.r.location?.line} db ${json(out.r.db).slice(0, 800)}`);
        if (orders?.added === 1) {
          added = true;
          const row = orders.rows[0].row;
          expect(row.customer).toBe('ava');
          expect(row.status).toBe('new');
          expect(json(row)).not.toContain('tok-ava-42');
        }
        if (orders?.changed === 1) {
          changed = true;
          expect(orders.rows[0].changes.status).toEqual({ before: 'new', after: 'paid' });
        }
        if (out.r.location?.line === bp2) break;
      }
      expect(added, 'a step showed the inserted row').toBe(true);
      expect(changed, 'a step showed status new → paid').toBe(true);
      // The panel shows the same change, raw, for the person (a narrow side
      // pane stacks the IDE: the dock is under Tools).
      await showDebugDock(page);
      const strip = ideIn(page).getByTestId('ide-dbw-strip');
      await expect(strip).toBeVisible();
      await strip.getByTestId('ide-dbw-strip-open').click();
      const panel = ideIn(page).getByTestId('ide-dbwatch-panel');
      await expect(panel).toBeVisible();
      await panel.getByTestId('ide-dbw-since-start').click();
      await expect(panel.getByTestId('ide-dbw-diff-row').first()).toContainText('ava');
      await expect(panel).toContainText('tok-ava-42');
    });

    await test.step('A: Proceed on the driving bar tells the agent; the agent continues to the next breakpoint, then the response arrives', async () => {
      console.log(`[step] ${new Date().toISOString()} A: Proceed on the driving bar tells the agent; the agent continues to the next breakpoint, then the response arrives`);
      const bar = ideIn(page).getByTestId('agent-driving-bar');
      const say = await mcp.call('ide_debug_control', { action: 'status', say: 'Paused before the Redis write: the order row is paid.' });
      expect(say.isError, say.text).toBeFalsy();
      await expect(bar.getByTestId('agent-driving-say')).toContainText('order row is paid');
      await bar.getByTestId('agent-driving-proceed').click();
      await expect.poll(() => proceeds.join('\n'), { timeout: 10_000 }).toContain(sessionId);
      expect(proceeds.join('\n')).toContain('proceed');
      let r = (await mcp.call('ide_debug_control', { action: 'continue', wait_ms: 5_000 })).r;
      if (r.state === 'paused' && r.location?.line === bp2) r = (await mcp.call('ide_debug_control', { action: 'continue', wait_ms: 5_000 })).r;
      console.log(`[after continue] ${json(r).slice(0, 1500)}`);
      expect(r.request?.status, json(r)).toBe(201);
      expect(r.request.in_flight).toBe(false);
    });

    await test.step('A: ide_debug_db — diff since the start, watch with a filter, unwatch, watch again', async () => {
      console.log(`[step] ${new Date().toISOString()} ide_debug_db`);
      const diff = await mcp.call('ide_debug_db', { action: 'diff', since: 'start' });
      expect(diff.isError, diff.text).toBeFalsy();
      const orders = diff.r.objects.find((o: { object: string }) => o.object === 'orders');
      expect(orders.added, json(diff.r)).toBe(1);
      expect(orders.rows[0].row).toMatchObject({ customer: 'ava', status: 'paid' });
      expect(json(diff.r)).not.toContain('tok-ava-42');
      const watched = await mcp.call('ide_debug_db', { action: 'watch', objects: [{ object: 'orders', filter: { column: 'customer', value: 'ava' } }] });
      expect(watched.isError, watched.text).toBeFalsy();
      expect(watched.r.watching).toBe(true);
      // The watch's first read (objects with their counts) comes with it.
      expect(watched.r.objects.map((o: { object: string }) => o.object)).toEqual(['orders']);
      await showDebugDock(page);
      await expect(ideIn(page).getByTestId('ide-dbw-strip')).toBeVisible();
      const off = await mcp.call('ide_debug_db', { action: 'unwatch' });
      expect(off.r.watching).toBe(false);
      await expect(ideIn(page).getByTestId('ide-dbw-strip')).toHaveCount(0);
      const nothing = await mcp.call('ide_debug_db', { action: 'diff' });
      expect(nothing.isError).toBe(true);
      expect(nothing.text).toMatch(/Nothing is watched/);
      const again = await mcp.call('ide_debug_db', { action: 'watch' });
      expect(again.isError, again.text).toBeFalsy();
      expect(again.r.objects.map((o: { object: string }) => o.object)).toContain('orders');
    });

    await test.step('A: the agent stops — its breakpoints go, the session ends', async () => {
      console.log(`[step] ${new Date().toISOString()} stop`);
      const out = await mcp.call('ide_debug_control', { action: 'stop' });
      expect(out.isError, out.text).toBeFalsy();
      expect(out.r.state).toBe('terminated');
      expect(out.r.breakpoints_removed).toBe(2);
      await ideIn(page).getByRole('tab', { name: 'Editor', exact: true }).or(ideIn(page).getByRole('button', { name: 'Editor', exact: true })).first().click();
      await expect(ideIn(page).locator('.cm-editor:visible [title*="orders-agent"]')).toHaveCount(0);
    });

    await test.step('C: a new start (asks again), the same request stops at the handler', async () => {
      console.log(`[step] ${new Date().toISOString()} restart`);
      const t0 = Date.now();
      const call = () => mcp.call('ide_debug_start', {
        target: { kind: 'main', file: 'main.go' },
        env: { ORDERS_DSN: dsn, ORDERS_REDIS: redisAddr, ORDERS_ADDR: `127.0.0.1:${APP_PORT}` },
        breakpoints: [{ file: 'main.go', line: bp1 }],
        ready_pattern: 'orders listening on',
      });
      const { r, text } = await confirmed(page, call, 'Debug');
      timings.restart_to_ready_ms = Date.now() - t0;
      expect(r.state, text).toBe('running');
    });

    await test.step('C: the person steps → the agent sees user_moved and does nothing', async () => {
      console.log(`[step] ${new Date().toISOString()} C: the person steps → the agent sees user_moved and does nothing`);
      const call = mcp.call('ide_debug_request', { method: 'POST', url: `http://127.0.0.1:${APP_PORT}/orders`, headers: { 'Content-Type': 'application/json' }, body: '{"customer":"bo","amount":7}' });
      const { r } = await confirmed(page, call, 'Send');
      expect(r.stop?.location?.line).toBe(bp1);
      await showDebugDock(page);
      await ideIn(page).getByTestId('ide-debug-step-over').click();
      await expect(ideIn(page).getByTestId('ide-debug-location')).toContainText(String(bp1 + 1), { timeout: 20_000 });
      const out = await mcp.call('ide_debug_control', { action: 'step_over', wait_ms: 3_000 });
      expect(out.r.user_moved, json(out.r)).toBe(true);
      expect(out.r.location.line).toBe(bp1 + 1);
    });

    await test.step('C: Stop on the driving bar ends the agent’s control; the debug session stays the person’s', async () => {
      console.log(`[step] ${new Date().toISOString()} C: Stop on the driving bar ends the agent’s control; the debug session stays the person’s`);
      await ideIn(page).getByTestId('agent-driving-stop').click();
      const out = await mcp.call('ide_debug_control', { action: 'continue', wait_ms: 1_000 });
      expect(out.isError, out.text).toBe(true);
      await expect(ideIn(page).getByTestId('ide-debug-resume')).toBeEnabled();
      await ideIn(page).getByTestId('ide-debug-resume').click();
      await expect(ideIn(page).getByTestId('ide-debug-pause')).toBeEnabled({ timeout: 15_000 });
      await ideIn(page).getByTestId('ide-debug-stop').click();
      await expect(ideIn(page).getByTestId('ide-debugger-panel')).toContainText('Terminated', { timeout: 15_000 });
      // The agent asking again right away is held back (no nagging after a Stop).
      const again = await mcp.call('request_control', {});
      expect(again.text).toMatch(/moments ago/);
    });

  } finally {
    mcp.kill();
    other.kill();
  }
});

test('B: a tagged component test on its own (Testcontainers-labelled) database — first stop on the exact line, DB Watch armed with zero clicks', async ({ page }) => {
  test.setTimeout(420_000);
  const line = lineOf('orders_component_test.go', 's.Require().NoError(markPaid(db, id))');
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  // The other agent session: A ended with the person stopping orders-agent's
  // control, and an agent is not let ask again right away.
  const mcp = await startMcp(otherId, otherToken);
  // The run's own database, labelled the way Testcontainers labels its
  // containers (real users' component tests start them with testcontainers-go).
  const tc = docker(['run', '-d', '--rm', '--name', `otto-e2e-dbg-tc-${RUN}`,
    '--label', 'org.testcontainers=true', '--label', `org.testcontainers.sessionId=${RUN}`, '--label', 'org.testcontainers.lang=go',
    '-e', 'MARIADB_ROOT_PASSWORD=secret', '-e', 'MARIADB_DATABASE=shop', '-p', '127.0.0.1::3306', 'mariadb:11.8.9']);
  containers.push(tc);
  for (let i = 0; i < 120; i++) {
    try {
      docker(['exec', tc, 'mariadb', '-uroot', '-psecret', '-e', 'SELECT 1', 'shop'], 10_000);
      break;
    } catch {
      await sleep(500);
    }
  }
  const testDsn = `root:secret@tcp(127.0.0.1:${hostPort(tc, '3306/tcp')})/shop`;
  try {
    await page.locator('.shell').getByText('other-agent', { exact: true }).first().click();
    await grant(page, mcp, 'other-agent');
    const t0 = Date.now();
    const call = () => mcp.call('ide_debug_start', {
      target: { kind: 'test', file: 'orders_component_test.go', name: 'TestOrdersSuite/TestCreate' },
      build_tags: ['component'],
      env: { ORDERS_TEST_DSN: testDsn },
      breakpoints: [{ file: 'orders_component_test.go', line }],
      wait_ms: 100_000,
    });
    const { r: started, asked, text: startText } = await confirmed(page, call, 'Debug', /component/);
    console.log(`[B confirm] ${asked}`);
    expect(started.state, startText).toBeTruthy();
    let r = started;
    while (r.state !== 'paused' && r.state !== 'terminated' && Date.now() - t0 < 300_000) {
      r = (await mcp.call('ide_debug_control', { action: 'wait', wait_ms: 60_000 })).r;
    }
    timings.component_start_to_first_stop_ms = Date.now() - t0;
    console.log(`[B stop] ${json(r).slice(0, 2500)}`);
    expect(r.state, json(r)).toBe('paused');
    expect(r.location.line).toBe(line);
    expect(r.location.file).toContain('orders_component_test.go');
    // Zero clicks: DB Watch armed itself on the run's own Testcontainer.
    await showDebugDock(page);
    const strip = ideIn(page).getByTestId('ide-dbw-strip');
    await expect(strip).toBeVisible({ timeout: 30_000 });
    await expect(strip).toContainText(/watching 1 table/);
    await strip.getByTestId('ide-dbw-strip-open').click();
    const panel = ideIn(page).getByTestId('ide-dbwatch-panel');
    await expect(panel.getByTestId('ide-dbw-target')).toContainText(/Testcontainer/);
    // The baseline is read at the first stop; a person reads the stop first, so
    // step once it is in (an instant step can land inside the baseline).
    const store = () => page.frameLocator('[data-testid="side-pane-frame"]').locator('body').evaluate(async () => {
      const u = '/src/modules/ide/dbwatch/ideDbWatchState.svelte.ts';
      const { ideDbWatch } = await import(/* @vite-ignore */ u);
      return { n: ideDbWatch.watch?.checkpoint_count ?? 0, busy: !!ideDbWatch.watch?.busy };
    });
    await expect.poll(async () => { const s = await store(); return s.n >= 1 && !s.busy; }, { timeout: 15_000 }).toBe(true);
    await sleep(1_000);
    const t1 = Date.now();
    const step = await mcp.call('ide_debug_control', { action: 'step_over', wait_ms: 20_000 });
    timings.component_step_ms = Date.now() - t1;
    console.log(`[B step] line ${step.r.location?.line} state ${step.r.state} db ${json(step.r.db).slice(0, 1200)}`);
    expect(step.r.db.objects[0].rows[0].changes.status).toEqual({ before: 'new', after: 'paid' });
    // A second stop: now "since last stop" and "since start" differ, and both are offered.
    await mcp.call('ide_debug_control', { action: 'step_over', wait_ms: 20_000 });
    await expect(panel.getByTestId('ide-dbw-since-last')).toBeVisible();
    await expect(panel.getByTestId('ide-dbw-since-start')).toBeVisible();
    await panel.getByTestId('ide-dbw-since-start').click();
    await expect(panel.getByTestId('ide-dbw-diff-row').first()).toContainText(/new\s*→\s*paid/, { timeout: 15_000 });
    const out = await mcp.call('ide_debug_control', { action: 'stop' });
    expect(out.isError, out.text).toBeFalsy();
  } finally {
    mcp.kill();
    docker(['rm', '-f', tc]);
  }
});
