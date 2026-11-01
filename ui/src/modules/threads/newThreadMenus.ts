import type { Repo, Session, WorkspaceWithRole } from '../../lib/api/types';
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { INTERACTIVE_SLASH_COMMANDS, type ComposerSlashCommand } from '../../lib/slashActions';
import { compactPath, projectNameFromCwd } from './threadsHelpers';

export const NEW_THREAD_STARTERS = [
  {
    label: 'Review uncommitted changes',
    prompt:
      'Inspect git status and uncommitted diffs in this repo, highlight any bugs or edge cases, and suggest concrete fixes.',
  },
  {
    label: 'Trace a bug across the repo',
    prompt:
      'Help me trace a bug end-to-end across this codebase: locate the entry point, data flow, and root cause before proposing a minimal fix.',
  },
  {
    label: 'Write tests for recent edits',
    prompt:
      'Check the most recently modified files, identify uncovered behavior and edge cases, and write clean unit or integration tests.',
  },
  {
    label: 'Coordinate 2 sessions in split view',
    prompt:
      'Outline a clear plan splitting this task into two independent tracks so I can run a second worker thread beside you in Split view.',
  },
] as const;

const NEW_STAGE_SLASH_NAMES = new Set([
  'model',
  'effort',
  'plan',
  'mode',
  'code',
  'permissions',
  'review',
]);

export const STAGE_SLASH_COMMANDS: readonly ComposerSlashCommand[] =
  INTERACTIVE_SLASH_COMMANDS.filter((c) => NEW_STAGE_SLASH_NAMES.has(c.name));

export function buildWorkspaceMenuItems(opts: {
  workspaces: WorkspaceWithRole[];
  currentId: string | null;
  scratchActive: boolean;
  homePath: string;
  onSelectWorkspace: (w: WorkspaceWithRole) => void;
  onSelectScratch: () => void;
}): MenuItem[] {
  return [
    ...opts.workspaces.map(
      (w): MenuItem => ({
        label: w.name,
        hint: compactPath(w.root_path, opts.homePath),
        checked: !opts.scratchActive && opts.currentId === w.id,
        action: () => opts.onSelectWorkspace(w),
      }),
    ),
    { separator: true },
    {
      label: 'No workspace (Scratch)',
      hint: compactPath(opts.homePath, opts.homePath),
      checked: opts.scratchActive,
      action: opts.onSelectScratch,
    },
  ];
}

export function buildFolderMenuItems(opts: {
  scratchActive: boolean;
  currentWorkspace: WorkspaceWithRole | null;
  repos: Repo[];
  sessions: Session[];
  cwd: string;
  homePath: string;
  onPickPath: (path: string) => void;
  onBrowse: () => void;
}): MenuItem[] {
  const candidates: { label: string; path: string }[] = [];
  const seen = new Set<string>();
  const add = (label: string, path: string | null | undefined) => {
    const p = (path ?? '').trim().replace(/\/+$/, '');
    if (!p || seen.has(p)) return;
    seen.add(p);
    candidates.push({ label, path: p });
  };
  if (!opts.scratchActive && opts.currentWorkspace?.root_path) {
    add(opts.currentWorkspace.name, opts.currentWorkspace.root_path);
  }
  for (const r of opts.repos) add(r.name, r.path);
  for (const s of opts.sessions) {
    if (s.cwd) add(projectNameFromCwd(s.cwd), s.cwd);
  }
  const normalizedCwd = opts.cwd.replace(/\/+$/, '');
  return [
    ...candidates.slice(0, 10).map(
      (c): MenuItem => ({
        label: c.label,
        hint: compactPath(c.path, opts.homePath),
        checked: normalizedCwd === c.path,
        action: () => opts.onPickPath(c.path),
      }),
    ),
    ...(candidates.length > 0 ? [{ separator: true }] : []),
    {
      label: 'Browse folder…',
      icon: 'folder',
      action: opts.onBrowse,
    },
  ];
}
