// The git graph's mutating actions (arch-1-09: split out of GraphView.svelte):
// checkout (with the stash → switch → restore path), branch create / rename /
// delete, tags, stashes, worktrees, submodules, pull, rebase, cherry-pick,
// revert and GitFlow starts — each over `gitApi` / the git store, with its
// confirmation and its toasts. GraphView owns the graph state and passes it
// in as a host (getters for state, its refresh helpers as functions); the
// menus (graphMenus.ts) and the refs sidebar call these.

import { api, isDirtyGitRefusal } from '../../../lib/api/client';
import type { CommitInfo, RebasePreview, RefBranch, Repo, RepoStatusResp, StashInfo, SubmoduleInfo, WorktreeInfo } from '../../../lib/api/types';
import { toasts } from '../../../lib/toast.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import { confirmOutward } from '../../../lib/confirmOutward';
import { toastError } from '../../../lib/toastError';
import { plural } from '../../../lib/plural';
import { copyTextOrThrow } from '../../../lib/clipboard';
import { git } from '../../../lib/stores/git.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { runPull } from '../pullFlow';

/** What the refs tree knows about branch names (GraphView's `refKnowledge`). */
export interface RefKnowledge {
  remoteNames: Set<string>;
  localNames: Set<string>;
  currentBranch: string | null;
}

/** GraphView's side of the actions: live state (getters; the busy flags and
 *  the side lists are written back) and the refresh helpers it owns. */
export interface GraphActionsHost {
  readonly repoId: string;
  readonly workspaceId: string;
  readonly onstatus: (s: RepoStatusResp) => void;
  checkoutBusy: string;
  openWtBusy: string;
  worktrees: WorktreeInfo[];
  submodules: SubmoduleInfo[];
  readonly currentRepoPath: string;
  readonly refKnowledge: RefKnowledge;
  normPath(p: string): string;
  adoptRefsRev(): void;
  refreshAfter(status?: RepoStatusResp): Promise<void>;
  refreshAfterCheckout(status: RepoStatusResp, stashed: boolean): Promise<void>;
  mutate(path: string, body: unknown, okTitle: string, okDetail?: string): Promise<void>;
  holdDiff(): void;
  stashShortMsg(s: StashInfo): string;
}

export type GraphActions = ReturnType<typeof createGraphActions>;

