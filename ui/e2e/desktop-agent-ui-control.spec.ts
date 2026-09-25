import { test, expect, type APIRequestContext, type FrameLocator, type Page, type Route } from '@playwright/test';
import { execFileSync, spawn, type ChildProcess, type ChildProcessWithoutNullStreams } from 'node:child_process';
import * as readline from 'node:readline';
import { existsSync, mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { MOCK_TABLES, mockDbRoutes, seedMockDbConnection } from './db-mock';

// ─────────────────────────────────────────────────────────────────────────────
// Agent UI control, end to end (desktop-browser only): an agent session drives
// the Database Explorer through `ottod mcp-tools` (the stdio MCP server a
// Claude/Codex session talks to) with the SESSION's own token, and every step
// happens visibly in the user's window, in the side pane beside the session:
//
//   1. the first ungranted call raises the "Allow UI control" prompt beside the
//      session → Allow → the call goes through;
//   2. otto_ui_open → the side pane opens on Connections;
//   3. otto_ui_db_new_tab → an agent-attributed query tab with the statement;
//   4. otto_ui_db_run_query → the grid fills in the pane, the tool result
//      carries the same rows — and the run went out `read_only`;
//   5. otto_ui_db_page / otto_ui_db_set_view → the pager / JSON view move;
//   6. an UPDATE → the attributed write confirm; Cancel → `cancelled_by_user`,
//      and nothing but the refused read-only attempt reached the engine;
//   7. Stop on the driving bar → the grant is gone → `pending_grant`;
//   8. with no Otto window open → reads fall back headless (`ui_visible:false`),
//      UI-only commands say `no_ui_client`.
//
// The DB engine is the Docker-free mock (db-mock.ts: a real connection profile,
// engine calls answered by page.route — which also covers the side pane's
// same-origin iframe). The headless steps can't use page.route (the DAEMON
// runs them), so they list connections (no engine) and, when `redis-server` is
// installed, run a real read against a throwaway Redis.
// ─────────────────────────────────────────────────────────────────────────────

const SLOT = process.env.OTTO_E2E_SLOT ?? '0';
const CLIENT_ID = 'e2e-uictl-device';
// The connection library is GLOBAL on the daemon: unique names keep a rerun
// (or another project's beforeAll) from making the by-name lookup ambiguous.
const RUN = Math.random().toString(36).slice(2, 8);
const CONN_NAME = `agent-shop-${RUN}`;
const REDIS_NAME = `agent-redis-${RUN}`;

test.describe.configure({ mode: 'serial' });

let ws = '';
let root = '';
let connId = '';
let sessionId = '';
let sessionToken = '';

function daemonMeta(): { dataDir: string; port: string } {
  const p = join(process.cwd(), 'e2e', `.auth-${SLOT}`, 'daemon.json');
  return JSON.parse(readFileSync(p, 'utf8')) as { dataDir: string; port: string };
}

/** A minimal newline-delimited JSON-RPC 2.0 stdio client for an MCP server
 *  (as desktop-mcp-api-tools.spec.ts), with a per-request timeout: a UI
 *  command can legitimately wait on a person for a while. */
class McpStdio {
  private proc: ChildProcessWithoutNullStreams;
  private rl: readline.Interface;
  private pending = new Map<number, (msg: Record<string, unknown>) => void>();
  private nextId = 1;
  stderr = '';

  constructor(command: string, args: string[], env: NodeJS.ProcessEnv) {
    this.proc = spawn(command, args, { env, stdio: ['pipe', 'pipe', 'pipe'] });
    this.proc.stderr.on('data', (d: Buffer) => {
      this.stderr = (this.stderr + d.toString()).slice(-8000);
    });
    this.rl = readline.createInterface({ input: this.proc.stdout });
    this.rl.on('line', (line) => {
      const t = line.trim();
      if (!t) return;
      let msg: Record<string, unknown>;
      try {
        msg = JSON.parse(t);
      } catch {
        return;
      }
      const id = msg.id;
      if (typeof id === 'number' && this.pending.has(id)) {
        const resolve = this.pending.get(id)!;
        this.pending.delete(id);
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

  notify(method: string): void {
    this.proc.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method })}\n`);
  }

  /** `tools/call` → the tool's text + isError flag + the text parsed as JSON. */
  async call(name: string, args: Record<string, unknown> = {}, timeoutMs = 90_000): Promise<ToolOut> {
    const resp = await this.request('tools/call', { name, arguments: args }, timeoutMs);
    if (resp.error) {
      return { isError: true, text: JSON.stringify(resp.error), json: resp.error };
    }
    const result = resp.result as { content?: { text?: string }[]; isError?: boolean } | undefined;
    const text = result?.content?.[0]?.text ?? '';
    let json: unknown = null;
    try {
      json = JSON.parse(text);
    } catch {
      /* plain text */
    }
    return { isError: Boolean(result?.isError), text, json };
  }

  kill(): void {
    try {
      this.proc.kill('SIGKILL');
    } catch {
      /* ignore */
    }
  }
}

interface ToolOut {
  isError: boolean;
  text: string;
  json: unknown;
}

/** Find the command's own result object inside the governed envelope — the
 *  first object carrying `key` (the envelope nests it under `result`). */
function dig(v: unknown, key: string): Record<string, any> | null {
  if (!v || typeof v !== 'object') return null;
  if (!Array.isArray(v) && key in (v as object)) return v as Record<string, any>;
  for (const child of Object.values(v as object)) {
    const hit = dig(child, key);
    if (hit) return hit;
  }
  return null;
}

/** Environment for the inward `ottod mcp-tools` bridge — ONLY our routing (see
 *  desktop-mcp-api-tools.spec.ts: an inherited OTTO_* would drive the
 *  developer's real daemon instead of this run's isolated one). */
function bridgeEnv(routing: Record<string, string>): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (!key.startsWith('OTTO_')) env[key] = value;
  }
  return { ...env, OTTO_SECRETS: process.env.OTTO_SECRETS ?? 'file', ...routing };
}

async function startMcp(sid = sessionId, token = sessionToken): Promise<McpStdio> {
  const { base } = await apiCtx();
  const { dataDir } = daemonMeta();
  const bin = process.env.OTTO_E2E_BIN;
  if (!bin) throw new Error('OTTO_E2E_BIN must point at the ottod under test');
  const mcp = new McpStdio(
    bin,
    ['mcp-tools'],
    bridgeEnv({
      OTTO_MCP_BASE: base,
      OTTO_MCP_TOKEN: token,
      OTTO_SESSION_ID: sid,
      OTTO_WORKSPACE_ID: ws,
      OTTO_DATA_DIR: dataDir,
    }),
  );
  await mcp.request('initialize', {
    protocolVersion: '2024-11-05',
    capabilities: {},
    clientInfo: { name: 'e2e-uictl', version: '1' },
  });
  mcp.notify('notifications/initialized');
  return mcp;
}

async function postJson<T>(ctx: APIRequestContext, url: string, data: unknown): Promise<T> {
  const r = await ctx.post(url, { data });
  expect(r.ok(), `POST ${url} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return (await r.json()) as T;
}

const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));

/** The agent session's own MCP token: the daemon puts it in the session's PTY
 *  env (`OTTO_MCP_TOKEN`) and never on disk, so ask the shell for it. */
async function readSessionToken(ctx: APIRequestContext, base: string, sid: string): Promise<string> {
  const file = join(root, `.e2e-session-token-${sid}`);
  for (let attempt = 0; attempt < 3 && !existsSync(file); attempt++) {
    await ctx.post(`${base}/api/v1/sessions/${sid}/input`, {
      data: { text: `printf '%s' "$OTTO_MCP_TOKEN" > '${file}'`, submit: true },
    });
    for (let i = 0; i < 40 && !existsSync(file); i++) await sleep(250);
  }
  const tok = existsSync(file) ? readFileSync(file, 'utf8').trim() : '';
  expect(tok, 'the agent session must carry a per-session OTTO_MCP_TOKEN').not.toBe('');
  return tok;
}

// ── The engine: the shared mock, plus paging + the read-only refusal ────────

interface SeenQuery {
  statement: string;
  read_only: boolean;
  confirm_write: boolean;
}

/**
 * db-mock answers `query` with every row and a fixed `auto_limited`; this
 * override (registered after it, so it wins) honours `max_rows` / `offset` for
 * the pager and refuses a write sent `read_only` the way otto-dbviewer does
 * (403, `read_only: …`). Every query body is recorded.
 */
async function engineOverride(page: Page, seen: SeenQuery[]): Promise<void> {
  const orders = MOCK_TABLES.find((t) => t.name === 'orders')!;
  await page.route(new RegExp(`/connections/${connId}/db/query$`), async (route: Route) => {
    const body = (route.request().postDataJSON() ?? {}) as Record<string, any>;
    const statement = String(body.statement ?? '');
    seen.push({ statement, read_only: body.read_only === true, confirm_write: body.confirm_write === true });
    const isWrite = /^\s*(update|insert|delete|drop|alter|truncate|create)\b/i.test(statement);
    if (isWrite && body.read_only === true) {
      return route.fulfill({
        status: 403,
        contentType: 'application/json',
        body: JSON.stringify({
          code: 'forbidden',
          message: 'read_only: this run was requested read-only; this statement is classified as a write/DDL',
        }),
      });
    }
    if (isWrite) {
      return route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({ columns: [], rows: [], rows_affected: 1, stats: { duration_ms: 3, row_count: 0 }, truncated: false }),
      });
    }
    if (!/\bfrom\s+`?orders`?/i.test(statement)) return route.fallback();
    const pageSize = Number(body.max_rows) > 0 ? Math.min(Number(body.max_rows), orders.rows) : orders.rows;
    const offset = Number(body.offset) > 0 ? Number(body.offset) : 0;
    const n = Math.max(0, Math.min(pageSize, orders.rows - offset));
    const rows = Array.from({ length: n }, (_, r) => {
      const id = offset + r + 1;
      return [id, (id * 13) % 97, ['paid', 'shipped', 'pending'][id % 3], '19.99', 'USD', '2026-09-01 10:00:00', null, null, null, 0, 1, 'eu-west'];
    });
    return route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        columns: orders.cols.map((c) => ({ name: c.name, type_hint: c.type.toUpperCase() })),
        rows,
        stats: { duration_ms: 12, row_count: n },
        truncated: false,
        auto_limited: pageSize,
      }),
    });
  });
}

