import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedShellSession } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block', contextOptions: { reducedMotion: 'reduce' } });
let workspaceId = '';
let sessionId = '';
const imageArtifact = { id: 'image', kind: 'image', label: 'Release architecture illustration with the complete deliberately long descriptive identity.svg', path: '/tmp/release/review/complete-architecture-illustration-for-the-september-release.svg', url: null, mime: 'image/svg+xml', produced_at: '2026-09-25T10:00:00Z', turn_id: 'turn' };
const svg = '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="320" height="180" fill="gray"/><text x="20" y="90">Synthetic release illustration</text></svg>';
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  sessionId = await seedShellSession(ctx, base, workspaceId);
  await ctx.dispose();
});
async function boot(page: Page, outputs = true) {
  await page.addInitScript(({ workspaceId, outputs }) => {
    localStorage.setItem('otto_workspace', workspaceId);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_right_open', outputs ? '1' : '0');
    localStorage.setItem('otto_right_tab', 'outputs');
    localStorage.setItem('otto_view_mode', 'tabs');
  }, { workspaceId, outputs });
  await page.goto(`/#/agents/${sessionId}`);
  await expect(page.locator('.xterm-screen')).toBeVisible();
}
for (const scheme of ['light', 'dark']) {
  test(`split preserves a readable full terminal grid ${scheme}`, async ({ page }, info) => {
    const resizeColumns: number[] = [];
    page.on('websocket', socket => socket.on('framesent', ({ payload }) => {
      if (typeof payload !== 'string') return;
      try { const frame = JSON.parse(payload); if (frame.type === 'resize') resizeColumns.push(frame.cols); } catch { /* binary input */ }
    }));
    const { ctx, base } = await apiCtx();
    const res = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/sessions`, { data: { kind: 'agent', provider: 'shell', title: 'Release review', cwd: '/tmp', meta: { origin: 'e2e', nested_provider: 'claude', e2e_transcript_path: `${process.cwd()}/../crates/otto-transcript/fixtures/claude/01-basic-tools.jsonl` } } });
    const session = await res.json();
    await ctx.post(`${base}/api/v1/sessions/${session.id}/input`, { data: { text: "printf '%079dZ\\n' 1", submit: true } });
    await ctx.dispose();
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript(scheme => localStorage.setItem('otto_scheme', scheme), scheme);
    await boot(page, false);
    await page.goto(`/#/agents/${session.id}`);
    await page.getByRole('tab', { name: 'Split', exact: true }).click();
    await expect(page.locator('.conv[data-loaded="true"]')).toBeVisible();
    const host = page.locator('.term-host');
    await page.waitForTimeout(1600);
    await expect.poll(() => host.evaluate(el => parseFloat(getComputedStyle(el.querySelector('.xterm-rows')!).fontSize))).toBeGreaterThanOrEqual(11);
    await expect.poll(() => host.getAttribute('data-cols')).toMatch(/^(8\d|9\d|\d{3,})$/);
    const measure = await host.evaluate(el => {
      const x = el as HTMLElement;
      const screen = el.querySelector('.xterm-screen')!.getBoundingClientRect();
      const before = x.scrollLeft;
      x.scrollLeft = x.scrollWidth;
      return { scrollable: x.scrollLeft > before, screenWidth: screen.width, available: x.clientWidth, right: screen.right - x.scrollLeft, hostRight: x.getBoundingClientRect().right };
    });
    expect(measure.scrollable).toBe(true);
    expect(measure.right).toBeLessThanOrEqual(measure.hostRight + 1);
    await expectNoHorizontalOverflow(page);
    expect(resizeColumns.length).toBeGreaterThan(0);
    expect(Math.min(...resizeColumns), 'no transient narrow PTY grid').toBeGreaterThanOrEqual(80);
    await host.evaluate(el => { el.scrollLeft = 0; });
    await host.focus();
    await expect(host).toBeFocused();
    await page.keyboard.press('ArrowRight');
    await expect.poll(() => host.evaluate(el => el.scrollLeft)).toBeGreaterThan(0);
    await host.evaluate(el => { el.scrollLeft = 0; });
    await page.screenshot({ path: info.outputPath('readable-split.png'), animations: 'disabled' });
    await page.getByRole('tab', { name: 'Terminal', exact: true }).click();
    await expect.poll(() => host.evaluate(el => parseFloat(getComputedStyle(el.querySelector('.xterm-rows')!).fontSize))).toBe(13);
    await expect.poll(() => host.evaluate(el => el.scrollWidth - el.clientWidth)).toBeLessThanOrEqual(1);
    for (let pass = 0; pass < 2; pass++) {
      await page.getByRole('tab', { name: 'Split', exact: true }).click();
      await expect.poll(() => host.evaluate(el => parseFloat(getComputedStyle(el.querySelector('.xterm-rows')!).fontSize))).toBe(11);
      await page.getByRole('tab', { name: 'Terminal', exact: true }).click();
      await expect.poll(() => host.evaluate(el => parseFloat(getComputedStyle(el.querySelector('.xterm-rows')!).fontSize))).toBe(13);
    }
    await page.waitForTimeout(1200);
    const settled = resizeColumns.length;
    await page.waitForTimeout(700);
    expect(resizeColumns.length, 'no resize churn after the pane settles').toBe(settled);
    expect(Math.min(...resizeColumns)).toBeGreaterThanOrEqual(80);
  });
}
test('Outputs opens its first item and exposes the complete identity without hover', async ({ page }, info) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route(`**/sessions/${sessionId}/artifacts`, route => route.fulfill({ json: [imageArtifact] }));
  await page.route(`**/sessions/${sessionId}/artifacts/image`, route => route.fulfill({ body: svg, contentType: 'image/svg+xml' }));
  await boot(page);
  const panel = page.getByTestId('outputs-panel');
  await expect(panel.getByRole('option')).toHaveAttribute('aria-selected', 'true');
  await expect(panel.getByRole('img', { name: imageArtifact.label })).toBeVisible();
  await expect(panel.getByText(imageArtifact.path, { exact: true })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('outputs-identity-phone.png'), animations: 'disabled' });
});
test('Outputs revokes image URLs resolving after panel unmount', async ({ page }) => {
  await page.addInitScript(() => {
    const created: string[] = [], revoked: string[] = [];
    Object.assign(window, { blobAudit: { created, revoked } });
    const create = URL.createObjectURL.bind(URL), revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = blob => { const url = create(blob); created.push(url); return url; };
    URL.revokeObjectURL = url => { revoked.push(url); revoke(url); };
  });
  await page.route(`**/sessions/${sessionId}/artifacts`, route => route.fulfill({ json: [imageArtifact] }));
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  let requested = false;
  await page.route(`**/sessions/${sessionId}/artifacts/image`, async route => {
    requested = true;
    await held;
    await route.fulfill({ body: svg, contentType: 'image/svg+xml' });
  });
  await boot(page);
  await page.getByTestId('outputs-panel').getByRole('option').click();
  await expect.poll(() => requested).toBe(true);
  await page.getByRole('tab', { name: 'Outputs', exact: true }).focus();
  await page.keyboard.press('ArrowLeft');
  await expect(page.getByTestId('outputs-panel')).toBeHidden();
  release();
  await expect.poll(() => page.evaluate(() => (window as unknown as { blobAudit: { created: string[] } }).blobAudit.created.length)).toBeGreaterThan(0);
  await expect.poll(() => page.evaluate(() => {
    const { created, revoked } = (window as unknown as { blobAudit: { created: string[]; revoked: string[] } }).blobAudit;
    return created.filter(url => !revoked.includes(url));
  })).toEqual([]);
});

