// Structured reading view for typed OKF notes (Service, API Endpoint, Runbook,
// Repository, Decision, Metric, Data Asset, Flow). Pure model code: no Svelte,
// no fetches — StructuredNote.svelte renders the model and LiveContext.svelte
// matches its entity hints against the rest of Otto (K8s monitor, repos, DB
// connections + dashboards, API collections). Plain notes (no recognised
// `type`) get `null` and keep the unchanged markdown view.

/** The recognised note kinds; everything else renders as plain markdown. */
export type NoteKind = 'service' | 'api' | 'runbook' | 'repository' | 'decision' | 'metric' | 'data' | 'flow';

const KIND_BY_TYPE: Record<string, NoteKind> = {
  service: 'service', microservice: 'service', workload: 'service', application: 'service', app: 'service',
  'api endpoint': 'api', endpoint: 'api', api: 'api', 'api operation': 'api', operation: 'api',
  runbook: 'runbook', playbook: 'runbook',
  repository: 'repository', repo: 'repository', codebase: 'repository',
  decision: 'decision', adr: 'decision', 'architecture decision': 'decision',
  metric: 'metric', kpi: 'metric',
  'data asset': 'data', 'database table': 'data', table: 'data', collection: 'data', 'redis key': 'data', datastore: 'data',
  flow: 'flow', 'runtime flow': 'flow', workflow: 'flow',
};

export const KIND_LABEL: Record<NoteKind, string> = {
  service: 'Service', api: 'API endpoint', runbook: 'Runbook', repository: 'Repository',
  decision: 'Decision', metric: 'Metric', data: 'Data asset', flow: 'Flow',
};

/** Required content per kind (okf-authoring concept patterns). Each entry is
 *  the section label plus heading keywords that satisfy it. */
const REQUIRED: Record<NoteKind, [string, string[]][]> = {
  service: [
    ['Responsibility', ['overview', 'responsibilit', 'purpose']],
    ['Entry points', ['entry point', 'endpoint', 'api', 'interface']],
    ['Dependencies', ['dependenc', 'depends']],
    ['Configuration', ['config', 'environment']],
    ['Failure behavior', ['failure', 'error', 'resilien']],
    ['Operations', ['operation', 'runbook', 'monitor', 'deploy']],
  ],
  api: [
    ['Authentication', ['auth']],
    ['Parameters', ['parameter']],
    ['Request', ['request']],
    ['Response', ['response', 'success']],
    ['Errors', ['error']],
    ['Side effects', ['side effect', 'flow']],
  ],
  runbook: [
    ['Symptoms', ['symptom', 'trigger', 'alert']],
    ['Diagnosis', ['diagnos', 'investigat', 'triage']],
    ['Safe actions', ['action', 'mitigat', 'remediat', 'fix', 'steps']],
    ['Verification', ['verif', 'confirm', 'check']],
    ['Rollback / escalation', ['rollback', 'escalat']],
  ],
  repository: [
    ['Overview', ['overview', 'purpose']],
    ['Structure', ['structure', 'layout', 'module', 'architecture']],
    ['Build & test', ['build', 'test', 'develop']],
    ['Dependencies', ['dependenc']],
  ],
  decision: [
    ['Context', ['context', 'problem', 'background']],
    ['Options', ['option', 'alternative', 'considered']],
    ['Decision', ['decision', 'outcome']],
    ['Consequences', ['consequence', 'impact', 'trade']],
  ],
  metric: [
    ['Definition', ['definition', 'overview']],
    ['Formula', ['formula', 'calculat', 'computation']],
    ['Grain / window', ['grain', 'window']],
    ['Source', ['source']],
    ['Interpretation', ['interpret', 'threshold', 'target']],
  ],
  data: [
    ['Grain', ['grain', 'overview', 'purpose']],
    ['Fields', ['field', 'schema', 'column']],
    ['Access paths', ['access', 'read', 'write', 'quer']],
    ['Indexes / TTL', ['index', 'ttl', 'retention']],
    ['Relationships', ['relationship', 'join']],
  ],
  flow: [
    ['Trigger', ['trigger']],
    ['Stages', ['stage', 'step', 'sequence']],
    ['Failures & retries', ['failure', 'retr', 'error', 'idempot']],
    ['Side effects', ['side effect', 'state change']],
  ],
};

