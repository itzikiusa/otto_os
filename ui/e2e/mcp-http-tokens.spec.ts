// End-to-end for "extend the MCP: HTTP transport + multiple scoped tokens".
// Runs against the isolated test daemon (which MUST be the freshly-built ottod
// from THIS worktree so /api/v1/mcp/http + /api/v1/mcp/tokens exist). Three flows:
//
//   1. HTTP TRANSPORT + PER-TOKEN SCOPE: mint scoped MCP tokens (read-only /
//      tool-restricted, and writes-allowed), then drive the Streamable-HTTP MCP
//      endpoint POST /api/v1/mcp/http with each token and prove the scope: a
//      read-only token's tools/list hides mutating tools, a tool-restricted
//      token only sees + can call its tools, an out-of-scope / mutating call is
//      denied "token scope: …", while a writes-allowed token reaches the normal
//      approval gate instead. A bad/blank bearer is 401.
//   2. MULTI-USER: a token minted for a SECOND user authenticates as that user
//      and is listed with the owning username — different users, different access.
//   3. UI: the Otto Server tab surfaces the HTTP URL + the tokens it created.
//
// Gated to a single Playwright project so it runs once.

import { test, expect, request, type APIRequestContext } from '@playwright/test';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

test.beforeEach(({}, testInfo) => {
  test.skip(testInfo.project.name !== 'iphone-portrait', 'mcp-http-tokens runs once');
});

interface RpcResp {
  jsonrpc: string;
  id: unknown;
  result?: Record<string, unknown>;
  error?: { code: number; message: string };
}

/** A request context that authenticates as one specific MCP token. */
async function tokenCtx(token: string): Promise<APIRequestContext> {
  return request.newContext({
    extraHTTPHeaders: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
  });
}

/** One JSON-RPC round trip over the Streamable-HTTP MCP transport. */
async function rpc(
  ctx: APIRequestContext,
  base: string,
  method: string,
  params?: unknown,
): Promise<RpcResp> {
  const r = await ctx.post(`${base}/api/v1/mcp/http`, {
    data: { jsonrpc: '2.0', id: 1, method, ...(params ? { params } : {}) },
  });
  expect(r.ok(), `${method} → ${r.status()} ${await r.text()}`).toBeTruthy();
  return (await r.json()) as RpcResp;
}

/** The text payload of a tools/call result (we encode the governed envelope there). */
function callText(resp: RpcResp): string {
  const content = (resp.result?.content as { type: string; text: string }[]) ?? [];
  return content.map((c) => c.text ?? '').join('\n');
}

function toolNames(resp: RpcResp): string[] {
  return ((resp.result?.tools as { name: string }[]) ?? []).map((t) => t.name);
}

