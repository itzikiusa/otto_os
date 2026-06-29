// Full end-to-end test for "Connections MCP": every agent session can discover
// the user's DB connections and run READ-ONLY queries via the first-party `otto`
// MCP server. This spec exercises the real wiring, not mocks:
//
//   1. A `shell` agent session writes the workspace `.mcp.json` with the `otto`
//      server (proves the server is attached to a session).
//   2. We spawn the REAL `ottod mcp-tools` subprocess with that server's env and
//      speak JSON-RPC over stdio: initialize → tools/list → tools/call.
//   3. Against a REAL Redis (seeded by the harness): otto_list_connections returns
//      the connection, otto_db_query returns real rows, and a write is REFUSED.
//   4. The /db/mcp-query HTTP endpoint rejects a write with 403 `mcp_read_only`.
//   5. The settings UI shows the Connections MCP toggle.
//
// Runs once (gated to a single Playwright project) so it spins exactly one Redis.

import { test, expect } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import * as readline from 'node:readline';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedRedis } from './seed';

const SLOT = process.env.OTTO_E2E_SLOT ?? '0';

/** {dataDir, port} the test daemon was launched with (written by global-setup). */
function daemonMeta(): { dataDir: string; port: string } {
  const p = join(process.cwd(), 'e2e', `.auth-${SLOT}`, 'daemon.json');
  return JSON.parse(readFileSync(p, 'utf8')) as { dataDir: string; port: string };
}

/** A minimal newline-delimited JSON-RPC 2.0 stdio client for an MCP server. */
class McpStdio {
  private proc: ChildProcessWithoutNullStreams;
  private rl: readline.Interface;
  private pending = new Map<number, (msg: Record<string, unknown>) => void>();
  private nextId = 1;

  constructor(command: string, args: string[], env: NodeJS.ProcessEnv) {
    this.proc = spawn(command, args, { env, stdio: ['pipe', 'pipe', 'pipe'] });
    // stdout carries the protocol; stderr is logs we ignore.
    this.proc.stderr.on('data', () => {});
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

  request(method: string, params?: unknown): Promise<Record<string, unknown>> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`mcp ${method} timed out`));
      }, 20_000);
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

  /** Parse a tools/call result's single text-content block as JSON. */
  static toolJson(resp: Record<string, unknown>): unknown {
    const result = resp.result as { content?: { text?: string }[] } | undefined;
    const text = result?.content?.[0]?.text ?? '{}';
    return JSON.parse(text);
  }

  static isError(resp: Record<string, unknown>): boolean {
    return Boolean((resp.result as { isError?: boolean } | undefined)?.isError);
  }

  static rawText(resp: Record<string, unknown>): string {
    const result = resp.result as { content?: { text?: string }[] } | undefined;
    return result?.content?.[0]?.text ?? '';
  }

  kill(): void {
    try {
      this.proc.kill('SIGKILL');
    } catch {
      /* ignore */
    }
  }
}

test.beforeEach(({}, testInfo) => {
  // The DB-driving part spins a real Redis on a fixed port; run it under exactly
  // one project so we don't launch five Redis servers in parallel.
  test.skip(testInfo.project.name !== 'iphone-portrait', 'connections-mcp runs once');
});