export interface FieldItem {
  value: string;
  /** Vault-relative note path when the value is a resolved wikilink. */
  path?: string;
}
export interface FieldCard {
  key: string;
  title: string;
  items: FieldItem[];
}
export interface SectionCheck {
  label: string;
  present: boolean;
}
/** Names LiveContext matches against the rest of Otto (normalised later). */
export interface EntityHints {
  services: string[];
  repos: string[];
  databases: string[];
  collections: string[];
  dashboards: string[];
  /** Explicit `k8s: {cluster, namespace}` scope from frontmatter, if any. */
  k8sCluster: string | null;
  k8sNamespace: string | null;
}
export interface StructuredModel {
  kind: NoteKind;
  label: string;
  status: string | null;
  owners: string[];
  tags: string[];
  resource: string | null;
  /** `METHOD /path` for API endpoints (from `resource` / `method`+`path`). */
  operation: { method: string; path: string } | null;
  cards: FieldCard[];
  sections: SectionCheck[];
  hints: EntityHints;
}

export interface NoteLike {
  meta: { path: string; title: string; okf_type: string | null; frontmatter: unknown; tags: string[]; headings: { level: number; text: string }[]; reserved: boolean };
  raw: string;
  outgoing: { raw_target: string; dst_path: string | null; alias: string | null }[];
}

const mapping = (v: unknown): Record<string, unknown> =>
  v !== null && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {};

/** A frontmatter value as a list of display strings: scalars, arrays of
 *  scalars, and arrays/maps of objects (`{name: …}` / `{id: …}` / key: value). */
export function asList(v: unknown): string[] {
  if (v == null || v === '') return [];
  if (typeof v === 'string') return v.split(/\s*,\s*/).map((s) => s.trim()).filter(Boolean);
  if (typeof v === 'number' || typeof v === 'boolean') return [String(v)];
  if (Array.isArray(v)) return v.flatMap((x) => (x !== null && typeof x === 'object' ? [objLabel(x)] : asList(x))).filter(Boolean);
  if (typeof v === 'object') {
    return Object.entries(v as Record<string, unknown>).map(([k, x]) =>
      x == null || x === '' ? k : typeof x === 'object' ? `${k}: ${objLabel(x)}` : `${k}: ${String(x)}`,
    );
  }
  return [];
}

function objLabel(o: object): string {
  const m = o as Record<string, unknown>;
  for (const k of ['name', 'title', 'id', 'url', 'path', 'value']) {
    if (typeof m[k] === 'string' && m[k]) return m[k] as string;
  }
  return Object.entries(m).filter(([, x]) => typeof x !== 'object').map(([k, x]) => `${k}: ${String(x)}`).join(', ');
}

/** First present key wins (case-insensitive frontmatter keys). */
function pick(fm: Record<string, unknown>, keys: string[]): unknown {
  const lower = new Map(Object.entries(fm).map(([k, v]) => [k.toLowerCase(), v]));
  for (const k of keys) if (lower.has(k)) return lower.get(k);
  return undefined;
}

export function noteKind(okfType: string | null | undefined): NoteKind | null {
  if (!okfType) return null;
  return KIND_BY_TYPE[okfType.trim().toLowerCase().replace(/[_-]+/g, ' ')] ?? null;
}

/** Body lines under the first heading whose text contains one of `words`,
 *  up to the next heading of the same or a higher level. */
export function sectionLines(raw: string, words: string[]): string[] {
  const lines = raw.split('\n');
  let start = -1, level = 0, fence = false;
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    if (/^(```|~~~)/.test(l)) fence = !fence;
    if (fence) continue;
    const h = /^(#{1,6})\s+(.*)$/.exec(l);
    if (!h) continue;
    if (start >= 0 && h[1].length <= level) return lines.slice(start, i);
    if (start < 0 && words.some((w) => h[2].toLowerCase().includes(w))) {
      start = i + 1;
      level = h[1].length;
    }
  }
  return start >= 0 ? lines.slice(start) : [];
}

/** Bullet items + first table column (minus the header/separator rows) of a
 *  section, wikilinks reduced to their label. Capped at `max`. */
export function sectionItems(lines: string[], max = 12): string[] {
  const out: string[] = [];
  let tableRow = 0;
  for (const l of lines) {
    const bullet = /^\s{0,3}(?:[-*+]|\d+[.)])\s+(.*)$/.exec(l);
    if (bullet) {
      out.push(clean(bullet[1]));
    } else if (/^\s*\|/.test(l)) {
      tableRow++;
      const cell = l.split('|')[1]?.trim() ?? '';
      if (tableRow > 2 && cell && !/^:?-{2,}:?$/.test(cell)) out.push(clean(cell));
    } else if (l.trim()) {
      tableRow = 0;
    }
    if (out.length >= max) break;
  }
  return out.filter(Boolean);
}

function clean(s: string): string {
  return s
    .replace(/!?\[\[([^\]|#]+)(?:#[^\]|]*)?(?:\|([^\]]+))?\]\]/g, (_m, t: string, a?: string) => a ?? t)
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1')
    .replace(/[`*_]/g, '')
    .trim()
    .slice(0, 160);
}

