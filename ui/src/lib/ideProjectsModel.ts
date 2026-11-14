// Shared Named IDE Projects model, canonical path helper, localStorage persistence,
// and backend `/ide/projects` hydration shared across IDE and Threads.
// Pure TypeScript (zero Svelte runes or extensionless imports) so `node --test` runs directly.

export interface NamedIdeProject {
  id: string;
  name: string;
  primaryRoot: string;
  folders: string[];
  pinned: boolean;
  updatedAt: number;
}

export const LS_IDE_PROJECTS = 'otto.ide.projects.v1';
const LS_IDE_PROJECTS_LEGACY = 'otto.ide.namedProjects.v1';
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

function basenameOfProjectPath(raw: string): string {
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

function safeLsGet(key: string): string | null {
  try {
    return typeof localStorage !== 'undefined' ? localStorage.getItem(key) : null;
  } catch {
    return null;
  }
}

function safeLsSet(key: string, value: string): void {
  try {
    if (typeof localStorage !== 'undefined') localStorage.setItem(key, value);
  } catch {
    /* ignore storage quota errors */
  }
}

function safeLsRemove(key: string): void {
  try {
    if (typeof localStorage !== 'undefined') localStorage.removeItem(key);
  } catch {
    /* ignore storage access errors */
  }
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
  const canonicalRaw = safeLsGet(LS_IDE_PROJECTS);
  if (canonicalRaw !== null) {
    return parseNamedIdeProjectsJson(canonicalRaw);
  }
  const legacyRaw = safeLsGet(LS_IDE_PROJECTS_LEGACY);
  if (legacyRaw !== null) {
    const migrated = parseNamedIdeProjectsJson(legacyRaw);
    saveNamedIdeProjectsLocal(migrated);
    return migrated;
  }
  return [];
}

export function saveNamedIdeProjectsLocal(projects: NamedIdeProject[]): void {
  const normalized: NamedIdeProject[] = [];
  const seenIds = new Set<string>();
  for (let i = 0; i < projects.length; i++) {
    const proj = parseNamedIdeProject(projects[i]);
    if (!proj || seenIds.has(proj.id)) continue;
    seenIds.add(proj.id);
    normalized.push(proj);
  }
  safeLsSet(LS_IDE_PROJECTS, JSON.stringify(normalized));
  safeLsRemove(LS_IDE_PROJECTS_LEGACY);
  if (typeof window !== 'undefined') {
    window.dispatchEvent(
      new CustomEvent(IDE_PROJECTS_CHANGED_EVENT, { detail: { projects: normalized } }),
    );
  }
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

function resolveApiBase(): string {
  const override = safeLsGet('otto_base');
  if (override) return override;
  if (typeof location !== 'undefined') {
    if (
      location.port === '5173' ||
      location.protocol === 'tauri:' ||
      location.protocol === 'file:'
    ) {
      return 'http://127.0.0.1:7700';
    }
    return location.origin;
  }
  return 'http://127.0.0.1:7700';
}

function resolveBearerToken(): string | null {
  try {
    if (typeof sessionStorage !== 'undefined') {
      const imp = sessionStorage.getItem('otto_imp_token');
      if (imp) return imp;
    }
  } catch {
    /* ignore */
  }
  return safeLsGet('otto_token');
}

export async function fetchNamedIdeProjectsFromServer(): Promise<NamedIdeProject[] | null> {
  if (safeLsGet('otto_mock') === '1') return null;
  if (typeof fetch !== 'function') return null;
  try {
    const headers: Record<string, string> = { Accept: 'application/json' };
    const token = resolveBearerToken();
    if (token) headers.Authorization = `Bearer ${token}`;
    const res = await fetch(`${resolveApiBase()}/api/v1/ide/projects`, {
      method: 'GET',
      headers,
    });
    if (!res.ok) return null;
    const body = (await res.json()) as { projects?: ReadonlyArray<Record<string, unknown>> } | null;
    const remote = Array.isArray(body?.projects) ? body.projects : [];
    const local = loadNamedIdeProjectsLocal();
    const merged = mergeBackendIdeProjects(local, remote, true);
    saveNamedIdeProjectsLocal(merged);
    return merged;
  } catch {
    return null;
  }
}
