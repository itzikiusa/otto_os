// Git + project menus shared by the new-thread meta row and the right project
// card: branch switch, checkout (worktree) picker, "Open in …", project
// scripts. Every action routes through an existing Otto store/endpoint — the
// git store's checkout, `GET /repos/{id}/worktrees`, the Git page, the
// inspector — so nothing here owns git state.
import { api } from '../../lib/api/client';
import { gitApi } from '../../lib/api/git';
import type { Repo, WorktreeCreateReq, WorktreeCreated, WorktreeInfo, WorktreeSetupResult, WorktreeSetupSetting } from '../../lib/api/types';
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

/** `feature/very-long-name…tail` → keeps both ends readable (T3's ref list). */
export function middleEllipsis(text: string, max = 44): string {
  if (text.length <= max) return text;
  const keep = max - 1;
  const head = Math.ceil(keep / 2);
  return `${text.slice(0, head)}…${text.slice(text.length - (keep - head))}`;
}

/** Local branches of `repo`, searchable, tagged current / default / worktree;
 *  picking one runs `git checkout`. A branch another worktree holds can't be
 *  checked out here, so its row says where it is instead. */
export async function openBranchMenu(anchor: Element, repo: Repo): Promise<void> {
  const [read, trees] = await Promise.all([
    gitApi.readRefs(repo.id),
    api.get<WorktreeInfo[]>(`/repos/${encodeURIComponent(repo.id)}/worktrees`).catch(() => [] as WorktreeInfo[]),
  ]);
  if (!read.refs) {
    toasts.error('Couldn’t load branches', loadErrorText(read.error));
    return;
  }
  const home = ws.scratch?.root_path ?? null;
  const elsewhere = new Map(trees.filter((w) => !w.is_main && w.branch).map((w) => [w.branch as string, w.path]));
  const base = read.refs.base_branch ?? null;
  const local = [...read.refs.local].sort(
    (x, y) => Number(y.is_current) - Number(x.is_current) || Number(y.name === base) - Number(x.name === base),
  );
  const items: MenuItem[] = local.map((b): MenuItem => {
    const wt = !b.is_current ? elsewhere.get(b.name) : undefined;
    const tags = [b.is_current ? 'current' : null, b.name === base ? 'default' : null, wt ? 'worktree' : null]
      .filter(Boolean)
      .join(' · ');
    const sync = b.ahead || b.behind ? `↑${b.ahead ?? 0} ↓${b.behind ?? 0}` : '';
    return {
      label: middleEllipsis(b.name),
      title: wt ? `${b.name} — checked out in ${compactPath(wt, home)}` : b.name,
      hint: [tags, sync].filter(Boolean).join('  ') || undefined,
      checked: b.is_current,
      disabled: !!wt,
      action: b.is_current || wt ? undefined : () => void checkoutBranch(repo, b.name),
    };
  });
  items.push(
    { separator: true, pinned: true },
    { label: 'New branch…', icon: 'plus', pinned: true, action: () => void createBranch(repo) },
    { label: 'Open in Git', icon: 'external', pinned: true, action: () => router.go(`git/${encodeURIComponent(repo.id)}`) },
  );
  ctxMenu.showAt(anchor, items, { filter: true, filterPlaceholder: 'Search refs…', align: 'end' });
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

/** The new-thread stage's "New worktree" intent (NewThreadStage.svelte): the
 *  worktree is made when the first message is sent, so choosing it only
 *  records it — no name asked first. */
export interface NewWorktreeChoice {
  /** It is the stage's current choice. */
  pending: boolean;
  /** Chose "New worktree" (from `base`, the main checkout's branch). */
  onNew: (base: string | null) => void;
  /** "Name the branch…" (only while pending). */
  onName: () => void;
}

/** "Current checkout" vs the repo's other worktrees (T3's checkout picker);
 *  `threadsHere` counts the open threads already working in a checkout. */
export async function openCheckoutMenu(
  anchor: Element,
  repo: Repo,
  cwd: string,
  onPick: (path: string, branch: string | null) => void,
  threadsHere: ((path: string) => number) | undefined,
  newWt: NewWorktreeChoice,
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
    .map((w): MenuItem => {
      const n = threadsHere?.(w.path) ?? 0;
      return {
        label: w.is_main ? 'Current checkout' : projectNameFromCwd(w.path),
        hint: [w.branch ?? 'detached', n > 0 ? `${n} ${n === 1 ? 'thread' : 'threads'} here` : ''].filter(Boolean).join(' · '),
        title: compactPath(w.path, home),
        checked: !newWt.pending && (here === w.path.replace(/\/+$/, '') || (w.is_main && here.startsWith(`${w.path.replace(/\/+$/, '')}/`))),
        action: () => onPick(w.path, w.branch),
      };
    });
  const base = list.find((w) => w.is_main)?.branch ?? git.statusById[repo.id]?.branch ?? null;
  items.push(
    { label: 'New worktree', icon: 'plus', checked: newWt.pending, hint: base ? `from ${base}` : undefined, title: 'Made when you send: a new branch named from your message, in its own folder', action: () => newWt.onNew(base) },
    ...(newWt.pending ? [{ label: 'Name the branch…', icon: 'edit', action: newWt.onName } satisfies MenuItem] : []),
    { separator: true },
    { label: 'Manage worktrees in Git', icon: 'worktree', action: () => router.go(`git/${encodeURIComponent(repo.id)}`) },
  );
  ctxMenu.showAt(anchor, items, { align: 'start' });
}

/** The main checkout's branch: what a new worktree starts from. */
export async function worktreeBase(repo: Repo): Promise<string | null> {
  const list = await api.get<WorktreeInfo[]>(`/repos/${encodeURIComponent(repo.id)}/worktrees`).catch(() => [] as WorktreeInfo[]);
  return list.find((w) => w.is_main)?.branch ?? git.statusById[repo.id]?.branch ?? null;
}

/** Every branch name a new worktree's branch must not reuse: the local
 *  branches (`GET /repos/{id}/refs`); the repo's base branch alongside. */
export async function takenBranches(repo: Repo): Promise<{ branches: string[]; base: string | null }> {
  const read = await gitApi.readRefs(repo.id);
  if (!read.refs) throw new Error(loadErrorText(read.error));
  return { branches: read.refs.local.map((b) => b.name), base: read.refs.base_branch ?? null };
}

/** A new branch in a new worktree beside the repo (`POST /repos/{id}/worktrees`),
 *  given what the main checkout's `.worktreeinclude` names (`prepared`). */
export function createWorktree(repo: Repo, branch: string, base: string | null): Promise<WorktreeCreated> {
  return api.post<WorktreeCreated>(`/repos/${encodeURIComponent(repo.id)}/worktrees`, { branch, ...(base ? { base } : {}) } satisfies WorktreeCreateReq);
}

/** Run the repo's setup command in its new worktree at `path` (on the daemon). */
export function runWorktreeSetup(repo: Repo, path: string): Promise<WorktreeSetupResult> {
  return api.post<WorktreeSetupResult>(`/repos/${encodeURIComponent(repo.id)}/worktrees/setup`, { path });
}

/** Set (`command`) or clear (null) the command every new worktree of `repo`
 *  runs once it is made — kept on the daemon (setting `worktree_setup`), so
 *  a worktree made from the phone runs it too. */
async function saveWorktreeSetup(repo: Repo, command: string | null): Promise<void> {
  try {
    // Keyed by the main checkout: every worktree of the repository shares it.
    const list = await api.get<WorktreeInfo[]>(`/repos/${encodeURIComponent(repo.id)}/worktrees`);
    const key = list.find((w) => w.is_main)?.path ?? repo.path;
    const all = await api.get<Record<string, unknown>>('/settings');
    const next: WorktreeSetupSetting = { ...((all.worktree_setup as WorktreeSetupSetting | undefined) ?? {}) };
    const cmd = command?.trim() ?? '';
    if (cmd) next[key] = { name: cmd.length > 40 ? `${cmd.slice(0, 39)}…` : cmd, command: cmd };
    else delete next[key];
    await api.put('/settings', { worktree_setup: next });
    toasts.success(cmd ? 'New worktrees will run' : 'No worktree setup', cmd || undefined);
  } catch (e) {
    toastError('Couldn’t save the worktree setup', e);
  }
}

/** A branch name the person types (the same rule `git` applies, roughly). */
export const BRANCH_NAME_OK = (v: string): string | null =>
  /^[^\s~^:?*[\\]+$/.test(v.trim()) && !v.trim().startsWith('-') ? null : 'Enter a valid branch name';

/** Otto has no native-editor launcher, so "Open in" offers the places Otto
 *  itself can open a folder: the Files inspector, the Git page, a terminal
 *  thread — plus copying the path for any outside editor, and the folder's
 *  project scripts (run one in a terminal thread, or add one). */
export function openOpenInMenu(anchor: Element, opts: { cwd: string; repo: Repo | null }): void {
  const scripts = threads.scriptsFor(opts.cwd);
  const items: MenuItem[] = [
    { label: 'Files', icon: 'file', hint: 'Inspector', action: () => threads.setInspectorTab('files') },
    ...(opts.repo
      ? [{ label: 'Git', icon: 'branch', action: () => router.go(`git/${encodeURIComponent(opts.repo!.id)}`) }]
      : []),
    { label: 'Terminal', icon: 'terminal', hint: 'New shell thread', action: () => void runInShell(opts.cwd, null) },
    { separator: true },
    { label: 'Copy path', icon: 'copy', action: () => void copyWithToast(opts.cwd, 'path') },
    { separator: true },
    ...scripts.map((s): MenuItem => ({
      label: s.name,
      icon: 'play',
      title: `Run “${s.command}” in a terminal thread`,
      children: [
        { label: 'Run', icon: 'play', action: () => void runInShell(opts.cwd, s) },
        { label: 'Copy command', icon: 'copy', action: () => void copyWithToast(s.command, 'command') },
        { label: 'Remove script', icon: 'x', action: () => threads.removeScript(opts.cwd, s.name) },
      ],
    })),
    { label: 'Add project script…', icon: 'plus', disabled: !opts.cwd, action: () => void promptNewScript(opts.cwd) },
    // What a new worktree of this repo runs once it is made: one of these scripts or another command.
    ...(opts.repo
      ? [{
          label: 'Worktree setup',
          icon: 'branch',
          title: 'A command every new worktree of this repo runs once it is made',
          children: [
            ...scripts.map((s): MenuItem => ({ label: `Run “${s.name}”`, icon: 'play', action: () => void saveWorktreeSetup(opts.repo!, s.command) })),
            { label: 'Other command…', icon: 'plus', action: () => void promptWorktreeSetup(opts.repo!) },
            { label: 'None', icon: 'x', action: () => void saveWorktreeSetup(opts.repo!, null) },
          ],
        } satisfies MenuItem]
      : []),
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

async function promptWorktreeSetup(repo: Repo): Promise<void> {
  const command = await confirmer.promptText('Runs in every new worktree of this repo once it is made, e.g. "npm ci". Its output shows while it runs.', {
    title: 'Worktree setup',
    confirmLabel: 'Save',
    placeholder: 'npm ci',
    validate: (v) => (v.trim() ? null : 'Enter a command'),
  });
  if (command) await saveWorktreeSetup(repo, command);
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

