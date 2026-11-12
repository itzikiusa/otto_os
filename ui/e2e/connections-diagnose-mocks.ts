// Shared fixture for the "Diagnose with an agent" specs: an ssh profile that
// can never connect (`.invalid`), its Test probe fulfilled as a failure, and
// the runtime-thread API answered in the page (no agent CLI is spawned).
import { expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

export interface DiagnoseFixture {
  connId: string;
  name: string;
  /** Bodies the page POSTed: the thread create and the opening prompt. */
  created: Record<string, unknown>[];
  prompts: Record<string, unknown>[];
  dispose: () => Promise<void>;
}

export async function seedDiagnose(page: Page): Promise<DiagnoseFixture> {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  const name = `diag-${Date.now()}`;
  const res = await ctx.post(`${base}/api/v1/workspaces/${ws}/connections`, {
    data: { name, kind: 'ssh', params: { host: 'fixture.invalid', port: 2222, user: 'deploy', identity_file: '~/.ssh/id_fixture' } },
  });
  expect(res.ok()).toBeTruthy();
  const { id } = await res.json();
  const created: Record<string, unknown>[] = [];
  const prompts: Record<string, unknown>[] = [];

  await page.route(`**/connections/${id}/test`, (route) =>
    route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        ok: false,
        latency_ms: null,
        warn_argv: false,
        message: 'Could not resolve hostname fixture.invalid',
        hint: 'Check the host name, or your VPN / DNS.',
        detail: 'ssh: Could not resolve hostname fixture.invalid: nodename nor servname provided',
      }),
    }),
  );
  const now = new Date().toISOString();
  const row = {
    id: 'rt-diag-1', workspace_id: ws, provider: 'claude', title: `Diagnose ${name}`, cwd: '/tmp',
    status: 'working', access_mode: 'approval-required', created_by: 'e2e', created_at: now, last_active_at: now, last_sequence: 0,
  };
  await page.route('**/api/v1/**runtime-sessions**', async (route) => {
    const req = route.request();
    const url = new URL(req.url());
    const json = (body: unknown) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    if (/\/workspaces\/[^/]+\/runtime-sessions$/.test(url.pathname)) {
      if (req.method() === 'GET') return json([]);
      created.push(req.postDataJSON());
      return json(row);
    }
    if (url.pathname.endsWith('/rt-diag-1/prompt')) {
      prompts.push(req.postDataJSON());
      return json({ turn_id: 't1', sequence: 1 });
    }
    if (url.pathname.endsWith('/rt-diag-1/events')) return json({ kind: 'events', events: [], last_sequence: 0 });
    if (url.pathname.endsWith('/rt-diag-1/peers')) return json([]);
    if (url.pathname.endsWith('/rt-diag-1')) return json(row);
    return route.fallback();
  });
  return { connId: id, name, created, prompts, dispose: () => ctx.dispose() };
}
