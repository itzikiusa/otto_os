// Lazy-loaded command palette registration, Color Scheme picker menu, and overflow
// menu for `IdePage.svelte` (keeps `page:ide` first-paint bundle lean).

import type { Command } from '../../../lib/commands.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { ide } from '../ideStore.svelte';
import { ideAgent } from '../store/ideAgentController.svelte';
import {
  dispatchOpenLocalHistory,
  type IdeQuickSchemeId,
  type QuickSchemePrefs,
  writeQuickSchemePref,
} from './ideShellCommands';

export const QUICK_SCHEME_ITEMS: readonly { id: IdeQuickSchemeId; label: string }[] = [
  { id: 'auto', label: 'Follow Otto (Auto)' },
  { id: 'jetbrains-islands-dark', label: 'JetBrains Islands Dark' },
  { id: 'jetbrains-darcula', label: 'JetBrains Darcula' },
  { id: 'jetbrains-islands-light', label: 'JetBrains Islands Light' },
  { id: 'vscode-dark-modern', label: 'VS Code Dark Modern' },
  { id: 'vscode-light-modern', label: 'VS Code Light Modern' },
];

function openAnchoredMenu(anchor: HTMLElement | MouseEvent, items: MenuItem[]): void {
  if (anchor instanceof HTMLElement) ctxMenu.showAt(anchor, items, { align: 'end' });
  else ctxMenu.show(anchor, items);
}

export function showIdeSchemeMenu(
  anchor: HTMLElement | MouseEvent,
  cur: QuickSchemePrefs,
  onChange: (next: QuickSchemePrefs) => void,
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
  openAnchoredMenu(anchor, items);
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
    { id: 'ide.environments', title: 'Shell environment & variables…', group: 'IDE', run: () => ide.openSettings('build.env') },
    { id: 'ide.ssh', title: 'Connect remote SSH project…', group: 'IDE', run: () => ide.openSettings('build.ssh') },
    { id: 'ide.share', title: 'Share live IDE session…', group: 'IDE', run: () => (ide.shareOpen = true) },
    { id: 'ide.plugins', title: 'Manage IDE plugins & language servers…', group: 'IDE', run: () => ide.openSettings('plugins') },
  ];
}

export function showIdeMoreMenu(
  anchor: HTMLElement | MouseEvent,
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
    { label: 'Connect remote SSH…', icon: 'server', action: () => ide.openSettings('build.ssh') },
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
  openAnchoredMenu(anchor, items);
}