test('terminal preserves visible scrollback when a delayed compact arrives', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  await ctx.post(`${base}/api/v1/sessions/${sessionId}/input`, { data: { text: 'for i in $(seq 1 180); do echo SCROLL-REVIEW-$i; done', submit: true } });
  await ctx.dispose();
  let release: (() => void) | undefined;
  const grids: string[] = [];
  let snapshotRequests = 0;
  await page.routeWebSocket('**/ws/term/**', socket => {
    const server = socket.connectToServer();
    socket.onMessage(message => {
      if (typeof message === 'string') { const frame = JSON.parse(message); if (frame.type === 'resize') grids.push(`${frame.cols}x${frame.rows}`); if (frame.type === 'scrollback') snapshotRequests++; }
      server.send(message);
    });
    let snapshots = 0;
    const queued: (string | Buffer)[] = [];
    server.onMessage(message => {
      if (release) { queued.push(message); return; }
      if (typeof message === 'string' && JSON.parse(message).type === 'scrollback' && ++snapshots === 2) {
        queued.push(message);
        // Preserve WS ordering: delay this snapshot AND every later frame.
        release = () => { for (const frame of queued.splice(0)) socket.send(frame); };
      } else socket.send(message);
    });
  });
  await boot(page, false);
  await page.waitForTimeout(1600);
  await page.locator('.xterm-screen').hover();
  await page.mouse.wheel(0, 100000);
  await page.waitForTimeout(300);
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect.poll(() => !!release).toBe(true);
  const previousGrid = grids.at(-1);
  await page.setViewportSize({ width: 1200, height: 850 });
  await expect.poll(() => grids.at(-1)).not.toBe(previousGrid);
  await page.waitForTimeout(1300);
  expect(snapshotRequests, 'attach plus only one pending optional compact').toBe(2);
  const firstRow = page.locator('.xterm-rows > div').first();
  const slider = page.locator('.xterm-scrollable-element > .scrollbar.vertical > .slider');
  const track = page.locator('.xterm-scrollable-element > .scrollbar.vertical');
  await page.locator('.xterm-screen').hover();
  await page.mouse.wheel(0, 100000);
  await expect.poll(() => slider.evaluate(el => parseFloat(getComputedStyle(el).top))).toBeGreaterThan(0);
  expect((await slider.boundingBox())!.height).toBeLessThan((await track.boundingBox())!.height);
  const bottomRow = await firstRow.innerText();
  const bottomTop = await slider.evaluate(el => parseFloat(getComputedStyle(el).top));
  for (let step = 0; step < 10; step++) {
    await page.mouse.wheel(0, -1000);
    await page.waitForTimeout(40);
  }
  await expect.poll(() => firstRow.innerText()).not.toBe(bottomRow);
  await expect.poll(() => slider.evaluate(el => parseFloat(getComputedStyle(el).top))).toBeLessThan(bottomTop - 10);
  const readingTop = await slider.evaluate(el => parseFloat(getComputedStyle(el).top));
  const readingRow = await firstRow.innerText();
  release!();
  await page.waitForTimeout(300);
  await expect(firstRow).toHaveText(readingRow);
  await expect.poll(() => slider.evaluate(el => parseFloat(getComputedStyle(el).top))).toBe(readingTop);
});

