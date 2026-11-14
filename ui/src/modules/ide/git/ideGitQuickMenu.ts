// Quick `Git >` context-menu actions + repository operations (`Fetch`, `Pull`,
// `Push…`, `Branches…`, `New Branch…`, `Stash / Unstash…`, `Commit File…`,
// `Add / Stage`, `Annotate with Git Blame`, `Show Diff`, `Compare with Revision…`,
// `Compare with Branch or Tag…`, `Show History`, `Rollback…`) reusing Otto's
// `gitApi` (`ui/src/lib/api/git.ts`) and `/repos/:id/*` endpoints.

import { api } from '../../../lib/api/client';
import { gitApi, pullReport } from '../../../lib/api/git';
import type { Repo, RepoStatusResp, StashInfo } from '../../../lib/api/types';
import { confirmer } from '../../../lib/confirm.svelte';
import { ctxMenu, type MenuItem, type MenuOptions } from '../../../lib/contextmenu.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { ide, ideBasename, type IdeBottomTab } from '../ideStore.svelte';
import { ideGit } from './ideGitController.svelte';

export type IdeMenuAnchor = MouseEvent | KeyboardEvent | Element | null;

export function captureMenuAnchor(anchor: IdeMenuAnchor): IdeMenuAnchor {
  if (!anchor || anchor instanceof Element) return anchor;
  const el = anchor.currentTarget instanceof HTMLElement ? anchor.currentTarget : null;
  const keyboard = !('clientX' in anchor) || (anchor.type === 'click' && anchor.detail === 0);
  if (el && (keyboard || (anchor.type === 'click' && el.getBoundingClientRect().height <= 48))) {
    return el;
  }
  if ('clientX' in anchor) {
    return new MouseEvent('contextmenu', { clientX: anchor.clientX, clientY: anchor.clientY });
  }
  return el;
}

export function showIdeMenu(anchor: IdeMenuAnchor, items: MenuItem[], opts?: MenuOptions): void {
  if (!anchor || anchor instanceof Element) ctxMenu.showAt(anchor, items, opts);
  else ctxMenu.show(anchor, items, opts);
}

export type IdeGitCompareLauncher = (
  mode: 'git-revision' | 'git-branch',
  absPath: string,
) => void;

async function requireRepo(absPath: string): Promise<Repo | null> {
  const repo = await ideGit.resolveRepoForPath(absPath);
  if (!repo) toasts.info('This folder isn’t inside a detected Git repository.');
  return repo;
}

export async function gitFetchRepo(absPath: string): Promise<void> {
  const repo = await requireRepo(absPath);
  if (!repo) return;
  try {
    const st = await api.post<RepoStatusResp>(`/repos/${encodeURIComponent(repo.id)}/fetch`, {});
    ideGit.applyStatus(repo.id, st);
    toasts.success('Fetched from remote', repo.name);
  } catch (e) {
    toastError('Couldn’t fetch from remote', e);
  }
}

export async function gitPullRepo(absPath: string): Promise<void> {
  const repo = await requireRepo(absPath);
  if (!repo) return;
  try {
    const res = await gitApi.pull(repo.id);
    ideGit.applyStatus(repo.id, res.status);
    await ide.loadTree();
    for (let i = 0; i < ide.tabs.length; i++) {
      const p = ide.tabs[i].path;
      if (p.startsWith(`${repo.path.replace(/\/+$/, '')}/`)) {
        await ideGit.reloadIdeTabIfOpen(p);
      }
    }
    const report = pullReport(res);
    if (report.tone === 'warn') toasts.warn(report.title, report.body);
    else toasts.success(report.title, report.body ?? repo.name);
  } catch (e) {
    toastError('Couldn’t pull from remote', e);
  }
}

