// Context menu builders and file/folder CRUD prompts for `IdeProjectTree.svelte`.

import { copyWithToast } from '../../../lib/clipboard';
import { confirmer } from '../../../lib/confirm.svelte';
import { type MenuItem } from '../../../lib/contextmenu.svelte';
import {
  buildGitSubmenuItems,
  captureMenuAnchor,
  showIdeMenu,
  type IdeMenuAnchor,
} from '../git/ideGitQuickMenu';
import { ide, ideBasename, type IdeFsEntry } from '../ideStore.svelte';
import { ideAgent } from '../store/ideAgentController.svelte';
import { resolveRepoRootForPath } from '../store/ideRepoTestAndBuildTags';
import { promptPutLocalHistoryLabel, type IdeCompareMode } from './ideComparisonLoader';
import { buildManifestMenuItems } from './ideManifestActions';

export interface FlatTreeNode {
  entry: IdeFsEntry;
  depth: number;
  isProjectRoot?: boolean;
  isPrimaryRoot?: boolean;
  defaultExpanded?: boolean;
  subtitle?: string;
  /** A folder's failed listing, rendered as an inline row with Retry under it. */
  loadError?: string;
  /** For a `loadError` row: the folder to retry. */
  errorDir?: string;
}

export function parentDirOfPath(fullPath: string): string {
  const idx = fullPath.lastIndexOf('/');
  return idx > 0 ? fullPath.slice(0, idx) : ide.target.root;
}

export function rootForProjectPath(targetPath: string): string {
  const clean = targetPath.replace(/\/+$/, '') || '/';
  let best = '';
  for (let i = 0; i < ide.projectRoots.length; i++) {
    const rp = ide.projectRoots[i].path.replace(/\/+$/, '') || '/';
    if ((clean === rp || clean.startsWith(`${rp}/`)) && rp.length > best.length) best = rp;
  }
  return best || (ide.target.root.replace(/\/+$/, '') || '/');
}

export async function promptCreateTreeEntry(
  baseDir: string,
  op: 'create_file' | 'mkdir',
  extHint = '',
): Promise<void> {
  const isDir = op === 'mkdir';
  const relLabel = ide.relativePath(baseDir);
  const displayDir = relLabel === baseDir ? ideBasename(baseDir) : relLabel || '.';
  const name = await confirmer.promptText(
    isDir ? `Folder name inside ${displayDir}` : `File name inside ${displayDir}`,
    {
      title: isDir ? 'New folder' : extHint === '.go' ? 'New Go file' : 'New file',
      confirmLabel: 'Create',
      placeholder: isDir ? 'pkg/handlers' : extHint === '.go' ? 'service.go' : 'handler.go',
      validate: (v) => (v.includes('..') ? 'Relative parent segments (..) aren’t allowed.' : null),
    },
  );
  if (!name) return;
  const cleanName = extHint && !name.includes('.') ? `${name}${extHint}` : name;
  const targetPath = `${baseDir.replace(/\/+$/, '')}/${cleanName.replace(/^\/+/, '')}`;
  if (isDir) await ide.mutateFs('mkdir', targetPath);
  else await ide.createFileWithTemplate(targetPath);
}

export async function promptRenameTreeEntry(entry: IdeFsEntry): Promise<void> {
  const nextName = await confirmer.promptText(`New name for “${entry.name}”`, {
    title: 'Rename',
    confirmLabel: 'Rename',
    initial: entry.name,
    validate: (v) =>
      v.includes('/') || v.includes('..') ? 'Enter a single file or folder name.' : null,
  });
  if (!nextName || nextName === entry.name) return;
  await ide.mutateFs(
    'rename',
    entry.path,
    `${parentDirOfPath(entry.path).replace(/\/+$/, '')}/${nextName}`,
  );
}

/** The delete prompt. Open tabs under the deleted path that hold unsaved edits are
 *  dropped with it, so the prompt names them instead of losing them silently. */
