import type { Repo, Session } from '../../lib/api/types';
import { isForeground } from '../../lib/stores/workspace.svelte';

export type PreviewKind = 'markdown' | 'json' | 'csv' | 'svg' | 'image' | 'code';

/** Whether a session is a raw interactive shell PTY (not a coding-agent CLI). */
export function isPlainShellSession(s: Session | null | undefined): boolean {
  if (!s) return false;
  return s.provider === 'shell' && typeof (s.meta as Record<string, unknown> | undefined)?.nested_provider !== 'string';
}

/** Whether a session belongs in the Threads workspace (foreground agent & shell threads, plus delegated workers). */
export function isThreadSession(s: Session): boolean {
  if (s.kind === 'connection') return false;
  if (isForeground(s)) return true;
  // Delegated worker sessions spawned from another thread belong in the thread tree.
  return delegatedParentId(s) !== null;
}

/** Parent session ID when this session was delegated or handed over from another thread. */
export function delegatedParentId(s: Session | null | undefined): string | null {
  if (!s?.meta) return null;
  const m = s.meta as Record<string, unknown>;
  if (typeof m.delegated_by === 'string' && m.delegated_by.trim() !== '') return m.delegated_by.trim();
  if (typeof m.parent_session_id === 'string' && m.parent_session_id.trim() !== '') return m.parent_session_id.trim();
  if (typeof m.handover_from === 'string' && m.handover_from.trim() !== '') return m.handover_from.trim();
  const work = m.work as { parent_session_id?: unknown } | undefined;
  if (typeof work?.parent_session_id === 'string' && work.parent_session_id.trim() !== '') {
    return work.parent_session_id.trim();
  }
  return null;
}

/** Short human project folder label from a working directory or workspace name. */
export function projectNameFromCwd(cwd: string | null | undefined, fallback = 'Workspace'): string {
  const clean = (cwd ?? '').trim().replace(/\/+$/, '');
  if (!clean) return fallback;
  if (clean === '~') return 'Home';
  const parts = clean.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? fallback;
}

/** Compact path for chips and subtitles (`~/repo/sub` or last two segments). */
export function compactPath(path: string | null | undefined, homePath?: string | null): string {
  const clean = (path ?? '').trim().replace(/\/+$/, '');
  if (!clean) return '~';
  if (homePath) {
    const h = homePath.replace(/\/+$/, '');
    if (clean === h) return '~';
    if (clean.startsWith(`${h}/`)) return `~${clean.slice(h.length)}`;
  }
  const segs = clean.split('/').filter(Boolean);
  if (segs.length <= 2) return clean;
  return `…/${segs.slice(-2).join('/')}`;
}

/** Relative path inside a workspace root when nested, else the full path. */
export function relToRoot(filePath: string, rootPath: string | null | undefined): string {
  const f = filePath.trim();
  const r = (rootPath ?? '').trim().replace(/\/+$/, '');
  if (r && f.startsWith(`${r}/`)) return f.slice(r.length + 1);
  return f;
}

/** Best-known git branch for a session from its metadata or matching workspace repo. */
export function sessionBranch(
  session: Session | null | undefined,
  repos: readonly Repo[],
  statusById: Record<string, { branch?: string | null } | null | undefined>,
): string | null {
  if (!session) return null;
  const m = (session.meta ?? {}) as Record<string, unknown>;
  for (const key of ['git_branch', 'branch', 'worktree_branch']) {
    const v = m[key];
    if (typeof v === 'string' && v.trim() !== '') return v.trim();
  }
  const cwd = (session.cwd ?? '').replace(/\/+$/, '');
  if (!cwd) return null;
  const repo = matchRepoForCwd(cwd, repos);
  if (!repo) return null;
  return statusById[repo.id]?.branch ?? null;
}

/** Best-matching repo whose path contains (or equals) `cwd`. */
export function matchRepoForCwd(cwd: string | null | undefined, repos: readonly Repo[]): Repo | null {
  const clean = (cwd ?? '').trim().replace(/\/+$/, '');
  if (!clean || repos.length === 0) return repos[0] ?? null;
  let best: Repo | null = null;
  let bestLen = -1;
  for (const r of repos) {
    const rp = r.path.replace(/\/+$/, '');
    if (clean === rp || clean.startsWith(`${rp}/`) || rp.startsWith(`${clean}/`)) {
      if (rp.length > bestLen) {
        best = r;
        bestLen = rp.length;
      }
    }
  }
  return best ?? repos[0] ?? null;
}

/** Harness display name for T3/PandaCode-style cards and badges. */
export function harnessLabel(provider: string): string {
  switch (provider) {
    case 'claude':
      return 'Claude Code';
    case 'codex':
      return 'Codex';
    case 'agy':
      return 'Antigravity';
    case 'shell':
      return 'Shell';
    default:
      return provider.charAt(0).toUpperCase() + provider.slice(1);
  }
}

/** File basename (`src/App.svelte` -> `App.svelte`). */
export function baseName(path: string): string {
  const clean = path.replace(/\/+$/, '');
  const idx = clean.lastIndexOf('/');
  return idx >= 0 ? clean.slice(idx + 1) : clean;
}

/** Rich preview kind supported by `ThreadFileTabs`. */
export function previewKindForPath(path: string): PreviewKind {
  const lower = path.toLowerCase();
  if (/\.(md|markdown|mdx)$/.test(lower)) return 'markdown';
  if (/\.json$/.test(lower)) return 'json';
  if (/\.(csv|tsv)$/.test(lower)) return 'csv';
  if (/\.svg$/.test(lower)) return 'svg';
  if (/\.(png|jpe?g|gif|webp|avif|ico|bmp)$/.test(lower)) return 'image';
  return 'code';
}

/** Lightweight CSV/TSV parser for the PandaCode-style structured table preview. */
export function parseDelimitedTable(raw: string, delimiter = ',', maxRows = 120): { headers: string[]; rows: string[][] } {
  const lines = raw
    .split(/\r?\n/)
    .map((l) => l.trimEnd())
    .filter((l) => l.length > 0)
    .slice(0, maxRows + 1);
  if (lines.length === 0) return { headers: [], rows: [] };
  const splitLine = (line: string): string[] => {
    const out: string[] = [];
    let cur = '';
    let quoted = false;
    for (let i = 0; i < line.length; i++) {
      const ch = line[i];
      if (ch === '"') {
        if (quoted && line[i + 1] === '"') {
          cur += '"';
          i++;
        } else {
          quoted = !quoted;
        }
      } else if (ch === delimiter && !quoted) {
        out.push(cur);
        cur = '';
      } else {
        cur += ch;
      }
    }
    out.push(cur);
    return out;
  };
  const headers = splitLine(lines[0]);
  const rows = lines.slice(1).map(splitLine);
  return { headers, rows };
}
