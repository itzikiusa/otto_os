import { test, expect, type Browser } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';
import { processResources } from './process-resources';
import type { TelemetryConfig } from '../src/lib/api/types';

type RawLoad = { sockets: WebSocket[]; timer: number; sent: number[]; received: number[] };
type LoadWindow = Window & { __matchedLoad: RawLoad };
function duration(name: string, fallback: number) {
  const value = Number(process.env[name] ?? fallback);
  if (!Number.isInteger(value) || value < 5 || value > 180) throw new Error(`${name} must be 5–180 seconds`);
  return value;
}

// Counterbalances the long off-before-on run. Each mode gets a NEW Chromium
// process, identical shell boot, five owned cat CLIs and no module traversal.
// No automatic page/browser fixture: that would leave another idle browser in
// the driver's resource tree while the manually owned browser is measured.
test.use({ trace: 'off', video: 'off', serviceWorkers: 'block' });
test('matched N5 telemetry: fresh Chromium on-before-off and quiet recovery', async ({ playwright }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'Matched comparison uses Chromium like the original raw run');
  const seconds = duration('OTTO_MATCHED_LOAD_SECONDS', 90);
  const recovery = duration('OTTO_MATCHED_RECOVERY_SECONDS', 60);
  test.setTimeout((2 * (seconds + recovery) + 200) * 1000);
  const { ctx, base, token } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base, 'Matched telemetry load');
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  const daemon = JSON.parse(readFileSync(`e2e/.auth-${slot}/daemon.json`, 'utf8')) as { pid: number };
  const original: TelemetryConfig = await (await ctx.get(`${base}/api/v1/telemetry/config`)).json();
  const samples: (ReturnType<typeof processResources>[number] & { enabled: boolean; phase: string })[] = [];
  const phases: unknown[] = [];
  const owned = new Set<string>();
  const output = info.outputPath('matched-n5-load.json');
  let browser: Browser | undefined;
  const sample = (enabled: boolean, phase: string) => {
    const rows = processResources(daemon.pid);
    if (!rows.some((row) => row.process === 'browser' && row.pids.length > 0)) throw new Error('Owned browser is missing from resource sample');
    samples.push(...rows.map((row) => ({ ...row, enabled, phase })));
  };
  try {
    for (const enabled of [true, false]) {
      console.log(`matched N5: collection=${enabled ? 'on' : 'off'}, fresh Chromium`);
      expect((await ctx.put(`${base}/api/v1/telemetry/config`, { data: { ...original, enabled } })).ok()).toBeTruthy();
      await expect.poll(async () => (await (await ctx.get(`${base}/api/v1/telemetry/status`)).json()).collector_ready, { timeout: 100_000 }).toBe(enabled);
      browser = await playwright.chromium.launch();
      const context = await browser.newContext({
        baseURL: info.project.use.baseURL,
        storageState: `e2e/.auth-${slot}/state.json`,
        viewport: { width: 1280, height: 800 }, serviceWorkers: 'block',
      });
      await context.addInitScript((id) => {
        localStorage.setItem('otto_workspace', id);
        localStorage.setItem('otto_firstrun_dismissed', '1');
      }, workspace);
      const page = await context.newPage();
      await page.goto('/#/agents');
      await expect(page.locator('.shell')).toBeVisible();
      await expect.poll(() => page.evaluate(async () => (await import(/* @vite-ignore */ String('/src/lib/telemetry.ts'))).telemetryState().enabled)).toBe(enabled);
      const cdp = await context.newCDPSession(page);
      // Observe the JS heap without forcing GC or changing its natural timing.
      const initialHeap = await cdp.send('Runtime.getHeapUsage');
      sample(enabled, 'before-load');
      const ids: string[] = [];
      for (let index = 0; index < 5; index++) {
        const response = await ctx.post(`${base}/api/v1/workspaces/${workspace}/sessions`, {
          data: { kind: 'agent', provider: 'codex', title: 'Matched telemetry fixture', cwd: '/tmp', meta: { origin: 'manual' } },
        });
        expect(response.ok()).toBeTruthy();
        const { id } = await response.json(); ids.push(id); owned.add(id);
      }
      await page.evaluate(async ({ ids, base, token }) => {
        const state: RawLoad = { sockets: [], timer: 0, sent: ids.map(() => 0), received: ids.map(() => 0) };
        (window as unknown as LoadWindow).__matchedLoad = state;
        await Promise.all(ids.map((id, index) => new Promise<void>((resolve, reject) => {
          const socket = new WebSocket(`${base.replace('http', 'ws')}/ws/term/${id}`, ['otto-bearer', token]);
          socket.binaryType = 'arraybuffer';
          state.sockets.push(socket);
          socket.onopen = () => resolve(); socket.onerror = () => reject(new Error('Owned fixture WebSocket failed'));
          socket.onmessage = (event) => { if (event.data instanceof ArrayBuffer) state.received[index] += event.data.byteLength; };
        })));
        // Same payload/cadence as the original raw run: 4324 input bytes/s/session.
        state.timer = window.setInterval(() => {
          for (const [index, socket] of state.sockets.entries()) if (socket.readyState === WebSocket.OPEN) {
            const data = 'telemetry-fixture '.repeat(60) + '\n';
            socket.send(JSON.stringify({ type: 'input', data: btoa(data) })); state.sent[index] += data.length;
          }
        }, 250);
      }, { ids, base, token });
      const started = Date.now();
      const stop = started + seconds * 1000;
      const listUrl = `${base}/api/v1/workspaces/${workspace}/sessions?ids=${encodeURIComponent(ids.join(','))}`;
      const workloads = Array.from({ length: 5 }, async () => {
        const result = { requests: 0, response_bytes: 0, request_ms: 0 };
        while (Date.now() < stop) {
          const at = performance.now();
          const response = await fetch(listUrl, { headers: { Authorization: `Bearer ${token}` }, signal: AbortSignal.timeout(10_000) });
          if (!response.ok) throw new Error(`Fixture request returned HTTP ${response.status}`);
          const body = await response.arrayBuffer();
          result.requests++; result.response_bytes += body.byteLength; result.request_ms += performance.now() - at;
          await new Promise((resolve) => setTimeout(resolve, 50));
        }
        return result;
      });
      while (Date.now() < stop) {
        sample(enabled, 'load');
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
      const complete = await Promise.all(workloads);
      const elapsed = (Date.now() - started) / 1000;
      const protocol = await page.evaluate(() => {
        const state = (window as unknown as LoadWindow).__matchedLoad;
        clearInterval(state.timer); state.sockets.forEach((socket) => socket.close());
        return { input_bytes: state.sent, output_bytes: state.received };
      });
      expect(protocol.input_bytes.every((bytes) => bytes > 0 && bytes <= Math.ceil(elapsed * 4 + 1) * 1081)).toBeTruthy();
      expect(protocol.output_bytes.every((bytes) => bytes > 0)).toBeTruthy();
      const loadedHeap = await cdp.send('Runtime.getHeapUsage');
      for (const id of ids) { expect((await ctx.delete(`${base}/api/v1/sessions/${id}`)).ok()).toBeTruthy(); owned.delete(id); }
      await page.evaluate(async () => (await import(/* @vite-ignore */ String('/src/lib/telemetry.ts'))).flushTelemetry());
      for (let second = 0; second < recovery; second++) {
        sample(enabled, 'recovery');
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
      const requests = complete.reduce((sum, row) => sum + row.requests, 0);
      phases.push({ enabled, concurrency: 5, requested_load_seconds: seconds, load_seconds: elapsed, recovery_samples: recovery,
        requests, requests_per_second: requests / elapsed,
        response_bytes: complete.reduce((sum, row) => sum + row.response_bytes, 0),
        mean_request_ms: complete.reduce((sum, row) => sum + row.request_ms, 0) / requests,
        protocol, heap: { before: initialHeap, after_load: loadedHeap, after_recovery: await cdp.send('Runtime.getHeapUsage') },
        status: await (await ctx.get(`${base}/api/v1/telemetry/status`)).json(),
      });
      await browser.close(); browser = undefined;
    }
  } finally {
    await browser?.close();
    for (const id of owned) await ctx.delete(`${base}/api/v1/sessions/${id}`).catch(() => {});
    writeFileSync(output, JSON.stringify({ engine: 'chromium', order: [true, false], fresh_browser_per_phase: true,
      module_walk: false, concurrency: 5, input_bytes_per_second_per_session: 4324,
      requested_load_seconds: seconds, requested_recovery_samples: recovery, phases, samples,
      limitations: ['Same daemon and ClickHouse across phases; browser state is reset, backend caches are not.',
        'One on-before-off pair complements the earlier off-before-on sequence; not a statistical confidence interval.',
        'ps CPU estimates and RSS include shared pages; driver excluded. JS heap observed without forced GC.',
        'Raw real local PTY transport with fake provider CLI, not rendered-terminal or remote-inference performance.'],
    }, null, 2));
    await info.attach('matched-n5-load', { path: output, contentType: 'application/json' });
    await ctx.put(`${base}/api/v1/telemetry/config`, { data: original });
    await ctx.dispose();
  }
});
