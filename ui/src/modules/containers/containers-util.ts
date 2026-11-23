// Pure, dependency-free helpers for Otto's Containers module (compose grouping,
// initial selection, search/state filtering, env secret masking, port
// formatting, run-modal parsers, and status mapping). Kept free of Svelte/DOM
// imports so `ui/unit/containers.test.ts` can exercise every edge case in Node.

import type {
  ContainerLogLine,
  ContainerSummary,
  ContainersStatusResp,
  EngineKind,
  EnvEntry,
  ImageSummary,
  NetworkSummary,
  PortBindingReq,
  PortMapping,
  RuntimeStatusResp,
  VolumeSummary,
} from '../../lib/api/types/containers';
import type { BadgeTone } from '../../lib/status';

export type ContainersMainTab = 'containers' | 'images' | 'volumes' | 'networks';
export type ContainerDetailTab = 'overview' | 'logs' | 'terminal' | 'stats';
export type ContainerStateFilter = 'all' | 'running' | 'stopped' | 'paused';
export type LogStreamFilter = 'all' | 'stdout' | 'stderr';

export const SECRET_MASK = '••••••••';

const COMPOSE_PROJECT_LABEL = 'com.docker.compose.project';
const COMPOSE_SERVICE_LABEL = 'com.docker.compose.service';

/** A display group in the Containers list: either a Compose project or standalone containers. */
export interface ContainerGroup {
  /** Compose project name, or `null` for standalone containers. */
  project: string | null;
  /** Stable key for Svelte `{#each}` blocks. */
  key: string;
  label: string;
  containers: ContainerSummary[];
  runningCount: number;
  totalCount: number;
}

export interface ContainerStateBadge {
  label: string;
  tone: BadgeTone;
  live: boolean;
}

/** Extract the Compose project name from a container summary (field or label). */
export function composeProjectOf(c: Pick<ContainerSummary, 'compose_project' | 'labels'>): string | null {
  const direct = c.compose_project?.trim();
  if (direct) return direct;
  const fromLabel = c.labels?.[COMPOSE_PROJECT_LABEL]?.trim();
  return fromLabel || null;
}

/** Extract the Compose service name from a container summary (field or label). */
export function composeServiceOf(c: Pick<ContainerSummary, 'compose_service' | 'labels'>): string | null {
  const direct = c.compose_service?.trim();
  if (direct) return direct;
  const fromLabel = c.labels?.[COMPOSE_SERVICE_LABEL]?.trim();
  return fromLabel || null;
}

/** Order containers for display: running first, then paused/restarting, then newest created, then name. */
export function compareContainers(a: ContainerSummary, b: ContainerSummary): number {
  const rank = (state: string): number => {
    const s = state.toLowerCase();
    if (s === 'running') return 0;
    if (s === 'restarting') return 1;
    if (s === 'paused') return 2;
    return 3;
  };
  const ra = rank(a.state);
  const rb = rank(b.state);
  if (ra !== rb) return ra - rb;
  if (a.created !== b.created) return b.created - a.created;
  return a.name.localeCompare(b.name);
}

/** Group containers by `com.docker.compose.project`. Named projects come first
 *  (sorted alphabetically, running projects prioritized when tied by name),
 *  followed by standalone containers if any. */
export function groupContainersByProject(containers: readonly ContainerSummary[]): ContainerGroup[] {
  const byProject = new Map<string, ContainerSummary[]>();
  const standalone: ContainerSummary[] = [];

  for (const c of containers) {
    const project = composeProjectOf(c);
    if (project) {
      const bucket = byProject.get(project);
      if (bucket) bucket.push(c);
      else byProject.set(project, [c]);
    } else {
      standalone.push(c);
    }
  }

  const groups: ContainerGroup[] = [];
  const projectNames = [...byProject.keys()].sort((a, b) => a.localeCompare(b));
  for (const project of projectNames) {
    const items = [...(byProject.get(project) ?? [])].sort(compareContainers);
    const runningCount = items.filter((c) => c.state.toLowerCase() === 'running').length;
    groups.push({
      project,
      key: `stack:${project}`,
      label: project,
      containers: items,
      runningCount,
      totalCount: items.length,
    });
  }

  if (standalone.length > 0) {
    const items = [...standalone].sort(compareContainers);
    const runningCount = items.filter((c) => c.state.toLowerCase() === 'running').length;
    groups.push({
      project: null,
      key: 'standalone',
      label: 'Standalone',
      containers: items,
      runningCount,
      totalCount: items.length,
    });
  }

  return groups;
}