// ── Seed: workspace, the mock connection, the agent session + its token ─────

test.beforeAll(async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  root = mkdtempSync(join(tmpdir(), 'otto-uictl-'));
  const workspace = await postJson<{ id: string }>(ctx, `${base}/api/v1/workspaces`, {
    name: 'UI control E2E',
    root_path: root,
  });
  ws = workspace.id;
  connId = await seedMockDbConnection(ctx, base, ws, CONN_NAME, 'mysql');
  // `meta.client_id` = the device the session was started from: the daemon
  // routes the session's UI commands to THAT device's window (Q4).
  const session = await postJson<{ id: string }>(ctx, `${base}/api/v1/workspaces/${ws}/sessions`, {
    kind: 'agent',
    provider: 'shell',
    title: 'uictl-agent',
    cwd: root,
    meta: { client_id: CLIENT_ID },
  });
  sessionId = session.id;
  sessionToken = await readSessionToken(ctx, base, sessionId);
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript(
    ([w, cid]) => {
      // Once per test (the flag is shared with the side pane's iframe): a
      // clean split, this workspace, this device.
      if (!sessionStorage.getItem('uictl-reset')) {
        sessionStorage.setItem('uictl-reset', '1');
        localStorage.removeItem('otto_side_pane');
      }
      localStorage.setItem('otto_workspace', w as string);
      localStorage.setItem('otto_client_id', cid as string);
      localStorage.setItem('otto_rail_expanded', '0');
    },
    [ws, CLIENT_ID],
  );
});