/** Outward-facing action: ALWAYS confirms destination branch & repository first. */
export async function gitPushRepo(absPath: string): Promise<void> {
  const repo = await requireRepo(absPath);
  if (!repo) return;
  const st = ideGit.activeStatus;
  const branch = st?.branch || 'current branch';
  const remoteTarget = st?.upstream || repo.remote_url || 'origin';
  const ok = await confirmer.ask(
    `Push local commits on “${branch}” in ${repo.name} to ${remoteTarget}? Collaborators with access to the remote repository will see these commits.`,
    {
      title: `Push ${branch} to remote?`,
      confirmLabel: 'Push commits',
    },
  );
  if (!ok) return;
  try {
    const next = await gitApi.push(repo.id);
    ideGit.applyStatus(repo.id, next);
    toasts.success(`Pushed ${branch}`, remoteTarget);
  } catch (e) {
    toastError('Couldn’t push commits', e);
  }
}

export async function gitCreateNewBranch(absPath: string): Promise<void> {
  const repo = await requireRepo(absPath);
  if (!repo) return;
  const name = await confirmer.promptText(`Create and switch to a new branch in ${repo.name}`, {
    title: 'New Git branch',
    confirmLabel: 'Create & checkout',
    placeholder: 'feat/my-change',
    validate: (v) =>
      /\s/.test(v) || v.includes('..') ? 'Branch names cannot contain spaces or "..".' : null,
  });
  if (!name) return;
  try {
    const st = await gitApi.checkout(repo.id, { branch: name.trim(), create: true });
    ideGit.applyStatus(repo.id, st);
    toasts.success(`Switched to new branch ${name.trim()}`, repo.name);
  } catch (e) {
    toastError('Couldn’t create branch', e);
  }
}

