import { test, expect, type APIRequestContext } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import * as readline from 'node:readline';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { openPage } from './helpers';

// API Client through MCP: route contracts, inward/outward catalogs, and the
// agent source marker in the live History UI. Desktop-browser project only.

const SLOT = process.env.OTTO_E2E_SLOT ?? '0';
const API_TOOLS = [
  'otto_api_list',
  'otto_api_get_request',
  'otto_api_history',
  'otto_api_execute',
  'otto_api_upsert_request',
  'otto_api_run_automation',
] as const;

let ws = '';
let root = '';
let requestId = '';

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

async function postJson<T>(ctx: APIRequestContext, url: string, data: unknown): Promise<T> {
  const response = await ctx.post(url, { data });
  expect(
    response.ok(),
    `POST ${url} → ${response.status()} ${await response.text()}`,
  ).toBeTruthy();
  return response.json() as Promise<T>;
}

/** Environment for the inward `ottod mcp-tools` bridge.
 *
 * Playwright itself is usually launched from INSIDE an Otto agent session, whose
 * `OTTO_MCP_BASE` / `OTTO_MCP_TOKEN` / `OTTO_WORKSPACE_ID` point at the developer's
 * REAL daemon on 7700. Inheriting them (`...process.env`) silently drives the
 * bridge against that daemon instead of this run's isolated one — routes added in
 * this tree come back "no such route", and none of the seeded data is visible. So
 * drop every inherited `OTTO_*` var and hand the bridge only OUR routing.
 */
function bridgeEnv(routing: Record<string, string>): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (!key.startsWith('OTTO_')) env[key] = value;
  }
  return { ...env, OTTO_SECRETS: process.env.OTTO_SECRETS ?? 'file', ...routing };
}

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  root = mkdtempSync(join(tmpdir(), 'otto-apimcp-'));
  const workspace = await postJson<{ id: string }>(ctx, `${base}/api/v1/workspaces`, {
    name: 'API MCP E2E',
    root_path: root,
  });
  ws = workspace.id;
  const api = `${base}/api/v1/workspaces/${ws}/api-client`;

  const saved = await postJson<{ id: string }>(ctx, `${api}/requests`, {
    name: 'Alpha health',
    method: 'GET',
    url: 'http://alpha.e2e-nowhere.invalid/healthz',
  });
  requestId = saved.id;

  const session = await postJson<{ id: string }>(
    ctx,
    `${base}/api/v1/workspaces/${ws}/sessions`,
    { kind: 'agent', provider: 'shell', title: 'apimcp', cwd: root, meta: {} },
  );

  // Both offline calls fail fast, but each failure must still write history.
  const agentRun = await ctx.post(`${api}/requests/${requestId}/execute`, {
    headers: { 'X-Otto-Session': session.id },
    data: { shape: 'agent', timeout_ms: 2000 },
  });
  expect(
    agentRun.status(),
    `saved request execute → ${agentRun.status()} ${await agentRun.text()}`,
  ).toBe(502);
  await ctx.post(`${api}/execute`, {
    data: {
      method: 'GET',
      url: 'http://beta.e2e-nowhere.invalid/x',
      timeout_ms: 2000,
    },
  });
  await ctx.dispose().catch(() => {});
});