for (const scheme of ['light', 'dark']) {
  test(`tray shows failed work fetch with Retry ${scheme}`, async ({ page }, info) => {
    await page.setViewportSize({ width: 360, height: 520 });
    await page.addInitScript(scheme => localStorage.setItem('otto_scheme', scheme), scheme);
    await page.route('**/api/v1/workspaces', route => route.fulfill({ json: [{ id: workspaceId, name: 'Release workspace', role: 'owner' }] }));
    let fail = true;
    await page.route('**/api/v1/workspaces/*/sessions', route => route.fulfill(fail ? { status: 502, json: { code: 'upstream', message: 'Sessions temporarily unavailable' } } : { json: [] }));
    await page.route('**/api/v1/mcp/approvals?*', route => route.fulfill({ json: [] }));
    await page.route('**/api/v1/notifications', route => route.fulfill({ json: [] }));
    await page.goto('/#/tray');
    await expect(page.getByRole('alert')).toContainText('Sessions temporarily unavailable');
    await expect(page.getByText('No agents are working.')).toBeHidden();
    fail = false;
    await page.getByRole('button', { name: 'Retry', exact: true }).click();
    await expect(page.getByText('No agents are working.')).toBeVisible();
    await expect(page.getByRole('alert')).toBeHidden();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath('tray-recovered.png'), animations: 'disabled' });
  });
}

