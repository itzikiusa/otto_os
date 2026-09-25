import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block', contextOptions: { reducedMotion: 'reduce' } });
let workspaceId = '';
let sessionId = '';
const artifacts = [
  { id: 'report', kind: 'report', label: 'Release readiness report with a deliberately long descriptive title.md', path: '/tmp/report.md', url: null, mime: 'text/markdown', produced_at: '2026-09-25T10:00:00Z', turn_id: 'turn' },
  { id: 'link', kind: 'url', label: 'Preview documentation', path: null, url: 'https://example.com/review', mime: null, produced_at: '2026-09-25T10:01:00Z', turn_id: 'turn' },
];
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  sessionId = await seedShellSession(ctx, base, workspaceId);
  await ctx.dispose();
});
async function boot(page: Page) {
  await page.addInitScript(({ workspaceId }) => {
    localStorage.setItem('otto_workspace', workspaceId);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_right_open', '1');
    localStorage.setItem('otto_right_tab', 'outputs');
    localStorage.setItem('otto_view_mode', 'tabs');
  }, { workspaceId });
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.getByTestId('outputs-panel')).toBeVisible();
}
test('Outputs distinguishes a failed list from empty and retries in place', async ({ page }) => {
  // The recovered list opens its first preview automatically.
  await page.route(`**/sessions/${sessionId}/artifacts/report`, route => route.fulfill({ body: '# Report', contentType: 'text/markdown' }));
  let failed = true;
  await page.route(`**/sessions/${sessionId}/artifacts`, route => route.fulfill(failed
    ? { status: 502, json: { code: 'upstream', message: 'Artifact index is temporarily unavailable' } }
    : { json: artifacts }));
  await boot(page);
  const panel = page.getByTestId('outputs-panel');
  await expect(panel.getByRole('alert')).toContainText('Artifact index is temporarily unavailable');
  await expect(panel.getByText('Nothing produced yet.', { exact: false })).toBeHidden();
  failed = false;
  await panel.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(panel.getByRole('option')).toHaveCount(2);
  await expect(panel.getByRole('alert')).toBeHidden();
});
test('Outputs switching from a pending preview to a link clears loading', async ({ page }) => {
  await page.route(`**/sessions/${sessionId}/artifacts`, route => route.fulfill({ json: artifacts }));
  let release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  await page.route(`**/sessions/${sessionId}/artifacts/report`, async route => {
    await pending;
    await route.fulfill({ body: '# Old report', contentType: 'text/markdown' });
  });
  await boot(page);
  const panel = page.getByTestId('outputs-panel');
  await panel.getByRole('option').first().click();
  await expect(panel.getByText('Loading preview…')).toBeVisible();
  await panel.getByRole('option').nth(1).click();
  try {
    await expect(panel.getByRole('link', { name: 'https://example.com/review' })).toBeVisible();
    await expect(panel.getByText('Loading preview…')).toBeHidden();
  } finally { release(); }
});
for (const variant of [
  { theme: 'native', scheme: 'light', direction: 'ltr', width: 1440, height: 900 },
  { theme: 'native', scheme: 'dark', direction: 'ltr', width: 1440, height: 900 },
  { theme: 'warm', scheme: 'light', direction: 'rtl', width: 1024, height: 900 },
  { theme: 'warm', scheme: 'dark', direction: 'rtl', width: 390, height: 844 },
  { theme: 'pro-dark', scheme: 'dark', direction: 'ltr', width: 1440, height: 900 },
]) {
  test(`loaded Outputs ${variant.theme} ${variant.scheme} ${variant.width}`, async ({ page }, info) => {
    await page.setViewportSize({ width: variant.width, height: variant.height });
    await page.addInitScript(v => {
      localStorage.setItem('otto_theme', v.theme);
      localStorage.setItem('otto_scheme', v.scheme);
      localStorage.setItem('otto_direction', v.direction);
    }, variant);
    await page.route(`**/sessions/${sessionId}/artifacts`, route => route.fulfill({ json: artifacts }));
    await page.route(`**/sessions/${sessionId}/artifacts/report`, route => route.fulfill({ contentType: 'text/markdown', body: '# Release readiness\n\nThe agent completed the requested checks. Review the changes before publishing.\n\n' + Array.from({ length: 24 }, (_, i) => `- Check ${i + 1}: passed`).join('\n') }));
    await boot(page);
    const panel = page.getByTestId('outputs-panel');
    await panel.getByRole('option').first().click();
    await expect(panel.getByRole('heading', { name: 'Release readiness' })).toBeVisible();
    await expectFullyInViewport(page, panel);
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath('loaded-outputs.png'), animations: 'disabled' });
  });
}