test('HTTP transport enforces each token scope; bad token is 401', async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();

  // A workspace + a workflow to read back / target through the tools.
  const root = mkdtempSync(join(tmpdir(), 'mcp-http-'));
  const ws = (await (await ctx.post(`${base}/api/v1/workspaces`, {
    data: { name: 'MCP HTTP', root_path: root },
  })).json()).id as string;
  const wf = (await (await ctx.post(`${base}/api/v1/workspaces/${ws}/workflows`, {
    data: { name: 'HTTP Flow' },
  })).json()) as { id: string };
  expect(wf.id, 'workflow created').toBeTruthy();

  // Enable the outward server with a read tool, a mutating tool, and a second
  // read tool we will deliberately leave OUT of the restricted token's scope.
  const patch = await ctx.patch(`${base}/api/v1/mcp/otto-server`, {
    data: { enabled: true, tools: ['otto.list_workflows', 'otto.run_workflow', 'otto.list_repos'] },
  });
  expect(patch.ok(), `enable otto-server → ${patch.status()} ${await patch.text()}`).toBeTruthy();

  // --- mint two scoped tokens for root -------------------------------------
  // A: read-only, restricted to exactly list_workflows.
  const tokA = (await (await ctx.post(`${base}/api/v1/mcp/tokens`, {
    data: { label: 'ro-workflows', scope: { tools: ['list_workflows'], allow_writes: false } },
  })).json()) as { token: string; info: { token_prefix: string } };
  expect(tokA.token, 'token A minted').toBeTruthy();
  // B: all tools, writes allowed.
  const tokB = (await (await ctx.post(`${base}/api/v1/mcp/tokens`, {
    data: { label: 'rw-all', scope: { tools: null, allow_writes: true } },
  })).json()) as { token: string };
  expect(tokB.token, 'token B minted').toBeTruthy();

  // --- token A: scope-restricted, read-only --------------------------------
  const ctxA = await tokenCtx(tokA.token);

  const init = await rpc(ctxA, base, 'initialize', {
    protocolVersion: '2025-03-26',
    capabilities: {},
    clientInfo: { name: 'e2e', version: '1' },
  });
  expect((init.result?.serverInfo as { name: string }).name, 'serverInfo.name').toBe('otto');
  expect(init.result?.protocolVersion, 'protocol echoed').toBe('2025-03-26');

  const listA = await rpc(ctxA, base, 'tools/list');
  const namesA = toolNames(listA);
  expect(namesA, 'A sees its one allowed read tool').toContain('otto.list_workflows');
  expect(namesA, 'A (read-only) never sees a mutating tool').not.toContain('otto.run_workflow');
  expect(namesA, 'A does not see a tool outside its allow-list').not.toContain('otto.list_repos');

  // A read tool in scope executes and returns the seeded workflow.
  const okCall = await rpc(ctxA, base, 'tools/call', {
    name: 'otto.list_workflows',
    arguments: { workspace_id: ws },
  });
  expect(okCall.result?.isError, 'list_workflows ok').toBeFalsy();
  expect(callText(okCall), 'returns the seeded workflow').toContain(wf.id);

  // A mutating tool is denied by scope (read-only) — never reaches approval.
  const mutCall = await rpc(ctxA, base, 'tools/call', {
    name: 'otto.run_workflow',
    arguments: { workflow_id: wf.id },
  });
  expect(mutCall.result?.isError, 'run_workflow blocked').toBeTruthy();
  expect(callText(mutCall), 'denied by token scope').toContain('token scope');

  // A tool outside the allow-list is denied by scope too.
  const offCall = await rpc(ctxA, base, 'tools/call', {
    name: 'otto.list_repos',
    arguments: { workspace_id: ws },
  });
  expect(offCall.result?.isError, 'list_repos blocked').toBeTruthy();
  expect(callText(offCall), 'denied: not in allowed set').toContain('token scope');

  await ctxA.dispose();

  // --- token B: all tools, writes allowed ----------------------------------
  const ctxB = await tokenCtx(tokB.token);
  const listB = await rpc(ctxB, base, 'tools/list');
  const namesB = toolNames(listB);
  expect(namesB, 'B sees the mutating tool').toContain('otto.run_workflow');
  expect(namesB, 'B sees read tools too').toContain('otto.list_workflows');

  // The mutating call is NOT a scope denial — it reaches the normal approval gate.
  const bMut = await rpc(ctxB, base, 'tools/call', {
    name: 'otto.run_workflow',
    arguments: { workflow_id: wf.id },
  });
  const bText = callText(bMut);
  expect(bText, 'B is not blocked by scope').not.toContain('token scope');
  expect(bText, 'B reaches the approval gate').toContain('pending_approval');
  await ctxB.dispose();

  // --- a bad / blank bearer is rejected ------------------------------------
  const bad = await tokenCtx('deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef');
  const badResp = await bad.post(`${base}/api/v1/mcp/http`, {
    data: { jsonrpc: '2.0', id: 1, method: 'tools/list' },
  });
  expect(badResp.status(), 'invalid token → 401').toBe(401);
  await bad.dispose();

  await ctx.dispose();
});

