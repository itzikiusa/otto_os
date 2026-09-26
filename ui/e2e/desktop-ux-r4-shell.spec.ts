import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block', viewport: { width: 1440, height: 900 }, contextOptions: { reducedMotion: 'reduce' } });
let workspaceId = '', sessionId = '';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  sessionId = await seedShellSession(ctx, base, workspaceId);
  await ctx.dispose();
});
async function boot(page: Page, tab = 'notes') {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(({ workspaceId, tab }) => {
    localStorage.setItem('otto_workspace', workspaceId);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_right_open', '1');
    localStorage.setItem('otto_right_tab', tab);
    localStorage.setItem('otto_view_mode', 'tabs');
  }, { workspaceId, tab });
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.xterm-screen')).toBeVisible();
}

test('panel notes retain the focused unsaved draft through desktop tablet phone and back', async ({ page }, info) => {
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  await page.route(`**/workspaces/${workspaceId}`, async route => {
    if (route.request().method() !== 'PATCH') return route.continue();
    await gate; await route.continue();
  });
  await boot(page);
  const notes = page.locator('textarea.notes');
  await notes.fill('Draft before the workspace save completes');
  await notes.evaluate(el => { (el as HTMLTextAreaElement).setSelectionRange(6, 12); });
  for (const width of [834, 390, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(notes).toHaveValue('Draft before the workspace save completes');
    await expect(notes, `notes focus at ${width}px`).toBeFocused();
    expect(await notes.evaluate(el => [(el as HTMLTextAreaElement).selectionStart, (el as HTMLTextAreaElement).selectionEnd])).toEqual([6, 12]);
    await expectNoHorizontalOverflow(page);
    if (width !== 1440) await page.screenshot({ path: info.outputPath(`notes-${width}.png`) });
  }
  await expect(page.getByRole('dialog', { name: 'Activity', exact: true })).toHaveCount(0);
  release();
});

test('terminal zoom changes the readable narrow grid and reset restores automatic fit', async ({ page }, info) => {
  const { ctx, base } = await apiCtx();
  const session = await (await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/sessions`, { data: {
    kind: 'agent', provider: 'shell', title: 'Zoom review', cwd: '/tmp',
    meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: `${process.cwd()}/../crates/otto-transcript/fixtures/claude/01-basic-tools.jsonl` },
  } })).json();
  await ctx.dispose();
  await boot(page);
  await page.getByRole('button', { name: 'Collapse panel', exact: true }).click();
  await page.goto(`/#/agents/${session.id}`);
  await page.getByRole('tab', { name: 'Split', exact: true }).click();
  const host = page.locator('.term-host');
  const size = () => host.evaluate(el => parseFloat(getComputedStyle(el.querySelector('.xterm-rows')!).fontSize));
  await expect.poll(size).toBe(11);
  await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
  await expect.poll(size).toBe(12);
  await page.getByRole('button', { name: 'Zoom out', exact: true }).click();
  await expect.poll(size).toBe(11);
  await page.getByRole('button', { name: 'Zoom in', exact: true }).click();
  await page.getByRole('button', { name: 'Reset terminal zoom', exact: true }).click();
  await expect.poll(size).toBe(11);
  await expect.poll(() => host.getAttribute('data-cols')).toMatch(/^(8\d|9\d|\d{3,})$/);
  await page.screenshot({ path: info.outputPath('zoom-readable.png') });
});

test('Notes reports a failed save inline and retries the preserved text', async ({ page }, info) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  let fail = true;
  await page.route(`**/workspaces/${workspaceId}`, async route => {
    if (route.request().method() !== 'PATCH' || !fail) return route.continue();
    await route.fulfill({ status: 502, json: { code: 'upstream', message: 'Workspace save unavailable' } });
  });
  await boot(page);
  const notes = page.locator('textarea.notes');
  await notes.fill('Preserve this review note on failure');
  await expect(page.locator('.notes-wrap').getByRole('alert')).toContainText('Workspace save unavailable');
  await expect(notes).toHaveValue('Preserve this review note on failure');
  fail = false;
  await page.locator('.notes-wrap').getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.locator('.notes-foot')).toContainText('Saved');
  await page.screenshot({ path: info.outputPath('notes-saved.png') });
  await page.reload();
  await expect(notes).toHaveValue('Preserve this review note on failure');
});