test('Outputs reselecting an artifact ignores an older in-flight preview', async ({ page }) => {
  await page.route(`**/sessions/${sessionId}/artifacts`, route => route.fulfill({ json: artifacts }));
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  let requests = 0;
  await page.route(`**/sessions/${sessionId}/artifacts/report`, async route => {
    const first = ++requests === 1;
    if (first) await held;
    await route.fulfill({ body: first ? '# Obsolete response' : '# Current response', contentType: 'text/markdown' });
  });
  await boot(page);
  const panel = page.getByTestId('outputs-panel');
  await panel.getByRole('option').first().click();
  await expect.poll(() => requests).toBe(1);
  await panel.getByRole('option').nth(1).click();
  await panel.getByRole('option').first().click();
  await expect(panel.getByRole('heading', { name: 'Current response' })).toBeVisible();
  const completed = page.waitForResponse(`**/sessions/${sessionId}/artifacts/report`);
  release();
  await completed;
  await expect(panel.getByRole('heading', { name: 'Obsolete response' })).toBeHidden();
  await expect(panel.getByRole('heading', { name: 'Current response' })).toBeVisible();
});

test('Outputs list supports arrow and Home/End keyboard selection', async ({ page }) => {
  await page.route(`**/sessions/${sessionId}/artifacts`, route => route.fulfill({ json: artifacts }));
  await page.route(`**/sessions/${sessionId}/artifacts/report`, route => route.fulfill({ body: '# Report', contentType: 'text/markdown' }));
  await boot(page);
  const options = page.getByTestId('outputs-panel').getByRole('option');
  await options.first().focus();
  await page.keyboard.press('ArrowDown');
  await expect(options.nth(1)).toBeFocused();
  await expect(options.nth(1)).toHaveAttribute('aria-selected', 'true');
  await page.keyboard.press('Home');
  await expect(options.first()).toBeFocused();
  await page.keyboard.press('End');
  await expect(options.nth(1)).toBeFocused();
});

test('terminal preserves a selection when an in-flight resize snapshot arrives', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  await ctx.post(`${base}/api/v1/sessions/${sessionId}/input`, { data: { text: 'for i in $(seq 1 20); do echo SELECTION-REVIEW-$i; done', submit: true } });
  await ctx.dispose();
  let release: (() => void) | undefined;
  await page.routeWebSocket('**/ws/term/**', socket => {
    const server = socket.connectToServer();
    let snapshots = 0;
    server.onMessage(message => {
      if (typeof message === 'string' && JSON.parse(message).type === 'scrollback' && ++snapshots === 2) {
        release = () => socket.send(message);
      } else socket.send(message);
    });
  });
  await boot(page);
  await expect.poll(() => !!release).toBe(true);
  const box = await page.locator('.xterm-screen').boundingBox();
  if (!box) throw new Error('Terminal screen not rendered');
  await page.mouse.move(box.x + 8, box.y + 20);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width - 40, box.y + 100, { steps: 20 });
  await page.mouse.up();
  const mirror = page.locator('.xterm-helper-textarea');
  await expect(mirror).toHaveValue(/SELECTION-REVIEW/);
  release!();
  // The snapshot crosses the socket task and xterm's asynchronous write queue.
  await page.waitForTimeout(300);
  await expect(mirror).toHaveValue(/SELECTION-REVIEW/);
});

for (const scheme of ['light', 'dark']) {
  test(`loaded split session and settled Home prompt ${scheme}`, async ({ page }, info) => {
    const { ctx, base } = await apiCtx();
    const response = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/sessions`, { data: {
      kind: 'agent', provider: 'shell', title: 'Release review', cwd: '/tmp',
      meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: `${process.cwd()}/../crates/otto-transcript/fixtures/claude/01-basic-tools.jsonl` },
    } });
    expect(response.ok()).toBe(true);
    const session = await response.json();
    await ctx.dispose();
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript(({ workspaceId, scheme }) => {
      localStorage.setItem('otto_workspace', workspaceId);
      localStorage.setItem('otto_scheme', scheme);
      localStorage.setItem('otto_theme', 'native');
      localStorage.setItem('otto_right_open', '0');
    }, { workspaceId, scheme });
    await page.goto(`/#/agents/${session.id}`);
    await page.getByRole('tab', { name: 'Split', exact: true }).click();
    await expect(page.locator('.conv[data-loaded="true"]')).toBeVisible();
    await expect(page.locator('.xterm-screen')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath('loaded-split.png'), animations: 'disabled' });
    await page.goto('/#/home');
    await page.getByRole('button', { name: 'Add space', exact: true }).click();
    await expect(page.getByRole('textbox', { name: 'Space name', exact: true })).toBeVisible();
    await page.screenshot({ path: info.outputPath('settled-home-prompt.png'), animations: 'disabled' });
  });
}
