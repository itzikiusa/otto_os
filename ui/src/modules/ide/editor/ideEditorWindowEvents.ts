// Window custom-event subscriptions for `IdeEditorArea.svelte` (preferences,
// clipboard history, quick-fix application, Go build tags, LSP reconnect/diagnostics,
// git reloads, repo test actions, and file refactor prompts).

import type { IdeDiagnostic } from '../../../lib/api/types';
import type * as CmIdeMod from '../../../lib/components/cm-ide-extensions';
import { IDE_PREFS_EVENT } from '../../../lib/components/cm-ide-prefs';
import { ide, type IdeOpenTab } from '../ideStore.svelte';
import { extractSafeDeleteSymbol, formatSafeDeleteMessage } from './ideEditorHelpers';
import { putIdeLocalHistoryLabel } from './ideEditorStoreBridge.svelte';

export interface IdeEditorWindowEventsHost {
  onPrefsChanged: () => void;
  onOpenClipboardHistory: () => void;
  getPrimaryPath: () => string | null;
  getSplitPath: () => string | null;
  applyQuickFixEdits: (pane: 'primary' | 'split', edits: CmIdeMod.IdeConcreteTextEdit[]) => void;
  reconnectLspClients: (rootPath?: string) => void;
  refreshGitBase: (tab: IdeOpenTab) => void;
}

