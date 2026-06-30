// End-to-end for "expose all Otto features over the otto MCP server". Three real
// flows against the isolated test daemon (which MUST be the freshly-built ottod
// from THIS worktree so the expanded otto.* / otto_* tools exist):
//
//   1. Governed OUTWARD path: enable the Otto MCP server + two feature tools, then
//      drive the governed choke point POST /mcp/otto-tools/invoke — a read tool
//      (otto.list_workflows) executes and returns a seeded workflow; a dangerous
//      write tool (otto.run_workflow) is blocked on a pending approval.
//   2. Per-session INWARD path: a shell agent session writes .mcp.json; the REAL
//      `ottod mcp-tools` subprocess advertises the new read tools over stdio and
//      otto_list_workflows returns the seeded workflow.
//   3. UI: the Otto Server control-plane tab lists the new tools, grouped by
//      category, with a working filter.
//
// Runs once (gated to a single Playwright project) so we don't 5× the work.

import { test, expect } from '@playwright/test';
import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import * as readline from 'node:readline';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

const SLOT = process.env.OTTO_E2E_SLOT ?? '0';

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

test.beforeEach(({}, testInfo) => {
  test.skip(testInfo.project.name !== 'iphone-portrait', 'otto-features MCP runs once');
});

test('governed outward: a feature read executes, a feature write needs approval', async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();

  // A workspace + a workflow to read back through the tool.
  const root = mkdtempSync(join(tmpdir(), 'otto-feat-'));
  const ws = (await (await ctx.post(`${base}/api/v1/workspaces`, {
    data: { name: 'OttoMCP Features', root_path: root },
  })).json()).id as string;
  const wf = (await (await ctx.post(`${base}/api/v1/workspaces/${ws}/workflows`, {
    data: { name: 'E2E Feature Flow' },
  })).json()) as { id: string };
  expect(wf.id, 'workflow created').toBeTruthy();

  // Enable the outward server with exactly two tools: one read, one dangerous write.
  const patch = await ctx.patch(`${base}/api/v1/mcp/otto-server`, {
    data: { enabled: true, tools: ['otto.list_workflows', 'otto.run_workflow'] },
  });
  expect(patch.ok(), `enable otto-server → ${patch.status()} ${await patch.text()}`).toBeTruthy();

  // Status reflects the new tools + their feature category.
  const status = (await (await ctx.get(`${base}/api/v1/mcp/otto-server`)).json()) as {
    enabled: boolean;
    tools: { name: string; enabled: boolean; category?: string; mutating: boolean }[];
  };
  expect(status.enabled).toBe(true);
  const lw = status.tools.find((t) => t.name === 'otto.list_workflows')!;
  expect(lw.enabled, 'list_workflows enabled').toBe(true);
  expect(lw.category, 'list_workflows is categorised').toBe('Workflows');
  const rw = status.tools.find((t) => t.name === 'otto.run_workflow')!;
  expect(rw.mutating, 'run_workflow is mutating').toBe(true);

  // Read tool: allowed + executed; returns the seeded workflow.
  const inv1 = await ctx.post(`${base}/api/v1/mcp/otto-tools/invoke`, {
    data: { tool: 'otto.list_workflows', arguments: { workspace_id: ws } },
  });
  expect(inv1.ok(), `invoke list_workflows → ${inv1.status()} ${await inv1.text()}`).toBeTruthy();
  const r1 = (await inv1.json()) as { decision: string; executed: boolean; content: unknown };
  expect(r1.decision).toBe('allowed');
  expect(r1.executed).toBe(true);
  expect(JSON.stringify(r1.content), 'workflow list contains the seeded workflow').toContain(wf.id);

  // Dangerous write tool: blocked on a pending approval (default approval-on).
  const inv2 = await ctx.post(`${base}/api/v1/mcp/otto-tools/invoke`, {
    data: { tool: 'otto.run_workflow', arguments: { workflow_id: wf.id } },
  });
  expect(inv2.ok()).toBeTruthy();
  const r2 = (await inv2.json()) as { decision: string; executed: boolean; approval_id?: string };
  expect(r2.decision).toBe('pending_approval');
  expect(r2.executed).toBe(false);
  expect(r2.approval_id, 'pending invoke returns an approval id').toBeTruthy();

  await ctx.dispose();
});

