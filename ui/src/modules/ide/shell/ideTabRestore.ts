// Saved tab-strip restoration and background prefetch helpers extracted from `ideProjectHelpers.ts`.

import {
  autoOpenInitialEntry,
  ideTargetKey,
  type IdeFsEntry,
  type IdeOpenTab,
  type IdeTargetSpec,
} from '../store/ideTypes';
import { ideTabRestore, loadWorkspaceOpenTabs, type IdeOpenFileOpts } from '../store/ideBufferPerf';

export { ideTabRestore }; // the stacked layout keeps the pane the person picked while a restore opens tabs

type TabRestoreHost = {
  target: IdeTargetSpec; tabs: IdeOpenTab[]; activePath: string | null;
  openFile(rawPath: string, opts?: IdeOpenFileOpts): Promise<void>; openInSplit?(path: string, dir?: 'right' | 'down'): void;
};

export async function restoreWorkspaceTabsOrInitial(host: TabRestoreHost & {
  collabIsGuest: boolean; hasClosedAllTabs: boolean;
  treeRootEntries: IdeFsEntry[]; childrenByDir: Record<string, IdeFsEntry[]>;
  toggleDir(dirPath: string): Promise<void>;
}): Promise<void> {
  if (host.collabIsGuest || host.tabs.length > 0 || host.hasClosedAllTabs) return;
  if (typeof window !== 'undefined' && /[?&](file|collab)=/.test(window.location.hash)) return;
  ideTabRestore.active = true;
  try {
    await restoreTabs(host);
  } finally {
    // After the effects that react to these opens have run.
    await new Promise((r) => setTimeout(r, 0));
    ideTabRestore.active = false;
  }
}

async function restoreTabs(host: Parameters<typeof restoreWorkspaceTabsOrInitial>[0]): Promise<void> {
  const saved = loadWorkspaceOpenTabs(host.target.root);
  if (saved) {
    const key = ideTargetKey(host.target);
    await reopenSavedTabs(host, saved.paths, saved);
    if (ideTargetKey(host.target) !== key) return; // switched meanwhile: the rest belongs to the old project
    // Every saved file is gone (all reads failed): start as a fresh project instead.
    void prefetchSavedTabs(host).then(async () => {
      if (ideTargetKey(host.target) !== key || host.tabs.some((t) => !t.error)) return;
      host.tabs = [];
      await autoOpenInitialEntry(host);
    });
    return;
  }
  await autoOpenInitialEntry(host);
}

/** Re-open a saved tab strip: every tab shows at once in its saved order, unread; the split
 *  starts its read, and this resolves once the active tab is read (interactive lane). The
 *  hidden tabs wait for {@link prefetchSavedTabs} or their activation. */
export async function reopenSavedTabs(
  host: TabRestoreHost,
  paths: readonly string[],
  o: { activePath?: string | null; splitPath?: string | null; projectScoped?: readonly string[]; pinned?: readonly string[] } = {},
): Promise<void> {
  if (paths.length === 0) return;
  for (const p of paths) void host.openFile(p, { preview: false, background: true, defer: true, projectScoped: o.projectScoped?.includes(p) });
  if (o.pinned) for (const t of host.tabs) if (o.pinned.includes(t.path)) t.pinned = true;
  const open = (p?: string | null): p is string => !!p && host.tabs.some((t) => t.path === p);
  if (open(o.splitPath)) host.openInSplit?.(o.splitPath, 'right');
  await host.openFile(open(o.activePath) ? o.activePath : paths[0], { preview: false });
}

/** Read the tabs a restore left unread, two at a time on the background lane (the active
 *  tab's own extras queue there too), until the project changes. */
export async function prefetchSavedTabs(host: TabRestoreHost): Promise<void> {
  const key = ideTargetKey(host.target);
  const next = (): string | undefined => (ideTargetKey(host.target) === key ? host.tabs.find((t) => t.deferred)?.path : undefined);
  const worker = async (): Promise<void> => {
    for (let p = next(); p; p = next()) await host.openFile(p, { preview: false, background: true, lane: 'bg' });
  };
  await Promise.all([worker(), worker()]);
}