export function attachIdeEditorWindowEvents(host: IdeEditorWindowEventsHost): () => void {
  if (typeof window === 'undefined') return () => {};

  const onPrefs = (): void => host.onPrefsChanged();
  const onPasteHist = (): void => host.onOpenClipboardHistory();
  const onPutLabel = (e: Event): void => {
    const p = (e as CustomEvent<{ path?: string }>).detail?.path ?? ide.activePath;
    const t = ide.tabs.find((x) => x.path === p);
    if (t) void putIdeLocalHistoryLabel(t.path, t.content);
  };
  const onApplyFixes = (e: Event): void => {
    const det = (e as CustomEvent<{ path?: string; edits?: CmIdeMod.IdeConcreteTextEdit[] }>).detail;
    if (!det?.edits?.length) return;
    if (!det.path || det.path === host.getPrimaryPath()) host.applyQuickFixEdits('primary', det.edits);
    else if (det.path === host.getSplitPath()) host.applyQuickFixEdits('split', det.edits);
  };
  const onEnableGoTag = (e: Event): void => {
    const det = (e as CustomEvent<{ tags?: string[]; filePath?: string }>).detail;
    const tags = det?.tags ?? [];
    if (tags.length === 0) return;
    const targetFile = det?.filePath || ide.activePath || ide.target.root;
    const merged = [...ide.getRepoGoBuildTags(targetFile), ...tags];
    void ide.setRepoGoBuildTags(targetFile, merged);
  };
  const onReconnectLsp = (e: Event): void => {
    const rootPath = (e as CustomEvent<{ rootPath?: string }>).detail?.rootPath;
    host.reconnectLspClients(rootPath);
  };
  const onLspDiags = (e: Event): void => {
    const d = (e as CustomEvent<{ path: string; diagnostics: IdeDiagnostic[] }>).detail;
    ide.setDiagnostics(d.path, d.diagnostics);
  };
  const onGitReloaded = (e: Event): void => {
    const reloadedPath = (e as CustomEvent<{ path?: string }>).detail?.path;
    const tab = reloadedPath ? ide.tabs.find((t) => t.path === reloadedPath) : ide.activeTab;
    if (tab) host.refreshGitBase(tab);
  };
  const onFindInFiles = (e: Event): void => {
    const scopePath = (e as CustomEvent<{ path?: string }>).detail?.path;
    const dir = scopePath && scopePath.includes('/') ? scopePath.slice(0, scopePath.lastIndexOf('/')) : null;
    ide.openFindInPath({ scopePath: dir });
  };
  const onRepoTestAction = (e: Event): void => {
    const det = (e as CustomEvent<{ path?: string; mode?: 'run' | 'coverage' | 'debug' }>).detail;
    void ide.runRepoTests(det?.path ?? ide.activePath, det?.mode ?? 'run');
  };
  const onFileRefactor = (e: Event): void => {
    const det = (e as CustomEvent<{ op?: 'move' | 'copy' | 'safe-delete'; path?: string }>).detail;
    const targetPath = det?.path || ide.activePath;
    if (!targetPath || !det?.op) return;
    void import('../../../lib/confirm.svelte').then(async ({ confirmer }) => {
      if (det.op === 'move' || det.op === 'copy') {
        const isCopy = det.op === 'copy';
        const initial = isCopy ? targetPath.replace(/(\.[^./]+)$/, '_copy$1') : targetPath;
        const dest = await confirmer.promptText(
          isCopy ? `Create a copy of ${ide.relativePath(targetPath)} at:` : `Move or rename ${ide.relativePath(targetPath)} to:`,
          { title: isCopy ? 'Copy file' : 'Move file', confirmLabel: isCopy ? 'Copy' : 'Move', initial },
        );
        if (dest?.trim() && ide.resolvePath(dest.trim()) !== targetPath) {
          await ide.mutateFs(isCopy ? 'duplicate' : 'rename', targetPath, dest.trim());
        }
      } else if (det.op === 'safe-delete') {
        const baseSym = extractSafeDeleteSymbol(targetPath);
        let checked = false;
        if (baseSym) {
          ide.usages = [];
          ide.usagesSymbol = '';
          await ide.findReferences(targetPath, 1, 1, baseSym);
          checked = true;
        }
        const externalRefs = checked ? ide.usages.filter((u) => u.path !== targetPath).length : 0;
        const warn = formatSafeDeleteMessage({ relPath: ide.relativePath(targetPath), symbol: baseSym, checked, externalRefs });
        const ok = await confirmer.ask(warn, { title: 'Safe delete file', confirmLabel: 'Delete', danger: true });
        if (ok) await ide.mutateFs('delete', targetPath);
      }
    });
  };

  window.addEventListener(IDE_PREFS_EVENT, onPrefs);
  window.addEventListener('otto:ide-paste-history', onPasteHist);
  window.addEventListener('otto:ide-put-history-label', onPutLabel);
  window.addEventListener('otto:ide-apply-quickfix-edits', onApplyFixes);
  window.addEventListener('otto:ide-enable-go-build-tag', onEnableGoTag);
  window.addEventListener('otto:ide-reconnect-lsp', onReconnectLsp);
  window.addEventListener('otto:ide-lsp-diagnostics', onLspDiags);
  window.addEventListener('otto:ide-git-file-reloaded', onGitReloaded);
  window.addEventListener('otto:ide-open-find-in-files', onFindInFiles);
  window.addEventListener('otto:ide-repo-test-action', onRepoTestAction);
  window.addEventListener('otto:ide-editor-file-refactor', onFileRefactor);

  return () => {
    window.removeEventListener(IDE_PREFS_EVENT, onPrefs);
    window.removeEventListener('otto:ide-paste-history', onPasteHist);
    window.removeEventListener('otto:ide-put-history-label', onPutLabel);
    window.removeEventListener('otto:ide-apply-quickfix-edits', onApplyFixes);
    window.removeEventListener('otto:ide-enable-go-build-tag', onEnableGoTag);
    window.removeEventListener('otto:ide-reconnect-lsp', onReconnectLsp);
    window.removeEventListener('otto:ide-lsp-diagnostics', onLspDiags);
    window.removeEventListener('otto:ide-git-file-reloaded', onGitReloaded);
    window.removeEventListener('otto:ide-open-find-in-files', onFindInFiles);
    window.removeEventListener('otto:ide-repo-test-action', onRepoTestAction);
    window.removeEventListener('otto:ide-editor-file-refactor', onFileRefactor);
  };
}
