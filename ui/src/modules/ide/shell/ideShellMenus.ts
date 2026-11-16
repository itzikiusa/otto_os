// Lazy-loaded command palette registration, Color Scheme picker menu, and overflow
// menu for `IdePage.svelte` (keeps `page:ide` first-paint bundle lean).

import type { Command } from '../../../lib/commands.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { idePreviewMode } from '../editor/idePreviewMode.svelte';
import { ide } from '../ideStore.svelte';
import { ideAgent } from '../store/ideAgentController.svelte';
import {
  dispatchOpenLocalHistory,
  promptIdeGoToLine,
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

/** "Open Preview" / "… to the Side" on the active file; a code file says what previews. */
function runIdePreview(side: boolean): void {
  if (!idePreviewMode.kindOf(ide.activePath)) {
    toasts.info('No preview for this file', 'Preview shows Markdown, HTML, SVG, images, JSON, Mermaid and CSV/TSV files.');
    return;
  }
  if (side) idePreviewMode.openSide(ide.activePath);
  else idePreviewMode.toggle(ide.activePath);
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
    { id: 'ide.goToLine', title: 'Go to line / column…', group: 'IDE', shortcut: '⌘G', keywords: 'jump line number cursor', run: () => void promptIdeGoToLine() },
    { id: 'ide.reformat', title: 'Reformat code', group: 'IDE', shortcut: '⌘⌥L', keywords: 'format gofmt prettier rustfmt', run: () => void ide.formatActiveFile() },
    { id: 'ide.optimizeImports', title: 'Optimize imports', group: 'IDE', shortcut: '⌃⌥O', keywords: 'organize imports unused goimports', run: () => void ide.organizeImportsActiveFile() },
    { id: 'ide.dbWatch', title: 'IDE: Open Live DB Watch', group: 'IDE', keywords: 'database testcontainers mysql postgres mongo redis clickhouse diff', run: () => ide.openBottomTab('dbwatch') },
    { id: 'ide.preview', title: 'IDE: Open Preview', group: 'IDE', keywords: 'markdown readme html render image svg json mermaid', run: () => runIdePreview(false) },
    { id: 'ide.previewSide', title: 'IDE: Open Preview to the Side', group: 'IDE', keywords: 'markdown readme html render split', run: () => runIdePreview(true) },
    { id: 'ide.revealFile', title: 'Select opened file in project tree', group: 'IDE', run: () => void ide.revealInProjectTree() },
    { id: 'ide.toggleAgent', title: 'Toggle AI Agent', group: 'IDE', shortcut: '⇧⌘L', keywords: 'ai assistant chat context', run: opts.onTriggerAgent },
    { id: 'ide.inlineEdit', title: 'Inline AI edit…', group: 'IDE', shortcut: '⌘I', keywords: 'ai refactor rewrite selection', run: () => ideAgent.openInlineEdit() },
    { id: 'ide.toolchains', title: 'Project SDKs & Toolchains…', group: 'IDE', run: () => ide.openSettings('project.sdks') },
    { id: 'ide.environments', title: 'Shell environment & variables…', group: 'IDE', run: () => ide.openSettings('build.env') },
    { id: 'ide.ssh', title: 'Connect remote SSH project…', group: 'IDE', run: () => ide.openSettings('build.ssh') },
    { id: 'ide.share', title: 'Share live IDE session…', group: 'IDE', run: () => (ide.shareOpen = true) },
    { id: 'ide.plugins', title: 'Manage IDE plugins & language servers…', group: 'IDE', run: () => ide.openSettings('plugins') },
  ];
}

/** Touch screens get no keyboard-shortcut hints: there is no ⌘ to press. */
function shortcutHint(key: string): string | undefined {
  if (typeof window !== 'undefined' && window.matchMedia?.('(pointer: coarse)').matches) return undefined;
  return key;
}

export interface IdeMoreMenuOpts {
  explorerVisible: boolean;
  onToggleExplorer: () => void;
  onTriggerAgent: () => void;
  onAttachFolder: () => void;
  /** The stacked (phone) layout: tool windows open as panes, desktop-only
   *  toggles (explorer rail, build tool window, dock) are left out, and the
   *  header's Debug / Scratches live here. */
  stack?: boolean;
  onShowPane?: (pane: 'tools' | 'agent') => void;
  onOpenScratches?: () => void;
  scheme?: { cur: QuickSchemePrefs; onChange: (next: QuickSchemePrefs) => void };
}

