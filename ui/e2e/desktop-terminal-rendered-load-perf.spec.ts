import { test, expect } from '@playwright/test';
import { readFileSync, writeFileSync } from 'node:fs';
import { apiCtx, seedWorkspace } from './seed';
import { dist, useDefaultTerminalRenderer, watchFatalUiErrors } from './perf';
import { processResources } from './process-resources';
import { CREDIT_ACK_STEP } from '../src/lib/components/termFlow';
import type { TelemetryConfig } from '../src/lib/api/types';

type Probe = {
  sessionId(): string; pending(): number; queued(): number; text(): string;
  renderer(): string; disposed: boolean; onRender(cb: () => void): { dispose(): void } | undefined;
};
type TerminalRun = {
  running: boolean; timer: number; frame: number; frames: number[];
  rows: { id: string; sent: number; rendered: number; bytes: number; latencies: number[];
    pending: { marker: string; at: number } | null; missed: number; peakPending: number; peakQueued: number; renderer: string }[];
  subscriptions: { dispose(): void }[];
};
type LoadWindow = Window & { __ottoTermProbe: Probe[]; __renderedLoad: TerminalRun };

function duration(name: string, fallback: number): number {
  const value = Number(process.env[name] ?? fallback);
  if (!Number.isInteger(value) || value < 5 || value > 300) throw new Error(`${name} must be 5–300 seconds`);
  return value;
}

