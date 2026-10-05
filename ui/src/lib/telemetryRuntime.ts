import { installTelemetryRuntime, type UiSpan, type Measurement } from './telemetry';
import { pollWhileVisible, type Poller } from './poll';
/** Bounded, opt-in application measurements. No URLs, payloads or user text. */

export const TELEMETRY_COMPONENTS = [
  'agents', 'workbench', 'connections', 'history', 'assistant', 'home', 'git', 'database', 'api', 'vault',
  'mission-control', 'rooms', 'workflows', 'scheduled-tasks', 'personal-agents',
  'loops', 'swarm', 'product', 'design', 'canvas', 'browser', 'run-with-otto',
  'mcp', 'brokers', 'kubernetes', 'aws', 'insights', 'usage', 'skills-eval',
  'proof', 'settings', 'walkthroughs', 'plugin', 'snip', 'shell', 'other',
] as const;
const components = new Set<string>(TELEMETRY_COMPONENTS);
export function telemetryComponent(value: string): string {
  return components.has(value) ? value : 'other';
}

/** Never retain a path segment unless it is a known, static module name. */
export function apiComponent(path: string): string {
  const parts = path.split('?')[0].split('/').filter(Boolean);
  const root = parts[0];
  if (root === 'workspaces') return telemetryComponent(parts[2] ?? 'other');
  if (root === 'sessions') return 'agents';
  if (root === 'repos') return 'git';
  if (root === 'connections' || root === 'db') return 'database';
  if (root === 'k8s') return 'kubernetes';
  return telemetryComponent(root ?? 'other');
}

type Sender = (spans: UiSpan[], signal: AbortSignal) => Promise<void>;

type Context = { traceId: string; spanId: string };
const MAX_QUEUE = 500;
const BATCH_SIZE = 100;
let enabled = false;
let generation = 0;
let queue: UiSpan[] = [];
let sender: Sender | null = null;
let timer: Poller | null = null;
let pending: AbortController | null = null;
let observer: PerformanceObserver | null = null;
let navigation: Context | null = null;
let navGeneration = 0;
let dropped = 0;
let flushScheduled = false;

function id(bytes: number): string {
  return Array.from(crypto.getRandomValues(new Uint8Array(bytes)), (n) => n.toString(16).padStart(2, '0')).join('');
}

export function isTelemetryEnabled(): boolean { return enabled; }

export function telemetryState(): { enabled: boolean; queued: number; dropped: number } {
  return { enabled, queued: queue.length, dropped };
}

export function configureTelemetry(active: boolean, send?: Sender): void {
  if (send) sender = send;
  if (active === enabled) return;
  enabled = active;
  generation++;
  if (!active) {
    queue = [];
    navigation = null;
    pending?.abort();
    pending = null;
    timer?.stop();
    timer = null;
    observer?.disconnect();
    observer = null;
    return;
  }
  timer = pollWhileVisible(async (signal) => { await flushTelemetry(); await sampleFrameDelay(signal); }, { ms: 5000, immediate: false });
  // WebKit does not currently implement longtask. Absence means unsupported.
  if (typeof PerformanceObserver !== 'undefined' && PerformanceObserver.supportedEntryTypes.includes('longtask')) {
    observer = new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        const measure = startMeasurement('ui.long_task', 'shell', 'internal', entry.startTime);
        measure?.finish('ok', { 'otto.duration_ms': entry.duration });
      }
    });
    observer.observe({ type: 'longtask' });
  }
}

function scheduleFullBatch(): void {
  if (!enabled || pending || flushScheduled || queue.length < BATCH_SIZE) return;
  flushScheduled = true;
  queueMicrotask(() => { flushScheduled = false; void flushTelemetry(); });
}

export async function flushTelemetry(): Promise<void> {
  if (!enabled || pending || !sender || !queue.length) return;
  const controller = new AbortController();
  pending = controller;
  const batch = queue.splice(0, BATCH_SIZE);
  const epoch = generation;
  const timeout = setTimeout(() => controller.abort(), 3000);
  let accepted = false;
  try {
    await sender(batch, controller.signal);
    accepted = true;
  } catch {
    // Best effort, never a retry storm or an unbounded offline backlog.
    if (epoch === generation) dropped += batch.length;
  } finally {
    clearTimeout(timeout);
    if (pending === controller) pending = null;
    if (accepted && epoch === generation) scheduleFullBatch();
  }
}

