// IDE shell command palette entries, JetBrains / GoLand keyboard shortcuts (#70 §3),
// quick Color Scheme menu (#70 §1–2), and overflow menu items for `IdePage.svelte`.

import type { Command } from '../../../lib/commands.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { ide, ideBasename } from '../ideStore.svelte';
import { ideAgent } from '../store/ideAgentController.svelte';

const IDE_PREFS_KEY = 'otto.ide.preferences.v1';

export type IdeQuickSchemeId =
  | 'auto'
  | 'jetbrains-islands-dark'
  | 'jetbrains-darcula'
  | 'jetbrains-islands-light'
  | 'vscode-dark-modern'
  | 'vscode-light-modern';

export const QUICK_SCHEME_ITEMS: readonly { id: IdeQuickSchemeId; label: string }[] = [
  { id: 'auto', label: 'Follow Otto (Auto)' },
  { id: 'jetbrains-islands-dark', label: 'JetBrains Islands Dark' },
  { id: 'jetbrains-darcula', label: 'JetBrains Darcula' },
  { id: 'jetbrains-islands-light', label: 'JetBrains Islands Light' },
  { id: 'vscode-dark-modern', label: 'VS Code Dark Modern' },
  { id: 'vscode-light-modern', label: 'VS Code Light Modern' },
];

export function readQuickSchemePrefs(): { colorScheme: IdeQuickSchemeId; rainbowBrackets: boolean } {
  try {
    const raw = typeof localStorage !== 'undefined' ? localStorage.getItem(IDE_PREFS_KEY) : null;
    if (!raw) return { colorScheme: 'auto', rainbowBrackets: true };
    const parsed = JSON.parse(raw) as { colorScheme?: IdeQuickSchemeId; rainbowBrackets?: boolean };
    return { colorScheme: parsed.colorScheme ?? 'auto', rainbowBrackets: parsed.rainbowBrackets ?? true };
  } catch {
    return { colorScheme: 'auto', rainbowBrackets: true };
  }
}

export function writeQuickSchemePref(patch: { colorScheme?: IdeQuickSchemeId; rainbowBrackets?: boolean }): { colorScheme: IdeQuickSchemeId; rainbowBrackets: boolean } {
  try {
    const raw = localStorage.getItem(IDE_PREFS_KEY);
    const base = raw ? (JSON.parse(raw) as Record<string, unknown>) : {};
    const next = { ...base, ...patch };
    localStorage.setItem(IDE_PREFS_KEY, JSON.stringify(next));
    window.dispatchEvent(new CustomEvent('otto:ide-prefs-changed', { detail: next }));
  } catch { /* ignore */ }
  return readQuickSchemePrefs();
}

export function showIdeSchemeMenu(
  e: MouseEvent,
  cur: { colorScheme: IdeQuickSchemeId; rainbowBrackets: boolean },
  onChange: (next: { colorScheme: IdeQuickSchemeId; rainbowBrackets: boolean }) => void,
): void {
  const items: MenuItem[] = [
    ...QUICK_SCHEME_ITEMS.map((s) => ({
      label: s.label,
      checked: cur.colorScheme === s.id,
      action: () => onChange(writeQuickSchemePref({ colorScheme: s.id })),
    })),
    { separator: true },
    {
      label: 'Rainbow brackets',
      checked: cur.rainbowBrackets,
      action: () => onChange(writeQuickSchemePref({ rainbowBrackets: !cur.rainbowBrackets })),
    },
    { label: 'Color scheme & font settings…', icon: 'gear', action: () => ide.openSettings('editor.colors') },
  ];
  ctxMenu.show(e, items);
}

export function dispatchOpenLocalHistory(): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent('otto:ide-open-local-history', { detail: { path: ide.activePath ?? ide.target.root } }));
}

export async function promptIdeGoToLine(): Promise<void> {
  if (!ide.activePath) return;
  const raw = await confirmer.promptText('Go to line[:column]:', {
    title: 'Go to Line / Column',
    placeholder: `${ide.cursorInfo.line}:${ide.cursorInfo.col}`,
    confirmLabel: 'Go',
  });
  if (!raw?.trim() || !ide.activePath) return;
  const [lStr, cStr] = raw.trim().split(':');
  ide.jumpTo(ide.activePath, Math.max(1, Number.parseInt(lStr ?? '1', 10) || 1), Math.max(1, Number.parseInt(cStr ?? '1', 10) || 1));
}