test('Connections MCP: real otto MCP server queries a live DB read-only', async ({ page }, testInfo) => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  const { dataDir } = daemonMeta();

  // A workspace with a known on-disk root so we can read its `.mcp.json`.
  const root = mkdtempSync(join(tmpdir(), 'otto-cmcp-ws-'));
  const wsRes = await ctx.post(`${base}/api/v1/workspaces`, {
    data: { name: 'Connections MCP E2E', root_path: root },
  });
  expect(wsRes.ok(), `create workspace → ${wsRes.status()}`).toBeTruthy();
  const wsId = (await wsRes.json()).id as string;

  // A real Redis + a redis connection (skips the live-query asserts if the host
  // has no redis-server). Port keyed off the E2E slot + worker so parallel runs
  // (other agents / projects) never collide.
  const redisPort =
    6400 + Number(process.env.OTTO_E2E_SLOT ?? '0') * 4 + (testInfo.workerIndex ?? 0);
  const redis = await seedRedis(ctx, base, wsId, redisPort);

  // A shell agent session triggers Otto to write the workspace `.mcp.json` with
  // the `otto` server (default-on). Shell spawns cleanly in the E2E daemon.
  const sessRes = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'cmcp', cwd: root, meta: {} },
  });
  expect(sessRes.ok(), `create session → ${sessRes.status()} ${await sessRes.text()}`).toBeTruthy();

  // ---- (1) the otto server is attached to the session ----------------------
  // The .mcp.json is written synchronously during session create.
  const mcpDoc = JSON.parse(readFileSync(join(root, '.mcp.json'), 'utf8')) as {
    mcpServers?: Record<string, { command: string; args: string[]; env?: Record<string, string> }>;
  };
  const otto = mcpDoc.mcpServers?.otto;
  expect(otto, '.mcp.json must contain the otto MCP server').toBeTruthy();
  expect(otto!.args).toEqual(['mcp-tools']);
  expect(otto!.env?.OTTO_MCP_TOKEN, 'otto env must carry a per-session token').toBeTruthy();
  expect(otto!.env?.OTTO_WORKSPACE_ID).toBe(wsId);

  // ---- (2)+(3) drive the REAL mcp-tools server over stdio ------------------
  const mcp = new McpStdio(otto!.command, otto!.args, {
    ...process.env,
    ...otto!.env,
    // Point audit at the test DB, never the user's real data dir.
    OTTO_DATA_DIR: dataDir,
  });
  try {
    const init = await mcp.request('initialize', {
      protocolVersion: '2024-11-05',
      capabilities: {},
      clientInfo: { name: 'e2e', version: '1' },
    });
    expect((init.result as { serverInfo?: { name?: string } }).serverInfo?.name).toBe('otto');
    mcp.notify('notifications/initialized');

    const listed = await mcp.request('tools/list');
    const names = ((listed.result as { tools: { name: string }[] }).tools ?? []).map((t) => t.name);
    for (const t of [
      'otto_list_connections',
      'otto_db_schema',
      'otto_db_children',
      'otto_db_object',
      'otto_db_query',
    ]) {
      expect(names, `tools/list must advertise ${t}`).toContain(t);
    }

    if (redis) {
      // Discovery: the seeded redis connection is listed.
      const lc = await mcp.request('tools/call', {
        name: 'otto_list_connections',
        arguments: {},
      });
      expect(McpStdio.isError(lc)).toBeFalsy();
      const conns = (McpStdio.toolJson(lc) as { connections: { id: string; kind: string }[] })
        .connections;
      expect(conns.map((c) => c.id)).toContain(redis.connId);

      // Read: a real value comes back from the live Redis.
      const q = await mcp.request('tools/call', {
        name: 'otto_db_query',
        arguments: { connection_id: redis.connId, statement: 'GET e2e:key:0' },
      });
      expect(McpStdio.isError(q), `read query errored: ${McpStdio.rawText(q)}`).toBeFalsy();
      expect(McpStdio.rawText(q)).toContain('value-0');

      // Write: refused server-side (read-only enforcement over the real engine).
      const w = await mcp.request('tools/call', {
        name: 'otto_db_query',
        arguments: { connection_id: redis.connId, statement: 'SET e2e:key:0 HACKED' },
      });
      expect(McpStdio.isError(w), 'a write over MCP must be reported as an error').toBeTruthy();
      expect(McpStdio.rawText(w).toLowerCase()).toContain('mcp_read_only');

      // And the write truly did NOT take effect.
      const after = await mcp.request('tools/call', {
        name: 'otto_db_query',
        arguments: { connection_id: redis.connId, statement: 'GET e2e:key:0' },
      });
      expect(McpStdio.rawText(after)).toContain('value-0');
      // eslint-disable-next-line no-console
      console.log('[cmcp] live Redis: read OK, write refused, value intact ✓');
    } else {
      // eslint-disable-next-line no-console
      console.log('[cmcp] redis-server not available — skipped live-query asserts');
    }
  } finally {
    mcp.kill();
    redis?.proc.kill('SIGKILL');
  }

  // ---- (4) HTTP endpoint refuses a write with 403 mcp_read_only ------------
  const conn = await ctx.post(`${base}/api/v1/workspaces/${wsId}/connections`, {
    data: {
      name: 'cmcp-mysql-guard',
      kind: 'mysql',
      params: { host: '127.0.0.1', port: 1, user: 'x', db: 'x' },
      secret: null,
      environment: 'dev',
      read_only: false,
    },
  });
  const connId = (await conn.json()).id as string;
  const drop = await ctx.post(`${base}/api/v1/connections/${connId}/db/mcp-query`, {
    data: { statement: 'DROP TABLE t' },
  });
  expect(drop.status(), 'a write must be rejected before connecting').toBe(403);
  expect(((await drop.json()) as { message: string }).message).toContain('mcp_read_only');

  // A read passes the gate (then fails at connect — but NOT with a 403/read-only).
  let selStatus = -1;
  try {
    const sel = await ctx.post(`${base}/api/v1/connections/${connId}/db/mcp-query`, {
      data: { statement: 'SELECT 1' },
      timeout: 8_000,
    });
    selStatus = sel.status();
  } catch {
    selStatus = -1; // still connecting after 8s ⇒ it passed the read-only gate
  }
  expect(selStatus, 'a SELECT must not be refused as a write').not.toBe(403);

  // ---- (5) the settings UI shows the toggle --------------------------------
  await page.goto('/#/settings/mcp-servers');
  const card = page.locator('[data-testid="connections-mcp"]');
  await expect(card).toBeVisible({ timeout: 30_000 });
  await expect(card.getByText('Connections MCP')).toBeVisible();
  await expect(card.locator('input[type="checkbox"]')).toBeVisible();
});
