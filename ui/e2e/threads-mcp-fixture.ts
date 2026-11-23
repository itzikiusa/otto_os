import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { enableMcpFixtureCatalog } from './mcp-catalog-fixture';

// MCP in a thread, end to end with the REAL bridge: otto-fake-claude's
// `mcp <server> <tool> <args>` launches `ottod mcp-tools` from its
// --mcp-config and calls an Otto tool through it (see the fake's header).
// One throwaway workspace per spec file; `project` is the one Playwright
// project the file runs on.
export function mcpThreads(project: string) {
  const s = { ctx: null as unknown as APIRequestContext, base: '', wsId: '', dir: '' };
  test.beforeAll(async () => {
    const c = await apiCtx();
    s.ctx = c.ctx;
    s.base = c.base;
    s.dir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-mcp-'));
    writeFileSync(join(s.dir, 'README.md'), '# MCP\n');
    const r = await s.ctx.post(`${s.base}/api/v1/workspaces`, { data: { name: 'MCP Threads WS', root_path: s.dir } });
    expect(r.ok(), await r.text()).toBeTruthy();
    s.wsId = (await r.json()).id as string;
    await enableMcpFixtureCatalog(s.ctx, s.base);
  });
  test.afterAll(async () => {
    if (s.ctx && s.wsId) {
      const rows = (await (await s.ctx.get(`${s.base}/api/v1/workspaces/${s.wsId}/runtime-sessions`)).json()) as { id: string }[];
      for (const r of rows) await s.ctx.post(`${s.base}/api/v1/runtime-sessions/${r.id}/close`).catch(() => {});
    }
    await s.ctx?.dispose();
    if (s.dir) rmSync(s.dir, { recursive: true, force: true });
  });
  test.beforeEach(async ({ page }, info) => {
    test.skip(info.project.name !== project, `${project} project only`);
    await page.addInitScript((id) => {
      if (sessionStorage.getItem('threads-mcp-init')) return;
      sessionStorage.setItem('threads-mcp-init', '1');
      localStorage.setItem('otto_workspace', id as string);
      localStorage.removeItem('otto_threads_v1');
    }, s.wsId);
  });

  /** A Claude thread that sends `mcp <server> <tool> <args>` (args may name
   *  the workspace as `$WS`). */
  async function send(page: Page, title: string, line: string) {
    const created = await s.ctx.post(`${s.base}/api/v1/workspaces/${s.wsId}/runtime-sessions`, {
      data: { provider: 'claude', cwd: s.dir, title },
    });
    expect(created.ok(), await created.text()).toBeTruthy();
    const id = (await created.json()).id as string;
    await page.goto(`/#/threads/${id}`);
    const thread = page.getByTestId(`runtime-thread-${id}`);
    await expect(thread).toBeVisible({ timeout: 30_000 });
    await thread.getByTestId('runtime-composer').fill(line.replaceAll('$WS', s.wsId));
    await thread.getByRole('button', { name: 'Send message' }).click();
    return { id, thread };
  }

  /** `otto_broadcast_message` (approval-gated): the dock that asks for it. */
  async function askBroadcast(page: Page, title: string, text: string) {
    const { id, thread } = await send(page, title, `mcp otto otto_broadcast_message ${JSON.stringify({ workspace_id: '$WS', text })}`);
    const dock = thread.getByTestId('runtime-attention');
    await expect(dock).toBeVisible({ timeout: 30_000 });
    await expect(dock).toHaveAttribute('data-kind', 'approval');
    await expect(dock).toContainText('Claude wants to use otto · broadcast_message');
    await expect(dock).toContainText(`Broadcast a message to live agent sessions: ${text}`);
    return { id, thread, dock };
  }

  return { state: s, send, askBroadcast };
}