/** Pick what the Containers view should open on: remembered id if still present,
 *  else first running container (newest running first), else newest container. */
export function pickInitialContainer(
  containers: readonly ContainerSummary[],
  rememberedId?: string | null,
): string | null {
  if (containers.length === 0) return null;
  if (rememberedId && containers.some((c) => c.id === rememberedId)) {
    return rememberedId;
  }
  const running = containers
    .filter((c) => c.state.toLowerCase() === 'running')
    .sort((a, b) => b.created - a.created || a.name.localeCompare(b.name));
  if (running.length > 0) return running[0].id;
  const newest = [...containers].sort((a, b) => b.created - a.created || a.name.localeCompare(b.name));
  return newest[0].id;
}

/** Filter containers by search text and state bucket. */
export function filterContainers(
  containers: readonly ContainerSummary[],
  query: string,
  stateFilter: ContainerStateFilter = 'all',
): ContainerSummary[] {
  const q = query.trim().toLowerCase();
  return containers.filter((c) => {
    const st = c.state.toLowerCase();
    if (stateFilter === 'running' && st !== 'running' && st !== 'restarting') return false;
    if (stateFilter === 'paused' && st !== 'paused') return false;
    if (stateFilter === 'stopped' && (st === 'running' || st === 'restarting' || st === 'paused')) return false;
    if (!q) return true;
    const project = composeProjectOf(c) ?? '';
    const service = composeServiceOf(c) ?? '';
    if (
      c.name.toLowerCase().includes(q) ||
      c.short_id.toLowerCase().includes(q) ||
      c.id.toLowerCase().includes(q) ||
      c.image.toLowerCase().includes(q) ||
      project.toLowerCase().includes(q) ||
      service.toLowerCase().includes(q)
    ) {
      return true;
    }
    return c.ports.some(
      (p) =>
        String(p.private_port).includes(q) ||
        (p.public_port != null && String(p.public_port).includes(q)),
    );
  });
}

/** Mask an env value when it is a secret and not explicitly revealed. */
export function maskEnvValue(value: string, isSecret: boolean, revealed = false): string {
  if (!isSecret || revealed) return value;
  return SECRET_MASK;
}

/** Resolve how an `EnvEntry` renders given the set of user-revealed keys.
 *  Relies solely on backend-authoritative `entry.secret` and `entry.masked` flags. */
export function displayEnvEntry(
  entry: EnvEntry,
  revealedKeys: ReadonlySet<string>,
): { isSecret: boolean; isMasked: boolean; text: string } {
  const isSecret = Boolean(entry.secret || entry.masked);
  const revealed = revealedKeys.has(entry.key);
  if (!isSecret) {
    return { isSecret: false, isMasked: false, text: entry.value };
  }
  if (revealed && !entry.masked) {
    return { isSecret: true, isMasked: false, text: entry.value };
  }
  return { isSecret: true, isMasked: true, text: SECRET_MASK };
}

/** Build a clickable localhost URL for a published TCP port (or null if unexposed / non-TCP). */
export function portUrl(p: PortMapping): string | null {
  if (p.url) return p.url;
  if (p.public_port == null) return null;
  if ((p.protocol || 'tcp').toLowerCase() !== 'tcp') return null;
  const host = !p.ip || p.ip === '0.0.0.0' || p.ip === '::' || p.ip === ':::' ? 'localhost' : p.ip;
  return `http://${host}:${p.public_port}`;
}

/** Human label for a single port mapping (`8080 → 80` or `5432/udp` or `80/tcp`). */
export function formatPortMapping(p: PortMapping): string {
  const proto = (p.protocol || 'tcp').toLowerCase();
  const suffix = proto === 'tcp' ? '' : `/${proto}`;
  if (p.public_port != null) {
    if (p.public_port === p.private_port) return `${p.public_port}${suffix}`;
    return `${p.public_port} → ${p.private_port}${suffix}`;
  }
  return `${p.private_port}${suffix}`;
}