test('per-session mcp-tools advertises + returns the new feature reads', async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  const { dataDir } = daemonMeta();

  const root = mkdtempSync(join(tmpdir(), 'otto-feat-ps-'));
  const ws = (await (await ctx.post(`${base}/api/v1/workspaces`, {
    data: { name: 'OttoMCP PerSession', root_path: root },
  })).json()).id as string;
  const wf = (await (await ctx.post(`${base}/api/v1/workspaces/${ws}/workflows`, {
    data: { name: 'PS Feature Flow' },
  })).json()) as { id: string };

  // A shell agent session triggers Otto to write the workspace `.mcp.json`.
  const sess = await ctx.post(`${base}/api/v1/workspaces/${ws}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title: 'psfeat', cwd: root, meta: {} },
  });
  expect(sess.ok(), `create session → ${sess.status()} ${await sess.text()}`).toBeTruthy();

  const mcpDoc = JSON.parse(readFileSync(join(root, '.mcp.json'), 'utf8')) as {
    mcpServers?: Record<string, { command: string; args: string[]; env?: Record<string, string> }>;
  };
  const otto = mcpDoc.mcpServers?.otto;
  expect(otto, '.mcp.json must contain the otto MCP server').toBeTruthy();

  const mcp = new McpStdio(otto!.command, otto!.args, {
    ...process.env,
    ...otto!.env,
    OTTO_DATA_DIR: dataDir,
  });
  try {
    await mcp.request('initialize', {
      protocolVersion: '2024-11-05',
      capabilities: {},
      clientInfo: { name: 'e2e', version: '1' },
    });
    mcp.notify('notifications/initialized');

    const listed = await mcp.request('tools/list');
    const names = ((listed.result as { tools: { name: string }[] }).tools ?? []).map((t) => t.name);
    for (const t of [
      'otto_list_workflows',
      'otto_list_broker_clusters',
      'otto_search_memory',
      'otto_list_findings',
      'otto_list_improvement_edits',
    ]) {
      expect(names, `tools/list must advertise ${t}`).toContain(t);
    }

    const call = await mcp.request('tools/call', { name: 'otto_list_workflows', arguments: {} });
    expect(McpStdio.isError(call), `otto_list_workflows errored: ${McpStdio.rawText(call)}`).toBeFalsy();
    expect(McpStdio.rawText(call), 'returns the seeded workflow').toContain(wf.id);
  } finally {
    mcp.kill();
  }

  await ctx.dispose();
});

test('Otto Server tab lists the new tools, grouped + filterable', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  // Ensure the catalog is populated + a workspace is selected (the page gates on one).
  const ws = (await (await ctx.post(`${base}/api/v1/workspaces`, {
    data: { name: 'OttoMCP UI', root_path: mkdtempSync(join(tmpdir(), 'otto-feat-ui-')) },
  })).json()).id as string;
  await ctx.dispose();

  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);

  await page.goto('/#/mcp');
  await expect(page.locator('.mcp-page')).toBeVisible({ timeout: 30_000 });

  // Open the Otto Server tab.
  await page.locator('.tabs button', { hasText: 'Otto Server' }).click();

  // The filterable, grouped checklist renders the new feature tools.
  const filter = page.locator('.otto .filter');
  await expect(filter).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.otto .grp-name', { hasText: 'Workflows' })).toBeVisible();
  await expect(page.locator('.otto .t-name', { hasText: 'otto.list_workflows' })).toBeVisible();

  // Filtering narrows the list to message-broker tools.
  await filter.fill('broker');
  await expect(page.locator('.otto .t-name', { hasText: 'otto.list_broker_clusters' })).toBeVisible();
  await expect(page.locator('.otto .t-name', { hasText: 'otto.list_workflows' })).toHaveCount(0);
});
