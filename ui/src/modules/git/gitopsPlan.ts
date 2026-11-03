// Pure planning for the worktree / stash / branch-cleanup flows: what a
// worktree row says about the data it holds, what a removal confirm must
// spell out, which branches may be bulk-deleted, and the exact local / remote
// work a bulk delete sends. No Svelte, no fetches — unit-tested in
// ui/unit/gitopsPlan.test.ts.
import type { BranchDeleteItem, BranchDeleteResult, RefBranch, WorktreeInfo } from '../../lib/api/types';
import { plural } from '../../lib/plural.ts';

// ── Worktrees ────────────────────────────────────────────────────────────────

/** Display name for a worktree: its last path segment. */
export function worktreeName(w: Pick<WorktreeInfo, 'path'>): string {
  return w.path.split('/').filter(Boolean).pop() ?? w.path;
}

/** Compact facts for a worktree row (badges) — only what is non-zero. */
export interface WorktreeFacts {
  /** Tracked files with changes. */
  changes: number;
  untracked: number;
  /** Commits nothing else holds (no remote, not in base). */
  unpushed: number;
  /** Commits ahead of its upstream. */
  ahead: number;
  /** Commits not in the base branch. */
  unmerged: number;
  /** A branch that exists on no remote. */
  localOnly: boolean;
  /** Status could not be read. */
  unknown: boolean;
}

export function worktreeFacts(w: WorktreeInfo): WorktreeFacts {
  return {
    changes: w.changes ?? 0,
    untracked: w.untracked ?? 0,
    unpushed: w.unpushed ?? 0,
    ahead: w.ahead ?? 0,
    unmerged: w.unmerged ?? 0,
    localOnly: !!w.branch && !w.on_remote && !w.is_main && !w.prunable,
    unknown: !w.prunable && w.dirty_known === false,
  };
}

/** One-line status for a row's tooltip / an accessible label. */
export function worktreeSummary(w: WorktreeInfo): string {
  if (w.prunable) return 'Stale entry — its folder is gone';
  const f = worktreeFacts(w);
  const parts: string[] = [];
  if (f.unknown) parts.push('changes unknown');
  else if (f.changes === 0 && f.untracked === 0) parts.push('no uncommitted changes');
  if (f.changes) parts.push(plural(f.changes, 'uncommitted change'));
  if (f.untracked) parts.push(plural(f.untracked, 'untracked file'));
  if (w.upstream) {
    if (w.upstream_gone) parts.push(`upstream ${w.upstream} is gone`);
    else if (f.ahead) parts.push(`${plural(f.ahead, 'commit')} ahead of ${w.upstream}`);
    else parts.push(`in sync with ${w.upstream}`);
  } else if (w.branch && !w.is_main) {
    parts.push(w.on_remote ? 'on the remote (no upstream set)' : 'branch not on any remote');
  }
  if (f.unpushed) parts.push(`${plural(f.unpushed, 'commit')} exist only here`);
  if (w.base && !w.is_main && f.unmerged) parts.push(`${plural(f.unmerged, 'commit')} not in ${w.base}`);
  if (w.locked) parts.push(w.lock_reason ? `locked: ${w.lock_reason}` : 'locked');
  return parts.join(' · ');
}

/** What the remove confirm says and sends, from a FRESH status
 *  (GET …/worktrees/status). `blocked` = the daemon will refuse even a force
 *  (unknown status / unverifiable tree) — show `message`, don't ask. */
export interface WorktreeRemovePlan {
  blocked: boolean;
  force: boolean;
  title: string;
  confirmLabel: string;
  message: string;
}

export function worktreeRemovePlan(w: WorktreeInfo): WorktreeRemovePlan {
  const name = worktreeName(w);
  const keep = w.branch ? ` The branch “${w.branch}” and its commits are kept.` : '';
  if (w.prunable) {
    return { blocked: false, force: false, title: 'Remove stale worktree entry', confirmLabel: 'Remove entry',
      message: `The folder of “${name}” is already gone. Remove its stale entry from the repo?` };
  }
  if (w.dirty_known === false) {
    const why = (w.risks ?? []).find((r) => /verify/.test(r)) ?? 'Otto couldn’t read its status (git status failed there)';
    return { blocked: true, force: false, title: 'Can’t remove this worktree', confirmLabel: '',
      message: `${why}. Nothing was deleted — inspect ${w.path} in a terminal, then retry.` };
  }
  const risks = w.risks ?? [];
  if (risks.length === 0) {
    return { blocked: false, force: false, title: 'Remove worktree', confirmLabel: 'Remove',
      message: `Remove worktree “${name}” at ${w.path}? It has no uncommitted changes and no commits that exist only here.${keep}` };
  }
  return { blocked: false, force: true, title: 'Force-remove worktree', confirmLabel: 'Force remove',
    message: `“${name}” at ${w.path} still holds work:\n\n${risks.map((r) => `• ${r}`).join('\n')}\n\nForce-removing deletes the folder and everything uncommitted in it. This cannot be undone.${keep}` };
}

// ── Stashes ──────────────────────────────────────────────────────────────────

/** "just now" / "5 min ago" / "3 h ago" / "2 d ago" / a date for older. */
export function relativeAge(iso: string, now: number = Date.now()): string {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '';
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86_400) return `${Math.round(s / 3600)} h ago`;
  if (s < 30 * 86_400) return `${Math.round(s / 86_400)} d ago`;
  return new Date(t).toISOString().slice(0, 10);
}