/** Deduplicated published-first port list for compact row summaries. */
export function dedupePorts(ports: readonly PortMapping[]): PortMapping[] {
  const seen = new Set<string>();
  const out: PortMapping[] = [];
  const ordered = [...ports].sort((a, b) => {
    const ap = a.public_port != null ? 0 : 1;
    const bp = b.public_port != null ? 0 : 1;
    if (ap !== bp) return ap - bp;
    return (a.public_port ?? a.private_port) - (b.public_port ?? b.private_port);
  });
  for (const p of ordered) {
    const key = `${p.public_port ?? ''}:${p.private_port}/${(p.protocol || 'tcp').toLowerCase()}`;
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(p);
  }
  return out;
}

/** Compact summary string for container list rows (e.g. `8080 → 80, +2`). */
export function formatPortsSummary(ports: readonly PortMapping[], maxShown = 2): string {
  const published = dedupePorts(ports).filter((p) => p.public_port != null);
  if (published.length === 0) return '';
  const shown = published.slice(0, maxShown).map(formatPortMapping);
  const extra = published.length - shown.length;
  return extra > 0 ? `${shown.join(', ')} +${extra}` : shown.join(', ');
}

/** Map Docker container state to Otto's semantic badge vocabulary. */
export function containerStateInfo(state: string | null | undefined): ContainerStateBadge {
  switch ((state ?? '').trim().toLowerCase()) {
    case 'running':
      return { label: 'Running', tone: 'ok', live: true };
    case 'restarting':
      return { label: 'Restarting', tone: 'warn', live: true };
    case 'paused':
      return { label: 'Paused', tone: 'warn', live: false };
    case 'created':
      return { label: 'Created', tone: 'info', live: false };
    case 'dead':
      return { label: 'Dead', tone: 'bad', live: false };
    case 'exited':
      return { label: 'Stopped', tone: 'neutral', live: false };
    default:
      return { label: state ? state[0].toUpperCase() + state.slice(1) : 'Unknown', tone: 'neutral', live: false };
  }
}

/** Map Docker healthcheck state to Otto's semantic badge vocabulary. */
export function containerHealthInfo(health: string | null | undefined): ContainerStateBadge | null {
  switch ((health ?? '').trim().toLowerCase()) {
    case 'healthy':
      return { label: 'Healthy', tone: 'ok', live: false };
    case 'unhealthy':
      return { label: 'Unhealthy', tone: 'bad', live: false };
    case 'starting':
      return { label: 'Starting', tone: 'warn', live: true };
    default:
      return null;
  }
}

/** Human display name for a container engine kind. */
export function engineKindLabel(kind: EngineKind | string | null | undefined): string {
  switch (kind) {
    case 'kiri':
      return 'Kiri';
    case 'otto':
      return 'Otto Engine';
    case 'docker_desktop':
      return 'Docker Desktop';
    case 'colima':
      return 'Colima';
    case 'orbstack':
      return 'OrbStack';
    case 'rancher_desktop':
      return 'Rancher Desktop';
    case 'podman':
      return 'Podman';
    case 'lima':
      return 'Lima';
    case 'custom':
      return 'Custom daemon';
    default:
      return 'Docker daemon';
  }
}

/** Map embedded runtime VM state to Otto's semantic badge vocabulary. */
export function runtimeStateInfo(rt: RuntimeStatusResp | null | undefined): ContainerStateBadge {
  if (!rt || !rt.installed) {
    return { label: 'Not installed', tone: 'neutral', live: false };
  }
  switch (rt.vm_state.toLowerCase()) {
    case 'running':
      return { label: 'Running', tone: 'ok', live: true };
    case 'asleep':
      return { label: 'Asleep', tone: 'info', live: false };
    case 'stopped':
      return { label: 'Stopped', tone: 'warn', live: false };
    default:
      return { label: 'Offline', tone: 'warn', live: false };
  }
}

