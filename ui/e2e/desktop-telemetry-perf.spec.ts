import { test, expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { apiCtx, seedWorkspace } from './seed';
import type { TelemetryConfig } from '../src/lib/api/types';

// Explicit coverage: every built-in module gets cold import + warm navigation.
// Populated widget/algorithm workloads are listed in the companion manifest.
// No providers or production resources are contacted; the daemon is disposable.
test.use({ serviceWorkers: 'block', trace: 'off', video: 'off' });
test('all application modules: off/on navigation cost and concurrent CPU/RAM', async ({ page, context }, info) => {
  test.setTimeout((Number(process.env.OTTO_TELEMETRY_LOAD_SECONDS ?? '20') * 6 + 300) * 1000);
  const { ctx, base, token } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  const original: TelemetryConfig = await (await ctx.get(`${base}/api/v1/telemetry/config`)).json();
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  const daemon: { pid: number } = JSON.parse(readFileSync(`e2e/.auth-${slot}/daemon.json`, 'utf8'));
  let modules: string[] = [];
  const rows: { enabled: boolean; module: string; duration_ms: number }[] = [];
  const requestPhases: { enabled: boolean; concurrency: number; seconds: number; requests: number; response_bytes: number; mean_request_ms: number; requests_per_second: number }[] = [];
  const samples: { enabled: boolean; concurrency: number; process: string; phase: 'navigation' | 'load'; cpu_percent: number; rss_mb: number; at: number }[] = [];
  const ownedSessions: string[] = [];
  const measurement = (enabled: boolean, concurrency: number, phase: 'navigation' | 'load' = 'load') => {
    const at = Date.now();
    // Read only process IDs/parents and resource totals, never command arguments.
    const output = execFileSync('ps', ['-axo', 'pid=,ppid=,%cpu=,rss=,comm='], { encoding: 'utf8' });
    const processes = output.trim().split('\n').map((line) => {
      const match = /^\s*(\d+)\s+(\d+)\s+([\d.]+)\s+(\d+)\s+(.+)$/.exec(line);
      return match ? { pid: Number(match[1]), parent: Number(match[2]), cpu: Number(match[3]), rss: Number(match[4]) / 1024, name: match[5] } : null;
    }).filter((row): row is NonNullable<typeof row> => row !== null);
    const parents = new Map(processes.map((row) => [row.pid, row.parent]));
    const descendant = (pid: number, root: number): boolean => {
      for (let depth = 0; depth < 64 && pid > 1; depth++) { if (pid === root) return true; pid = parents.get(pid) ?? 0; }
      return false;
    };
    const groups = new Map<string, { cpu: number; rss: number }>();
    for (const row of processes) {
      let label: string;
      if (row.pid === process.pid) label = 'load-driver';
      else if (row.pid === daemon.pid) label = 'daemon';
      else if (descendant(row.pid, daemon.pid)) label = row.name.includes('otelcol') ? 'collector' : row.name.includes('clickhouse') ? 'clickhouse' : 'agent-children';
      else if (descendant(row.pid, process.pid) && /chromium|chrome|webkit|MiniBrowser/i.test(row.name)) label = 'browser';
      else continue;
      const group = groups.get(label) ?? { cpu: 0, rss: 0 };
      group.cpu += row.cpu; group.rss += row.rss; groups.set(label, group);
    }
    const total = { cpu: 0, rss: 0 };
    for (const [label, group] of groups) {
      samples.push({ enabled, concurrency, process: label, phase, cpu_percent: group.cpu, rss_mb: group.rss, at });
      if (label !== 'load-driver') { total.cpu += group.cpu; total.rss += group.rss; }
    }
    samples.push({ enabled, concurrency, process: 'total', phase, cpu_percent: total.cpu, rss_mb: total.rss, at });
  };
  await context.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspace);
  await page.goto('/#/agents');
  await expect(page.locator('.shell')).toBeVisible();
  const snipImage = await page.evaluate(() => {
    const canvas = document.createElement('canvas');
    canvas.width = 400; canvas.height = 300;
    canvas.getContext('2d')!.fillRect(0, 0, 400, 300);
    return canvas.toDataURL('image/png').split(',')[1];
  });
  const snipResponse = await ctx.post(`${base}/api/v1/snips`, { data: { data_b64: snipImage, filename: 'telemetry-fixture.png' } });
  expect(snipResponse.ok()).toBeTruthy();
  const snipId: string = (await snipResponse.json()).id;
  modules = await page.evaluate(async () => {
    const { SIDEBAR_MODULES } = await import(/* @vite-ignore */ String('/src/lib/sidebar.ts'));
    // Secondary built-in routes are reachable through commands/settings rather
    // than sidebar rows. Plugin routes require an installed third-party plugin.
    return [...new Set<string>([...SIDEBAR_MODULES.map((m: { id: string }) => m.id),
      'database', 'brokers', 'canvas', 'settings', 'walkthroughs', 'snip'])];
  });
  try {
    for (const enabled of [false, true]) {
      expect((await ctx.put(`${base}/api/v1/telemetry/config`, { data: { ...original, enabled } })).ok()).toBeTruthy();
      await expect.poll(async () => (await (await ctx.get(`${base}/api/v1/telemetry/status`)).json()).collector_ready, { timeout: 100_000 }).toBe(enabled);
      await page.goto('/#/agents');
      // Hash-only goto keeps the existing document and its opt-out boot alive.
      // Reload after out-of-band API configuration so this phase boots afresh.
      await page.reload();
      await expect(page.locator('.shell')).toBeVisible();
      await expect.poll(() => page.evaluate(async () => (await import(/* @vite-ignore */ String('/src/lib/telemetry.ts'))).telemetryState().enabled)).toBe(enabled);
      for (const phase of ['warmup', 'measure']) {
        for (const module of modules) {
          const elapsed = await page.evaluate(async ({ name, route }) => {
            const pages = await import(/* @vite-ignore */ String('/src/shell/pages.svelte.ts'));
            const router = (await import(/* @vite-ignore */ String('/src/lib/router.svelte.ts'))).router;
            const started = performance.now();
            router.go(route);
            await pages.preloadRoute([name]);
            await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
            return performance.now() - started;
          }, { name: module, route: module === 'snip' ? `snip/${snipId}` : module });
          await expect.poll(() => page.evaluate(async () => (await import(/* @vite-ignore */ String('/src/lib/router.svelte.ts'))).router.module)).toBe(module);
          if (phase === 'measure') rows.push({ enabled, module, duration_ms: elapsed });
          // Snip is deliberately a standalone editor without app chrome.
          await expect(page.locator(module === 'snip' ? '.snip-canvas' : '.shell .content').first()).toBeVisible();
          measurement(enabled, 1, 'navigation');
        }
      }
      await page.evaluate(async () => (await import(/* @vite-ignore */ String('/src/lib/router.svelte.ts'))).router.go('agents'));
      await expect(page.locator('.shell')).toBeVisible();
      if (enabled) await expect.poll(async () => {
        const overview = await (await ctx.get(`${base}/api/v1/telemetry/overview?hours=1`)).json();
        return overview.operations.map((op: { name: string }) => op.name);
      }, { timeout: 30_000 }).toEqual(expect.arrayContaining(['ui.navigation', 'ui.render']));
      // Concurrent read workloads, sampled over time. Separate from UI timings.
      for (const concurrency of [1, 3, 5]) {
        console.log(`telemetry load: collection=${enabled ? 'on' : 'off'}, agents=${concurrency}`);
        const sessions: string[] = [];
        for (let i = 0; i < concurrency; i++) {
          const response = await ctx.post(`${base}/api/v1/workspaces/${workspace}/sessions`, { data: { kind: 'agent', provider: 'codex', title: 'Telemetry load fixture', cwd: '/tmp', meta: { origin: 'manual' } } });
          expect(response.ok()).toBeTruthy();
          const id: string = (await response.json()).id;
          sessions.push(id); ownedSessions.push(id);
        }
        await page.evaluate(async ({ ids, base, token }) => {
          const state = { sockets: [] as WebSocket[], timer: 0 };
          (window as unknown as { __telemetryLoad: typeof state }).__telemetryLoad = state;
          await Promise.all(ids.map((id) => new Promise<void>((resolve, reject) => {
            const socket = new WebSocket(`${base.replace('http', 'ws')}/ws/term/${id}`, ['otto-bearer', token]);
            state.sockets.push(socket);
            socket.onopen = () => resolve(); socket.onerror = () => reject(new Error('Fixture terminal did not open'));
            // The isolated daemon runs harmless CLI shims (cat), never a real provider.
          })));
          state.timer = window.setInterval(() => {
            for (const socket of state.sockets) if (socket.readyState === WebSocket.OPEN) socket.send(JSON.stringify({ type: 'input', data: btoa('telemetry-fixture '.repeat(60) + '\n') }));
          }, 250);
        }, { ids: sessions, base, token });
        const duration = Number(process.env.OTTO_TELEMETRY_LOAD_SECONDS ?? '20');
        const phaseStarted = Date.now();
        const stop = phaseStarted + duration * 1000;
        const listUrl = `${base}/api/v1/workspaces/${workspace}/sessions?ids=${encodeURIComponent(sessions.join(','))}`;
        const workloads = Array.from({ length: concurrency }, async () => {
          const result = { requests: 0, bytes: 0, milliseconds: 0 };
          while (Date.now() < stop) {
            const started = performance.now();
            // Playwright APIRequest/expect creates a retained test step for
            // every call, making long runs progressively measure the driver.
            // Native HTTP keeps the measured loop out of that bookkeeping.
            const response = await fetch(listUrl, { headers: { Authorization: `Bearer ${token}` }, signal: AbortSignal.timeout(10_000) });
            if (!response.ok) throw new Error(`Fixture load request returned HTTP ${response.status}`);
            const body = await response.arrayBuffer();
            result.requests++; result.bytes += body.byteLength;
            result.milliseconds += performance.now() - started;
            await new Promise((resolve) => setTimeout(resolve, 50));
          }
          return result;
        });
        while (Date.now() < stop) {
          measurement(enabled, concurrency);
          await new Promise((resolve) => setTimeout(resolve, 1000));
        }
        const completed = await Promise.all(workloads);
        const seconds = (Date.now() - phaseStarted) / 1000;
        const requests = completed.reduce((sum, row) => sum + row.requests, 0);
        requestPhases.push({ enabled, concurrency, seconds, requests,
          response_bytes: completed.reduce((sum, row) => sum + row.bytes, 0),
          mean_request_ms: completed.reduce((sum, row) => sum + row.milliseconds, 0) / requests,
          requests_per_second: requests / seconds });
        await page.evaluate(() => {
          const state = (window as unknown as { __telemetryLoad: { sockets: WebSocket[]; timer: number } }).__telemetryLoad;
          clearInterval(state.timer); state.sockets.forEach((socket) => socket.close());
        });
        // Delete these owned throwaway fixtures, not just their processes:
        // later phases must see the same list size and navigation state.
        for (const id of sessions) expect((await ctx.delete(`${base}/api/v1/sessions/${id}`)).ok()).toBeTruthy();
      }
    }
    expect(new Set(rows.map((r) => r.module)).size).toBe(modules.length);
    expect(rows.every((r) => Number.isFinite(r.duration_ms) && r.duration_ms < 10_000)).toBeTruthy();
    await page.evaluate(async () => (await import(/* @vite-ignore */ String('/src/lib/telemetry.ts'))).flushTelemetry());
    const status = await (await ctx.get(`${base}/api/v1/telemetry/status`)).json();
    const overview = await (await ctx.get(`${base}/api/v1/telemetry/overview?hours=1`)).json();
    const output = info.outputPath('component-load.json');
    writeFileSync(output, JSON.stringify({ engine: info.project.name, request_client: 'node-fetch', browser_trace: false, modules, rows, samples, request_phases: requestPhases, status, observed_operations: overview.operations, note: 'ps CPU is a platform process estimate, not exclusive operation CPU; time/RSS are sampled. Total excludes the separately reported load driver. Widget load cases are separate.' }, null, 2));
    await info.attach('component-load', { path: output, contentType: 'application/json' });
  } finally {
    for (const id of ownedSessions) await ctx.delete(`${base}/api/v1/sessions/${id}`).catch(() => {});
    await ctx.put(`${base}/api/v1/telemetry/config`, { data: original });
    await ctx.dispose();
  }
});