export function openIdeRecentFilesMenu(): void {
  const anchor = document.querySelector<HTMLElement>('[data-testid="ide-more-btn"]');
  if (!anchor) return;
  const items: MenuItem[] =
    ide.recentPaths.length > 0
      ? ide.recentPaths.map((p) => ({
          label: ideBasename(p),
          hint: ide.relativePath(p),
          icon: 'file',
          action: () => void ide.openFile(p, { preview: false }),
        }))
      : [{ label: 'No recent files yet', disabled: true }];
  ctxMenu.showAt(anchor, items, { align: 'end' });
}

export function buildIdePaletteCommands(opts: {
  onAttachFolder: () => void;
  onTriggerAgent: () => void;
}): Command[] {
  const openSearchAll = (): void => {
    ide.searchScopePath = null;
    ide.searchReplaceMode = false;
    ide.searchInitialTab = 'all';
    ide.searchEverywhereOpen = true;
  };
  return [
    { id: 'ide.settings', title: 'IDE: Settings & Project Structure…', group: 'IDE', shortcut: '⌘,', keywords: 'preferences goroot jdk sdk keymap theme', run: () => ide.openSettings('project.project') },
    { id: 'ide.localHistory', title: 'IDE: Show Local History', group: 'IDE', keywords: 'revisions timeline restore diff', run: () => dispatchOpenLocalHistory() },
    { id: 'ide.gitChanges', title: 'IDE: Open Git Changes', group: 'IDE', shortcut: '⌘9', keywords: 'vcs commit diff branches', run: () => ide.openBottomTab('git') },
    { id: 'ide.colorScheme', title: 'IDE: Color Scheme & Rainbow Brackets…', group: 'IDE', keywords: 'darcula islands light dark', run: () => ide.openSettings('editor.colors') },
    { id: 'ide.buildTool', title: 'IDE: Toggle Maven / Gradle Tool Window', group: 'IDE', keywords: 'pom lifecycle spring profiles', run: () => ide.toggleBuildTool() },
    { id: 'ide.bulkRepos', title: 'IDE: Add Multiple Repositories…', group: 'IDE', keywords: 'multi-repo microservices folders', run: () => (ide.bulkRepoPickerMode = 'attach') },
    { id: 'ide.searchEverywhere', title: 'Search everywhere…', group: 'IDE', shortcut: '⇧⇧', keywords: 'find symbol file action replace', run: openSearchAll },
    { id: 'ide.findInPath', title: 'Find in path…', group: 'IDE', shortcut: '⌘⇧F', run: () => ide.openFindInPath({ replace: false }) },
    { id: 'ide.replaceInPath', title: 'Replace in path…', group: 'IDE', shortcut: '⌘⇧R', run: () => ide.openFindInPath({ replace: true }) },
    { id: 'ide.run', title: 'Run configuration', group: 'IDE', shortcut: '⌃R', run: () => void ide.startRun() },
    { id: 'ide.debug', title: 'Debug configuration', group: 'IDE', shortcut: '⌃D', run: () => void ide.startDebug() },
    { id: 'ide.editConfigs', title: 'Edit run / debug configurations…', group: 'IDE', run: () => ide.openEnvironmentsModal('configs') },
    { id: 'ide.addFolder', title: 'Add folder to project…', group: 'IDE', run: opts.onAttachFolder },
    { id: 'ide.revealFile', title: 'Select opened file in project tree', group: 'IDE', run: () => void ide.revealInProjectTree() },
    { id: 'ide.toggleAgent', title: 'Toggle AI Agent', group: 'IDE', shortcut: '⌘L', keywords: 'ai assistant chat context', run: opts.onTriggerAgent },
    { id: 'ide.inlineEdit', title: 'Inline AI edit…', group: 'IDE', shortcut: '⌘I', keywords: 'ai refactor rewrite selection', run: () => ideAgent.openInlineEdit() },
    { id: 'ide.toolchains', title: 'Project SDKs & Toolchains…', group: 'IDE', run: () => ide.openSettings('project.sdks') },
    { id: 'ide.environments', title: 'Shell environment & variables…', group: 'IDE', run: () => ide.openSettings('build.shell') },
    { id: 'ide.ssh', title: 'Connect remote SSH project…', group: 'IDE', run: () => ide.openSettings('build.ssh') },
    { id: 'ide.share', title: 'Share live IDE session…', group: 'IDE', run: () => (ide.shareOpen = true) },
    { id: 'ide.plugins', title: 'Manage IDE plugins & language servers…', group: 'IDE', run: () => ide.openSettings('plugins') },
  ];
}

