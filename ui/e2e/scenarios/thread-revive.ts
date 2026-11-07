import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from '../seed';
import { expectNoHorizontalOverflow } from '../helpers';

export function threadReviveScenario(): void {
  // WebKit service-worker fetches bypass page.route fixtures.
  test.use({ serviceWorkers: 'block' });
  test('revive keeps the draft, retries failure, and fits the available pane', async ({ page }, info) => {
    const { ctx, base } = await apiCtx();
    const workspaceId = await seedWorkspace(ctx, base);
    await ctx.dispose();
    const ts = '2026-11-07T08:00:00Z';
    const sessions = ['revive-alpha', 'revive-beta'].map((id, i) => ({
      id, workspace_id: workspaceId, provider: 'claude', title: i ? 'Second thread' : 'Continue this work', cwd: '/tmp',
      status: 'dormant', access_mode: 'approval-required', created_by: 'root', created_at: ts, last_active_at: ts,
      last_sequence: 1, parent_id: null, meta: {}, model: null,
    }));
    await page.addInitScript(id => {
      localStorage.setItem('otto_workspace', id);
      localStorage.removeItem('otto_threads_v1');
    }, workspaceId);
    await page.route(`**/api/v1/workspaces/${workspaceId}/runtime-sessions`, r => r.fulfill({ json: sessions }));
    for (const session of sessions) await page.route(`**/api/v1/runtime-sessions/${session.id}`, r => r.fulfill({ json: session }));
    const item = { turn_id: 't1', status: 'completed', title: null, started_at: ts, completed_at: ts, updated_at: ts };
    await page.route('**/api/v1/runtime-sessions/revive-*/events**', r => r.fulfill({ json: {
      kind: 'snapshot', snapshot_sequence: 1, snapshot: {
        provider_session: { status: 'stopped', cwd: '/tmp', model: null, updated_at: ts, last_error: null },
        provider_thread: null,
        turns: [{ id: 't1', ordinal: 1, status: 'completed', started_at: ts, completed_at: ts }],
        turn_items: [
          { ...item, id: 'u1', ordinal: 1, type: 'user_message', text: 'Review the next release and keep the findings here.' },
          { ...item, id: 'a1', ordinal: 2, type: 'assistant_message', message_id: 'm1', streaming: false,
            text: 'The release review is saved in this conversation. The next step is to verify the remaining changes and record the results.\n\n| Area | Status | Next step |\n| --- | --- | --- |\n| Thread recovery | Ready to verify | Resume the agent and keep the existing conversation |\n| Responsive layout | In review | Check a wide display, split panes and a phone |' },
        ], runtime_requests: [], plans: [],
      },
    } }));
    const drafts = new Map<string, { revision: number; draft: unknown }>();
    await page.route('**/api/v1/runtime-sessions/revive-*/draft', r => {
      const id = r.request().url().split('/').at(-2)!;
      const current = drafts.get(id) ?? { revision: 0, draft: null };
      if (r.request().method() === 'PUT') drafts.set(id, { revision: current.revision + 1, draft: r.request().postDataJSON().draft });
      return r.fulfill({ json: drafts.get(id) ?? current });
    });
    let revived = 0;
    let prompts = 0;
    await page.route('**/api/v1/runtime-sessions/revive-alpha/reopen', r => {
      revived++;
      if (revived === 1) return r.fulfill({ status: 400, json: { code: 'invalid', message: 'Agent executable unavailable. Retry after installing it.' } });
      sessions[0].status = 'idle';
      return r.fulfill({ json: sessions[0] });
    });
    await page.route('**/api/v1/runtime-sessions/revive-*/prompt', r => { prompts++; return r.fulfill({ json: {} }); });
    const desktop = info.project.name === 'desktop-browser';
    if (desktop) await page.setViewportSize({ width: 2048, height: 1100 });
    await page.goto('/#/threads/revive-alpha');
    const thread = page.getByTestId('runtime-thread-revive-alpha');
    await expect(thread).toBeVisible();
    const composer = thread.getByTestId('runtime-composer');
    await composer.fill('Keep my unfinished draft');
    await thread.getByRole('button', { name: 'Revive', exact: true }).click();
    await expect(thread.getByText('Agent executable unavailable. Retry after installing it.')).toBeVisible();
    await expect(composer).toHaveValue('Keep my unfinished draft');
    await thread.getByRole('button', { name: 'Revive', exact: true }).click();
    await expect(thread.getByTestId('runtime-reopen')).toHaveCount(0);
    await expect(composer).toHaveValue('Keep my unfinished draft');
    expect(revived).toBe(2);
    expect(prompts).toBe(0);
    await expect(thread.getByText('Review the next release and keep the findings here.')).toBeVisible();
    await expect(thread.getByText('The release review is saved in this conversation.', { exact: false })).toBeVisible();
    await expect(thread.getByText('Saving…', { exact: true })).toHaveCount(0);
    const pane = (await thread.boundingBox())!;
    const col = (await thread.locator('.rt-col').boundingBox())!;
    const dock = (await thread.locator('.rt-dock').boundingBox())!;
    expect(col.width).toBeGreaterThan(pane.width * 0.95);
    expect(Math.abs(col.x - dock.x)).toBeLessThan(2);
    expect(Math.abs(col.width - dock.width)).toBeLessThan(2);
    if (desktop) expect(col.width).toBeGreaterThan(1200);
    await expectNoHorizontalOverflow(page);
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.screenshot({ path: info.outputPath(`thread-width-${scheme}.png`), animations: 'disabled' });
    }
    if (desktop) {
      await page.evaluate(async () => {
        const path = '/src/modules/threads/threadsStore.svelte.ts';
        const { threads } = await import(/* @vite-ignore */ path);
        threads.openBeside('revive-beta');
      });
      await expect(page.getByTestId('runtime-thread-revive-beta')).toBeVisible();
      for (const id of ['revive-alpha', 'revive-beta']) {
        const split = page.getByTestId(`runtime-thread-${id}`);
        const box = (await split.boundingBox())!;
        const splitCol = (await split.locator('.rt-col').boundingBox())!;
        expect(splitCol.width).toBeGreaterThan(box.width * 0.95);
        expect(splitCol.width).toBeLessThan(col.width);
      }
      await expectNoHorizontalOverflow(page);
      await page.screenshot({ path: info.outputPath('thread-width-split.png'), animations: 'disabled' });
    }
  });
}