/** Map a display value back to a resolved outgoing link (by raw target or
 *  alias, case-insensitive) so a card item can navigate. */
function linkFor(value: string, outgoing: NoteLike['outgoing']): string | undefined {
  const v = value.toLowerCase();
  for (const o of outgoing) {
    if (!o.dst_path) continue;
    const t = o.raw_target.replace(/\.md$/i, '').toLowerCase();
    const base = t.split('/').pop() ?? t;
    if (v === t || v === base || (o.alias && v === o.alias.toLowerCase())) return o.dst_path;
  }
  return undefined;
}

const CARD_SPECS: { key: string; title: string; fm: string[]; headings: string[]; kinds?: NoteKind[] }[] = [
  { key: 'endpoints', title: 'Endpoints', fm: ['endpoints', 'routes', 'entry_points', 'apis'], headings: ['endpoint', 'entry point', 'routes'], kinds: ['service', 'repository', 'flow'] },
  { key: 'environments', title: 'Environments', fm: ['environments', 'envs', 'environment', 'deployments'], headings: ['environment', 'deployment'] },
  { key: 'dependencies', title: 'Dependencies', fm: ['dependencies', 'depends_on', 'deps', 'upstream', 'downstream'], headings: ['dependenc', 'depends on'] },
  { key: 'datastores', title: 'Data stores', fm: ['databases', 'database', 'datastores', 'db', 'tables', 'connections'], headings: ['data store', 'datastore', 'database', 'storage'], kinds: ['service', 'repository', 'api', 'flow', 'metric'] },
  { key: 'errors', title: 'Errors', fm: ['errors'], headings: ['error'], kinds: ['api'] },
  { key: 'options', title: 'Options considered', fm: ['options', 'alternatives'], headings: ['option', 'alternative'], kinds: ['decision'] },
  { key: 'steps', title: 'Safe actions', fm: ['steps', 'actions'], headings: ['safe action', 'mitigat', 'remediat', 'steps'], kinds: ['runbook'] },
  { key: 'dimensions', title: 'Dimensions', fm: ['dimensions'], headings: ['dimension'], kinds: ['metric'] },
  { key: 'slos', title: 'SLOs', fm: ['slo', 'slos', 'sla'], headings: ['slo'], kinds: ['service', 'api', 'metric'] },
];

const HTTP_OP = /^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\s+(\S+)/i;

/** Build the structured model, or `null` for a plain/reserved note. */
export function structuredModel(note: NoteLike): StructuredModel | null {
  const kind = noteKind(note.meta.okf_type);
  if (!kind || note.meta.reserved) return null;
  const fm = mapping(note.meta.frontmatter);
  const owners = asList(pick(fm, ['owners', 'owner', 'team', 'maintainers', 'maintainer']));
  const statusRaw = pick(fm, ['status', 'state', 'lifecycle']);
  const status = typeof statusRaw === 'string' && statusRaw.trim() ? statusRaw.trim() : null;
  const resourceRaw = pick(fm, ['resource']);
  const resource = typeof resourceRaw === 'string' && resourceRaw.trim() ? resourceRaw.trim() : null;

  let operation: StructuredModel['operation'] = null;
  if (kind === 'api') {
    const m = resource ? HTTP_OP.exec(resource) : null;
    const method = pick(fm, ['method']), path = pick(fm, ['path', 'route']);
    if (m) operation = { method: m[1].toUpperCase(), path: m[2] };
    else if (typeof method === 'string' && typeof path === 'string') operation = { method: method.toUpperCase(), path };
  }

  const cards: FieldCard[] = [];
  for (const spec of CARD_SPECS) {
    if (spec.kinds && !spec.kinds.includes(kind)) continue;
    let values = asList(pick(fm, spec.fm));
    if (!values.length) values = sectionItems(sectionLines(note.raw, spec.headings));
    if (!values.length) continue;
    const seen = new Set<string>();
    const items = values
      .filter((v) => (seen.has(v.toLowerCase()) ? false : (seen.add(v.toLowerCase()), true)))
      .map((value) => ({ value, path: linkFor(value.split(':')[0].trim(), note.outgoing) }));
    cards.push({ key: spec.key, title: spec.title, items });
  }

  const headings = note.meta.headings.map((h) => h.text.toLowerCase());
  const sections = REQUIRED[kind].map(([label, words]) => ({
    label,
    present: headings.some((h) => words.some((w) => h.includes(w))),
  }));

  const k8s = mapping(pick(fm, ['k8s', 'kubernetes']));
  const stem = (note.meta.path.split('/').pop() ?? '').replace(/\.md$/i, '');
  const services = uniq([
    ...asList(pick(fm, ['service', 'services', 'workload', 'workloads', 'app', 'deployment'])),
    ...asList(k8s.workload ?? k8s.workloads ?? k8s.deployment),
    ...(kind === 'service' || kind === 'repository' ? [note.meta.title, stem] : []),
  ]);
  const repoHints = asList(pick(fm, ['repository', 'repo', 'repos', 'repositories', 'git']));
  if (resource && !HTTP_OP.test(resource) && /^(\/|~|https?:\/\/|git@)/.test(resource)) repoHints.push(resource);
  if (kind === 'repository') repoHints.push(note.meta.title, stem);
  const datastores = cards.find((c) => c.key === 'datastores')?.items.map((i) => i.value.split(':')[0].trim()) ?? [];
  const str = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v.trim() : null);

  return {
    kind,
    label: KIND_LABEL[kind],
    status,
    owners,
    tags: note.meta.tags,
    resource,
    operation,
    cards,
    sections,
    hints: {
      services,
      repos: uniq(repoHints),
      databases: uniq([...asList(pick(fm, ['connections', 'connection'])), ...datastores]),
      collections: uniq([...asList(pick(fm, ['api_collection', 'api_collections', 'collection', 'collections'])), ...services]),
      dashboards: uniq(asList(pick(fm, ['dashboard', 'dashboards']))),
      k8sCluster: str(k8s.cluster),
      k8sNamespace: str(k8s.namespace ?? k8s.ns),
    },
  };
}