export function showIdeMoreMenu(
  e: MouseEvent,
  opts: {
    explorerVisible: boolean;
    onToggleExplorer: () => void;
    onTriggerAgent: () => void;
    onAttachFolder: () => void;
  },
): void {
  const btName = ide.buildToolKind === 'gradle' ? 'Gradle' : 'Maven';
  const items: MenuItem[] = [
    { label: 'Settings & Project Structure…', icon: 'gear', hint: '⌘,', action: () => ide.openSettings('project.project') },
    { label: 'Project SDKs & Toolchains…', icon: 'box', action: () => ide.openSettings('project.sdks') },
    { label: 'Local History…', icon: 'clock', disabled: !ide.hasTarget, action: () => dispatchOpenLocalHistory() },
    { label: 'Git Changes', icon: 'branch', hint: '⌘9', disabled: !ide.hasTarget, action: () => ide.openBottomTab('git') },
    { label: 'Remote SSH…', icon: 'server', action: () => ide.openSettings('build.ssh') },
    { separator: true },
    {
      label: 'Search everywhere…',
      icon: 'search',
      hint: '⇧⇧',
      action: () => { ide.searchScopePath = null; ide.searchReplaceMode = false; ide.searchInitialTab = 'all'; ide.searchEverywhereOpen = true; },
    },
    { label: 'Find in path…', icon: 'search', hint: '⌘⇧F', disabled: !ide.hasTarget, action: () => ide.openFindInPath({ replace: false }) },
    { label: 'Multi-line edit (prefix / suffix)', icon: 'edit', checked: ide.prefixSuffixOpen, disabled: !ide.activeTab, action: () => (ide.prefixSuffixOpen = !ide.prefixSuffixOpen) },
    { label: 'Format on save', checked: ide.formatOnSave, action: () => ide.setFormatOnSave(!ide.formatOnSave) },
    { separator: true },
    { label: opts.explorerVisible ? 'Hide project explorer' : 'Show project explorer', icon: 'sidebar', hint: '⌘1', action: opts.onToggleExplorer },
    ...(ide.buildToolKind
      ? [{ label: ide.buildToolOpen ? `Hide ${btName} tool window` : `Show ${btName} tool window`, icon: 'layers' as const, action: () => ide.toggleBuildTool() }]
      : []),
    { label: ide.bottomDockOpen ? 'Hide bottom tool dock' : 'Show bottom tool dock', icon: 'terminal', action: () => ide.toggleBottomDock() },
    { label: ideAgent.panelOpen ? 'Hide AI Agent' : 'Show AI Agent', icon: 'assistant', hint: '⌘L', action: opts.onTriggerAgent },
    { separator: true },
    { label: 'Add folder to project…', icon: 'folder', disabled: !ide.hasTarget || ide.target.kind !== 'local', action: opts.onAttachFolder },
    { label: 'Add multiple repositories…', icon: 'layers', disabled: ide.target.kind !== 'local' && ide.hasTarget, action: () => (ide.bulkRepoPickerMode = ide.hasTarget ? 'attach' : 'create') },
    { label: 'Run configurations…', icon: 'play', action: () => ide.openEnvironmentsModal('configs') },
    { label: 'Share live session…', icon: 'share', action: () => (ide.shareOpen = true) },
    { label: 'Plugins & Language Servers…', icon: 'plug', action: () => ide.openSettings('plugins') },
  ];
  ctxMenu.show(e, items);
}

const inXterm = (t: EventTarget | null): boolean => t instanceof Element && Boolean(t.closest('.xterm'));