test('a token minted for a second user authenticates as that user + is listed', async () => {
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();

  // Create a second, non-root user.
  const username = `mcp-user-${Date.now()}-${Math.floor(Math.random() * 1e6)}`;
  const created = await ctx.post(`${base}/api/v1/users`, {
    data: { username, password: 'mcp-user-pw-123456', display_name: 'MCP User' },
  });
  expect(created.ok(), `create user → ${created.status()} ${await created.text()}`).toBeTruthy();
  const uid = ((await created.json()) as { id: string }).id;

  // Mint a token OWNED BY that user (admin minting for another user).
  const minted = await ctx.post(`${base}/api/v1/mcp/tokens`, {
    data: { user_id: uid, label: 'for-second-user', scope: { tools: null, allow_writes: false } },
  });
  expect(minted.ok(), `mint for user → ${minted.status()} ${await minted.text()}`).toBeTruthy();
  const m = (await minted.json()) as {
    token: string;
    info: { user_id: string; username: string; scope: { allow_writes: boolean } };
  };
  expect(m.info.user_id, 'owned by the second user').toBe(uid);
  expect(m.info.username).toBe(username);
  expect(m.info.scope.allow_writes, 'read-only as requested').toBe(false);

  // The cross-user list surfaces it with the owning username (never the secret).
  const listed = (await (await ctx.get(`${base}/api/v1/mcp/tokens`)).json()) as {
    tokens: { id: string; username: string; token_prefix: string }[];
  };
  const row = listed.tokens.find((t) => t.username === username);
  expect(row, 'second user token is listed').toBeTruthy();
  expect(JSON.stringify(listed), 'list never carries a raw secret').not.toContain(m.token);

  // The token authenticates over the transport AS the second user (initialize ok).
  const c = await tokenCtx(m.token);
  const init = await rpc(c, base, 'initialize', {
    protocolVersion: '2025-03-26',
    capabilities: {},
    clientInfo: { name: 'e2e', version: '1' },
  });
  expect((init.result?.serverInfo as { name: string }).name).toBe('otto');
  await c.dispose();

  // Revoke it; it stops authenticating.
  const del = await ctx.delete(`${base}/api/v1/mcp/tokens/${row!.id}`);
  expect(del.status(), 'revoke → 204').toBe(204);
  const c2 = await tokenCtx(m.token);
  const after = await c2.post(`${base}/api/v1/mcp/http`, {
    data: { jsonrpc: '2.0', id: 1, method: 'tools/list' },
  });
  expect(after.status(), 'revoked token → 401').toBe(401);
  await c2.dispose();

  await ctx.dispose();
});

test('Otto Server tab surfaces the HTTP URL + a tokens panel', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = (await (await ctx.post(`${base}/api/v1/workspaces`, {
    data: { name: 'MCP HTTP UI', root_path: mkdtempSync(join(tmpdir(), 'mcp-http-ui-')) },
  })).json()).id as string;
  await ctx.dispose();

  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, ws);

  await page.goto('/#/mcp');
  await expect(page.locator('.mcp-page')).toBeVisible({ timeout: 30_000 });
  await page.locator('.tabs button', { hasText: 'Otto Server' }).click();

  // The HTTP transport URL is shown.
  const url = page.locator('[data-testid="mcp-http-url"]');
  await expect(url).toBeVisible({ timeout: 20_000 });
  await expect(url).toContainText('/api/v1/mcp/http');

  // The tokens panel renders (root is mcp:admin via root bypass).
  await expect(page.locator('[data-testid="mcp-tokens"]')).toBeVisible();

  // Create a token through the UI and see the one-time secret banner.
  await page.locator('.otto .tools-head button', { hasText: 'New token' }).click();
  await page.locator('.otto .create .fld input').first().fill('ui-made');
  await page.locator('[data-testid="mcp-create-token"]').click();
  await expect(page.locator('[data-testid="mcp-created-token"]')).toBeVisible({ timeout: 20_000 });
});