export function deleteConfirmText(entryName: string, where: string, unsavedNames: readonly string[]): string {
  const base = `Permanently delete “${entryName}” from ${where}?`;
  if (unsavedNames.length === 0) return base;
  const shown = unsavedNames.slice(0, 3).map((n) => `“${n}”`).join(', ');
  const more = unsavedNames.length > 3 ? ` and ${unsavedNames.length - 3} more` : '';
  const verb = unsavedNames.length === 1 ? 'has' : 'have';
  return `${base} ${shown}${more} ${verb} unsaved changes that will be lost.`;
}

export async function confirmDeleteTreeEntry(entry: IdeFsEntry): Promise<void> {
  const unsaved = ide.tabs
    .filter((t) => (t.path === entry.path || t.path.startsWith(`${entry.path.replace(/\/+$/, '')}/`)) && ide.getBuffer(t.path) !== t.savedContent)
    .map((t) => t.name);
  const ok = await confirmer.ask(
    deleteConfirmText(entry.name, ide.target.kind === 'ssh' ? 'the remote host' : 'disk', unsaved),
    {
      title: `Delete ${entry.kind === 'dir' ? 'folder' : 'file'}`,
      confirmLabel: 'Delete',
      danger: true,
    },
  );
  if (ok) await ide.mutateFs('delete', entry.path);
}

export async function confirmDetachProjectRoot(rootPath: string, rootName: string): Promise<void> {
  const ok = await confirmer.ask(
    `Detach “${rootName}” from this project? No files on disk will be deleted.`,
    { title: 'Remove folder from project?', confirmLabel: 'Remove folder', danger: true },
  );
  if (ok) await ide.removeProjectFolder(rootPath);
}

export interface TreeMenuCallbacks {
  openCompare: (opts: {
    mode: IdeCompareMode;
    path?: string;
    isDir?: boolean;
    filter?: 'all' | 'deleted';
  }) => void;
  onOpenFolderPicker: () => void;
  onManifestOutput: (title: string, output: string) => void;
}