export function handleIdeChord(e: KeyboardEvent, overlayOpen: boolean, toggleExplorer: () => void): boolean {
  if (overlayOpen) return false;
  const mod = e.metaKey || e.ctrlKey;
  if (mod && !e.shiftKey && !e.altKey && e.code === 'Comma') { e.preventDefault(); ide.openSettings(); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'KeyS') { e.preventDefault(); void ide.saveFile(); return true; }
  if (mod && !e.shiftKey && e.altKey && e.code === 'KeyS') { e.preventDefault(); void ide.saveAll(); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'KeyW' && ide.activePath) { e.preventDefault(); void ide.closeTab(ide.activePath); return true; }
  if (mod && e.shiftKey && !e.altKey && e.code === 'KeyT') { e.preventDefault(); void ide.reopenClosedTab(); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'KeyE') { e.preventDefault(); openIdeRecentFilesMenu(); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'KeyG') { e.preventDefault(); void promptIdeGoToLine(); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'Digit1') { e.preventDefault(); toggleExplorer(); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'Digit4') { e.preventDefault(); ide.openBottomTab('run'); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'Digit5') { e.preventDefault(); ide.openBottomTab('debug'); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'Digit6') { e.preventDefault(); ide.openBottomTab('problems'); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'Digit9') { e.preventDefault(); ide.openBottomTab('git'); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'BracketLeft' && ide.canNavigateBack) { e.preventDefault(); void ide.navigateBack(); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'BracketRight' && ide.canNavigateForward) { e.preventDefault(); void ide.navigateForward(); return true; }
  if (mod && e.shiftKey && !e.altKey && e.code === 'KeyF') { e.preventDefault(); ide.openFindInPath({ replace: false }); return true; }
  if (mod && e.shiftKey && !e.altKey && e.code === 'KeyR') { e.preventDefault(); ide.openFindInPath({ replace: true }); return true; }
  if (mod && e.shiftKey && !e.altKey && e.code === 'KeyO') {
    e.preventDefault(); ide.searchScopePath = null; ide.searchReplaceMode = false; ide.searchInitialTab = 'files'; ide.searchEverywhereOpen = true; return true;
  }
  if (mod && !e.shiftKey && !e.altKey && (e.code === 'KeyO' || e.key === 'F12')) {
    e.preventDefault(); ide.searchScopePath = null; ide.searchReplaceMode = false; ide.searchInitialTab = 'symbols'; ide.searchEverywhereOpen = true; return true;
  }
  if (mod && !e.shiftKey && !e.altKey && e.code === 'KeyI' && !(e.ctrlKey && !e.metaKey && inXterm(e.target))) {
    e.preventDefault(); ideAgent.openInlineEdit(); return true;
  }
  if (mod && e.altKey && !e.shiftKey && e.code === 'KeyL') { e.preventDefault(); void ide.formatActiveFile(); return true; }
  if (mod && e.altKey && !e.shiftKey && e.code === 'KeyR' && ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.resumeDebug(); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.key === 'F8' && ide.activePath) { e.preventDefault(); ide.toggleBreakpoint(ide.activePath, ide.cursorInfo.line); return true; }
  if (mod && !e.shiftKey && !e.altKey && e.key === 'F2') { e.preventDefault(); void ide.stopActiveExecution(); return true; }
  return false;
}

export function handleIdeWindowKeyDown(e: KeyboardEvent, overlayOpen: boolean, triggerAgent: () => void): void {
  if (overlayOpen) return;
  if (!e.defaultPrevented && (e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && !(e.ctrlKey && !e.metaKey && inXterm(e.target))) {
    if (e.code === 'KeyL') { e.preventDefault(); triggerAgent(); return; }
    if (e.code === 'KeyI') { e.preventDefault(); ideAgent.openInlineEdit(); return; }
  }
  if (e.altKey && !e.metaKey && !e.ctrlKey && !e.shiftKey && e.key === 'F12') { e.preventDefault(); ide.openBottomTab('terminal'); }
  else if (e.altKey && !e.metaKey && !e.ctrlKey && !e.shiftKey && e.key === 'F7' && ide.activePath) { e.preventDefault(); void ide.findReferences(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col); }
  else if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && e.code === 'KeyR') { e.preventDefault(); void ide.startRun(); }
  else if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && e.code === 'KeyD') { e.preventDefault(); void ide.startDebug(); }
  else if (e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && e.key === 'F6' && ide.activePath) { e.preventDefault(); void ide.openRenameModal(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col); }
  else if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === 'F8' && ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.stepOver(); }
  else if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === 'F7' && ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.stepInto(); }
  else if (e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && e.key === 'F8' && ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.stepOut(); }
  else if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === 'F9' && ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.resumeDebug(); }
  else if (e.altKey && !e.metaKey && !e.ctrlKey && !e.shiftKey && e.key === 'F9' && ide.activeDebug?.state === 'paused') { e.preventDefault(); void ide.runToCursor(); }
}