test.beforeEach(async ({ page }, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.addInitScript((workspaceId) => {
    localStorage.setItem('otto_workspace', workspaceId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);
});

test('saved-request execute records an agent-sourced history row', async () => {
  const { ctx, base } = await apiCtx();
  const api = `${base}/api/v1/workspaces/${ws}/api-client`;
  try {
    const historyResponse = await ctx.get(`${api}/history?source=agent`);
    expect(
      historyResponse.ok(),
      `agent history → ${historyResponse.status()} ${await historyResponse.text()}`,
    ).toBeTruthy();
    const history = (await historyResponse.json()) as {
      id: string;
      request: { request_id?: string | null; source?: { kind?: string } };
    }[];
    expect(history).toHaveLength(1);
    expect(history[0]?.request.source?.kind).toBe('agent');
    expect(history[0]?.request.request_id).toBe(requestId);

    const detail = await ctx.get(`${api}/history/${history[0]!.id}`);
    expect(detail.status(), `history detail → ${detail.status()}`).toBe(200);

    const overviewResponse = await ctx.get(`${api}/overview?q=alpha`);
    expect(
      overviewResponse.ok(),
      `API overview → ${overviewResponse.status()} ${await overviewResponse.text()}`,
    ).toBeTruthy();
    const overview = (await overviewResponse.json()) as {
      requests: { name: string; agent_authored: boolean }[];
    };
    expect(overview.requests[0]?.name).toBe('Alpha health');
    expect(overview.requests[0]?.agent_authored).toBe(false);
  } finally {
    await ctx.dispose().catch(() => {});
  }
});

test('inward mcp-tools advertises the API tools and lists the seeded request', async () => {
  test.setTimeout(120_000);
  const { ctx, base, token } = await apiCtx();
  const { dataDir } = daemonMeta();

  // Its OWN workspace: the bridge's `otto_api_execute` really does write a
  // history row, and the row-count specs in this file must not see it whichever
  // way Playwright schedules the tests across workers.
  const bridgeRoot = mkdtempSync(join(tmpdir(), 'otto-apimcp-bridge-'));
  const workspace = await postJson<{ id: string }>(ctx, `${base}/api/v1/workspaces`, {
    name: 'API MCP Bridge',
    root_path: bridgeRoot,
  });
  await postJson(ctx, `${base}/api/v1/workspaces/${workspace.id}/api-client/requests`, {
    name: 'Alpha health',
    method: 'GET',
    url: 'http://alpha.e2e-nowhere.invalid/healthz',
  });
  // Spawning an agent session is what renders the workspace `.mcp.json`.
  const session = await postJson<{ id: string }>(
    ctx,
    `${base}/api/v1/workspaces/${workspace.id}/sessions`,
    { kind: 'agent', provider: 'shell', title: 'apimcp-bridge', cwd: bridgeRoot, meta: {} },
  );
  await ctx.dispose().catch(() => {});

  const mcpDoc = JSON.parse(readFileSync(join(bridgeRoot, '.mcp.json'), 'utf8')) as {
    mcpServers?: Record<string, { command: string; args: string[]; env?: Record<string, string> }>;
  };
  const otto = mcpDoc.mcpServers?.otto;
  expect(otto, '.mcp.json must contain the otto MCP server').toBeTruthy();

  // `.mcp.json` is deliberately identity-neutral (command/args only — the daemon
  // never persists a session token into a file several sessions share), so the
  // routing env is ours to supply. Run the binary under test and point it at the
  // isolated e2e daemon with a token/workspace that exist THERE.
  const mcp = new McpStdio(
    process.env.OTTO_E2E_BIN ?? otto!.command,
    ['mcp-tools'],
    bridgeEnv({
      OTTO_MCP_BASE: base,
      OTTO_MCP_TOKEN: token,
      OTTO_SESSION_ID: session.id,
      OTTO_WORKSPACE_ID: workspace.id,
      OTTO_DATA_DIR: dataDir,
    }),
  );
  try {
    await mcp.request('initialize', {
      protocolVersion: '2024-11-05',
      capabilities: {},
      clientInfo: { name: 'e2e', version: '1' },
    });
    mcp.notify('notifications/initialized');

    const listed = await mcp.request('tools/list');
    const names = ((listed.result as { tools: { name: string }[] }).tools ?? []).map((t) => t.name);
    for (const name of API_TOOLS) {
      expect(names, `tools/list must advertise ${name}`).toContain(name);
    }

    const listCall = await mcp.request('tools/call', {
      name: 'otto_api_list',
      arguments: { q: 'alpha' },
    });
    expect(
      McpStdio.isError(listCall),
      `otto_api_list errored: ${McpStdio.rawText(listCall)}`,
    ).toBeFalsy();
    expect(McpStdio.rawText(listCall)).toContain('Alpha health');

    const executeCall = await mcp.request('tools/call', {
      name: 'otto_api_execute',
      arguments: { name: 'Alpha health' },
    });
    const executeText = McpStdio.rawText(executeCall);
    expect(executeText).toMatch(/daemon returned|502/i);
    expect(executeText).not.toContain('request_id or name');
  } finally {
    mcp.kill();
  }
});

test('outward catalog lists the API tools', async () => {
  const { ctx, base } = await apiCtx();
  try {
    const response = await ctx.get(`${base}/api/v1/mcp/otto-server`);
    expect(
      response.ok(),
      `Otto MCP catalog → ${response.status()} ${await response.text()}`,
    ).toBeTruthy();
    const catalog = (await response.json()) as {
      tools: { name: string; mutating: boolean; category?: string }[];
    };
    expect(catalog.tools).toContainEqual(
      expect.objectContaining({
        name: 'otto.api_list',
        mutating: false,
        category: 'API Client',
      }),
    );
    expect(catalog.tools).toContainEqual(
      expect.objectContaining({
        name: 'otto.api_execute',
        mutating: true,
        category: 'API Client',
      }),
    );
  } finally {
    await ctx.dispose().catch(() => {});
  }
});

test('history shows the agent chip and the agent filter', async ({ page }) => {
  await openPage(page, 'api');
  await page.getByRole('tab', { name: 'History' }).click();
  const rows = page.locator('.hist-row');
  await expect(rows).toHaveCount(2);
  await expect(page.locator('.hist-row .src-chip.agent')).toHaveCount(1);

  await page.getByLabel('Show only agent runs').click();
  await expect(rows).toHaveCount(1);
  await page.getByLabel('Show only agent runs').click();
  await expect(rows).toHaveCount(2);
});