const attributesAllowed = new Set(['http.request.method', 'http.response.status_code', 'otto.lane', 'otto.phase']);
export function startMeasurement(name: string, component: string, kind = 'internal', start = performance.now()): Measurement | null {
  if (!enabled) return null;
  const epoch = generation;
  const context = { traceId: navigation?.traceId ?? id(16), spanId: id(8) };
  const parentId = navigation?.spanId ?? null;
  let done = false;
  return {
    traceparent: `00-${context.traceId}-${context.spanId}-01`,
    finish(status = 'ok', attributes = {}) {
      if (done || !enabled || generation !== epoch) return;
      done = true;
      if (status === 'canceled') return;
      if (queue.length >= MAX_QUEUE) { dropped++; return; }
      const safe: Record<string, string | number> = {};
      for (const [key, value] of Object.entries(attributes)) {
        if (attributesAllowed.has(key) && (typeof value === 'number' ? Number.isFinite(value) : value.length <= 32)) safe[key] = value;
      }
      const duration = typeof attributes['otto.duration_ms'] === 'number'
        ? attributes['otto.duration_ms'] : performance.now() - start;
      if (!Number.isFinite(duration) || duration < 0) return;
      queue.push({ trace_id: context.traceId, span_id: context.spanId, parent_span_id: parentId,
        name, component: telemetryComponent(component), kind,
        start_unix_nano: Math.round((performance.timeOrigin + start) * 1e6),
        duration_ms: Math.min(duration, 3600000), status: status === 'ok' ? 'ok' : 'error', attributes: safe });
      scheduleFullBatch();
    },
  };
}

/** Scope the next navigation; async API spans inherit its trace until first paint. */
export function beginNavigation(module: string): (status?: string) => void {
  if (!enabled) return () => {};
  const seq = ++navGeneration;
  navigation = null;
  const measure = startMeasurement('ui.navigation', module);
  if (measure) {
    const [, traceId, spanId] = measure.traceparent.split('-');
    navigation = { traceId, spanId };
  }
  return (status = 'ok') => {
    measure?.finish(status);
    if (seq === navGeneration) navigation = null;
  };
}


/** Complete only foreground frame samples; background throttling is not latency. */
function afterFrames(signal: AbortSignal | undefined, complete: (painted: boolean) => void): void {
  if (signal?.aborted || typeof requestAnimationFrame === 'undefined' || typeof document === 'undefined' || document.hidden) { complete(false); return; }
  let frame = 0;
  let finished = false;
  const finish = (painted: boolean) => {
    if (finished) return;
    finished = true;
    cancelAnimationFrame(frame);
    clearTimeout(timeout);
    signal?.removeEventListener('abort', canceled);
    document.removeEventListener('visibilitychange', visibility);
    complete(painted && !document.hidden && !signal?.aborted);
  };
  const canceled = () => finish(false);
  const visibility = () => { if (document.hidden) canceled(); };
  const timeout = setTimeout(canceled, 1000);
  signal?.addEventListener('abort', canceled, { once: true });
  document.addEventListener('visibilitychange', visibility);
  frame = requestAnimationFrame(() => { frame = requestAnimationFrame(() => finish(true)); });
}

export function finishNavigationPaint(component: string, done: (status?: string) => void): void {
  if (!enabled) { done('canceled'); return; }
  const render = startMeasurement('ui.render', component);
  afterFrames(undefined, (painted) => {
    render?.finish(painted ? 'ok' : 'canceled');
    done(painted ? 'ok' : 'canceled');
  });
}

/** One two-frame latency probe per export tick; no perpetual 60 Hz monitor. */
function sampleFrameDelay(signal: AbortSignal): Promise<void> {
  if (!enabled || signal.aborted) return Promise.resolve();
  const start = performance.now();
  return new Promise((resolve) => afterFrames(signal, (painted) => {
    if (painted && enabled && performance.now() - start > 50) startMeasurement('ui.frame_delay', 'shell', 'internal', start)?.finish();
    resolve();
  }));
}

installTelemetryRuntime({ configureTelemetry, isTelemetryEnabled, telemetryState, apiComponent, startMeasurement, beginNavigation, finishNavigationPaint, flushTelemetry });
