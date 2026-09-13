import { test, expect, type APIRequestContext } from '@playwright/test';
import { createServer, type Server } from 'node:http';
import { createHash } from 'node:crypto';
import { apiCtx, seedWorkspace } from './seed';

let ctx: APIRequestContext, base = '', token = '', api = '', workspace = '', upstream = '';
let server: Server;
let tokenForms: URLSearchParams[] = [];
let received: { url: string; auth: string }[] = [];
const secret = 'isolated-api-run-secret';

async function post(path: string, data: unknown) {
  const r = await ctx.post(`${api}${path}`, { data });
  expect(r.ok(), `${path}: ${r.status()} ${await r.text()}`).toBeTruthy();
  return r.json();
}
async function waitRun(id: string, status: string) {
  await expect.poll(async () => (await (await ctx.get(`${api}/automation-runs/${id}`)).json()).status, { timeout: 20_000 }).toBe(status);
  return (await ctx.get(`${api}/automation-runs/${id}`)).json();
}

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  ({ ctx, base, token } = await apiCtx());
  workspace = await seedWorkspace(ctx, base);
  expect((await ctx.patch(`${base}/api/v1/workspaces/${workspace}`, { data: { settings: { api_client: { allow_local: true } } } })).ok()).toBeTruthy();
  api = `${base}/api/v1/workspaces/${workspace}/api-client`;
  received = []; tokenForms = [];
  server = createServer((req, res) => {
    received.push({ url: req.url ?? '', auth: req.headers.authorization ?? '' });
    if (req.url === '/token') {
      let body = ''; req.on('data', chunk => { body += String(chunk); });
      req.on('end', () => {
        tokenForms.push(new URLSearchParams(body));
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ access_token: secret, refresh_token: 'isolated-refresh', token_type: 'Bearer' }));
      });
      return;
    }
    if (req.url?.startsWith('/slow')) return;
    if (req.url?.startsWith('/sse')) {
      res.writeHead(200, { 'Content-Type': 'text/event-stream' });
      res.end('event: hello\ndata: prepared stream\n\n'); return;
    }
    res.writeHead(req.url?.startsWith('/fail') ? 500 : 200, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ done: true }));
  });
  await new Promise<void>(resolve => server.listen(0, '127.0.0.1', resolve));
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('fixture did not bind');
  upstream = `http://127.0.0.1:${address.port}`;
});

test.afterEach(async () => {
  server?.closeAllConnections();
  await new Promise<void>(resolve => server ? server.close(() => resolve()) : resolve());
  await ctx?.dispose();
});

