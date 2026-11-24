import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { useDefaultTerminalRenderer } from './perf';

// A7b probe: six tiled shells streaming on the Agents page; the page-side
// rAF probe counts frames over 50 ms and the worst frame (printed, and held to
// a loose ceiling so a regression shows).
test('six tiled streaming terminals: frame delay on the Agents page', async ({ page }, info) => {
  test.skip(!['desktop-browser', 'desktop-webkit'].includes(info.project.name), 'desktop projects only');
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const ids: string[] = [];
  for (let i = 0; i < 6; i++) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, { data: { kind: 'agent', provider: 'shell', title: `Stream ${i}`, cwd: '/tmp', meta: { origin: 'e2e' } } });
    ids.push((await r.json()).id);
  }
  // The real renderer (the suite pins the DOM one for text assertions).
  if (process.env.OTTO_FRAMES_RENDERER !== 'dom') await useDefaultTerminalRenderer(page);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_agents_fold_nav', '1');
  }, wsId);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/agents');
  await page.getByTestId('agents-select-toggle').click();
  for (let i = 0; i < 6; i++) await page.getByLabel(`Select Stream ${i}`).check();
  await page.getByTestId('agents-tile-selected').click();
  await expect(page.locator('[data-tile-id] .term-host')).toHaveCount(6, { timeout: 30_000 });
  for (const id of ids) {
    await ctx.post(`${base}/api/v1/sessions/${id}/input`, { data: { text: 'for i in $(seq 1 4000); do echo "line $i $(date +%s%N) lorem ipsum dolor sit amet consectetur"; sleep 0.003; done', submit: true } });
  }
  // CPU profile (Chromium): the top self-time functions while streaming.
  const cdp = info.project.name === 'desktop-browser' ? await page.context().newCDPSession(page) : null;
  if (cdp) { await cdp.send('Profiler.enable'); await cdp.send('Profiler.start'); }
  const r = await page.evaluate(() => new Promise<{ frames: number; over50: number; worst: number }>((resolve) => {
    const gaps: number[] = [];
    let last = performance.now();
    const end = last + 10_000;
    const tick = (t: number) => { gaps.push(t - last); last = t; if (t < end) requestAnimationFrame(tick); else resolve({ frames: gaps.length, over50: gaps.filter((g) => g > 50).length, worst: Math.round(Math.max(...gaps)) }); };
    requestAnimationFrame(tick);
  }));
  console.log('FRAMES', info.project.name, JSON.stringify(r));
  if (cdp) {
    const { profile } = (await cdp.send('Profiler.stop')) as { profile: { nodes: { id: number; callFrame: { functionName: string; url: string; lineNumber: number } }[]; samples: number[]; timeDeltas: number[] } };
    const self = new Map<string, number>();
    const byId = new Map(profile.nodes.map((n) => [n.id, n]));
    profile.samples.forEach((id, i) => {
      const n = byId.get(id)!; const k = `${n.callFrame.functionName || '(anon)'} ${n.callFrame.url.split('/').pop()}:${n.callFrame.lineNumber}`;
      self.set(k, (self.get(k) ?? 0) + (profile.timeDeltas[i] ?? 0) / 1000);
    });
    const total = [...self.values()].reduce((a, b) => a + b, 0);
    console.log('PROFILE', JSON.stringify([...self].sort((a, b) => b[1] - a[1]).slice(0, 12).map(([k, v]) => [k, Math.round(v), `${Math.round((v / total) * 100)}%`])));
  }
  // Ceiling for the pinned DOM renderer (the suite default); the WebGL run
  // under headless software GL is reported, not gated.
  if (process.env.OTTO_FRAMES_RENDERER === 'dom') expect(r.over50, 'frames over 50 ms').toBeLessThan(10);
});

// Spec A2: a streaming thread pane (otto-fake-claude `stream`) tiled beside
// the six streaming terminals holds the same bar — no frame over 50 ms in
// the WebKit run (the engine closest to the app's WKWebView).
test('a streaming chat pane beside six streaming terminals: frame delay', async ({ page }, info) => {
  test.skip(!['desktop-browser', 'desktop-webkit'].includes(info.project.name), 'desktop projects only');
  test.setTimeout(120_000);
  const { ctx, base } = await apiCtx();
  const wsId = await seedWorkspace(ctx, base);
  const ids: string[] = [];
  for (let i = 0; i < 6; i++) {
    const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, { data: { kind: 'agent', provider: 'shell', title: `Stream ${i}`, cwd: '/tmp', meta: { origin: 'e2e' } } });
    ids.push((await r.json()).id);
  }
  const t = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'claude', cwd: '/tmp', title: 'Stream chat' } });
  expect(t.ok(), await t.text()).toBeTruthy();
  const threadId = (await t.json()).id as string;
  if (process.env.OTTO_FRAMES_RENDERER !== 'dom') await useDefaultTerminalRenderer(page);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
    localStorage.setItem('otto_agents_fold_nav', '1');
  }, wsId);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/#/agents');
  await page.getByTestId('agents-select-toggle').click();
  for (let i = 0; i < 6; i++) await page.getByLabel(`Select Stream ${i}`).check();
  await page.getByTestId('agents-tile-selected').click();
  await expect(page.locator('[data-tile-id] .term-host')).toHaveCount(6, { timeout: 30_000 });
  // A plain click keeps the grid: the thread tiles beside the terminals.
  await page.getByTestId(`agents-row-${threadId}`).click();
  await expect(page.getByTestId(`thread-pane-${threadId}`).getByTestId('runtime-composer')).toBeVisible({ timeout: 30_000 });
  for (const id of ids) {
    await ctx.post(`${base}/api/v1/sessions/${id}/input`, { data: { text: 'for i in $(seq 1 4000); do echo "line $i $(date +%s%N) lorem ipsum dolor sit amet consectetur"; sleep 0.003; done', submit: true } });
  }
  const p = await ctx.post(`${base}/api/v1/runtime-sessions/${threadId}/prompt`, { data: { text: 'stream 500 20' } });
  expect(p.ok(), await p.text()).toBeTruthy();
  await expect(page.getByTestId(`thread-pane-${threadId}`).getByText(/c3 /)).toBeVisible({ timeout: 15_000 });
  const r = await page.evaluate(() => new Promise<{ frames: number; over50: number; worst: number }>((resolve) => {
    const gaps: number[] = [];
    let last = performance.now();
    const end = last + 8_000;
    const tick = (t: number) => { gaps.push(t - last); last = t; if (t < end) requestAnimationFrame(tick); else resolve({ frames: gaps.length, over50: gaps.filter((g) => g > 50).length, worst: Math.round(Math.max(...gaps)) }); };
    requestAnimationFrame(tick);
  }));
  console.log('FRAMES+CHAT', info.project.name, JSON.stringify(r));
  await ctx.post(`${base}/api/v1/runtime-sessions/${threadId}/close`).catch(() => {});
  // Gated on the pinned DOM renderer, like the six-terminal probe above.
  if (info.project.name === 'desktop-webkit' && process.env.OTTO_FRAMES_RENDERER === 'dom') expect(r.over50, 'frames over 50 ms').toBe(0);
});