test('standalone bar answers, exposes errors, and cancels a proposed plan without executing', async ({ page }, info) => {
  await page.setViewportSize({ width: 640, height: 500 });
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_orch_fallback', '1');
    localStorage.removeItem('otto_bar_spaces');
  }, workspaceId);
  let fail = false, executions = 0;
  await page.route('**/workspaces/*/orchestrate', route => route.fulfill(fail
    ? { status: 502, json: { code: 'upstream', message: 'Planner unavailable' } }
    : { json: { plan: [{ action: 'spawn_sessions', count: 1, provider: 'claude' }], optimized_text: null } }));
  await page.route('**/workspaces/*/orchestrate/execute', route => { executions++; return route.fulfill({ json: { results: [] } }); });
  await page.goto('/#/bar');
  const input = page.getByRole('combobox', { name: 'Ask Otto or search commands' });
  await input.fill('Evaluate deployment readiness for this synthetic request'); await input.press('Meta+Enter');
  const turn = page.locator('.turn').last();
  await expect(turn).toContainText('Here’s the plan');
  await turn.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(turn).toContainText('Cancelled — nothing ran.');
  expect(executions).toBe(0);
  fail = true;
  await input.fill('Check another synthetic request'); await input.press('Meta+Enter');
  await expect(page.locator('.turn').last()).toContainText('Planner unavailable');
  await expect(input).toBeEditable();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('standalone-answer-error.png') });
});

test('API request draft and focus survive all shell breakpoints', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript(id => { localStorage.setItem('otto_workspace', id); localStorage.setItem('otto_firstrun_dismissed', '1'); }, workspaceId);
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  await page.route(`**/workspaces/${workspaceId}/api-client/requests`, async route => { await gate; await route.fulfill({ json: [] }); });
  await page.goto('/#/api');
  const url = page.getByLabel('Request URL', { exact: true });
  await url.fill('https://fixture.invalid/unsaved-breakpoint-review');
  release();
  await expect(page.getByText('Loading saved requests…', { exact: true })).toHaveCount(0);
  for (const width of [834, 390, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await expect(url).toHaveValue('https://fixture.invalid/unsaved-breakpoint-review');
    await expect(url).toBeFocused();
    await expectNoHorizontalOverflow(page);
  }
});