test('automation stores dataset results and redacted history that survives page reload', async ({ page }) => {
  const env = await post('/environments', { name: 'Selected environment', variables: { host: upstream }, secret_keys: ['credential'], secret_values: { credential: secret } });
  const request = await post('/requests', { name: 'Dataset request', method: 'GET', url: '{{host}}/ok?case={{case}}', auth: { type: 'bearer', token: '{{credential}}' } });
  const automation = await post('/automations', { name: 'Durable dataset run', steps: [{ request_id: request.id }] });
  const started = await post(`/automations/${automation.id}/runs`, { environment_id: env.id, dataset: [{ case: 'first' }, { case: 'second' }], stop_on_failure: true });
  const run = await waitRun(started.id, 'passed');
  expect(received).toEqual([{ url: '/ok?case=first', auth: `Bearer ${secret}` }, { url: '/ok?case=second', auth: `Bearer ${secret}` }]);
  expect(run.environment_id).toBe(env.id);
  expect(run.result_rows).toEqual([0, 1]);
  expect(run.report.steps).toHaveLength(2);
  expect(run.result_ids).toHaveLength(2);
  expect(new Set(run.result_ids).size).toBe(2);
  expect(JSON.stringify(run)).not.toContain(secret);
  const history = await (await ctx.get(`${api}/history`)).json();
  expect(JSON.stringify(history)).not.toContain(secret);
  const entries = Array.isArray(history) ? history : history.items;
  const correlated = entries.filter((row: { request: { automation_run_id?: string } }) => row.request.automation_run_id === run.id);
  expect(correlated).toHaveLength(2);
  expect(correlated.map((row: {request: {step_result_id: string}}) => row.request.step_result_id).sort()).toEqual([...run.result_ids].sort());
  expect(correlated.map((row: {request: {dataset_row: number}}) => row.request.dataset_row).sort()).toEqual([0, 1]);
  await page.addInitScript(id => localStorage.setItem('otto_workspace', id), workspace);
  await page.goto('/#/api');
  await page.getByRole('tab', { name: 'Automations', exact: true }).click();
  await page.getByText('Durable dataset run', { exact: true }).click();
  await page.getByText(/Run history \(/).click();
  await page.getByRole('button', { name: /passed · 2 steps/ }).click();
  await expect(page.getByText(`Run ${run.id}`, { exact: false })).toBeVisible();
  await page.reload();
  await page.getByRole('tab', { name: 'Automations', exact: true }).click();
  await page.getByText('Durable dataset run', { exact: true }).click();
  await page.getByText(/Run history \(/).click();
  await expect(page.getByRole('button', { name: /passed · 2 steps/ })).toBeVisible();
});

test('stop-on-failure prevents later requests and cancellation persists its outcome', async () => {
  const bad = await post('/requests', { name: 'Fail first', method: 'GET', url: `${upstream}/fail` });
  const good = await post('/requests', { name: 'Never sent', method: 'GET', url: `${upstream}/ok` });
  const automation = await post('/automations', { name: 'Stop failure', steps: [{ request_id: bad.id, assertions: [{ kind: 'status', op: 'eq', value: '200' }] }, { request_id: good.id }] });
  const started = await post(`/automations/${automation.id}/runs`, { stop_on_failure: true });
  const failed = await waitRun(started.id, 'failed');
  expect(failed.report.steps).toHaveLength(1);
  expect(received.map(r => r.url)).toEqual(['/fail']);
  const slow = await post('/requests', { name: 'Wait for cancel', method: 'GET', url: `${upstream}/slow` });
  const cancellable = await post('/automations', { name: 'Cancel fixture', steps: [{ request_id: slow.id }, { request_id: good.id }] });
  const pending = await post(`/automations/${cancellable.id}/runs`, {});
  await expect.poll(() => received.some(r => r.url === '/slow')).toBe(true);
  await post(`/automation-runs/${pending.id}/cancel`, {});
  const cancelled = await waitRun(pending.id, 'cancelled');
  expect(cancelled.report.passed).toBe(false);
  expect(received.some(r => r.url === '/ok')).toBe(false);
});

test('SSE applies selected environment, query parameters and saved authorization', async ({ page }) => {
  const env = await post('/environments', { name: 'SSE environment', variables: { host: upstream }, secret_keys: ['credential'], secret_values: { credential: secret } });
  const saved = await post('/requests', { name: 'Stored stream token', method: 'GET', url: `${upstream}/sse`, auth: { type: 'bearer', token: secret } });
  await page.goto('/#/api');
  const messages = await page.evaluate(async ({ base, token, workspace, env, saved }) => {
    return new Promise<unknown[]>((resolve, reject) => {
      const frames: unknown[] = [];
      const socket = new WebSocket(`${base.replace('http:', 'ws:')}/ws/api-client/stream?token=${encodeURIComponent(token)}&workspace_id=${workspace}`);
      const timer = setTimeout(() => { socket.close(); reject(new Error('stream fixture timed out')); }, 10_000);
      socket.onopen = () => socket.send(JSON.stringify({ action: 'open', kind: 'sse', request: {
        method: 'GET', url: '{{host}}/sse', headers: [], query: [{ key: 'scope', value: 'fixture', enabled: true }],
        auth: saved.auth, environment_id: env.id, body_mode: 'none', body: '',
      } }));
      socket.onmessage = event => {
        const frame = JSON.parse(String(event.data)); frames.push(frame);
        if (frame.type === 'event' || frame.type === 'error') { clearTimeout(timer); socket.close(); resolve(frames); }
      };
      socket.onerror = () => { clearTimeout(timer); reject(new Error('stream socket failed')); };
    });
  }, { base, token, workspace, env, saved });
  expect(messages).toContainEqual(expect.objectContaining({ type: 'event', event: 'hello', data: 'prepared stream' }));
  expect(received).toEqual([{ url: '/sse?scope=fixture', auth: `Bearer ${secret}` }]);
  expect(JSON.stringify(messages)).not.toContain(secret);
});


test('PKCE exchanges a one-use callback and saves a usable Keychain token', async () => {
  const request = await post('/requests', { name: 'PKCE request', method: 'GET', url: `${upstream}/ok`, auth: {
    type: 'oauth2', grant: 'authorization_code', authorization_url: `${upstream}/authorize`,
    token_url: `${upstream}/token`, client_id: 'fixture-client', scope: 'read',
  } });
  const flow = await post('/oauth2/authorize', { request_id: request.id });
  const authorization = new URL(flow.authorization_url);
  expect(authorization.searchParams.get('code_challenge_method')).toBe('S256');
  expect(authorization.searchParams.get('state')).toBe(flow.flow_id);
  expect(flow.redirect_uri).toBe(`${base}/api/v1/api-client/oauth2/callback`);
  const callback = `${flow.redirect_uri}?state=${encodeURIComponent(flow.flow_id)}&code=fixture-code`;
  const result = await ctx.get(callback);
  expect(await result.text()).toContain('Authorization complete');
  expect(tokenForms).toHaveLength(1);
  expect(tokenForms[0].get('grant_type')).toBe('authorization_code');
  expect(tokenForms[0].get('code')).toBe('fixture-code');
  expect(tokenForms[0].get('redirect_uri')).toBe(flow.redirect_uri);
  const verifier = tokenForms[0].get('code_verifier') ?? '';
  expect(createHash('sha256').update(verifier).digest('base64url')).toBe(authorization.searchParams.get('code_challenge'));
  const repeated = await ctx.get(callback);
  expect(await repeated.text()).toContain('already used');
  expect(tokenForms).toHaveLength(1);
  const status = await (await ctx.get(`${api}/oauth2/flows/${flow.flow_id}`)).json();
  expect(status.status).toBe('completed');
  const rows = await (await ctx.get(`${api}/requests`)).json();
  const stored = rows.find((row: { id: string }) => row.id === request.id);
  expect(stored.auth.access_token).toEqual({ $secret: `otto.api.request.${request.id}` });
  expect(JSON.stringify(stored)).not.toContain(secret);
  const executed = await ctx.post(`${api}/requests/${request.id}/execute`, { data: {} });
  expect(executed.ok(), await executed.text()).toBeTruthy();
  expect(received.at(-1)).toEqual({ url: '/ok', auth: `Bearer ${secret}` });
});