/**
 * The first ungranted call. The daemon only raises the prompt when one of the
 * user's windows has said `hello` on its events socket — right after a
 * `page.goto` it may not have yet ("no Otto window is open right now, so they
 * could not be asked"), and an agent simply retries. Returns the call's result.
 */
async function firstUngranted(mcp: McpStdio, tool: string, args: Record<string, unknown>): Promise<ToolOut> {
  const until = Date.now() + 20_000;
  for (;;) {
    const out = await mcp.call(tool, args);
    if (!out.text.includes('could not be asked') || Date.now() > until) return out;
    await sleep(500);
  }
}

const pane = (page: Page) => page.getByTestId('side-pane');
const frame = (page: Page): FrameLocator => page.frameLocator('[data-testid="side-pane-frame"]');

test('an agent drives the Database Explorer visibly in the side pane', async ({ page }) => {
  test.setTimeout(300_000);
  const seen: SeenQuery[] = [];
  await mockDbRoutes(page, connId);
  await engineOverride(page, seen);

  // The main pane shows the agent's own session — the side-by-side case.
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });

  const mcp = await startMcp();
  try {
    await test.step('the session sees the ui tools', async () => {
      const listed = await mcp.request('tools/list');
      const names = ((listed.result as { tools: { name: string }[] }).tools ?? []).map((t) => t.name);
      for (const n of ['otto_ui_open', 'otto_ui_db_new_tab', 'otto_ui_db_run_query', 'otto_ui_db_page', 'otto_ui_db_set_view']) {
        expect(names, `tools/list must advertise ${n}`).toContain(n);
      }
    });

    await test.step('first call → pending_grant + the prompt beside the session → Allow', async () => {
      const args = { module: 'connections', route: 'database', placement: 'side' };
      const first = firstUngranted(mcp, 'otto_ui_open', args);
      const prompt = page.getByTestId('ui-control-request');
      await expect(prompt).toBeVisible({ timeout: 20_000 });
      await expect(prompt).toContainText('uictl-agent');
      // The daemon may hold the call briefly for the grant, or answer at once.
      const pending = await first;
      await prompt.getByTestId('ui-control-allow').click();
      await expect(prompt).toHaveCount(0);
      let out = pending;
      if (out.text.includes('pending_grant')) {
        // "…they were asked in Otto; retry after they allow it."
        expect(out.isError).toBe(true);
        out = await mcp.call('otto_ui_open', args);
      }
      expect(out.isError, `ui_open: ${out.text}`).toBeFalsy();
      expect(dig(out.json, 'ui_visible')?.ui_visible).toBe(true);
    });

    await test.step('the side pane opens on Connections next to the session', async () => {
      await expect(pane(page)).toBeVisible({ timeout: 20_000 });
      await expect(pane(page)).toHaveAttribute('data-module', 'connections');
      await expect(page.getByTestId('side-pane-cover')).toHaveCount(0, { timeout: 30_000 });
      await expect(frame(page).getByTestId('page-header')).toContainText('Connections', { timeout: 15_000 });
      // The main pane still shows the session — nothing was taken over.
      expect(await page.evaluate(() => window.location.hash)).toContain(`agents/${sessionId}`);
    });

    let tabId = '';
    await test.step('ui_db_new_tab → an attributed tab with the statement', async () => {
      const out = await mcp.call('otto_ui_db_new_tab', {
        connection_id: CONN_NAME,
        statement: 'SELECT * FROM orders',
      });
      expect(out.isError, `new_tab: ${out.text}`).toBeFalsy();
      const r = dig(out.json, 'tab_id')!;
      tabId = String(r.tab_id);
      expect(tabId).toMatch(/^\d+$/);
      expect(r.connection_id).toBe(connId);
      const tab = frame(page).locator(`.qe-tab[data-db-tab-id="${tabId}"]`);
      await expect(tab).toBeVisible();
      await expect(tab).toHaveClass(/active/);
      await expect(tab.locator('.qe-tab-agent')).toBeVisible();
      await expect(tab.locator('.qe-tab-agent')).toHaveAttribute('aria-label', /uictl-agent/);
      await expect(frame(page).locator('.qe-edit .cm-content')).toContainText('SELECT * FROM orders');
      // The driving bar names who is driving, with a Stop.
      await expect(frame(page).getByTestId('agent-driving-bar')).toContainText('uictl-agent');
    });

    await test.step('ui_db_run_query → rows in the grid AND in the tool result (read-only)', async () => {
      const out = await mcp.call('otto_ui_db_run_query', { tab_id: tabId, row_limit: 100 });
      expect(out.isError, `run_query: ${out.text}`).toBeFalsy();
      const r = dig(out.json, 'rows')!;
      expect(dig(out.json, 'ui_visible')?.ui_visible).toBe(true);
      expect(r.columns.map((c: { name: string }) => c.name)).toEqual(expect.arrayContaining(['id', 'status', 'total']));
      expect(r.rows).toHaveLength(100);
      expect(r.rows[0][0]).toBe(1);
      expect(r.has_next).toBe(true);
      expect(r.page).toBe(1);
      // Visible: the grid in the pane shows the rows, the pager the range.
      await expect(frame(page).locator('.grid')).toBeVisible();
      await expect(frame(page).locator('.grid')).toContainText('eu-west');
      await expect(frame(page).locator('.pg-range')).toContainText('1–100');
      // The agent's run went out read-only.
      const run = seen.filter((q) => /from orders/i.test(q.statement));
      expect(run.length).toBeGreaterThan(0);
      expect(run.every((q) => q.read_only)).toBe(true);
    });

    await test.step('ui_db_page → the next page, visibly', async () => {
      const out = await mcp.call('otto_ui_db_page', { tab_id: tabId, delta: 1 });
      expect(out.isError, `page: ${out.text}`).toBeFalsy();
      const r = dig(out.json, 'rows')!;
      expect(r.rows[0][0]).toBe(101);
      expect(r.page).toBe(2);
      await expect(frame(page).locator('.pg-range')).toContainText('101–200');
      await expect(frame(page).getByRole('button', { name: 'Previous page' })).toBeEnabled();
    });

    await test.step('ui_db_set_view json → the JSON view', async () => {
      const out = await mcp.call('otto_ui_db_set_view', { tab_id: tabId, view: 'json' });
      expect(out.isError, `set_view: ${out.text}`).toBeFalsy();
      expect(dig(out.json, 'view')?.view).toBe('json');
      await expect(frame(page).locator('.jrec').first()).toBeVisible();
      // …and back, so the rest of the flow reads the grid.
      await mcp.call('otto_ui_db_set_view', { tab_id: tabId, view: 'grid' });
      await expect(frame(page).locator('.grid')).toBeVisible();
    });

    await test.step('a write asks the person first; Cancel → cancelled_by_user', async () => {
      const before = seen.length;
      const write = mcp.call('otto_ui_db_run_query', {
        tab_id: tabId,
        statement: "UPDATE orders SET status = 'refunded' WHERE id = 1",
      });
      const dialog = frame(page).getByRole('dialog');
      await expect(dialog).toBeVisible({ timeout: 20_000 });
      await expect(dialog).toContainText('uictl-agent');
      await expect(dialog).toContainText(CONN_NAME);
      // The "allow for this session" memory is offered on an unguarded connection.
      await expect(dialog.getByText(`Allow writes on ${CONN_NAME} for this session`)).toBeVisible();
      await dialog.getByRole('button', { name: 'Cancel' }).click();
      const out = await write;
      expect(out.isError).toBe(true);
      expect(out.text).toContain('cancelled_by_user');
      // Only the refused read-only attempt reached the engine — no real write.
      const writes = seen.slice(before).filter((q) => /^update/i.test(q.statement));
      expect(writes).toHaveLength(1);
      expect(writes[0]!.read_only).toBe(true);
    });

    await test.step('Stop on the driving bar revokes the grant → pending_grant', async () => {
      const bar = frame(page).getByTestId('agent-driving-bar');
      await expect(bar).toBeVisible();
      await bar.getByTestId('agent-driving-stop').click();
      const out = await mcp.call('otto_ui_db_list_connections', {}, 60_000);
      expect(out.isError).toBe(true);
      expect(out.text).toContain('pending_grant');
      // Stop was the person's answer: no re-prompt nag right after it (the
      // daemon keeps quiet for a while), and the bar is gone.
      await page.waitForTimeout(2_000);
      await expect(page.getByTestId('ui-control-request')).toHaveCount(0);
      await expect(frame(page).getByTestId('agent-driving-bar')).toHaveCount(0);
    });
  } finally {
    mcp.kill();
  }
});

