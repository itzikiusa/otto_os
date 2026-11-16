// Shared Named IDE Projects model, canonical path helper, localStorage persistence,
// and backend `/ide/projects` hydration shared across IDE and Threads.
// Pure TypeScript so unit tests run directly under Node.

import { lsGet, lsRemove, lsSet } from './storage.ts';

export interface NamedIdeProject {
  id: string;
  name: string;
  primaryRoot: string;
  folders: string[];
  pinned: boolean;
  updatedAt: number;
}

export const LS_IDE_PROJECTS = 'otto.ide.projects.v1';
export const LS_IDE_PROJECTS_LEGACY = 'otto.ide.namedProjects.v1';
export const IDE_PROJECTS_CHANGED_EVENT = 'otto:ide-projects-changed';

export function normalizeProjectPath(p: string): string {
  const trimmed = (p ?? '').trim();
  if (!trimmed || trimmed === '/') return trimmed;
  const clean = trimmed.replace(/\/+$/, '');
  if (clean === '/private/var' || clean.startsWith('/private/var/')) {
    return clean.slice('/private'.length);
  }
  if (clean === '/private/tmp' || clean.startsWith('/private/tmp/')) {
    return clean.slice('/private'.length);
  }
  return clean;
}

export function basenameOfProjectPath(raw: string): string {
  const clean = normalizeProjectPath(raw);
  if (!clean || clean === '~') return clean;
  const idx = clean.lastIndexOf('/');
  return idx >= 0 ? clean.slice(idx + 1) : clean;
}

function normalizeTimestamp(v: unknown): number {
  if (typeof v === 'number' && Number.isFinite(v) && v > 0) return v;
  if (typeof v === 'string' && v.trim()) {
    const asNum = Number(v);
    if (Number.isFinite(asNum) && asNum > 0) return asNum;
    const parsed = Date.parse(v);
    if (Number.isFinite(parsed) && parsed > 0) return parsed;
  }
  return Date.now();
}

function extractFolderString(item: unknown): string {
  if (typeof item === 'string') return normalizeProjectPath(item);
  if (item && typeof item === 'object') {
    const rec = item as Record<string, unknown>;
    if (typeof rec.path === 'string') return normalizeProjectPath(rec.path);
  }
  return '';
}

export function parseNamedIdeProject(raw: unknown): NamedIdeProject | null {
  if (!raw || typeof raw !== 'object') return null;
  const rec = raw as Record<string, unknown>;
  const rawPrimary =
    typeof rec.primaryRoot === 'string'
      ? rec.primaryRoot
      : typeof rec.primary_root === 'string'
        ? rec.primary_root
        : '';
  const cleanPrimary = normalizeProjectPath(rawPrimary);

  const folders: string[] = [];
  const pushFolder = (f: string): void => {
    const clean = normalizeProjectPath(f);
    if (!clean || clean === '~') return;
    if (!folders.includes(clean)) folders.push(clean);
  };

  if (cleanPrimary && cleanPrimary !== '~') pushFolder(cleanPrimary);
  if (Array.isArray(rec.folders)) {
    for (let i = 0; i < rec.folders.length; i++) {
      pushFolder(extractFolderString(rec.folders[i]));
    }
  }

  const primaryRoot = (cleanPrimary && cleanPrimary !== '~' ? cleanPrimary : folders[0]) ?? '';
  if (!primaryRoot) return null;
  if (folders[0] !== primaryRoot) {
    const rest = folders.filter((f) => f !== primaryRoot);
    folders.length = 0;
    folders.push(primaryRoot, ...rest);
  }

  const id =
    typeof rec.id === 'string' && rec.id.trim()
      ? rec.id.trim()
      : `ide-proj:${primaryRoot}`;
  const name =
    typeof rec.name === 'string' && rec.name.trim()
      ? rec.name.trim()
      : basenameOfProjectPath(primaryRoot) || 'Project';
  const pinned = rec.pinned !== false;
  const updatedAt = normalizeTimestamp(rec.updatedAt ?? rec.updated_at);

  return { id, name, primaryRoot, folders, pinned, updatedAt };
}

export function parseNamedIdeProjectsJson(raw: string | null | undefined): NamedIdeProject[] {
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    const out: NamedIdeProject[] = [];
    const seenIds = new Set<string>();
    for (let i = 0; i < parsed.length; i++) {
      const proj = parseNamedIdeProject(parsed[i]);
      if (!proj || seenIds.has(proj.id)) continue;
      seenIds.add(proj.id);
      out.push(proj);
    }
    return out;
  } catch {
    return [];
  }
}

export function loadNamedIdeProjectsLocal(): NamedIdeProject[] {
  const canonicalRaw = lsGet(LS_IDE_PROJECTS);
  if (canonicalRaw !== null) {
    return parseNamedIdeProjectsJson(canonicalRaw);
  }
  const legacyRaw = lsGet(LS_IDE_PROJECTS_LEGACY);
  if (legacyRaw !== null) {
    const migrated = parseNamedIdeProjectsJson(legacyRaw);
    saveNamedIdeProjectsLocal(migrated);
    return migrated;
  }
  return [];
}

