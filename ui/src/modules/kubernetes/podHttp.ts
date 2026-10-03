// Pod HTTP actions (K-3) — pure helpers shared by PodHttpPanel and its unit
// tests: the built-in Spring Boot actuator presets, `{{var}}` templates, the
// console-kind → workload-kind mapping and the default port guess.
import type { K8sPodHttpMethod, K8sResourceKind } from '../../lib/api/types';

export interface PodHttpPreset {
  id: string;
  name: string;
  method: K8sPodHttpMethod;
  path: string;
  headers?: Record<string, string>;
  body?: string;
  hint: string;
}

export const LOG_LEVELS = ['TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR', 'FATAL', 'OFF', 'RESET'] as const;

export const ACTUATOR_PRESETS: readonly PodHttpPreset[] = [
  { id: 'loggers', name: 'Loggers', method: 'GET', path: '/actuator/loggers', hint: 'Every logger and its effective level' },
  { id: 'logger', name: 'Get logger', method: 'GET', path: '/actuator/loggers/{{logger}}', hint: 'One logger, e.g. com.acme.billing' },
  {
    id: 'set-level',
    name: 'Set log level',
    method: 'POST',
    path: '/actuator/loggers/{{logger}}',
    headers: { 'Content-Type': 'application/json' },
    body: '{"configuredLevel":"{{level}}"}',
    hint: 'RESET clears the override (configuredLevel: null)',
  },
  { id: 'health', name: 'Health', method: 'GET', path: '/actuator/health', hint: 'Liveness / readiness detail' },
  { id: 'info', name: 'Info', method: 'GET', path: '/actuator/info', hint: 'Build + git info' },
  { id: 'env', name: 'Environment', method: 'GET', path: '/actuator/env', hint: 'Property sources (values masked by the app)' },
  { id: 'metrics', name: 'Metrics', method: 'GET', path: '/actuator/metrics', hint: 'Metric names' },
  { id: 'refresh', name: 'Refresh config', method: 'POST', path: '/actuator/refresh', headers: { 'Content-Type': 'application/json' }, hint: 'Spring Cloud context refresh' },
  { id: 'threaddump', name: 'Thread dump', method: 'GET', path: '/actuator/threaddump', headers: { Accept: 'text/plain' }, hint: 'Plain-text thread dump' },
];

/** The `{{name}}` placeholders in the given texts, in first-seen order. */
export function templateVars(...texts: (string | null | undefined)[]): string[] {
  const out: string[] = [];
  for (const t of texts) {
    for (const m of (t ?? '').matchAll(/\{\{\s*([A-Za-z_][A-Za-z0-9_.-]*)\s*\}\}/g)) {
      if (!out.includes(m[1])) out.push(m[1]);
    }
  }
  return out;
}

/** Fill `{{name}}` placeholders. `level = RESET` turns a quoted
 *  `"{{level}}"` into JSON `null` (actuator's "clear the override"). */
export function fillTemplate(text: string, vars: Record<string, string>): string {
  let s = text;
  if ((vars.level ?? '').toUpperCase() === 'RESET') s = s.replace(/"\{\{\s*level\s*\}\}"/g, 'null');
  return s.replace(/\{\{\s*([A-Za-z_][A-Za-z0-9_.-]*)\s*\}\}/g, (all, name: string) => (name in vars ? vars[name] : all));
}

/** Client-side mirror of the daemon's path rule (the daemon re-checks). */
export function pathProblem(path: string): string | null {
  if (!path.startsWith('/')) return 'The path must start with /';
  if (/\s/.test(path)) return 'The path cannot contain spaces';
  if (path.includes('..')) return 'The path cannot contain ..';
  if (path.includes('://')) return 'Give a path, not a URL';
  if (/\{\{/.test(path)) return 'Fill in every {{variable}} first';
  return null;
}

const WORKLOAD_KINDS: Partial<Record<K8sResourceKind, string>> = {
  deployments: 'deployment',
  statefulsets: 'statefulset',
  daemonsets: 'daemonset',
  replicasets: 'replicaset',
  jobs: 'job',
};

/** The pod-http `workload.kind` for a console kind, or null when the kind
 *  has no pods behind it. */
export function workloadKindFor(kind: K8sResourceKind): string | null {
  return WORKLOAD_KINDS[kind] ?? null;
}

interface PortSpec {
  name?: string;
  containerPort?: number;
}

/** Best guess at the HTTP/management port from a Pod or workload manifest:
 *  a port named management/actuator/admin, then http*, then the first one,
 *  else 8080. */
export function defaultPort(manifest: unknown): number {
  const m = manifest as { spec?: { containers?: { ports?: PortSpec[] }[]; template?: { spec?: { containers?: { ports?: PortSpec[] }[] } } } } | null;
  const containers = m?.spec?.containers ?? m?.spec?.template?.spec?.containers ?? [];
  const ports = containers.flatMap((c) => c.ports ?? []).filter((p) => typeof p.containerPort === 'number');
  const by = (re: RegExp): PortSpec | undefined => ports.find((p) => re.test(p.name ?? ''));
  const pick = by(/management|actuator|admin/i) ?? by(/^http/i) ?? ports[0];
  return pick?.containerPort ?? 8080;
}

/** Pretty-print a JSON body; anything else comes back as-is. */
export function prettyBody(body: string): string {
  const t = body.trim();
  if (!t.startsWith('{') && !t.startsWith('[')) return body;
  try {
    return JSON.stringify(JSON.parse(t), null, 2);
  } catch {
    return body;
  }
}