export function showTreeNodeContextMenu(
  anchor: IdeMenuAnchor,
  node: FlatTreeNode,
  cb: TreeMenuCallbacks,
): void {
  const savedAnchor = captureMenuAnchor(anchor);
  const entry = node.entry;
  const isDir = entry.kind === 'dir';
  const baseDir = isDir ? entry.path : parentDirOfPath(entry.path);
  const reopenSelf = (): void => showTreeNodeContextMenu(savedAnchor, node, cb);

  const showHistorySub = (): void => {
    const sub: MenuItem[] = [
      { label: 'Back', icon: 'chevronLeft', pinned: true, action: reopenSelf },
      {
        label: 'Show History…',
        icon: 'clock',
        action: () => cb.openCompare({ mode: 'local-history', path: entry.path, isDir, filter: 'all' }),
      },
      {
        label: 'Put Label…',
        icon: 'tag',
        action: () => void promptPutLocalHistoryLabel(rootForProjectPath(entry.path), entry.path, { isDir }),
      },
    ];
    if (isDir) {
      sub.push({
        label: 'Restore Deleted Files…',
        icon: 'undo',
        action: () =>
          cb.openCompare({ mode: 'local-history', path: entry.path, isDir: true, filter: 'deleted' }),
      });
    }
    showIdeMenu(savedAnchor, sub);
  };

  const items: MenuItem[] = [];
  if (isDir) {
    const repo = resolveRepoRootForPath(ide.target.root, ide.projectRoots, entry.path);
    items.push(
      {
        label: `Run All Tests (${repo.name})`,
        icon: 'play',
        action: () => void ide.runRepoTests(repo.path, 'run'),
      },
      {
        label: `Run All Tests with Coverage (${repo.name})`,
        icon: 'shield',
        action: () => void ide.runRepoTests(repo.path, 'coverage'),
      },
      {
        label: `Debug All Tests (${repo.name})`,
        icon: 'bug',
        action: () => void ide.runRepoTests(repo.path, 'debug'),
      },
      { separator: true },
    );
  }
  if (!isDir) {
    items.push(
      { label: 'Open', icon: 'file', action: () => void ide.openFile(entry.path, { preview: false }) },
      {
        label: 'Split right',
        icon: 'split',
        action: () => void ide.openFile(entry.path, { preview: false, inSplit: true }),
      },
      {
        label: 'Add to Agent context',
        icon: 'assistant',
        action: () => void ideAgent.attachFile(entry.path, { openPanel: true }),
      },
      ...buildManifestMenuItems(entry.path, entry.name, cb.onManifestOutput),
      { separator: true },
    );
  }
  items.push(
    { label: 'New file…', icon: 'plus', action: () => void promptCreateTreeEntry(baseDir, 'create_file') },
    {
      label: 'New Go file…',
      icon: 'file',
      action: () => void promptCreateTreeEntry(baseDir, 'create_file', '.go'),
    },
    { label: 'New folder…', icon: 'folder', action: () => void promptCreateTreeEntry(baseDir, 'mkdir') },
  );
  if (ide.target.kind === 'local') {
    items.push(
      { label: 'Add folder to project…', icon: 'worktree', action: cb.onOpenFolderPicker },
      { separator: true },
      {
        label: 'Git',
        icon: 'branch',
        hint: '▸',
        action: () =>
          showIdeMenu(
            savedAnchor,
            buildGitSubmenuItems({
              absPath: entry.path,
              isDir,
              anchor: savedAnchor,
              openCompare: (mode, p) => cb.openCompare({ mode, path: p }),
              onBack: reopenSelf,
            }),
          ),
      },
      { label: 'Local History', icon: 'clock', hint: '▸', action: showHistorySub },
    );
    if (!isDir) {
      items.push({
        label: 'Compare with Clipboard',
        icon: 'columns',
        action: () => cb.openCompare({ mode: 'clipboard', path: entry.path }),
      });
    } else {
      items.push({
        label: 'Restore Deleted Files…',
        icon: 'undo',
        action: () =>
          cb.openCompare({ mode: 'local-history', path: entry.path, isDir: true, filter: 'deleted' }),
      });
    }
  }
  items.push(
    { separator: true },
    {
      label: 'Find in folder…',
      icon: 'search',
      hint: '⌘⇧F',
      action: () => ide.openFindInPath({ scopePath: baseDir, replace: false }),
    },
    {
      label: 'Replace in folder…',
      icon: 'edit',
      hint: '⌘⇧R',
      action: () => ide.openFindInPath({ scopePath: baseDir, replace: true }),
    },
    { label: 'Open in terminal', icon: 'terminal', action: () => ide.openTerminalAt(baseDir) },
    { separator: true },
    { label: 'Copy path', icon: 'copy', action: () => void copyWithToast(entry.path, 'Copied path') },
    {
      label: 'Copy relative path',
      icon: 'link',
      action: () => void copyWithToast(ide.relativePath(entry.path), 'Copied relative path'),
    },
  );

  if (node.isProjectRoot) {
    items.push(
      { separator: true },
      {
        label: ide.showHidden ? 'Hide dotfiles' : 'Show dotfiles',
        icon: ide.showHidden ? 'eyeOff' : 'eye',
        action: () => {
          ide.showHidden = !ide.showHidden;
          void ide.loadTree();
        },
      },
      { label: 'Refresh project tree', icon: 'refresh', action: () => void ide.loadTree() },
    );
    if (!node.isPrimaryRoot) {
      items.push(
        { separator: true },
        {
          label: 'Remove folder from project…',
          icon: 'trash',
          danger: true,
          action: () => void confirmDetachProjectRoot(entry.path, entry.name),
        },
      );
    }
  } else {
    items.push(
      { separator: true },
      { label: 'Rename…', icon: 'edit', hint: '⇧F6', action: () => void promptRenameTreeEntry(entry) },
    );
    if (!isDir) {
      items.push({
        label: 'Duplicate',
        icon: 'copy',
        action: () => void ide.mutateFs('duplicate', entry.path),
      });
    }
    items.push(
      { separator: true },
      {
        label: 'Delete…',
        icon: 'trash',
        hint: '⌘⌫',
        danger: true,
        action: () => void confirmDeleteTreeEntry(entry),
      },
    );
  }
  showIdeMenu(anchor, items);
}
