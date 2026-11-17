import type { Repo, Session, WorkspaceWithRole } from '../../lib/api/types';
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { INTERACTIVE_SLASH_COMMANDS, type ComposerSlashCommand } from '../../lib/slashActions';
import { canonicalPath, folderCandidates } from './newThreadDefaults';
import { compactPath } from './threadsHelpers';

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
  /** Recent thread folders not in `sessions` (structured threads). */
  recentCwds?: readonly string[];
}): MenuItem[] {
  const names = {
    homePath: opts.homePath,
    repos: opts.repos,
    workspaces: opts.currentWorkspace ? [opts.currentWorkspace] : [],
  };
  // The repo's own name beats the workspace's for the same folder.
  const entries = [
    ...opts.repos.map((r) => ({ path: r.path })),
    ...(!opts.scratchActive && opts.currentWorkspace?.root_path ? [{ path: opts.currentWorkspace.root_path }] : []),
    ...(opts.recentCwds ?? []).map((path) => ({ path })),
    ...opts.sessions.map((s) => ({ path: s.cwd })),
    // Home is always offered, by name — never a silent default.
    { path: opts.homePath },
  ];
  const here = canonicalPath(opts.cwd);
  const rows = folderCandidates(entries, names);
  return [
    ...rows.map(
      (c): MenuItem => ({
        label: c.label,
        hint: c.hint,
        title: c.path,
        checked: here === canonicalPath(c.path),
        action: () => opts.onPickPath(c.path),
      }),
    ),
    { separator: true },
    {
      label: 'Browse folder…',
      icon: 'folder',
      action: opts.onBrowse,
    },
  ];
}