/** Build the calm, actionable detail copy when the active container daemon is offline. */
export function buildOfflineDetail(
  status: ContainersStatusResp,
  runtime: RuntimeStatusResp | null,
  runtimeError: string | null = null,
): string {
  const lines: string[] = [];
  if (runtime?.installing) {
    lines.push(runtime.install_progress || 'Installing the embedded Kiri runtime…');
  } else if (runtimeError) {
    lines.push(runtimeError);
  } else if (runtime?.install_error) {
    lines.push(runtime.install_error);
  } else if (runtime?.last_error) {
    lines.push(runtime.last_error);
  } else if (status.error) {
    lines.push(status.error);
  } else {
    lines.push('No container runtime answered on the active Docker socket.');
  }

  lines.push(`Context: ${status.active_context} · Socket: ${status.socket_path}`);

  if (runtime?.installed) {
    if (runtime.can_start) {
      lines.push('Start the embedded Kiri runtime below, or switch to Docker Desktop, Colima, or OrbStack.');
    } else if (!runtime.guest_assets_present) {
      lines.push('Embedded Kiri binary was found, but its guest VM assets are missing.');
    }
  } else if (runtime?.can_install) {
    lines.push('Install and start the embedded Kiri runtime in one click, or switch to Docker Desktop, Colima, or OrbStack.');
  } else if (runtime?.install_hint) {
    lines.push(runtime.install_hint);
    lines.push('Or start Docker Desktop, Colima, or OrbStack and retry.');
  } else {
    lines.push('Start Docker Desktop, Colima, or OrbStack (or switch to another context), then retry.');
  }

  return lines.join('\n');
}

/** Parse multiline or comma-separated port specs (`8080:80`, `127.0.0.1:5432:5432/tcp`, `80/udp`). */
export function parsePortSpecs(raw: string): { ports: PortBindingReq[]; error: string | null } {
  const tokens = raw
    .split(/[\n,]+/)
    .map((s) => s.trim())
    .filter(Boolean);
  const ports: PortBindingReq[] = [];
  for (const token of tokens) {
    const slashParts = token.split('/');
    if (slashParts.length > 2) {
      return { ports: [], error: `Invalid port mapping “${token}”. Use host:container (for example 8080:80).` };
    }
    const protocol = (slashParts[1] || 'tcp').toLowerCase();
    if (protocol !== 'tcp' && protocol !== 'udp' && protocol !== 'sctp') {
      return { ports: [], error: `Unsupported protocol “${protocol}” in “${token}”. Use tcp or udp.` };
    }
    const parts = slashParts[0].split(':');
    let hostIp: string | null = null;
    let hostPortStr: string | null = null;
    let containerPortStr: string;
    if (parts.length === 1) {
      containerPortStr = parts[0];
    } else if (parts.length === 2) {
      hostPortStr = parts[0];
      containerPortStr = parts[1];
    } else if (parts.length === 3) {
      hostIp = parts[0] || null;
      hostPortStr = parts[1];
      containerPortStr = parts[2];
    } else {
      return { ports: [], error: `Invalid port mapping “${token}”. Use host:container or ip:host:container.` };
    }
    const containerPort = parseValidPort(containerPortStr);
    if (containerPort === null) {
      return { ports: [], error: `Invalid container port in “${token}” (1–65535).` };
    }
    let hostPort: number | null = null;
    if (hostPortStr !== null && hostPortStr !== '') {
      hostPort = parseValidPort(hostPortStr);
      if (hostPort === null) {
        return { ports: [], error: `Invalid host port in “${token}” (1–65535).` };
      }
    }
    ports.push({
      host_ip: hostIp,
      host_port: hostPort,
      container_port: containerPort,
      protocol,
    });
  }
  return { ports, error: null };
}

function parseValidPort(s: string): number | null {
  if (!/^\d+$/.test(s.trim())) return null;
  const n = Number(s.trim());
  return Number.isInteger(n) && n >= 1 && n <= 65535 ? n : null;
}