/** The IDE ⋯ menu: everything the one-row header does not show (G13). */
export function ideMoreMenuItems(opts: IdeMoreMenuOpts): MenuItem[] {
  const btName = ide.buildToolKind === 'gradle' ? 'Gradle' : 'Maven';
  const stack = opts.stack === true;
  // Stacked: a tool window is a pane, so pick its tab without touching the
  // workbench's persisted "dock open" preference.
  const showTool = (tab: 'git' | 'dbwatch'): void => {
    if (stack) {
      ide.activeBottomTab = tab;
      opts.onShowPane?.('tools');
    } else ide.openBottomTab(tab);
  };
  const scheme = opts.scheme;
  return [
    ...(stack
      ? [{ label: 'Debug', icon: 'bug' as const, hint: shortcutHint('⌃D'), disabled: !ide.hasTarget, action: () => void ide.startDebug() }]
      : []),
    { label: 'Run with coverage', icon: 'shield', disabled: !ide.hasTarget, action: () => void ide.startRun({ coverage: true }) },
    { label: 'Run configurations…', icon: 'play', action: () => ide.openEnvironmentsModal('configs') },
    { separator: true },
    {
      label: 'Search everywhere…',
      icon: 'search',
      hint: shortcutHint('⇧⇧'),
      action: () => { ide.searchScopePath = null; ide.searchReplaceMode = false; ide.searchInitialTab = 'all'; ide.searchEverywhereOpen = true; },
    },
    { label: 'Find in path…', icon: 'search', hint: shortcutHint('⌘⇧F'), disabled: !ide.hasTarget, action: () => ide.openFindInPath({ replace: false }) },
    { label: 'Multi-line edit (prefix / suffix)', icon: 'edit', checked: ide.prefixSuffixOpen, disabled: !ide.activeTab, action: () => (ide.prefixSuffixOpen = !ide.prefixSuffixOpen) },
    { label: 'Format on save', checked: ide.formatOnSave, action: () => ide.setFormatOnSave(!ide.formatOnSave) },
    ...(idePreviewMode.available(ide.activePath).length
      ? [
          { label: 'Open Preview', icon: 'image' as const, checked: idePreviewMode.modeFor(ide.activePath) === 'preview', action: () => runIdePreview(false) },
          ...(stack ? [] : [{ label: 'Open Preview to the Side', icon: 'editorSplit' as const, checked: idePreviewMode.modeFor(ide.activePath) === 'split', action: () => runIdePreview(true) }]),
        ]
      : []),
    { separator: true },
    { label: 'Git Changes', icon: 'branch', hint: shortcutHint('⌘9'), disabled: !ide.hasTarget, action: () => showTool('git') },
    { label: 'Live DB Watch', icon: 'database', disabled: !ide.hasTarget, action: () => showTool('dbwatch') },
    ...(stack
      ? [{ label: 'AI Agent', icon: 'assistant' as const, action: () => opts.onShowPane?.('agent') }]
      : [
          { label: opts.explorerVisible ? 'Hide project explorer' : 'Show project explorer', icon: 'sidebar' as const, hint: shortcutHint('⌘1'), action: opts.onToggleExplorer },
          ...(ide.buildToolKind
            ? [{ label: ide.buildToolOpen ? `Hide ${btName} tool window` : `Show ${btName} tool window`, icon: 'layers' as const, action: () => ide.toggleBuildTool() }]
            : []),
          { label: ide.bottomDockOpen ? 'Hide bottom tool dock' : 'Show bottom tool dock', icon: 'terminal' as const, action: () => ide.toggleBottomDock() },
          { label: ideAgent.panelOpen ? 'Hide AI Agent' : 'Show AI Agent', icon: 'assistant' as const, hint: shortcutHint('⇧⌘L'), action: opts.onTriggerAgent },
        ]),
    { separator: true },
    { label: 'Local History…', icon: 'clock', disabled: !ide.hasTarget, action: () => dispatchOpenLocalHistory() },
    { label: 'Add folder to project…', icon: 'folder', disabled: !ide.hasTarget || ide.target.kind !== 'local', action: opts.onAttachFolder },
    { label: 'Add multiple repositories…', icon: 'layers', disabled: ide.target.kind !== 'local' && ide.hasTarget, action: () => (ide.bulkRepoPickerMode = ide.hasTarget ? 'attach' : 'create') },
    { label: 'Connect remote SSH…', icon: 'server', action: () => ide.openSettings('build.ssh') },
    { label: 'Share live session…', icon: 'share', action: () => (ide.shareOpen = true) },
    ...(stack && opts.onOpenScratches ? [{ label: 'Open Scratches', icon: 'workbench' as const, action: opts.onOpenScratches }] : []),
    { separator: true },
    ...(scheme
      ? [{
          label: 'Color scheme',
          icon: 'palette' as const,
          children: [
            ...QUICK_SCHEME_ITEMS.map((it) => ({
              label: it.label,
              checked: scheme.cur.colorScheme === it.id,
              action: () => scheme.onChange(writeQuickSchemePref({ colorScheme: it.id })),
            })),
            { separator: true },
            { label: 'Rainbow brackets', checked: scheme.cur.rainbowBrackets, action: () => scheme.onChange(writeQuickSchemePref({ rainbowBrackets: !scheme.cur.rainbowBrackets })) },
          ],
        }]
      : []),
    { label: 'Settings & Project Structure…', icon: 'gear', hint: shortcutHint('⌘,'), action: () => ide.openSettings('project.project') },
    { label: 'Project SDKs & Toolchains…', icon: 'box', action: () => ide.openSettings('project.sdks') },
    { label: 'Plugins & Language Servers…', icon: 'plug', action: () => ide.openSettings('plugins') },
  ];
}

export function showIdeMoreMenu(anchor: HTMLElement | MouseEvent, opts: IdeMoreMenuOpts): void {
  openAnchoredMenu(anchor, ideMoreMenuItems(opts));
}