function uniq(xs: string[]): string[] {
  const seen = new Set<string>();
  return xs.map((x) => x.trim()).filter((x) => x && !seen.has(x.toLowerCase()) && (seen.add(x.toLowerCase()), true));
}

// -- live-context matching ------------------------------------------------------

/** Normalise an entity name for comparison: lowercase alphanumerics only
 *  (`Orders-API` ≡ `orders_api` ≡ `ordersapi`). */
export const norm = (s: string): string => s.toLowerCase().replace(/[^a-z0-9]/g, '');

/** Items whose name matches a hint exactly after normalisation. A hint shorter
 *  than 3 characters never matches (too ambiguous). */
export function matchByName<T>(hints: string[], items: T[], name: (t: T) => string): T[] {
  const keys = new Set(hints.map(norm).filter((h) => h.length >= 3));
  if (!keys.size) return [];
  return items.filter((t) => keys.has(norm(name(t))));
}

/** Repos matching a hint by path (exact or trailing-slash-insensitive),
 *  basename, repo name, or remote URL slug (`…/org/name(.git)`). */
export function matchRepos<T extends { name: string; path: string; remote_url: string | null }>(hints: string[], repos: T[]): T[] {
  const paths = new Set(hints.filter((h) => h.startsWith('/') || h.startsWith('~')).map((h) => h.replace(/\/+$/, '')));
  const names = new Set(hints.map((h) => norm(h.replace(/\.git$/i, '').split(/[/:]/).pop() ?? h)).filter((h) => h.length >= 3));
  return repos.filter((r) => {
    if (paths.has(r.path.replace(/\/+$/, ''))) return true;
    const slug = (r.remote_url ?? '').replace(/\.git$/i, '').split(/[/:]/).pop() ?? '';
    return names.has(norm(r.name)) || names.has(norm(r.path.split('/').pop() ?? '')) || (!!slug && names.has(norm(slug)));
  });
}

/** K8s workload kind → the resource-table route segment. */
export function workloadRouteKind(kind: string): string {
  const k = kind.toLowerCase();
  if (k.startsWith('statefulset')) return 'statefulsets';
  if (k.startsWith('daemonset')) return 'daemonsets';
  if (k.startsWith('cronjob')) return 'cronjobs';
  if (k.startsWith('job')) return 'jobs';
  return 'deployments';
}

/** Plain-text preview of a note body for hover cards: frontmatter, headings
 *  markup, code fences and link syntax stripped; first `max` characters. */
export function previewText(raw: string, max = 280): string {
  const body = raw.replace(/^---\n[\s\S]*?\n---\n?/, '');
  const text = body
    .replace(/```[\s\S]*?```/g, ' ')
    .split('\n')
    .filter((l) => !/^\s*(#|\||>|---)/.test(l))
    .map(clean)
    .join(' ')
    .replace(/\s+/g, ' ')
    .trim();
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}
