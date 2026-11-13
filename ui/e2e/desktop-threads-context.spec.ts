import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });

test('thread drafts, attachments and permissions stay with their owner; mobile commands open accessible drawers', async ({ page }, info) => {
  const { ctx, base } = await apiCtx();
  const workspaceId = await seedWorkspace(ctx, base);
  await ctx.dispose();
  const ids = ['focus-thread-alpha', 'focus-thread-beta'];
  const ts = '2026-11-07T08:00:00Z';
  const sessions = ids.map((id, index) => ({
    id, workspace_id: workspaceId, provider: 'claude', title: index ? 'Beta thread' : 'Alpha thread', cwd: '/tmp',
    status: 'idle', access_mode: 'approval-required', created_by: 'root', created_at: ts, last_active_at: ts,
    last_sequence: 1, parent_id: null, meta: {}, model: null,
  }));
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.removeItem('otto_threads_v1');
  }, workspaceId);
  await page.route(`**/api/v1/workspaces/${workspaceId}/runtime-sessions`, (route) => route.fulfill({ json: sessions }));
  await page.route('**/api/v1/runtime-sessions/focus-thread-*/events**', (route) => route.fulfill({ json: {
    kind: 'snapshot', snapshot_sequence: 1, snapshot: {
      provider_session: { status: 'ready', cwd: '/tmp', model: null, updated_at: ts, last_error: null },
      provider_thread: null, turns: [], turn_items: [], runtime_requests: [], plans: [],
    },
  } }));
  const drafts = new Map<string, { revision: number; draft: unknown }>();
  await page.route('**/api/v1/runtime-sessions/focus-thread-*/draft', route => {
    const id = route.request().url().split('/').at(-2)!;
    const current = drafts.get(id) ?? { revision: 0, draft: null };
    if (route.request().method() === 'PUT') {
      const body = route.request().postDataJSON();
      drafts.set(id, { revision: current.revision + 1, draft: body.draft });
    }
    return route.fulfill({ json: drafts.get(id) ?? current });
  });
  const sent: { id: string; body: Record<string, unknown> }[] = [];
  await page.route('**/api/v1/runtime-sessions/focus-thread-*/prompt', (route) => {
    sent.push({ id: route.request().url().split('/').at(-2)!, body: route.request().postDataJSON() });
    return route.fulfill({ json: { sequence: 1 } });
  });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/threads');
  const alpha = page.getByTestId(`runtime-thread-${ids[0]}`);
  const beta = page.getByTestId(`runtime-thread-${ids[1]}`);
  await expect(alpha).toBeVisible();
  await alpha.getByTestId('runtime-composer').fill('Alpha private draft');
  await alpha.locator('input[type=file]').setInputFiles({ name: 'alpha.png', mimeType: 'image/png', buffer: Buffer.from('image fixture') });
  await expect(alpha.getByTestId('runtime-images').getByRole('img', { name: 'alpha.png' })).toBeVisible();
  await alpha.getByTestId('runtime-access').click();
  await page.getByRole('menuitemcheckbox', { name: 'Full access', exact: true }).click();
  await page.getByRole('button', { name: 'Allow full access', exact: true }).click();
  await expect(alpha.getByTestId('runtime-access')).toContainText('Full access');

  await page.getByTestId('threads-sidebar').getByText('Beta thread', { exact: true }).click();
  await expect(beta).toBeVisible();
  await expect(beta.getByTestId('runtime-composer')).toHaveValue('');
  await expect(beta.getByTestId('runtime-images')).toHaveCount(0);
  await expect(beta.getByTestId('runtime-access')).toContainText('Supervised');
  await beta.getByTestId('runtime-composer').fill('Beta message');
  await beta.getByTestId('runtime-send').click();
  await expect.poll(() => sent.length).toBe(1);
  expect(sent[0].id).toBe(ids[1]);
  expect(sent[0].body).toEqual({ text: 'Beta message' });

  await page.getByTestId('threads-sidebar').getByText('Alpha thread', { exact: true }).click();
  await expect(alpha).toBeVisible();
  await expect(alpha.getByTestId('runtime-composer')).toHaveValue('Alpha private draft');
  await expect(alpha.getByTestId('runtime-images').getByRole('img', { name: 'alpha.png' })).toBeVisible();
  await expect(alpha.getByTestId('runtime-access')).toContainText('Full access');
  // A→B→A must restore the same draft, not clear it to fix the leak.
  await alpha.getByTestId('runtime-images').getByRole('button', { name: 'Remove alpha.png' }).click();
  await alpha.getByTestId('runtime-composer').fill('');

  await page.evaluate(async () => {
    const path = '/src/modules/threads/threadsStore.svelte.ts';
    const { threads } = await import(/* @vite-ignore */ path);
    threads.inspectorOpen = true;
  });
  for (const width of [1024, 390]) {
    await page.setViewportSize({ width, height: 844 });
    await expect(page.getByRole('dialog', { name: 'Thread details', exact: true })).toHaveCount(0);
    const composer = alpha.getByTestId('runtime-composer');
    await composer.fill('/files');
    await composer.press('Enter');
    const details = page.getByRole('dialog', { name: 'Thread details', exact: true });
    await expect(details).toBeVisible();
    await expect(details.getByTestId('inspector-files-tab')).toBeVisible();
    await expectFullyInViewport(page, details);
    await page.keyboard.press('Escape');
    await expect(details).toHaveCount(0);
    await expect(composer).toBeFocused();
    await page.getByTestId('threads-toggle-sidebar').click();
    const sidebar = page.getByRole('dialog', { name: 'Threads', exact: true });
    await expect(sidebar).toBeVisible();
    await expectFullyInViewport(page, sidebar);
    await page.keyboard.press('Escape');
    await expect(sidebar).toHaveCount(0);
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`threads-context-${width}.png`) });
  }
});

test('a failed initial thread list shows Retry instead of an empty composer', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const workspaceId = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.removeItem('otto_threads_v1');
  }, workspaceId);
  let failed = true;
  await page.route(`**/api/v1/workspaces/${workspaceId}/runtime-sessions`, (route) => failed
    ? route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Thread service unavailable' } })
    : route.fulfill({ json: [] }));
  await page.goto('/#/threads');
  const error = page.getByTestId('load-error').filter({ hasText: 'Couldn’t load threads' });
  await expect(error).toBeVisible();
  await expect(page.getByTestId('new-thread-stage')).toHaveCount(0);
  failed = false;
  await error.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(error).toHaveCount(0);
  await expect(page.getByTestId('new-thread-stage')).toBeVisible();
});