/** Parse multiline `KEY=VALUE` environment input (ignoring blank lines and `#` comments). */
export function parseEnvLines(raw: string): { env: string[]; error: string | null } {
  const env: string[] = [];
  for (const line of raw.split('\n')) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    const eq = trimmed.indexOf('=');
    if (eq <= 0) {
      return { env: [], error: `Environment line “${trimmed}” must use KEY=VALUE format.` };
    }
    const key = trimmed.slice(0, eq).trim();
    if (!/^[A-Za-z_][A-Za-z0-9_.-]*$/.test(key)) {
      return { env: [], error: `Invalid environment variable name “${key}”.` };
    }
    env.push(`${key}=${trimmed.slice(eq + 1)}`);
  }
  return { env, error: null };
}

/** Parse a command string into argv tokens, honoring single and double quotes. */
export function parseCommandArgs(raw: string): string[] {
  const out: string[] = [];
  let cur = '';
  let quote: '"' | "'" | null = null;
  let escape = false;
  for (const ch of raw.trim()) {
    if (escape) {
      cur += ch;
      escape = false;
      continue;
    }
    if (ch === '\\' && quote !== "'") {
      escape = true;
      continue;
    }
    if (quote) {
      if (ch === quote) quote = null;
      else cur += ch;
      continue;
    }
    if (ch === '"' || ch === "'") {
      quote = ch;
      continue;
    }
    if (/\s/.test(ch)) {
      if (cur) {
        out.push(cur);
        cur = '';
      }
      continue;
    }
    cur += ch;
  }
  if (cur) out.push(cur);
  return out;
}

/** Filter container log lines by stream and case-insensitive substring query. */
export function filterLogLines(
  lines: readonly ContainerLogLine[],
  query: string,
  stream: LogStreamFilter = 'all',
): ContainerLogLine[] {
  const q = query.trim().toLowerCase();
  return lines.filter((l) => {
    if (stream !== 'all' && l.stream !== stream) return false;
    if (!q) return true;
    return l.text.toLowerCase().includes(q) || (l.timestamp?.toLowerCase().includes(q) ?? false);
  });
}

/** Format log lines for clipboard copy. */
export function formatLogsForCopy(lines: readonly ContainerLogLine[], includeTimestamps: boolean): string {
  return lines
    .map((l) => (includeTimestamps && l.timestamp ? `${l.timestamp} ${l.text}` : l.text))
    .join('\n');
}

/** Primary display label for an image (`repo:tag` or `<untagged>`). */
export function imageDisplayName(img: ImageSummary): { repo: string; tag: string } {
  const first = img.repo_tags.find((t) => t && t !== '<none>:<none>');
  if (!first) return { repo: '<untagged>', tag: img.short_id };
  const lastColon = first.lastIndexOf(':');
  const lastSlash = first.lastIndexOf('/');
  if (lastColon > lastSlash) {
    return { repo: first.slice(0, lastColon), tag: first.slice(lastColon + 1) };
  }
  return { repo: first, tag: 'latest' };
}

/** Filter images by query and optional dangling-only flag. */
export function filterImages(images: readonly ImageSummary[], query: string, onlyDangling = false): ImageSummary[] {
  const q = query.trim().toLowerCase();
  return images.filter((img) => {
    if (onlyDangling && !img.dangling) return false;
    if (!q) return true;
    return (
      img.short_id.toLowerCase().includes(q) ||
      img.id.toLowerCase().includes(q) ||
      img.repo_tags.some((t) => t.toLowerCase().includes(q))
    );
  });
}

/** Filter volumes by query and optional unused-only flag. */
export function filterVolumes(volumes: readonly VolumeSummary[], query: string, onlyUnused = false): VolumeSummary[] {
  const q = query.trim().toLowerCase();
  return volumes.filter((v) => {
    if (onlyUnused && v.in_use) return false;
    if (!q) return true;
    return (
      v.name.toLowerCase().includes(q) ||
      v.driver.toLowerCase().includes(q) ||
      v.mountpoint.toLowerCase().includes(q)
    );
  });
}

/** Filter networks by query. */
export function filterNetworks(networks: readonly NetworkSummary[], query: string): NetworkSummary[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...networks];
  return networks.filter(
    (n) =>
      n.name.toLowerCase().includes(q) ||
      n.short_id.toLowerCase().includes(q) ||
      n.driver.toLowerCase().includes(q) ||
      (n.subnet?.toLowerCase().includes(q) ?? false) ||
      n.containers.some((c) => c.name.toLowerCase().includes(q)),
  );
}
