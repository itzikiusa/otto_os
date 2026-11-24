import { test, expect } from '@playwright/test';
import { createServer, type Server } from 'node:http';
import type { AddressInfo } from 'node:net';
import { apiCtx, seedWorkspace } from './seed';

// OAuth sign-in of a governed remote MCP server, end to end against a local
// fake MCP server + authorization server (this process): the row says Needs
// sign-in, Sign in names the server and its authorization host before the
// "browser" (a Playwright popup) opens, the fake authorization endpoint
// redirects to the daemon's loopback listener, and the signed-in server then
// answers discovery on the bearer the daemon attached.

test.describe.configure({ mode: 'serial' });

let fake: Server;
let fakeBase = '';
let ws = '';
let serverId = '';
const tokensIssued: string[] = [];

function startFake(): Promise<void> {
  fake = createServer((req, res) => {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      const url = new URL(req.url ?? '/', fakeBase);
      const json = (status: number, v: unknown, headers: Record<string, string> = {}) => {
        res.writeHead(status, { 'content-type': 'application/json', ...headers });
        res.end(JSON.stringify(v));
      };
      if (url.pathname === '/mcp' && req.method === 'POST') {
        if (req.headers.authorization !== 'Bearer at-e2e') {
          return json(401, {}, { 'www-authenticate': `Bearer resource_metadata="${fakeBase}/.well-known/oauth-protected-resource/mcp"` });
        }
        const msg = JSON.parse(body || '{}');
        if (msg.id === undefined) { res.writeHead(202); return res.end(); }
        const result = msg.method === 'initialize'
          ? { protocolVersion: '2025-06-18', capabilities: {}, serverInfo: { name: 'fake', version: '1' } }
          : msg.method === 'tools/list'
            ? { tools: [{ name: 'list_items', description: 'list items', inputSchema: { type: 'object' }, annotations: { readOnlyHint: true } }] }
            : {};
        return json(200, { jsonrpc: '2.0', id: msg.id, result });
      }
      if (url.pathname === '/.well-known/oauth-protected-resource/mcp') {
        return json(200, { resource: `${fakeBase}/mcp`, authorization_servers: [fakeBase] });
      }
      if (url.pathname === '/.well-known/oauth-authorization-server') {
        return json(200, {
          issuer: fakeBase,
          authorization_endpoint: `${fakeBase}/authorize`,
          token_endpoint: `${fakeBase}/token`,
          registration_endpoint: `${fakeBase}/register`,
          code_challenge_methods_supported: ['S256'],
        });
      }
      if (url.pathname === '/register') return json(201, { client_id: 'otto-e2e' });
      if (url.pathname === '/authorize') {
        // The person "approves": straight back to Otto's loopback listener.
        const back = new URL(url.searchParams.get('redirect_uri') ?? '');
        back.searchParams.set('code', 'code-e2e');
        back.searchParams.set('state', url.searchParams.get('state') ?? '');
        res.writeHead(302, { location: back.toString() });
        return res.end();
      }
      if (url.pathname === '/token') {
        const form = new URLSearchParams(body);
        if (form.get('code') !== 'code-e2e' || !form.get('code_verifier')) return json(400, { error: 'invalid_grant' });
        tokensIssued.push(form.get('grant_type') ?? '');
        return json(200, { access_token: 'at-e2e', refresh_token: 'rt-e2e', expires_in: 3600 });
      }
      json(404, {});
    });
  });
  return new Promise((r) => fake.listen(0, '127.0.0.1', () => {
    fakeBase = `http://127.0.0.1:${(fake.address() as AddressInfo).port}`;
    r();
  }));
}

test.beforeAll(async () => {
  await startFake();
  const { ctx, base } = await apiCtx();
  ws = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${ws}/mcp/servers`, {
    data: { name: 'oauth-fake', transport: 'http', url: `${fakeBase}/mcp`, enabled: true },
  });
  expect(r.ok(), await r.text()).toBeTruthy();
  serverId = (await r.json()).id;
  const h = await ctx.post(`${base}/api/v1/mcp/servers/${serverId}/health`);
  const probed = await h.json();
  expect(JSON.stringify(probed)).toContain('"health_error":"Needs sign-in');
  await ctx.dispose();
});

test.afterAll(async () => {
  const { ctx, base } = await apiCtx();
  if (serverId) await ctx.delete(`${base}/api/v1/mcp/servers/${serverId}`).catch(() => {});
  await ctx.dispose();
  fake?.close();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((id) => {
    if (sessionStorage.getItem('mcp-oauth-seeded')) return;
    localStorage.setItem('otto_workspace', id as string);
    sessionStorage.setItem('mcp-oauth-seeded', '1');
  }, ws);
});

test('a server that needs sign-in signs in through the browser and then lists its tools', async ({ page, context }) => {
  await page.goto('/#/mcp/servers');
  const row = page.getByTestId('mcp-server-row').filter({ hasText: 'oauth-fake' });
  await expect(row).toContainText('Needs sign-in', { timeout: 30_000 });
  await row.getByTestId('mcp-server-sign-in').click();

  // The confirm names the server, its host and the authorization host first.
  const dialog = page.getByRole('dialog');
  await expect(dialog).toContainText('Sign in to oauth-fake (127.0.0.1)');
  await expect(dialog).toContainText('Your browser opens 127.0.0.1 to sign in');
  const popup = context.waitForEvent('page');
  await dialog.getByRole('button', { name: 'Open browser' }).click();
  const browser = await popup;
  await expect(browser.locator('body')).toContainText('Signed in', { timeout: 15_000 });
  await browser.close();

  await expect(page.getByText('Signed in', { exact: true }).first()).toBeVisible({ timeout: 15_000 });
  expect(tokensIssued).toEqual(['authorization_code']);
  await expect(row).not.toContainText('Needs sign-in');

  // The daemon attaches the bearer: discovery now succeeds.
  await row.getByRole('button', { name: 'Discover' }).click();
  await expect(page.getByText('1 tool from oauth-fake')).toBeVisible({ timeout: 15_000 });
});
