// Pure `kubectl` CLI snippet generators ("Copy as kubectl") for the
// Kubernetes console. Never emits kubeconfig paths, tokens, or credentials —
// only the context name, namespace, resource target, and view flags.

const SAFE_SHELL_TOKEN = /^[A-Za-z0-9_./:@=,+-]+$/;

/** POSIX single-quote shell escaping. Leaves safe bare tokens unquoted for
 *  readability; wraps anything containing whitespace, quotes, or shell
 *  metacharacters in single quotes with `'` escaped as `'\''`. */
export function shQuote(value: string): string {
  const clean = value.replace(/\0/g, '');
  if (clean.length === 0) return "''";
  if (SAFE_SHELL_TOKEN.test(clean)) return clean;
  return `'${clean.replace(/'/g, "'\\''")}'`;
}

/** Minimal cluster shape accepted by {@link clusterContext}. Deliberately
 *  excludes credential/kubeconfig fields so secrets can never enter a snippet. */
export interface K8sClusterContextLike {
  context_name?: string | null;
  context?: string | null;
  params?: Record<string, unknown> | null;
}

/** Extract a kubeconfig context name from a cluster record (`context_name`,
 *  `context`, or `params.context`), or `undefined` when none is set. */
export function clusterContext(cluster: K8sClusterContextLike | null | undefined): string | undefined {
  if (!cluster) return undefined;
  const candidates = [
    cluster.context_name,
    cluster.context,
    typeof cluster.params?.context === 'string' ? cluster.params.context : undefined,
  ];
  for (const raw of candidates) {
    const trimmed = raw?.trim();
    if (trimmed) return trimmed;
  }
  return undefined;
}

const KIND_SINGULAR: Record<string, string> = {
  pods: 'pod',
  deployments: 'deployment',
  statefulsets: 'statefulset',
  daemonsets: 'daemonset',
  replicasets: 'replicaset',
  jobs: 'job',
  cronjobs: 'cronjob',
  services: 'service',
  ingresses: 'ingress',
  configmaps: 'configmap',
  secrets: 'secret',
  pvcs: 'pvc',
  persistentvolumeclaims: 'pvc',
  hpas: 'hpa',
  horizontalpodautoscalers: 'hpa',
  nodes: 'node',
  events: 'event',
  rollouts: 'rollout',
  applications: 'application',
};

function normalizeKind(kind: string): string {
  const k = kind.trim().toLowerCase();
  return KIND_SINGULAR[k] ?? k;
}

function baseArgs(context?: string | null, ns?: string | null): string[] {
  const args = ['kubectl'];
  const ctx = context?.trim();
  if (ctx) args.push('--context', shQuote(ctx));
  const namespace = ns?.trim();
  if (namespace) args.push('-n', shQuote(namespace));
  return args;
}

export interface KubectlGetOpts {
  kind: string;
  name: string;
  ns?: string;
  context?: string;
  output?: string;
}

/** Build a `kubectl get <kind> <name> [-o <output>]` command. */
export function kubectlGetCmd(opts: KubectlGetOpts): string {
  const args = baseArgs(opts.context, opts.ns);
  args.push('get', shQuote(normalizeKind(opts.kind)), shQuote(opts.name.trim()));
  const out = opts.output?.trim();
  if (out) args.push('-o', shQuote(out));
  return args.join(' ');
}

export interface KubectlDescribeOpts {
  kind: string;
  name: string;
  ns?: string;
  context?: string;
}

/** Build a `kubectl describe <kind> <name>` command. */
export function kubectlDescribeCmd(opts: KubectlDescribeOpts): string {
  const args = baseArgs(opts.context, opts.ns);
  args.push('describe', shQuote(normalizeKind(opts.kind)), shQuote(opts.name.trim()));
  return args.join(' ');
}

export interface KubectlLogsOpts {
  pod?: string;
  selector?: string;
  ns?: string;
  container?: string;
  tail?: number;
  since?: string;
  previous?: boolean;
  follow?: boolean;
  timestamps?: boolean;
  context?: string;
}

/** Build a `kubectl logs` command for either a single pod or a workload
 *  label selector, reflecting the active log viewer flags. */
export function kubectlLogsCmd(opts: KubectlLogsOpts): string {
  const args = baseArgs(opts.context, opts.ns);
  args.push('logs');
  const pod = opts.pod?.trim();
  const selector = opts.selector?.trim();
  const container = opts.container?.trim();
  if (pod) {
    args.push(shQuote(pod));
    if (container) args.push('-c', shQuote(container));
  } else if (selector) {
    args.push('-l', shQuote(selector));
    if (container) args.push('-c', shQuote(container));
    else args.push('--all-containers');
    args.push('--prefix');
  } else if (container) {
    args.push('-c', shQuote(container));
  }
  if (opts.tail != null && Number.isFinite(opts.tail) && opts.tail >= 0) {
    args.push(`--tail=${Math.floor(opts.tail)}`);
  }
  const since = opts.since?.trim();
  if (since) {
    const flag = since.includes('T') && since.includes(':') ? '--since-time' : '--since';
    args.push(`${flag}=${shQuote(since)}`);
  }
  if (opts.previous) args.push('--previous');
  if (opts.timestamps) args.push('--timestamps');
  if (opts.follow) args.push('-f');
  return args.join(' ');
}

export interface KubectlExecOpts {
  pod: string;
  ns?: string;
  container?: string;
  shell?: string;
  context?: string;
}

/** Build an interactive `kubectl exec -it <pod> [-c <container>] -- <shell>` command. */
export function kubectlExecCmd(opts: KubectlExecOpts): string {
  const args = baseArgs(opts.context, opts.ns);
  args.push('exec', '-it', shQuote(opts.pod.trim()));
  const container = opts.container?.trim();
  if (container) args.push('-c', shQuote(container));
  const shell = opts.shell?.trim() || '/bin/sh';
  args.push('--', shQuote(shell));
  return args.join(' ');
}
