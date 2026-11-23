// Pod HTTP actions (K-3) — pure helpers shared by PodHttpPanel and its unit
// tests: the built-in Spring Boot actuator presets, `{{var}}` templates, the
// console-kind → workload-kind mapping and the default port guess.
import type { ApiBodyMode, ApiKeyVal, K8sPodHttpMethod, K8sResourceKind } from '../../lib/api/types';
import type { ApiDraft } from '../../lib/stores/apiClient/model';
import { newId } from '../../lib/id.ts';

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
  if (path.includes('#')) return 'The path cannot contain #';
  if (/%2f/i.test(path)) return 'The path cannot contain an encoded slash';
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

export interface PodHttpToApiDraftInput {
  clusterName: string;
  ns: string;
  pod?: string;
  workload?: { kind: string; name: string } | null;
  target: 'pod' | 'workload';
  method: K8sPodHttpMethod;
  port: number;
  path: string;
  headersText: string;
  body: string;
  vars: Record<string, string>;
}

/** Promote an ad-hoc Kubernetes Pod HTTP request into an API Client draft tab.
 *  Targets `http://127.0.0.1:<port><path>` and documents the matching
 *  `kubectl port-forward` command in `docs` so the user can replay or save it. */
export function podHttpToApiDraft(input: PodHttpToApiDraftInput): ApiDraft {
  const filledPath = fillTemplate(input.path.trim() || '/', input.vars);
  const normPath = filledPath.startsWith('/') ? filledPath : `/${filledPath}`;
  const port = Number.isFinite(input.port) && input.port > 0 ? Math.trunc(input.port) : 8080;
  const headers: ApiKeyVal[] = [];
  for (const line of input.headersText.split('\n')) {
    const i = line.indexOf(':');
    if (i <= 0) continue;
    const key = line.slice(0, i).trim();
    if (key) headers.push({ key, value: line.slice(i + 1).trim(), enabled: true });
  }
  const mutating = input.method !== 'GET';
  const filledBody = mutating && input.body.trim() ? fillTemplate(input.body, input.vars) : '';
  const hasJsonHeader = headers.some((h) => h.key.toLowerCase() === 'content-type' && h.value.toLowerCase().includes('json'));
  const trimmedBody = filledBody.trim();
  const body_mode: ApiBodyMode = !trimmedBody
    ? 'none'
    : hasJsonHeader || trimmedBody.startsWith('{') || trimmedBody.startsWith('[')
      ? 'json'
      : 'raw';
  const resource = input.target === 'pod' && input.pod
    ? `pod/${input.pod}`
    : input.workload
      ? `${input.workload.kind}/${input.workload.name}`
      : input.pod
        ? `pod/${input.pod}`
        : 'pod/<name>';
  const label = input.target === 'pod' && input.pod
    ? input.pod
    : (input.workload?.name ?? input.pod ?? 'pod');
  const pfCmd = `kubectl${input.ns ? ` -n ${input.ns}` : ''} port-forward ${resource} ${port}:${port}`;
  const docs = [
    `Promoted from Kubernetes Pod HTTP (${input.clusterName}${input.ns ? ` · ${input.ns}` : ''} · ${resource}).`,
    '',
    'Start a port-forward before sending:',
    '```bash',
    pfCmd,
    '```',
    '',
    'Enable **Allow local / private network targets** in API Client settings to send to `127.0.0.1`.',
  ].join('\n');

  return {
    tabId: newId(),
    requestId: null,
    name: `${label} · ${input.method} ${normPath}`,
    kind: 'http',
    method: input.method,
    url: `http://127.0.0.1:${port}${normPath}`,
    headers,
    query: [],
    body_mode,
    body: filledBody,
    auth: { type: 'none' },
    proto: '',
    grpc_method: '',
    docs,
  };
}

/** Maximum character budget for Kubernetes log snippets sent to an agent (16 KiB). */
export const K8S_LOG_PACKET_MAX_CHARS = 16_384;
export const K8S_LOG_PACKET_MAX_LINES = 200;

function scrubLogLine(text: string): string {
  return text
    .replace(/-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY-----/g, '[REDACTED_PRIVATE_KEY]')
    .replace(/\b(Bearer\s+)[A-Za-z0-9._~+/=-]{8,}/gi, '$1[REDACTED]')
    .replace(/\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/g, '[REDACTED_AWS_KEY]')
    .replace(/(https?:\/\/)([^/\s:@]+):([^/\s@]+)@/gi, '$1[REDACTED]@')
    .replace(
      /\b(password|passwd|secret|api[_-]?key|access[_-]?token|refresh[_-]?token)(["']?\s*[:=]\s*)(?:(")[^"]*"|(')[^']*'|[^\s,;}\]]+)/gi,
      (_m, key: string, sep: string, dq?: string, sq?: string) => `${key}${sep}${dq ? '"[REDACTED]"' : sq ? "'[REDACTED]'" : '[REDACTED]'}`,
    );
}

export interface K8sLogsPacketInput {
  clusterName: string;
  clusterEnv?: string | null;
  ns: string;
  pod?: string;
  selector?: string;
  container?: string;
  search?: string;
  lines: readonly string[];
  maxLines?: number;
  maxChars?: number;
}

/** Build a scrubbed, tail-capped context packet from visible Kubernetes log
 *  lines so an agent can diagnose crashes/errors without blowing its context. */
export function buildK8sLogsPacket(input: K8sLogsPacketInput) {
  const maxLines = input.maxLines ?? K8S_LOG_PACKET_MAX_LINES;
  const maxChars = input.maxChars ?? K8S_LOG_PACKET_MAX_CHARS;
  const tailLines = input.lines.length > maxLines ? input.lines.slice(input.lines.length - maxLines) : input.lines;
  const joined = scrubLogLine(tailLines.join('\n'));
  const charTrimmed = joined.length > maxChars;
  const logs = charTrimmed ? joined.slice(joined.length - maxChars) : joined;
  return {
    source: 'kubernetes_logs' as const,
    cluster: input.clusterName,
    environment: input.clusterEnv ?? null,
    namespace: input.ns,
    pod: input.pod || null,
    selector: input.selector || null,
    container: input.container || null,
    filter: input.search?.trim() || null,
    total_lines: input.lines.length,
    included_lines: tailLines.length,
    truncated: input.lines.length > maxLines || charTrimmed,
    logs,
  };
}
