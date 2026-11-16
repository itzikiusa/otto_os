import type { Repo, Session } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { harnessLabel, isPlainShellSession } from '../../lib/providers';
import { delegatedParentId, inThreadsScope, type SessionPool } from '../../lib/stores/sessionBuckets';

export { harnessLabel, isPlainShellSession };

// The file viewer's preview kinds and table parse live with the shared
// FilePreview (lib/) — re-exported so callers keep importing them from here.
export { previewKindForPath, type PreviewKind } from '../../lib/components/filePreviewKind';
export { parseDelimitedTable } from '../../lib/render/previewText';

/**
 * Whether a session belongs in the Threads workspace (`#/threads`): the shared
 * Agents-vs-Threads rule (`inThreadsScope`: its own `meta.surface`, else the
 * delegating parent's), minus what lives inside a thread rather than as one.
 */
export function isThreadSession(s: Session, pool: SessionPool = ws.sessions): boolean {
  if (s.kind === 'connection') return false;
  // A side chat lives in its main thread's right panel until promoted.
  if (sideChatOf(s)) return false;
  // The bottom terminal drawer's shells are not threads.
  if ((s.meta as Record<string, unknown> | undefined)?.threads_terminal === true) return false;
  return inThreadsScope(s, pool);
}

/** Main thread id when `s` is a side chat (`meta.side_of`), else null. */
export function sideChatOf(s: Session | null | undefined): string | null {
  const v = (s?.meta as Record<string, unknown> | undefined)?.side_of;
  return typeof v === 'string' && v.trim() !== '' ? v : null;
}

/** Parent session ID when this session was delegated or handed over from another thread. */
export { delegatedParentId };

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
  // Strict match: never another repo's branch for a folder no repo contains.
  const repo = repoForCwd(cwd, repos);
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

/** The repo whose folder holds `cwd` (deepest match), or null — no fallback to
 *  an unrelated repo, so the project card never shows another repo's branch. */
export function repoForCwd(cwd: string | null | undefined, repos: readonly Repo[]): Repo | null {
  const clean = (cwd ?? '').trim().replace(/\/+$/, '');
  if (!clean) return null;
  let best: Repo | null = null;
  for (const r of repos) {
    const rp = r.path.replace(/\/+$/, '');
    if ((clean === rp || clean.startsWith(`${rp}/`)) && (!best || rp.length > best.path.length)) best = r;
  }
  return best;
}

/** "GitHub" / "Bitbucket" / "GitLab" for a repo's forge, else null. */
export function forgeLabel(repo: Repo | null | undefined): string | null {
  const kind = repo?.forge && repo.forge !== 'unrecognized' ? repo.forge : repo?.provider;
  if (kind === 'github') return 'GitHub';
  if (kind === 'bitbucket') return 'Bitbucket';
  if (kind === 'gitlab') return 'GitLab';
  return null;
}

/** File basename (`src/App.svelte` -> `App.svelte`). */
export function baseName(path: string): string {
  const clean = path.replace(/\/+$/, '');
  const idx = clean.lastIndexOf('/');
  return idx >= 0 ? clean.slice(idx + 1) : clean;
}