let inFlightProjectsFetch: Promise<NamedIdeProject[] | null> | null = null;
let cachedServerProjects: NamedIdeProject[] | null = null;
let cachedServerProjectsAt = 0;
const PROJECTS_FETCH_TTL_MS = 15_000;

export function resetIdeProjectsFetchCacheForTests(): void {
  inFlightProjectsFetch = null;
  cachedServerProjects = null;
  cachedServerProjectsAt = 0;
}

if (typeof window !== 'undefined') {
  window.addEventListener('otto:ide-storage-cleared', () => {
    resetIdeProjectsFetchCacheForTests();
  });
}

export function normalizeNamedIdeProjects(
  projects: ReadonlyArray<NamedIdeProject | Record<string, unknown>>,
): NamedIdeProject[] {
  const normalized: NamedIdeProject[] = [];
  const seenIds = new Set<string>();
  for (let i = 0; i < projects.length; i++) {
    const proj = parseNamedIdeProject(projects[i]);
    if (!proj || seenIds.has(proj.id)) continue;
    seenIds.add(proj.id);
    normalized.push(proj);
  }
  return normalized;
}

export function saveNamedIdeProjectsLocal(
  projects: NamedIdeProject[],
  opts?: { source?: 'local' | 'server' },
): boolean {
  const normalized = normalizeNamedIdeProjects(projects);
  const serialized = JSON.stringify(normalized);
  const prevRaw = lsGet(LS_IDE_PROJECTS);
  const hadLegacy = lsGet(LS_IDE_PROJECTS_LEGACY) !== null;
  if (hadLegacy) lsRemove(LS_IDE_PROJECTS_LEGACY);
  cachedServerProjects = normalized;
  cachedServerProjectsAt = Date.now();
  if (prevRaw === serialized && !hadLegacy) {
    return false;
  }
  lsSet(LS_IDE_PROJECTS, serialized);
  if (typeof window !== 'undefined') {
    window.dispatchEvent(
      new CustomEvent(IDE_PROJECTS_CHANGED_EVENT, {
        detail: { projects: normalized, source: opts?.source ?? 'local' },
      }),
    );
  }
  return true;
}

export function mergeBackendIdeProjects(
  local: NamedIdeProject[],
  remote: ReadonlyArray<Record<string, unknown>>,
  authoritativeEmpty = true,
): NamedIdeProject[] {
  if (remote.length === 0) {
    return authoritativeEmpty ? [] : [...local];
  }
  const localById = new Map<string, NamedIdeProject>();
  const localByRoot = new Map<string, NamedIdeProject>();
  for (let i = 0; i < local.length; i++) {
    localById.set(local[i].id, local[i]);
    localByRoot.set(local[i].primaryRoot, local[i]);
  }
  const remoteParsed: NamedIdeProject[] = [];
  const seenIds = new Set<string>();
  for (let i = 0; i < remote.length; i++) {
    const rawItem = remote[i];
    const proj = parseNamedIdeProject(rawItem);
    if (!proj || seenIds.has(proj.id)) continue;
    if (typeof rawItem.pinned !== 'boolean') {
      const prev = localById.get(proj.id) ?? localByRoot.get(proj.primaryRoot);
      if (prev) proj.pinned = prev.pinned;
    }
    seenIds.add(proj.id);
    remoteParsed.push(proj);
  }
  if (authoritativeEmpty) {
    return remoteParsed;
  }
  const byId = new Map<string, NamedIdeProject>();
  for (let i = 0; i < local.length; i++) byId.set(local[i].id, local[i]);
  for (let i = 0; i < remoteParsed.length; i++) byId.set(remoteParsed[i].id, remoteParsed[i]);
  return [...byId.values()];
}

export async function fetchNamedIdeProjectsFromServer(opts?: {
  force?: boolean;
}): Promise<NamedIdeProject[] | null> {
  if (lsGet('otto_mock') === '1') return null;
  const force = Boolean(opts?.force);
  if (
    !force &&
    cachedServerProjects !== null &&
    Date.now() - cachedServerProjectsAt < PROJECTS_FETCH_TTL_MS
  ) {
    return cachedServerProjects;
  }
  if (!force && inFlightProjectsFetch) {
    return inFlightProjectsFetch;
  }
  let run: Promise<NamedIdeProject[] | null> | null = null;
  const started = (async (): Promise<NamedIdeProject[] | null> => {
    try {
      const { api } = await import('./api/client.ts');
      const body = await api.bg.get<{ projects?: ReadonlyArray<Record<string, unknown>> }>(
        '/ide/projects',
      );
      const remote = Array.isArray(body?.projects) ? body.projects : [];
      const local = loadNamedIdeProjectsLocal();
      const merged = mergeBackendIdeProjects(local, remote, true);
      saveNamedIdeProjectsLocal(merged, { source: 'server' });
      return merged;
    } catch {
      return null;
    } finally {
      if (run && inFlightProjectsFetch === run) {
        inFlightProjectsFetch = null;
      }
    }
  })();
  run = started;
  inFlightProjectsFetch = started;
  return started;
}