for (const scheme of ['light', 'dark']) {
  test(`Files recovers its loaded tree and keeps its preview across shell transitions ${scheme}`, async ({ page }, info) => {
    await page.addInitScript(scheme => localStorage.setItem('otto_scheme', scheme), scheme);
    let fail = true;
    await page.route('**/fs/browse?*', route => route.fulfill(fail
      ? { status: 502, json: { code: 'upstream', message: 'Folder listing unavailable' } }
      : { json: { path: '/tmp', parent: '/', is_git_repo: false, entries: [
        { name: 'release-review.md', path: '/tmp/release-review.md', is_dir: false, is_git_repo: false },
        { name: 'src', path: '/tmp/src', is_dir: true, is_git_repo: false },
      ] } }));
    await page.route('**/fs/read?*', route => route.fulfill({ json: { path: '/tmp/release-review.md', content: '# Release review\n\nSynthetic file preview with an accessible source view.\n\n- Notes saved\n- Layout verified', language: 'markdown', truncated: false } }));
    await boot(page, 'files');
    await expect(page.locator('.ft-wrap').getByRole('alert')).toContainText('Folder listing unavailable');
    fail = false; await page.locator('.ft-wrap').getByRole('button', { name: 'Retry', exact: true }).click();
    await page.getByRole('button', { name: 'release-review.md', exact: true }).click();
    const preview = page.frameLocator('.preview-frame');
    await expect(preview.getByRole('heading', { name: 'Release review', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Add file section', exact: true }).click();
    await expect(page.locator('.fp-section')).toHaveCount(2);
    for (const width of [834, 390, 1440]) {
      await page.setViewportSize({ width, height: 900 });
      await expect(page.locator('.fp-section')).toHaveCount(2);
      await expect(preview.getByRole('heading', { name: 'Release review', exact: true })).toBeVisible();
      await expectNoHorizontalOverflow(page);
      if (width === 390) await page.screenshot({ path: info.outputPath('files-phone.png') });
    }
    expect((await page.locator('.tree-pane.has-viewer').boundingBox())!.height, 'sparse tree leaves space for reading').toBeLessThanOrEqual(80);
    await page.getByRole('button', { name: 'Source', exact: true }).click();
    await expect(page.locator('.code-scroll .cm-content')).toContainText('Release review');
    const source = page.locator('.code-scroll .cm-content');
    await expect(source).toHaveAttribute('aria-readonly', 'true');
    const before = await source.locator('.cm-line').allTextContents();
    await source.click(); await source.press('End'); await source.press('x');
    await expect.poll(() => source.locator('.cm-line').allTextContents()).toEqual(before);
    await page.getByRole('button', { name: 'Preview', exact: true }).click();
    await page.screenshot({ path: info.outputPath('files-preview.png') });
  });
}

test('Outputs refreshes a selected artifact when its same-id producing turn changes', async ({ page }) => {
  const artifact = { id: 'report', kind: 'report', label: 'Release report', path: '/tmp/release.md', url: null, mime: 'text/markdown', produced_at: '2026-09-25T10:00:00Z', turn_id: 'turn-one' };
  let send!: (message: string) => void;
  await page.routeWebSocket('**/ws/events**', socket => {
    const server = socket.connectToServer();
    send = message => socket.send(message);
    socket.onMessage(message => server.send(message));
    server.onMessage(message => socket.send(message));
  });
  let body = '# First report';
  await page.route(`**/sessions/${sessionId}/artifacts`, route => route.fulfill({ json: [artifact] }));
  await page.route(`**/sessions/${sessionId}/artifacts/report`, route => route.fulfill({ body, contentType: 'text/markdown' }));
  await boot(page, 'outputs');
  await expect(page.getByRole('heading', { name: 'First report', exact: true })).toBeVisible();
  body = '# Updated report';
  send(JSON.stringify({ type: 'artifact_added', workspace_id: workspaceId, session_id: sessionId, artifact: { ...artifact, turn_id: 'turn-two', produced_at: '2026-09-25T10:01:00Z' } }));
  await expect(page.getByRole('heading', { name: 'Updated report', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'First report', exact: true })).toHaveCount(0);
});

test('Notes serializes saves so the newest draft wins a delayed older request', async ({ page }) => {
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const writes: string[] = [];
  await page.route(`**/workspaces/${workspaceId}`, async route => {
    if (route.request().method() !== 'PATCH') return route.continue();
    writes.push(route.request().postDataJSON().settings.notes);
    if (writes.length === 1) await gate;
    await route.continue();
  });
  await boot(page);
  const notes = page.getByRole('textbox', { name: 'Workspace notes', exact: true });
  await notes.fill('Older submitted text');
  await expect.poll(() => writes.length).toBe(1);
  await notes.fill('Newest text must remain on disk');
  await page.waitForTimeout(750); // pass the documented 600ms debounce while the first write is held
  expect(writes).toEqual(['Older submitted text']);
  release();
  await expect.poll(() => writes).toEqual(['Older submitted text', 'Newest text must remain on disk']);
  await expect(page.locator('.notes-foot')).toHaveText('Saved');
  await page.reload();
  await expect(notes).toHaveValue('Newest text must remain on disk');
});

test('Activity applies live task and trail updates without losing the open panel', async ({ page }, info) => {
  let send!: (message: string) => void;
  await page.routeWebSocket('**/ws/events**', socket => {
    const server = socket.connectToServer(); send = message => socket.send(message);
    socket.onMessage(message => server.send(message)); server.onMessage(message => socket.send(message));
  });
  await boot(page, 'activity');
  const now = '2026-09-25T10:00:00Z';
  const task = { id: 'live-task', session_id: sessionId, workspace_id: workspaceId, ext_id: '1', title: 'Verify release candidate', status: 'in_progress', position: 0, source: 'agent', description: null, nudge_pending: false, nudged_at: null, created_at: now, updated_at: now };
  send(JSON.stringify({ type: 'tasks_updated', workspace_id: workspaceId, session_id: sessionId, tasks: [task] }));
  await expect(page.locator('.rpanel').getByText(task.title, { exact: true })).toBeVisible();
  send(JSON.stringify({ type: 'tasks_updated', workspace_id: workspaceId, session_id: sessionId, tasks: [{ ...task, status: 'completed' }] }));
  await expect(page.locator('.rpanel').getByText('1/1', { exact: true })).toBeVisible();
  send(JSON.stringify({ type: 'trail_appended', workspace_id: workspaceId, session_id: sessionId, event: { id: 'event', session_id: sessionId, workspace_id: workspaceId, ts: now, source: 'agent', kind: 'command', level: 'info', summary: 'Synthetic release checks passed', detail: { command: 'verify-release' } } }));
  await expect(page.locator('.rpanel').getByText('Synthetic release checks passed', { exact: true })).toBeVisible();
  await page.screenshot({ path: info.outputPath('activity-live.png') });
});

test('a real restarted PTY epoch replaces a held old-process selection', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const id = await seedShellSession(ctx, base, workspaceId);
  const epochs: number[] = [];
  await page.routeWebSocket(`**/ws/term/${id}**`, socket => {
    const server = socket.connectToServer();
    socket.onMessage(message => server.send(message));
    server.onMessage(message => {
      if (typeof message === 'string') { const msg = JSON.parse(message); if (msg.type === 'scrollback') epochs.push(msg.epoch); }
      socket.send(message);
    });
  });
  await boot(page);
  await page.getByRole('button', { name: 'Collapse panel', exact: true }).click();
  await page.goto(`/#/agents/${id}`);
  await expect.poll(() => epochs.length).toBeGreaterThan(0);
  await page.waitForTimeout(1500); // settle the initial geometry before selecting output
  await ctx.post(`${base}/api/v1/sessions/${id}/input`, { data: { text: 'for i in $(seq 1 10); do echo OLD-EPOCH-REVIEW-$i; done', submit: true } });
  await expect(page.locator('.xterm-rows')).toContainText('OLD-EPOCH-REVIEW-10');
  await expect.poll(() => epochs.length).toBeGreaterThan(0);
  const oldEpoch = epochs.at(-1);
  const box = (await page.locator('.xterm-screen').boundingBox())!;
  await page.mouse.move(box.x + 8, box.y + 20); await page.mouse.down();
  await page.mouse.move(box.x + box.width - 40, box.y + 100, { steps: 20 }); await page.mouse.up();
  await expect(page.locator('.xterm-helper-textarea')).toHaveValue(/OLD-EPOCH-REVIEW/);
  const restarted = await ctx.post(`${base}/api/v1/sessions/${id}/restart`);
  expect(restarted.ok()).toBe(true);
  await expect.poll(() => epochs.at(-1)).not.toBe(oldEpoch);
  await ctx.post(`${base}/api/v1/sessions/${id}/input`, { data: { text: 'echo NEW-EPOCH-READY', submit: true } });
  await expect(page.locator('.xterm-rows')).toContainText('NEW-EPOCH-READY');
  await expect(page.locator('.xterm-rows')).not.toContainText('OLD-EPOCH-REVIEW');
  await ctx.dispose();
});

test('phone terminal zoom starts at the rendered readability floor', async ({ page }, info) => {
  await boot(page);
  await page.getByRole('button', { name: 'Collapse panel', exact: true }).click();
  await page.setViewportSize({ width: 390, height: 844 });
  const size = () => page.locator('.term-host').evaluate(el => parseFloat(getComputedStyle(el.querySelector('.xterm-rows')!).fontSize));
  await expect.poll(size).toBe(15);
  await page.getByRole('button', { name: 'Zoom in terminal', exact: true }).click();
  await expect.poll(size).toBe(16);
  await page.getByRole('button', { name: 'Zoom out terminal', exact: true }).click();
  await expect.poll(size).toBe(15);
  await page.screenshot({ path: info.outputPath('phone-terminal-zoom.png') });
});

test('queued Notes edits cannot be submitted under a replacement login', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const workspaces = await (await ctx.get(`${base}/api/v1/workspaces`)).json();
  await ctx.dispose();
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  const writes: string[] = [];
  // Both identities can read this workspace; a 401 must not mask the queue bug.
  await page.route('**/api/v1/workspaces', route => route.fulfill({ json: workspaces }));
  await page.route(`**/workspaces/${workspaceId}`, async route => {
    if (route.request().method() !== 'PATCH') return route.continue();
    writes.push(route.request().headers().authorization ?? '');
    if (writes.length === 1) await held;
    await route.fulfill({ json: { id: workspaceId, name: 'Notes context', root_path: '/tmp', settings: route.request().postDataJSON().settings } });
  });
  await boot(page);
  await page.locator('textarea.notes').fill('Old account first draft');
  await expect.poll(() => writes.length).toBe(1);
  await page.locator('textarea.notes').fill('Old account queued private draft');
  await page.waitForTimeout(800); // Let the real 600ms debounce enqueue behind the held write.
  await page.evaluate(async () => { const path = '/src/lib/api/client.ts'; (await import(path)).setToken('new-fixture-login'); });
  release();
  await page.waitForTimeout(800); // The released queue settles without dispatching a second write.
  expect(writes).toHaveLength(1);
});