// Real Terminal -> daemon PTY -> Terminal parser/renderer. Only provider CLI
// execution is substituted by global-setup's owned cat shim; no WS mocking.
test.use({ serviceWorkers: 'block', trace: 'off', video: 'off' });
test('1/3/5 rendered terminals: bounded input, resource curves and quiet recovery', async ({ page }, info) => {
  const seconds = duration('OTTO_TERMINAL_LOAD_SECONDS', 90);
  const recovery = duration('OTTO_TERMINAL_RECOVERY_SECONDS', 30);
  test.setTimeout((3 * (seconds + recovery) + 180) * 1000);
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base, 'Rendered terminal load');
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  const daemon = JSON.parse(readFileSync(`e2e/.auth-${slot}/daemon.json`, 'utf8')) as { pid: number };
  const original: TelemetryConfig = await (await ctx.get(`${base}/api/v1/telemetry/config`)).json();
  const owned = new Set<string>();
  const samples: (ReturnType<typeof processResources>[number] & { concurrency: number; phase: string })[] = [];
  const phases: unknown[] = [];
  const fatal = watchFatalUiErrors(page);
  const protocol = { credit: 0, ack: 0, input: 0 };
  page.on('websocket', (socket) => {
    if (!socket.url().includes('/ws/term/')) return;
    socket.on('framereceived', ({ payload }) => {
      if (typeof payload !== 'string') return;
      try { if (JSON.parse(payload).type === 'credit') protocol.credit++; } catch { /* binary output */ }
    });
    socket.on('framesent', ({ payload }) => {
      if (typeof payload !== 'string') return;
      try {
        const type = JSON.parse(payload).type;
        if (type === 'ack') protocol.ack++;
        if (type === 'input') protocol.input++;
      } catch { /* no payload retained */ }
    });
  });
  await useDefaultTerminalRenderer(page);
  await page.addInitScript(() => { (window as unknown as LoadWindow).__ottoTermProbe = []; });
  try {
    expect((await ctx.put(`${base}/api/v1/telemetry/config`, { data: { ...original, enabled: true } })).ok()).toBeTruthy();
    for (const concurrency of [1, 3, 5]) {
      const ids: string[] = [];
      for (let i = 0; i < concurrency; i++) {
        const response = await ctx.post(`${base}/api/v1/workspaces/${workspace}/sessions`, {
          data: { kind: 'agent', provider: 'codex', title: 'Owned rendered load', cwd: '/tmp', meta: { origin: 'manual' } },
        });
        expect(response.ok()).toBeTruthy();
        const { id } = await response.json();
        ids.push(id); owned.add(id);
      }
      const initialProtocol = { ...protocol };
      await page.goto(`/e2e/fixtures/terminal-load.html?workspace=${encodeURIComponent(workspace)}&ids=${encodeURIComponent(ids.join(','))}`);
      await expect.poll(() => page.evaluate(() => (window as unknown as LoadWindow).__ottoTermProbe.filter((p) => !p.disposed).length)).toBe(concurrency);
      // Do not benchmark a disconnected/read-only fixture or a blank terminal.
      await expect.poll(() => page.evaluate(() => (window as unknown as LoadWindow).__ottoTermProbe.filter((p) => !p.disposed && p.text().includes('otto e2e:')).length)).toBe(concurrency);
      await expect.poll(() => protocol.credit - initialProtocol.credit).toBe(concurrency);
      await page.evaluate((ids) => {
        const w = window as unknown as LoadWindow;
        const probes = ids.map((id) => w.__ottoTermProbe.find((p) => !p.disposed && p.sessionId() === id)!);
        const inputs = [...document.querySelectorAll<HTMLTextAreaElement>('.xterm-helper-textarea')];
        if (inputs.length !== ids.length) throw new Error('Every terminal needs a real xterm input');
        const state: TerminalRun = { running: true, timer: 0, frame: 0, frames: [], subscriptions: [], rows: ids.map((id, index) => ({
          id, sent: 0, rendered: 0, bytes: 0, latencies: [], pending: null, missed: 0, peakPending: 0, peakQueued: 0, renderer: probes[index].renderer(),
        })) };
        w.__renderedLoad = state;
        for (const [index, probe] of probes.entries()) {
          const row = state.rows[index];
          const subscription = probe.onRender(() => {
            if (!row.pending || !probe.text().includes(row.pending.marker)) return;
            row.latencies.push(performance.now() - row.pending.at);
            row.rendered++;
            row.pending = null;
          });
          if (!subscription) throw new Error('Renderer observation unavailable');
          state.subscriptions.push(subscription);
        }
        let lastFrame = performance.now();
        const frame = () => {
          const now = performance.now();
          if (state.running && state.frames.length < 20_000) state.frames.push(now - lastFrame);
          lastFrame = now;
          for (const [index, probe] of probes.entries()) {
            state.rows[index].peakPending = Math.max(state.rows[index].peakPending, probe.pending());
            state.rows[index].peakQueued = Math.max(state.rows[index].peakQueued, probe.queued());
          }
          state.frame = requestAnimationFrame(frame);
        };
        state.frame = requestAnimationFrame(frame);
        // One <=1 KiB paste/second/session through xterm's real input handler.
        // The marker is last so it remains in even the smallest tile viewport.
        state.timer = window.setInterval(() => {
          for (const [index, row] of state.rows.entries()) {
            if (row.pending) { row.missed++; continue; }
            const marker = `L${index}S${String(row.sent + 1).padStart(4, '0')}Z`;
            const text = ('x'.repeat(70) + '\n').repeat(12) + marker + '\n';
            row.pending = { marker, at: performance.now() };
            row.sent++; row.bytes += text.length;
            const clipboardData = new DataTransfer();
            clipboardData.setData('text/plain', text);
            inputs[index].dispatchEvent(new ClipboardEvent('paste', { bubbles: true, cancelable: true, clipboardData }));
          }
        }, 1000);
      }, ids);
      const started = Date.now();
      for (let second = 0; second < seconds; second++) {
        samples.push(...processResources(daemon.pid).map((row) => ({ ...row, concurrency, phase: 'rendered-load' })));
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
      await page.evaluate(() => {
        const state = (window as unknown as LoadWindow).__renderedLoad;
        state.running = false; clearInterval(state.timer);
      });
      for (let second = 0; second < recovery; second++) {
        samples.push(...processResources(daemon.pid).map((row) => ({ ...row, concurrency, phase: 'quiet-recovery' })));
        await new Promise((resolve) => setTimeout(resolve, 1000));
      }
      const rendered = await page.evaluate(() => {
        const w = window as unknown as LoadWindow;
        const state = w.__renderedLoad;
        cancelAnimationFrame(state.frame);
        state.subscriptions.forEach((subscription) => subscription.dispose());
        return { rows: state.rows, frames: state.frames, remaining: w.__ottoTermProbe.filter((p) => !p.disposed).map((p) => ({ id: p.sessionId(), pending: p.pending(), queued: p.queued() })) };
      });
      const protocolDelta = { credit: protocol.credit - initialProtocol.credit, ack: protocol.ack - initialProtocol.ack, input: protocol.input - initialProtocol.input };
      phases.push({ concurrency, load_seconds: seconds, recovery_seconds: recovery, elapsed_seconds: (Date.now() - started) / 1000,
        rows: rendered.rows.map((row) => ({ ...row, latency_ms: dist(row.latencies) })), frame_ms: dist(rendered.frames), remaining: rendered.remaining, protocol: protocolDelta });
      expect(rendered.frames.length, 'frame observations must be real').toBeGreaterThan(seconds);
      // A 5-second harness smoke sends less than the real 64 KiB ACK batch.
      // Scored 90-second phases cross it even without PTY echo amplification.
      if (rendered.rows.some((row) => row.bytes >= CREDIT_ACK_STEP)) {
        expect(protocolDelta.ack, 'actual Terminal credit acknowledgements').toBeGreaterThan(0);
      }
      expect(protocolDelta.input, 'paste reached the real terminal socket').toBeGreaterThan(0);
      for (const row of rendered.rows) {
        expect(row.sent, 'every tile made sustained progress').toBeGreaterThanOrEqual(seconds - 2);
        expect(row.rendered, 'every sent marker was parsed and rendered').toBe(row.sent);
        expect(row.pending).toBeNull();
        expect(row.bytes / seconds, 'bounded input per fixture').toBeLessThanOrEqual(4096);
        expect(dist(row.latencies).p95, 'bounded 1 KiB input-to-render latency').toBeLessThan(1000);
      }
      expect(rendered.remaining.every((row) => row.pending === 0 && row.queued === 0), 'quiet recovery drains all terminal queues').toBe(true);
      expect(fatal).toEqual([]);
      await page.goto('about:blank');
      for (const id of ids) {
        expect((await ctx.delete(`${base}/api/v1/sessions/${id}`)).ok()).toBeTruthy();
        owned.delete(id);
      }
    }
  } finally {
    // A failed measurement is still an artifact; never lose its samples.
    const output = info.outputPath('rendered-terminal-load.json');
    writeFileSync(output, JSON.stringify({ engine: info.project.name, samples, phases, protocol, fatal,
      note: 'Real Terminal clipboard input, PTY cat shim, credit/ack transport and xterm rendering. No socket mocks. ps CPU estimates and RSS sampled once/second; driver excluded from totals. Quiet recovery keeps widgets mounted. Timing gate 1 s; this is not a claim of production latency or long-duration leak freedom.' }, null, 2));
    await info.attach('rendered-terminal-load', { path: output, contentType: 'application/json' });
    await page.goto('about:blank').catch(() => {});
    for (const id of owned) await ctx.delete(`${base}/api/v1/sessions/${id}`).catch(() => {});
    await ctx.put(`${base}/api/v1/telemetry/config`, { data: original });
    await ctx.dispose();
  }
});
