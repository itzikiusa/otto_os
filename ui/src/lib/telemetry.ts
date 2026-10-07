/** Tiny synchronous hooks. Collection code is loaded by the authenticated boot chunk. */
export interface UiSpan {
  trace_id: string;
  span_id: string;
  parent_span_id: string | null;
  name: string;
  component: string;
  kind: string;
  start_unix_nano: number;
  duration_ms: number;
  status: string;
  attributes: Record<string, string | number>;
}

export interface Measurement {
  traceparent: string;
  /** `name` optionally refines the operation at finish; callers must use
   * the fixed browser ingestion operations, never a URL or identifier. */
  finish(status?: string, attributes?: Record<string, string | number>, name?: string): void;
}
type Sender = (spans: UiSpan[], signal: AbortSignal) => Promise<void>;
interface Runtime {
  configureTelemetry(active: boolean, send?: Sender): void;
  isTelemetryEnabled(): boolean;
  telemetryState(): { enabled: boolean; queued: number; dropped: number };
  apiComponent(path: string): string;
  startMeasurement(name: string, component: string, kind?: string, start?: number): Measurement | null;
  beginNavigation(module: string): (status?: string) => void;
  finishNavigationPaint(component: string, done: (status?: string) => void): void;
  flushTelemetry(): Promise<void>;
}
let runtime: Runtime | undefined;
const noop = () => {};
export function installTelemetryRuntime(value: Runtime): void { runtime = value; }
export function configureTelemetry(active: boolean, send?: Sender): void { runtime?.configureTelemetry(active, send); }
export function isTelemetryEnabled(): boolean { return runtime?.isTelemetryEnabled() ?? false; }
export function telemetryState(): { enabled: boolean; queued: number; dropped: number } { return runtime?.telemetryState() ?? { enabled: false, queued: 0, dropped: 0 }; }
export function apiComponent(path: string): string { return runtime?.isTelemetryEnabled() ? runtime.apiComponent(path) : 'other'; }
export function startMeasurement(name: string, component: string, kind?: string, start?: number): Measurement | null { return runtime?.startMeasurement(name, component, kind, start) ?? null; }
export function beginNavigation(module: string): (status?: string) => void { return runtime?.beginNavigation(module) ?? noop; }
export function finishNavigationPaint(component: string, done: (status?: string) => void): void { if (runtime) runtime.finishNavigationPaint(component, done); else done('canceled'); }
export async function flushTelemetry(): Promise<void> { await runtime?.flushTelemetry(); }

/** Browser ingest accepts fixed operation names only. Route headers can contain
 * identifiers from an older or mismatched daemon, so endpoint attribution stays
 * in the server spans. Keep the arguments for the API wrapper's finish hook. */
export function clientSpanName(_method: string, _route: string | null): string {
  return 'http.client';
}