export async function gitOpenBranchesMenu(absPath: string, anchor: IdeMenuAnchor): Promise<void> {
  const savedAnchor = captureMenuAnchor(anchor);
  const repo = await requireRepo(absPath);
  if (!repo) return;
  try {
    const res = await gitApi.readRefs(repo.id);
    const local = res.refs?.local ?? [];
    const remote = res.refs?.remote ?? [];
    const items: MenuItem[] = [
      {
        label: 'New branch…',
        icon: 'plus',
        pinned: true,
        action: () => void gitCreateNewBranch(absPath),
      },
    ];
    for (let i = 0; i < local.length; i++) {
      const br = local[i];
      items.push({
        label: br.name,
        icon: br.is_current ? 'check' : 'branch',
        hint: br.is_current ? 'Current' : br.sha?.slice(0, 7),
        disabled: br.is_current,
        action: () => {
          void gitApi
            .checkout(repo.id, { branch: br.name })
            .then(async (st) => {
              ideGit.applyStatus(repo.id, st);
              await ide.loadTree();
              toasts.success(`Checked out ${br.name}`, repo.name);
            })
            .catch((err) => toastError(`Couldn’t checkout ${br.name}`, err));
        },
      });
    }
    if (remote.length > 0) items.push({ separator: true });
    for (let i = 0; i < remote.length; i++) {
      const br = remote[i];
      items.push({
        label: br.name,
        icon: 'globe',
        hint: br.sha?.slice(0, 7),
        action: () => {
          const localName = br.name.replace(/^[^/]+\//, '');
          void gitApi
            .checkout(repo.id, { branch: localName, create: !local.some((l) => l.name === localName) })
            .then(async (st) => {
              ideGit.applyStatus(repo.id, st);
              await ide.loadTree();
              toasts.success(`Checked out ${localName}`, repo.name);
            })
            .catch((err) => toastError(`Couldn’t checkout ${br.name}`, err));
        },
      });
    }
    showIdeMenu(savedAnchor, items, { filter: true, filterPlaceholder: 'Filter branches…' });
  } catch (e) {
    toastError('Couldn’t load branches', e);
  }
}

export async function gitStashSave(absPath: string): Promise<void> {
  const repo = await requireRepo(absPath);
  if (!repo) return;
  const ok = await confirmer.ask(
    `Save uncommitted local changes in ${repo.name} onto the Git stash stack and clean the working tree?`,
    { title: 'Stash changes?', confirmLabel: 'Stash changes' },
  );
  if (!ok) return;
  try {
    const st = await api.post<RepoStatusResp>(`/repos/${encodeURIComponent(repo.id)}/stash`, {
      op: 'save',
    });
    ideGit.applyStatus(repo.id, st);
    await ide.loadTree();
    toasts.success('Stashed local changes', repo.name);
  } catch (e) {
    toastError('Couldn’t stash local changes', e);
  }
}

export async function gitUnstashMenu(absPath: string, anchor: IdeMenuAnchor): Promise<void> {
  const savedAnchor = captureMenuAnchor(anchor);
  const repo = await requireRepo(absPath);
  if (!repo) return;
  try {
    const stashes = await api.get<StashInfo[]>(`/repos/${encodeURIComponent(repo.id)}/stashes`);
    if (stashes.length === 0) {
      toasts.info('No stashes saved in this repository.');
      return;
    }
    const items: MenuItem[] = stashes.map((s) => ({
      label: `stash@{${s.index}}: ${s.message || s.sha.slice(0, 7)}`,
      icon: 'archive',
      hint: 'Pop',
      action: () => {
        void api
          .post<RepoStatusResp>(`/repos/${encodeURIComponent(repo.id)}/stash`, {
            op: 'pop',
            sha: s.sha,
          })
          .then(async (st) => {
            ideGit.applyStatus(repo.id, st);
            await ide.loadTree();
            toasts.success(`Popped stash@{${s.index}}`, repo.name);
          })
          .catch((err) => toastError('Couldn’t apply stash', err));
      },
    }));
    showIdeMenu(savedAnchor, items, { filter: true, filterPlaceholder: 'Filter stashes…' });
  } catch (e) {
    toastError('Couldn’t load stashes', e);
  }
}

export async function gitStageSingleFile(absPath: string): Promise<void> {
  const repo = await requireRepo(absPath);
  if (!repo) return;
  const rel = ideGit.repoRelativePath(repo, absPath);
  if (!rel) return;
  try {
    await ideGit.stagePaths(repo.id, [rel], true);
    toasts.success('Staged file', ideBasename(absPath));
  } catch (e) {
    toastError('Couldn’t stage file', e);
  }
}

export async function gitRollbackSingleFile(absPath: string): Promise<void> {
  const repo = await requireRepo(absPath);
  if (!repo) return;
  const rel = ideGit.repoRelativePath(repo, absPath);
  if (!rel) return;
  const ok = await confirmer.ask(
    `Rollback uncommitted changes in “${ideBasename(absPath)}”? This cannot be undone from Git (a Local History snapshot is preserved).`,
    { title: 'Rollback file?', confirmLabel: 'Rollback', danger: true },
  );
  if (!ok) return;
  try {
    await ideGit.discardPaths(repo.id, [rel], false);
    toasts.info('Rolled back changes', ideBasename(absPath));
  } catch (e) {
    toastError('Couldn’t rollback file', e);
  }
}

/** Build the `Git >` submenu items matching IntelliJ / GoLand (`uploaded_media_2.jpg`). */
export function buildGitSubmenuItems(opts: {
  absPath: string;
  isDir: boolean;
  anchor: IdeMenuAnchor;
  openCompare: IdeGitCompareLauncher;
  onBack?: () => void;
}): MenuItem[] {
  const { absPath, isDir, anchor, openCompare, onBack } = opts;
  const items: MenuItem[] = [];
  if (onBack) {
    items.push({ label: 'Back', icon: 'chevronLeft', pinned: true, action: onBack });
  }
  if (!isDir) {
    items.push(
      {
        label: 'Commit File…',
        icon: 'commit',
        hint: '⌘K',
        action: () => void ideGit.openChangesForFile(absPath, { focusCommit: true }),
      },
      { label: 'Add / Stage', icon: 'plus', hint: '⌥⌘A', action: () => void gitStageSingleFile(absPath) },
      {
        label: 'Annotate with Git Blame',
        icon: 'user',
        action: () => void ideGit.openFileHistoryTab(absPath, 'blame'),
      },
      { label: 'Show Diff', icon: 'split', hint: '⌘D', action: () => void ideGit.openChangesForFile(absPath) },
      {
        label: 'Compare with Revision…',
        icon: 'clock',
        action: () => openCompare('git-revision', absPath),
      },
      {
        label: 'Compare with Branch or Tag…',
        icon: 'branch',
        action: () => openCompare('git-branch', absPath),
      },
      {
        label: 'Show History',
        icon: 'clock',
        action: () => void ideGit.openFileHistoryTab(absPath, 'history'),
      },
      {
        label: 'Rollback…',
        icon: 'undo',
        hint: '⌥⌘Z',
        danger: true,
        action: () => void gitRollbackSingleFile(absPath),
      },
      { separator: true },
    );
  } else {
    items.push(
      {
        label: 'Local Changes…',
        icon: 'commit',
        action: () => {
          ideGit.selectRoot(absPath);
          ideGit.activeSubTab = 'changes';
          ide.openBottomTab('git' as IdeBottomTab);
        },
      },
      {
        label: 'Show Repository Log',
        icon: 'clock',
        action: () => {
          ideGit.selectRoot(absPath);
          ideGit.activeSubTab = 'log';
          ide.openBottomTab('git' as IdeBottomTab);
        },
      },
      { separator: true },
    );
  }
  items.push(
    { label: 'Push…', icon: 'arrowUp', hint: '⌘⇧K', action: () => void gitPushRepo(absPath) },
    { label: 'Pull…', icon: 'arrowDown', hint: '⌘T', action: () => void gitPullRepo(absPath) },
    { label: 'Fetch', icon: 'fetch', action: () => void gitFetchRepo(absPath) },
    { separator: true },
    { label: 'Branches…', icon: 'branch', action: () => void gitOpenBranchesMenu(absPath, anchor) },
    { label: 'New Branch…', icon: 'plus', action: () => void gitCreateNewBranch(absPath) },
    { separator: true },
    { label: 'Stash Changes…', icon: 'stash', action: () => void gitStashSave(absPath) },
    { label: 'Unstash Changes…', icon: 'archive', action: () => void gitUnstashMenu(absPath, anchor) },
  );
  return items;
}

/** Handle `otto:ide-git-action` window events dispatched from `CodeEditor` / shortcuts. */
export function runIdeGitActionEvent(
  detail: { action?: string; path?: string } | undefined,
  openCompare: IdeGitCompareLauncher,
): void {
  const targetPath = detail?.path || ide.activePath || ide.target.root;
  if (!targetPath || !detail?.action) return;
  const anchor: IdeMenuAnchor = null;
  switch (detail.action) {
    case 'commit':
      void ideGit.openChangesForFile(targetPath, { focusCommit: true });
      break;
    case 'stage':
      void gitStageSingleFile(targetPath);
      break;
    case 'blame':
      void ideGit.openFileHistoryTab(targetPath, 'blame');
      break;
    case 'diff':
      void ideGit.openChangesForFile(targetPath);
      break;
    case 'compare-revision':
      openCompare('git-revision', targetPath);
      break;
    case 'compare-branch':
      openCompare('git-branch', targetPath);
      break;
    case 'history':
      void ideGit.openFileHistoryTab(targetPath, 'history');
      break;
    case 'rollback':
      void gitRollbackSingleFile(targetPath);
      break;
    case 'push':
      void gitPushRepo(targetPath);
      break;
    case 'pull':
      void gitPullRepo(targetPath);
      break;
    case 'fetch':
      void gitFetchRepo(targetPath);
      break;
    case 'branches':
      void gitOpenBranchesMenu(targetPath, anchor);
      break;
    case 'new-branch':
      void gitCreateNewBranch(targetPath);
      break;
    case 'stash':
      void gitStashSave(targetPath);
      break;
    case 'unstash':
      void gitUnstashMenu(targetPath, anchor);
      break;
  }
}
