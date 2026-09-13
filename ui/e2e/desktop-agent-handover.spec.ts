import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test('handover preserves the brief and archives only after confirmed receipt', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const settings = await (await ctx.get(`${base}/api/v1/settings`)).json();
  const provider = 'e2e-handover';
  try {
    const configured = await ctx.put(`${base}/api/v1/settings`, { data: {
      providers: { ...settings.providers, [provider]: { cmd: '/bin/cat', args: [] } },
    } });
    expect(configured.ok()).toBeTruthy();
    const meta = await (await ctx.get(`${base}/api/v1/meta`)).json();
    expect(meta.tools.find((tool: { name: string }) => tool.name === provider)).toMatchObject({ found: true });
    const create = async (name: string, engine = provider) => {
      const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, { data: {
        kind: 'agent', provider: engine, title: name, cwd: '/tmp', meta: { origin: 'e2e' },
      } });
      expect(r.ok(), await r.text()).toBeTruthy(); return (await r.json()).id as string;
    };
    const source = await create('Handover source');
    const target = await create('Handover target');
    const shell = await create('Plain shell', 'shell');
    const rejected = await ctx.post(`${base}/api/v1/sessions/${source}/handover`, { data: {
      target: { kind: 'existing_session', session_id: shell }, brief: 'Do not execute as shell',
    } });
    expect(rejected.status()).toBe(400);
    expect((await ctx.post(`${base}/api/v1/sessions/${target}/kill`)).ok()).toBeTruthy();
    const sent = await ctx.post(`${base}/api/v1/sessions/${source}/handover`, { data: {
      target: { kind: 'existing_session', session_id: target }, brief: 'Preserved handover context',
      archive_source: true, include_git: false,
    } });
    expect(sent.ok(), await sent.text()).toBeTruthy();
    const initial = await sent.json();
    await expect.poll(async () => (await (await ctx.get(`${base}/api/v1/sessions/${target}`)).json()).meta.handover.state,
      { timeout: 25_000 }).toBe('failed');
    await page.addInitScript((id) => localStorage.setItem('otto_workspace', id), wsId);
    await page.goto(`/#/agents/${target}`);
    await page.locator('.handover-delivery summary').click();
    await expect(page.locator('.handover-delivery pre')).toHaveText('Preserved handover context');
    expect((await ctx.post(`${base}/api/v1/sessions/${target}/restart`)).ok()).toBeTruthy();
    await page.getByRole('button', { name: 'Retry delivery', exact: true }).click();
    await expect.poll(async () => (await (await ctx.get(`${base}/api/v1/sessions/${target}`)).json()).meta.handover.state,
      { timeout: 25_000 }).toBe('sent');
    const recovered = await (await ctx.get(`${base}/api/v1/sessions/${target}`)).json();
    const deliveryId = recovered.meta.handover.id;
    expect(deliveryId).not.toBe(initial.meta.handover.id);
    const sourceBefore = await (await ctx.get(`${base}/api/v1/sessions/${source}`)).json();
    expect(sourceBefore.archived).toBe(false);
    const stale = await ctx.post(`${base}/api/v1/sessions/${target}/handover/acknowledge`, { data: { delivery_id: 'stale' } });
    expect(stale.status()).toBe(409);
    const retrySent = await ctx.post(`${base}/api/v1/sessions/${target}/handover/retry`, { data: { delivery_id: deliveryId } });
    expect(retrySent.status()).toBe(409);
    await page.reload();
    await page.locator('.handover-delivery summary').click();
    await expect(page.locator('.handover-delivery pre')).toHaveText('Preserved handover context');
    await page.getByRole('button', { name: 'Confirm received', exact: true }).click();
    await expect(page.locator('.handover-delivery summary')).toHaveText('Handover · acknowledged');
    await expect.poll(async () => (await (await ctx.get(`${base}/api/v1/sessions/${source}`)).json()).archived).toBe(true);
    await page.reload();
    await expect(page.locator('.handover-delivery summary')).toHaveText('Handover · acknowledged');
  } finally {
    await ctx.put(`${base}/api/v1/settings`, { data: { providers: settings.providers ?? {} } });
    await ctx.dispose();
  }
});