test('Outputs respects explicit close and makes overflowing identity keyboard-scrollable', async ({ page }) => {
  const long = { ...imageArtifact, label: imageArtifact.label.repeat(8), path: '/tmp/' + 'release/long-component/'.repeat(40) + 'final.svg' };
  await page.setViewportSize({ width: 390, height: 844 });
  await page.route(`**/sessions/${sessionId}/artifacts`, route => route.fulfill({ json: [long] }));
  await page.route(`**/sessions/${sessionId}/artifacts/image`, route => route.fulfill({ body: svg, contentType: 'image/svg+xml' }));
  await boot(page);
  const panel = page.getByTestId('outputs-panel');
  await expect(panel.getByRole('img')).toBeVisible();
  for (const name of ['Output name', 'Output path']) {
    const region = panel.getByRole('region', { name, exact: true });
    await region.focus();
    await expect(region).toBeFocused();
    await page.keyboard.press('ArrowDown');
    await expect.poll(() => region.evaluate(el => el.scrollTop)).toBeGreaterThan(0);
  }
  await panel.getByRole('button', { name: 'Close preview', exact: true }).click();
  await expect(page.getByTestId('outputs-preview')).toBeHidden();
  await page.waitForTimeout(250);
  await expect(page.getByTestId('outputs-preview')).toBeHidden();
  await panel.getByRole('option').click();
  await expect(panel.getByRole('img')).toBeVisible();
});

test('desktop assistant bar renders its compact command surface', async ({ page }, info) => {
  await page.setViewportSize({ width: 640, height: 360 });
  await page.addInitScript(() => { localStorage.setItem('otto_scheme', 'dark'); localStorage.setItem('otto_orch_fallback', '0'); });
  await page.goto('/#/bar');
  const input = page.getByRole('combobox', { name: 'Ask Otto or search commands' });
  await expect(input).toBeVisible();
  await input.fill('review release');
  await expect(page.getByRole('option').filter({ hasText: 'Ask Otto' })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('desktop-bar.png'), animations: 'disabled' });
  await page.keyboard.press('Escape');
  await expect(input).toHaveValue('');
});

test('tray long approval list stays scrollable and opens its review destination', async ({ page }, info) => {
  await page.setViewportSize({ width: 360, height: 520 });
  await page.addInitScript(() => { localStorage.setItem('otto_theme', 'warm'); localStorage.setItem('otto_scheme', 'dark'); });
  await page.route('**/api/v1/workspaces', route => route.fulfill({ json: [{ id: workspaceId, name: 'Release workspace', role: 'owner' }] }));
  await page.route('**/api/v1/workspaces/*/sessions', route => route.fulfill({ json: [] }));
  await page.route('**/api/v1/mcp/approvals?*', route => route.fulfill({ json: Array.from({ length: 16 }, (_, i) => ({ id: `approval-${i}`, title: `Review release package ${i + 1} before publishing its generated documentation`, server_name: 'Release tools' })) }));
  await page.route('**/api/v1/notifications', route => route.fulfill({ json: [] }));
  await page.goto('/#/tray');
  const rows = page.getByRole('button', { name: /Review release package/ });
  await expect(rows).toHaveCount(16);
  await rows.last().scrollIntoViewIfNeeded();
  await expect(rows.last()).toBeInViewport();
  await expect(page.getByRole('button', { name: 'Open Otto', exact: true })).toBeInViewport();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: info.outputPath('tray-loaded-warm.png'), animations: 'disabled' });
  await rows.last().click();
  await expect(page).toHaveURL(/#\/mcp\/activity$/);
});

test('terminal waits for its attach snapshot before requesting an optional compact', async ({ page }) => {
  let requests = 0;
  let release: (() => void) | undefined;
  await page.routeWebSocket('**/ws/term/**', socket => {
    const server = socket.connectToServer();
    socket.onMessage(message => {
      if (typeof message === 'string' && JSON.parse(message).type === 'scrollback') requests++;
      server.send(message);
    });
    const queued: (string | Buffer)[] = [];
    server.onMessage(message => {
      if (release) { queued.push(message); return; }
      if (typeof message === 'string' && JSON.parse(message).type === 'scrollback') {
        queued.push(message);
        release = () => { for (const frame of queued.splice(0)) socket.send(frame); };
      } else socket.send(message);
    });
  });
  await boot(page, false);
  await expect.poll(() => !!release).toBe(true);
  try {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.waitForTimeout(1800);
    expect(requests, 'only the initial snapshot until its epoch is known').toBe(1);
  } finally { release?.(); }
});
