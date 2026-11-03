// Git + project menus shared by the new-thread meta row and the right project
// card: branch switch, checkout (worktree) picker, "Open in …", project
// scripts. Every action routes through an existing Otto store/endpoint — the
// git store's checkout, `GET /repos/{id}/worktrees`, the Git page, the
// inspector — so nothing here owns git state.
import { api } from '../../lib/api/client';
import { gitApi } from '../../lib/api/git';
import type { Repo, WorktreeInfo } from '../../lib/api/types';
import { copyWithToast } from '../../lib/clipboard';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { loadErrorText } from '../../lib/loadError';
import { router } from '../../lib/router.svelte';
import { git } from '../../lib/stores/git.svelte';
import { SCRATCH_WORKSPACE_ID, ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { compactPath, projectNameFromCwd } from './threadsHelpers';
import { threads, type ProjectScript } from './threadsStore.svelte';

/** Local branches of `repo` (current checked); picking one runs `git checkout`. */
export async function openBranchMenu(anchor: Element, repo: Repo): Promise<void> {
  const read = await gitApi.readRefs(repo.id);
  if (!read.refs) {
    toasts.error('Couldn’t load branches', loadErrorText(read.error));
    return;
  }
  const local = read.refs.local;
  const items: MenuItem[] = local.map(
    (b): MenuItem => ({
      label: b.name,
      checked: b.is_current,
      hint: b.ahead || b.behind ? `↑${b.ahead ?? 0} ↓${b.behind ?? 0}` : undefined,
      action: b.is_current ? undefined : () => void checkoutBranch(repo, b.name),
    }),
  );
  items.push(
    { separator: true, pinned: true },
    { label: 'New branch…', icon: 'plus', pinned: true, action: () => void createBranch(repo) },
    { label: 'Open in Git', icon: 'external', pinned: true, action: () => router.go(`git/${encodeURIComponent(repo.id)}`) },
  );
  ctxMenu.showAt(anchor, items, { filter: local.length > 8, filterPlaceholder: 'Filter branches…', align: 'end' });
}

async function checkoutBranch(repo: Repo, branch: string): Promise<void> {
  try {
    await git.checkout(repo.id, { branch });
    toasts.success('Switched branch', branch);
  } catch (e) {
    toastError(`Couldn’t switch to ${branch}`, e);
  }
}

async function createBranch(repo: Repo): Promise<void> {
  const name = await confirmer.promptText('The new branch starts from the current commit and is checked out.', {
    title: 'New branch',
    confirmLabel: 'Create',
    placeholder: 'feature/my-change',
    validate: (v) => (/^[^\s~^:?*[\\]+$/.test(v.trim()) ? null : 'Enter a valid branch name'),
  });
  if (!name) return;
  try {
    await git.checkout(repo.id, { branch: name.trim(), create: true });
    toasts.success('Created branch', name.trim());
  } catch (e) {
    toastError('Couldn’t create branch', e);
  }
}

/** "Current checkout" vs the repo's other worktrees (T3's checkout picker). */
export async function openCheckoutMenu(
  anchor: Element,
  repo: Repo,
  cwd: string,
  onPick: (path: string, branch: string | null) => void,
): Promise<void> {
  let list: WorktreeInfo[] = [];
  try {
    list = await api.get<WorktreeInfo[]>(`/repos/${encodeURIComponent(repo.id)}/worktrees`);
  } catch (e) {
    toastError('Couldn’t load worktrees', e);
    return;
  }
  const home = ws.scratch?.root_path ?? null;
  const here = cwd.replace(/\/+$/, '');
  const items: MenuItem[] = list
    .filter((w) => !w.prunable)
    .map(
      (w): MenuItem => ({
        label: w.is_main ? 'Current checkout' : projectNameFromCwd(w.path),
        hint: w.branch ?? 'detached',
        title: compactPath(w.path, home),
        checked: here === w.path.replace(/\/+$/, '') || (w.is_main && here.startsWith(`${w.path.replace(/\/+$/, '')}/`)),
        action: () => onPick(w.path, w.branch),
      }),
    );
  items.push(
    { separator: true },
    { label: 'Manage worktrees in Git', icon: 'worktree', action: () => router.go(`git/${encodeURIComponent(repo.id)}`) },
  );
  ctxMenu.showAt(anchor, items, { align: 'start' });
}

/** Otto has no native-editor launcher, so "Open in" offers the places Otto
 *  itself can open a folder: the Files inspector, the Git page, a terminal
 *  thread — plus copying the path for any outside editor. */
export function openOpenInMenu(anchor: Element, opts: { cwd: string; repo: Repo | null }): void {
  const items: MenuItem[] = [
    { label: 'Files', icon: 'file', hint: 'Inspector', action: () => threads.setInspectorTab('files') },
    ...(opts.repo
      ? [{ label: 'Git', icon: 'branch', action: () => router.go(`git/${encodeURIComponent(opts.repo!.id)}`) }]
      : []),
    { label: 'Terminal', icon: 'terminal', hint: 'New shell thread', action: () => void runInShell(opts.cwd, null) },
    { separator: true },
    { label: 'Copy path', icon: 'copy', action: () => void copyWithToast(opts.cwd, 'path') },
  ];
  ctxMenu.showAt(anchor, items, { align: 'end' });
}

/** Open a plain-shell thread in `cwd`; with a script, type its command in. */
export async function runInShell(cwd: string, script: ProjectScript | null): Promise<void> {
  try {
    const scratch = ws.current === null && ws.currentId !== SCRATCH_WORKSPACE_ID;
    const created = await ws.createSessionQuiet(
      {
        kind: 'agent',
        provider: 'shell',
        title: script ? script.name : `Terminal · ${projectNameFromCwd(cwd)}`,
        cwd: cwd || null,
        meta: { surface: 'threads' },
      },
      { scratch },
    );
    if (script) await ws.sendInput(created.id, script.command);
    threads.selectThread(created.id);
  } catch (e) {
    toastError(script ? `Couldn’t run ${script.name}` : 'Couldn’t open a terminal', e);
  }
}

export async function promptNewScript(project: string): Promise<void> {
  const command = await confirmer.promptText(
    'A shell command for this project, e.g. "npm run dev". Running it opens a terminal thread in the project folder.',
    {
      title: 'Add project script',
      confirmLabel: 'Add script',
      placeholder: 'npm run dev',
      validate: (v) => (v.trim() ? null : 'Enter a command'),
    },
  );
  if (!command) return;
  const cmd = command.trim();
  threads.addScript(project, { name: cmd.length > 40 ? `${cmd.slice(0, 39)}…` : cmd, command: cmd });
}

export function showScriptMenu(e: MouseEvent, project: string, script: ProjectScript): void {
  e.preventDefault();
  ctxMenu.show(e, [
    { label: 'Run', icon: 'play', action: () => void runInShell(project, script) },
    { label: 'Copy command', icon: 'copy', action: () => void copyWithToast(script.command, 'command') },
    { separator: true },
    { label: 'Remove script', icon: 'x', action: () => threads.removeScript(project, script.name) },
  ]);
}