export function createGraphActions(h: GraphActionsHost) {
  const { normPath, adoptRefsRev, refreshAfter, refreshAfterCheckout, mutate, holdDiff, stashShortMsg } = h;

  /** Copy `text` to the clipboard and toast success/failure with `label`. */
  async function clip(text: string, label: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success('Copied', label);
    } catch {
      toasts.error('Couldn’t copy to the clipboard', 'The clipboard isn’t available here. Select the text and copy it manually.');
    }
  }

  // ── GitFlow: start a feature/release/hotfix branch off a commit or branch tip.
  //    Reuses the same /branch endpoint (checkout:true) as the plain create. ──
  /** Normalize a user-typed flow name: trim, drop a leading flow prefix if they
   *  typed it, and collapse internal whitespace to dashes. '' → skip. */
  function normalizeFlowName(raw: string): string {
    return raw
      .trim()
      .replace(/^(feature|release|hotfix)\//i, '')
      .replace(/\s+/g, '-');
  }

  async function startFlowBranch(
    flow: 'feature' | 'release' | 'hotfix',
    startPoint: string,
  ): Promise<void> {
    const label = flow[0].toUpperCase() + flow.slice(1);
    const raw = await confirmer.promptText(`New ${flow} branch name`, {
      title: `Start ${flow}`,
      confirmLabel: 'Start',
      placeholder: 'short-name',
    });
    if (raw === null) return;
    const norm = normalizeFlowName(raw);
    if (norm === '') return;
    const name = `${flow}/${norm}`;
    await mutate('/branch', { name, start_point: startPoint, checkout: true }, `${label} branch created`, name);
  }

  async function cherryPick(c: CommitInfo): Promise<void> {
    await mutate('/cherry-pick', { sha: c.sha }, 'Cherry-picked', c.short_sha);
  }

  async function revertCommit(c: CommitInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Revert commit ${c.short_sha} — “${c.subject}”? This creates a new commit undoing its changes.`,
      { title: 'Revert commit', confirmLabel: 'Revert', danger: false },
    );
    if (!ok) return;
    await mutate('/revert', { sha: c.sha }, 'Reverted', c.short_sha);
  }

  async function createBranchAt(startPoint: string): Promise<void> {
    const name = await confirmer.promptText('New branch name', {
      title: 'Create branch',
      confirmLabel: 'Create',
      placeholder: 'feature/my-branch',
    });
    if (!name) return;
    await mutate(
      '/branch',
      { name, start_point: startPoint, checkout: true },
      'Branch created',
      name,
    );
  }

  async function createTagAt(sha: string, annotated: boolean): Promise<void> {
    const name = await confirmer.promptText('Tag name', {
      title: annotated ? 'Create annotated tag' : 'Create tag',
      confirmLabel: 'Create',
      placeholder: 'v1.0.0',
    });
    if (!name) return;
    let message: string | null = null;
    if (annotated) {
      message = await confirmer.promptText('Tag message', {
        title: 'Annotated tag message',
        confirmLabel: 'Create',
        placeholder: 'Release notes…',
      });
      if (!message) return;
    }
    // Ask whether to push the new tag straight to origin.
    const push = await confirmer.ask(`Push tag “${name}” to origin?`, {
      title: 'Push tag',
      confirmLabel: 'Push',
      danger: false,
    });
    await mutate(
      '/tag',
      annotated ? { name, sha, message, push } : { name, sha, push },
      'Tag created',
      name,
    );
  }

  async function createBranchFrom(ref: string): Promise<void> {
    const name = await confirmer.promptText(`New branch from ${ref}`, {
      title: 'Create branch',
      confirmLabel: 'Create',
      placeholder: 'feature/my-branch',
    });
    if (!name) return;
    await mutate('/branch', { name, start_point: ref, checkout: true }, 'Branch created', name);
  }

  async function renameBranch(from: string): Promise<void> {
    const to = await confirmer.promptText(`Rename branch “${from}” to`, {
      title: 'Rename branch',
      confirmLabel: 'Rename',
      initial: from,
    });
    if (!to || to === from) return;
    await mutate('/branch/rename', { from, to }, 'Branch renamed', `${from} → ${to}`);
  }

  /** `remote` names WHICH remote the `alsoRemote` half deletes on; it is sent
   *  as `remote_name` on both the safe and the forced request (the daemon
   *  otherwise defaults to origin). */
  async function deleteLocalBranch(name: string, alsoRemote: boolean, remote = 'origin'): Promise<void> {
    const ok = await confirmer.ask(
      alsoRemote
        ? `Delete branch “${name}” locally AND “${remote}/${name}” on the remote “${remote}”? This cannot be undone.`
        : `Delete local branch “${name}”?`,
      { title: 'Delete branch', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    const okTitle = alsoRemote ? 'Branch deleted (local + remote)' : 'Branch deleted';
    // Safe delete first (`-d`): git refuses when the branch holds unmerged
    // commits. Only then ask again, explicitly, before escalating to `-D` —
    // deleting unmerged work deserves its own consent, not a blanket force.
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${h.repoId}/branch/delete`, {
        name,
        remote: alsoRemote,
        ...(alsoRemote ? { remote_name: remote } : {}),
      });
      await refreshAfter(s);
      toasts.success(okTitle, name);
      return;
    } catch (e) {
      const msg = String((e as Error | null)?.message ?? e); // matched, never shown
      if (!/not fully merged/i.test(msg)) {
        toastError(`Couldn’t delete ${name}`, e);
        await refreshAfter().catch(() => {});
        return;
      }
      const force = await confirmer.ask(
        `“${name}” has commits that aren’t merged anywhere else. Force-delete it and DROP those commits? This cannot be undone.`,
        { title: 'Branch not fully merged', confirmLabel: 'Force delete', danger: true },
      );
      if (!force) return;
      await mutate(
        '/branch/delete',
        { name, remote: alsoRemote, ...(alsoRemote ? { remote_name: remote } : {}), force: true },
        okTitle,
        name,
      );
    }
  }

  async function deleteRemoteBranch(name: string, remote = 'origin'): Promise<void> {
    const full = `${remote}/${name}`;
    const ok = await confirmer.ask(`Delete branch “${full}” on the remote “${remote}”? This cannot be undone.`, {
      title: 'Delete remote branch',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    // Remote-only: leave any local branch of the same name intact. `remote_name`
    // pins WHICH remote — the daemon pushes `--delete` to that remote, not origin.
    await mutate(
      '/branch/delete',
      { name, remote: true, remote_name: remote, local: false, force: true },
      'Remote branch deleted',
      full,
    );
  }

  async function pushTag(name: string): Promise<void> {
    const ok = await confirmOutward({
      verb: 'Push tag',
      where: `origin · tag ${name}`,
      who: 'Everyone with access to the remote sees the tag; CI or release jobs may react to it.',
    });
    if (!ok) return;
    await mutate('/tag/push', { name }, 'Tag pushed', name);
  }

  async function deleteTag(name: string, remote: boolean): Promise<void> {
    const ok = remote // outward-facing like its push sibling: where + who (ux-design-5-15)
      ? await confirmOutward({ verb: 'Delete tag', where: `origin · tag ${name}`, danger: true,
          who: 'Everyone with access to the remote loses the tag; CI or release jobs may react.' })
      : await confirmer.ask(`Delete local tag “${name}”?`, { title: 'Delete tag', confirmLabel: 'Delete', danger: true });
    if (!ok) return;
    await mutate('/tag/delete', { name, remote }, remote ? 'Tag deleted on origin' : 'Tag deleted', name);
  }

  async function stashApply(s: StashInfo): Promise<void> {
    // SHA-anchored (not positional index): the backend resolves the live
    // stash@{N} so a renumbered stack can't apply the wrong entry.
    await mutate('/stash', { op: 'apply', sha: s.sha }, 'Stash applied', stashShortMsg(s));
  }

  async function stashDrop(s: StashInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Drop ${s.ref} — “${stashShortMsg(s)}”? This discards the stash and cannot be undone.`,
      { title: 'Drop stash', confirmLabel: 'Drop', danger: true },
    );
    if (!ok) return;
    // SHA-anchored drop — irreversible, so never trust a stale positional index.
    await mutate('/stash', { op: 'drop', sha: s.sha }, 'Stash dropped', s.ref);
  }

  /** Display name for a worktree: the last path segment. */
  function wtName(w: WorktreeInfo): string {
    return w.path.split('/').filter(Boolean).pop() ?? w.path;
  }

  /** True when this worktree is a different checkout than the open repo tab. */
  function isForeignWorktree(w: WorktreeInfo): boolean {
    if (w.prunable) return false;
    if (!h.currentRepoPath) return !w.is_main;
    return normPath(w.path) !== h.currentRepoPath;
  }

  /**
   * Open a linked (or main) worktree as its own git tab — never checkout the
   * branch into the current tree (git refuses: branch already checked out).
   * Registers the path via detect if needed, then opens/activates the tab.
   */
  async function openWorktree(w: WorktreeInfo): Promise<void> {
    if (w.prunable) {
      toasts.error('Couldn’t open the worktree', 'Its folder is gone — prune the stale entry first');
      return;
    }
    const path = normPath(w.path);
    if (h.currentRepoPath && path === h.currentRepoPath) {
      toasts.info('Already open', wtName(w));
      return;
    }
    // Prefer an already-registered repo at this path (any workspace).
    const existing = git.allRepos.find((r) => normPath(r.path) === path);
    if (existing) {
      git.openRepoTab(existing.id);
      toasts.success('Opened worktree', existing.name);
      return;
    }
    const parent = git.allRepos.find((r) => r.id === h.repoId);
    const wsId = h.workspaceId || parent?.workspace_id || ws.currentId;
    if (!wsId) {
      toasts.error('Couldn’t open worktree', 'No workspace available to register the worktree');
      return;
    }
    h.openWtBusy = w.path;
    try {
      const repo = await api.post<Repo>(`/workspaces/${wsId}/repos/detect`, { path: w.path });
      await git.loadAllRepos(true);
      git.openRepoTab(repo.id);
      toasts.success('Opened worktree', repo.name);
    } catch (e) {
      toastError('Couldn’t open worktree', e);
    } finally {
      h.openWtBusy = '';
    }
  }

  async function wtRemove(w: WorktreeInfo): Promise<void> {
    // A dirty tree needs --force and the confirm must say what that discards.
    // Removal always KEEPS the branch (and its commits) either way.
    const detail = !w.dirty_known
      ? ' Its changes couldn’t be checked; removal will proceed only if Git can do so safely.'
      : w.dirty
      ? ' It has UNCOMMITTED changes that will be discarded.'
      : w.locked
        ? ` It is locked${w.lock_reason ? ` (${w.lock_reason})` : ''}.`
        : '';
    const ok = await confirmer.ask(
      `Remove worktree “${wtName(w)}” at ${w.path}?${detail} The branch${w.branch ? ` “${w.branch}”` : ''} and its commits are kept.`,
      { title: 'Remove worktree', confirmLabel: 'Remove', danger: true },
    );
    if (!ok) return;
    try {
      h.worktrees = await api.post<WorktreeInfo[]>(`/repos/${h.repoId}/worktrees/remove`, {
        path: w.path,
        force: w.dirty_known === true && (w.dirty || w.locked),
      });
      toasts.success('Worktree removed', wtName(w));
    } catch (e) {
      toastError('Couldn’t remove the worktree', e);
    }
  }

  async function wtPrune(): Promise<void> {
    try {
      h.worktrees = await api.post<WorktreeInfo[]>(`/repos/${h.repoId}/worktrees/prune`, {});
      toasts.success('Pruned', 'Stale worktree entries removed');
    } catch (e) {
      toastError('Couldn’t prune the worktrees', e);
    }
  }

  async function subUpdate(path?: string): Promise<void> {
    try {
      h.submodules = await api.post<SubmoduleInfo[]>(`/repos/${h.repoId}/submodules/update`, {
        path: path ?? null,
      });
      toasts.success('Submodules updated', path ?? 'all');
    } catch (e) {
      toastError('Couldn’t update the submodule', e);
    }
  }

  /** Switch branches. NEVER pulls, fetches or merges — a dirty tree is cleared
   *  with stash → switch → restore (for both `create` values; the remote-row
   *  double-click used to dead-end on it), and bringing the branch up to date
   *  is the separate, explicit Pull action. */
  async function checkout(branch: string, create: boolean): Promise<void> {
    h.checkoutBusy = branch;
    try {
      const s = await git.checkout(h.repoId, { branch, create });
      adoptRefsRev();
      toasts.success(create ? 'Branch created' : `Switched to ${branch}`, create ? branch : undefined);
      await refreshAfterCheckout(s, false);
    } catch (e) {
      if (isDirtyGitRefusal(e)) {
        const ok = await confirmer.ask(
          `Your uncommitted changes overlap files that differ on “${branch}”. Otto will stash them, switch, and restore them on “${branch}”. Nothing is pulled or merged.`,
          { title: 'Stash, switch & restore', confirmLabel: 'Stash & switch', danger: false },
        );
        if (ok) {
          try {
            const s = await git.checkout(h.repoId, { branch, create, auto_stash: true });
            adoptRefsRev();
            if (s.changes.some((c) => c.kind === 'conflicted')) {
              toasts.warn(
                `Switched to ${branch} — restoring your changes hit conflicts`,
                'Open "Resolve conflicts".',
              );
            } else {
              toasts.success(
                `Switched to ${branch}`,
                'Your changes were restored; a recovery backup remains in Stashes.',
              );
            }
            await refreshAfterCheckout(s, true);
          } catch (e2) {
            // The switch landed but the restore didn't — the daemon's message
            // already ends with "run `git stash pop`".
            toastError(`Switched to ${branch}, but couldn’t restore your changes`, e2);
            await refreshAfter().catch(() => {});
          }
        }
      } else {
        toastError('Couldn’t check out the branch', e);
      }
    } finally {
      h.checkoutBusy = '';
    }
  }

  function checkoutRemote(b: RefBranch): void {
    holdDiff();
    void checkoutRemoteRef(b.name);
  }

  /** Check out a REMOTE-tracking ref (`origin/x`, `upstream/x`) the way every
   *  git client does: switch to the local twin when one exists (a `create`
   *  there died with "a branch named 'x' already exists"); otherwise create
   *  `x` AT that remote ref with tracking. Only `origin/*` may go through the
   *  checkout route's create (it bases on `origin/<name>`) — any other remote
   *  creates from its own ref via `/branch`, which used to fork `x` from the
   *  current HEAD instead and toast "Branch created". */
  async function checkoutRemoteRef(remoteRef: string): Promise<void> {
    const localName = remoteRef.replace(/^[^/]+\//, '');
    if (h.refKnowledge.localNames.has(localName)) return checkout(localName, false);
    if (remoteRef.startsWith('origin/')) return checkout(localName, true);
    h.checkoutBusy = localName;
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${h.repoId}/branch`, {
        name: localName,
        start_point: remoteRef,
        checkout: true,
      });
      toasts.success('Branch created', `${localName} tracking ${remoteRef}`);
      await refreshAfterCheckout(s, false);
    } catch (e) {
      toastError('Couldn’t check out the branch', e);
    } finally {
      h.checkoutBusy = '';
    }
  }

  /** Pull the CURRENT branch (the only branch a pull can target) and re-read the
   *  graph — new commits are exactly what a pull produces. */
  async function pullCurrent(): Promise<void> {
    await runPull(h.repoId, h.onstatus);
    await refreshAfter().catch(() => {});
  }

  /** Replay the current branch onto `onto`, after saying how much will move.
   *  A conflicting rebase comes back 200 with `op_in_progress` — `mutate`
   *  already routes that to the resolver. */
  async function rebaseOnto(onto: string): Promise<void> {
    const { currentBranch } = h.refKnowledge;
    if (!currentBranch) return;
    let commits: number | string = '?';
    try {
      const p = await api.get<RebasePreview>(
        `/repos/${h.repoId}/rebase-preview?onto=${encodeURIComponent(onto)}`,
      );
      commits = p.commits;
    } catch {
      // Preview is advisory — a repo whose daemon predates it still rebases.
    }
    const ok = await confirmer.ask(
      `Rebase \`${currentBranch}\` onto \`${onto}\`? ${typeof commits === 'number' ? plural(commits, 'commit') : '? commits'} will be replayed; conflicts open the resolver. Uncommitted changes are stashed and restored afterwards.`,
      { title: 'Rebase', confirmLabel: 'Rebase', danger: false },
    );
    if (ok) await mutate('/rebase', { onto, auto_stash: true }, 'Rebased', onto);
  }

  return {
    clip,
    normalizeFlowName,
    startFlowBranch,
    cherryPick,
    revertCommit,
    createBranchAt,
    createTagAt,
    createBranchFrom,
    renameBranch,
    deleteLocalBranch,
    deleteRemoteBranch,
    pushTag,
    deleteTag,
    stashApply,
    stashDrop,
    wtName,
    isForeignWorktree,
    openWorktree,
    wtRemove,
    wtPrune,
    subUpdate,
    checkout,
    checkoutRemote,
    checkoutRemoteRef,
    pullCurrent,
    rebaseOnto,
  };
}