test('Deny keeps the agent out: pending_grant, no grant stored, the prompt goes away', async ({ page }) => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  const other = await postJson<{ id: string }>(ctx, `${base}/api/v1/workspaces/${ws}/sessions`, {
    kind: 'agent',
    provider: 'shell',
    title: 'uictl-denied',
    cwd: root,
    meta: { client_id: CLIENT_ID },
  });
  const token = await readSessionToken(ctx, base, other.id);
  await page.goto(`/#/agents/${other.id}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 30_000 });
  const mcp = await startMcp(other.id, token);
  try {
    const first = await firstUngranted(mcp, 'otto_ui_db_list_connections', {});
    expect(first.isError).toBe(true);
    expect(first.text).toContain('pending_grant');
    const prompt = page.getByTestId('ui-control-request');
    await expect(prompt).toBeVisible({ timeout: 20_000 });
    await expect(prompt).toContainText('wants to drive Otto');
    await prompt.getByTestId('ui-control-deny').click();
    await expect(prompt).toHaveCount(0);
    // Nothing was granted, the agent is still out, and nothing opened.
    const got = await ctx.get(`${base}/api/v1/sessions/${other.id}`);
    const meta = ((await got.json()) as { meta?: Record<string, any> }).meta ?? {};
    expect(meta.ui_control?.enabled ?? false).toBe(false);
    const again = await mcp.call('otto_ui_db_list_connections', {});
    expect(again.isError).toBe(true);
    expect(again.text).toContain('pending_grant');
    await expect(page.getByTestId('side-pane')).toHaveCount(0);
    // A Deny is remembered on this device: no second prompt.
    await page.waitForTimeout(1_500);
    await expect(prompt).toHaveCount(0);
  } finally {
    mcp.kill();
    await ctx.dispose().catch(() => {});
  }
});

test('with no Otto window open, reads fall back headless and UI-only commands refuse', async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  // Re-grant as the human (the route refuses the session's own token).
  const agentTry = await ctx.post(`${base}/api/v1/sessions/${sessionId}/ui-control`, {
    data: { enabled: true },
    headers: { Authorization: `Bearer ${sessionToken}` },
  });
  expect(agentTry.status(), 'an agent must not grant itself UI control').toBe(403);
  await postJson(ctx, `${base}/api/v1/sessions/${sessionId}/ui-control`, { enabled: true });
  await ctx.dispose().catch(() => {});

  const mcp = await startMcp();
  try {
    const listed = await mcp.call('otto_ui_db_list_connections', {});
    expect(listed.isError, `headless list: ${listed.text}`).toBeFalsy();
    expect(dig(listed.json, 'ui_visible')?.ui_visible).toBe(false);
    expect(listed.text).toContain(CONN_NAME);

    // A UI-only command has no headless twin.
    const tab = await mcp.call('otto_ui_db_new_tab', { connection_id: CONN_NAME });
    expect(tab.isError).toBe(true);
    expect(tab.text).toContain('no_ui_client');

    // A read runs headless through the daemon's read-only MCP path. The mock
    // connection points at a closed port, so an engine error proves the
    // headless path ran (not `no_ui_client` / `pending_grant`).
    const run = await mcp.call('otto_ui_db_run_query', { connection_id: CONN_NAME, statement: 'SELECT 1' });
    expect(run.text).not.toContain('no_ui_client');
    expect(run.text).not.toContain('pending_grant');

    // …and against a REAL engine when one is at hand: a throwaway Redis.
    const redis = startRedis();
    test.info().annotations.push({
      type: 'headless-redis',
      description: redis ? `redis-server on :${redis.port}` : 'redis-server not installed — skipped',
    });
    if (redis) {
      try {
        const { ctx: c2, base: b2 } = await apiCtx();
        const r = await c2.post(`${b2}/api/v1/workspaces/${ws}/connections`, {
          data: {
            name: REDIS_NAME,
            kind: 'redis',
            params: { host: '127.0.0.1', port: redis.port, db: 0 },
            environment: 'dev',
            read_only: false,
          },
        });
        expect(r.ok(), `seed redis connection → ${r.status()} ${await r.text()}`).toBeTruthy();
        await c2.dispose().catch(() => {});
        const got = await mcp.call('otto_ui_db_run_query', { connection_id: REDIS_NAME, statement: 'GET agent:greeting' });
        expect(got.isError, `headless redis: ${got.text}`).toBeFalsy();
        expect(dig(got.json, 'ui_visible')?.ui_visible).toBe(false);
        expect(got.text).toContain('hello from redis');
        // Headless is read-only: a write is refused outright (no human there).
        const set = await mcp.call('otto_ui_db_run_query', { connection_id: REDIS_NAME, statement: 'SET agent:greeting x' });
        expect(set.isError).toBe(true);
        expect(execFileSync(REDIS_CLI, ['-p', String(redis.port), 'GET', 'agent:greeting']).toString().trim()).toBe(
          'hello from redis',
        );
      } finally {
        redis.proc.kill('SIGKILL');
      }
    }
  } finally {
    mcp.kill();
  }
});

const REDIS_SERVER = ['/opt/homebrew/bin/redis-server', '/usr/local/bin/redis-server'].find((p) => existsSync(p));
const REDIS_CLI = ['/opt/homebrew/bin/redis-cli', '/usr/local/bin/redis-cli'].find((p) => existsSync(p)) ?? 'redis-cli';

/** A throwaway, persistence-free redis-server on a free port with one key, or
 *  null when Redis isn't installed on this machine. */
function startRedis(): { proc: ChildProcess; port: number } | null {
  if (!REDIS_SERVER) return null;
  const port = 26000 + Math.floor(Math.random() * 4000);
  const proc = spawn(REDIS_SERVER, ['--port', String(port), '--save', '', '--appendonly', 'no', '--bind', '127.0.0.1'], {
    stdio: 'ignore',
  });
  for (let i = 0; i < 50; i++) {
    try {
      execFileSync(REDIS_CLI, ['-p', String(port), 'SET', 'agent:greeting', 'hello from redis'], { stdio: 'pipe' });
      return { proc, port };
    } catch {
      execFileSync('sleep', ['0.1']);
    }
  }
  proc.kill('SIGKILL');
  return null;
}