/** A stash's message without git's "On <branch>: " / "WIP on <branch>: " lead. */
export function stashTitle(message: string): string {
  return message.replace(/^(WIP on|On) [^:]+:\s*/, '').trim() || message;
}

/** Split a unified patch into per-file chunks keyed by the b/ path. */
export function splitPatch(patch: string): { path: string; text: string }[] {
  const out: { path: string; text: string }[] = [];
  for (const chunk of patch.split(/^(?=diff --git )/m)) {
    if (!chunk.startsWith('diff --git ')) continue;
    const head = chunk.slice(0, chunk.indexOf('\n') === -1 ? chunk.length : chunk.indexOf('\n'));
    const m = / b\/(.*)$/.exec(head);
    out.push({ path: m ? m[1] : head.slice(11), text: chunk });
  }
  return out;
}

// ── Branch cleanup ───────────────────────────────────────────────────────────

/** Branches no bulk delete may touch, plus why. */
export interface ProtectContext {
  current: string | null;
  base: string | null;
  /** Branches checked out in another worktree. */
  inWorktree: Set<string>;
}

const ALWAYS_PROTECTED = new Set(['main', 'master']);

/** Short branch name of a remote ref: `origin/feat/x` → `feat/x`. */
export function remoteParts(ref: string): { remote: string; branch: string } {
  const i = ref.indexOf('/');
  return i < 0 ? { remote: ref, branch: '' } : { remote: ref.slice(0, i), branch: ref.slice(i + 1) };
}

/** Why `b` can't be selected for a bulk delete; null when it can. */
export function protectionReason(b: RefBranch, ctx: ProtectContext): string | null {
  const name = b.remote ? remoteParts(b.name).branch : b.name;
  if (!b.remote && (b.is_current || name === ctx.current)) return 'checked out';
  if (ALWAYS_PROTECTED.has(name) || (ctx.base && name === ctx.base)) return 'default branch';
  if (b.remote && name === 'HEAD') return 'remote HEAD';
  if (!b.remote && ctx.inWorktree.has(name)) return 'checked out in a worktree';
  return null;
}

/** A local branch git's safe `-d` would refuse: nothing integration-side
 *  holds its tip. (git may still accept it when its upstream holds it.) */
export function isUnmerged(b: RefBranch): boolean {
  return !b.remote && !b.merged_into_base && (b.merged_into?.length ?? 0) === 0;
}

export interface BranchDeletePlan {
  items: BranchDeleteItem[];
  /** Local branches that will be force-deleted (`-D`). */
  forced: string[];
  /** remote → branch names deleted there (outward-facing). */
  remoteTargets: Map<string, string[]>;
}

/** Build the request for the selected branches. A selected local branch also
 *  deletes its upstream copy when `alsoRemote` and it has a live upstream;
 *  a selected remote ref deletes only that remote branch. `forceUnmerged`
 *  sends `-D` for the unmerged locals (the caller asked first). */
export function planBranchDelete(
  selected: RefBranch[],
  opts: { alsoRemote: boolean; forceUnmerged: boolean },
): BranchDeletePlan {
  const items: BranchDeleteItem[] = [];
  const forced: string[] = [];
  const remoteTargets = new Map<string, string[]>();
  const addRemote = (remote: string, branch: string): boolean => {
    const list = remoteTargets.get(remote) ?? [];
    if (list.includes(branch)) return false;
    list.push(branch);
    remoteTargets.set(remote, list);
    return true;
  };
  // Remote refs that a selected local branch already covers are not sent twice.
  const locals = selected.filter((b) => !b.remote);
  for (const b of locals) {
    const item: BranchDeleteItem = { name: b.name };
    if (isUnmerged(b) && opts.forceUnmerged) {
      item.force = true;
      forced.push(b.name);
    }
    if (opts.alsoRemote && b.upstream) {
      const { remote, branch } = remoteParts(b.upstream);
      if (branch && addRemote(remote, branch)) {
        item.remote = remote;
        if (branch !== b.name) item.remote_branch = branch;
      }
    }
    items.push(item);
  }
  for (const b of selected.filter((x) => x.remote)) {
    const { remote, branch } = remoteParts(b.name);
    if (!branch || !addRemote(remote, branch)) continue;
    items.push({ name: branch, local: false, remote });
  }
  return { items, forced, remoteTargets };
}

/** "origin: a, b · upstream: c" — the exact remote list a confirm shows. */
export function describeRemoteTargets(t: Map<string, string[]>): string {
  return [...t.entries()].map(([remote, names]) => `${remote}: ${names.join(', ')}`).join('\n');
}

/** Toast-ready summary of a bulk delete. */
export function summarizeDelete(results: BranchDeleteResult[]): {
  okCount: number;
  failed: BranchDeleteResult[];
  needForce: string[];
} {
  const failed = results.filter((r) => r.error);
  return {
    okCount: results.filter((r) => !r.error && (r.local_deleted || r.remote_deleted)).length,
    failed,
    needForce: failed.filter((r) => r.needs_force).map((r) => r.name),
  };
}

/** One result in words, for the inline result list. */
export function resultLine(r: BranchDeleteResult): string {
  const done: string[] = [];
  if (r.local_deleted) done.push('local deleted');
  if (r.remote_deleted && r.remote) done.push(`deleted on ${r.remote}`);
  if (r.error) done.push(r.needs_force ? 'not fully merged — needs force' : r.error);
  return done.join(' · ');
}
