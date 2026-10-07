import { expect, type APIRequestContext } from '@playwright/test';

// All real catalog writers on the shared isolated daemon add the SAME set.
// PATCH replaces the catalog, so separate read/union/write calls with different
// additions could erase another worker's tools even while preserving defaults.
const REQUIRED_TOOLS = [
  'otto.create_pr', 'otto.comment_pr', 'otto.merge_pr',
  'otto.list_repos', 'otto.list_workflows', 'otto.run_workflow',
];

/** Enable fixture tools without changing token scopes or approval policy. */
export async function enableMcpFixtureCatalog(ctx: APIRequestContext, base: string): Promise<void> {
  const response = await ctx.get(`${base}/api/v1/mcp/otto-server`);
  expect(response.ok(), 'read the isolated daemon MCP catalog').toBeTruthy();
  const status = await response.json() as { tools: { name: string; enabled: boolean }[] };
  const enabled = status.tools.filter(tool => tool.enabled).map(tool => tool.name);
  const patched = await ctx.patch(`${base}/api/v1/mcp/otto-server`, {
    data: { enabled: true, tools: [...new Set([...enabled, ...REQUIRED_TOOLS])] },
  });
  expect(patched.ok(), `enable fixture catalog → ${patched.status()} ${await patched.text()}`).toBeTruthy();
}
