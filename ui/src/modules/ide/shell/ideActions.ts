// ONE list of IDE actions. The ⌘K palette's IDE entries, the quick-open's Actions
// tab and the header ⋯ menu are each a filter over it (`homes`), so a command is
// named, keyed and wired once. Shortcut hints come from `ideShortcuts.ts` for the
// active keymap preset — a hint is never shown for a key that does nothing.
//
// ⋯ keeps only what has no other home on screen (review I1-09): run with coverage,
// run configurations, local history, add to project, SSH, share, the tool windows
// and Settings. Editor toggles (format on save, multi-line edit…) live in Settings
// and the palette.

import type { Command } from '../../../lib/commands.svelte';
import type { IconName } from '../../../lib/components/Icon.svelte';
import type { MenuItem } from '../../../lib/contextmenu.svelte';
import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
import { toasts } from '../../../lib/toast.svelte';
import {
  areIdeBreakpointsMuted,
  closeAllIdeTabs,
  closeUnmodifiedIdeTabs,
  navigateIdeBack,
  navigateIdeForward,
  toggleFormatOnSave,
  toggleIdeBreakpointsMuted,
} from '../editor/ideEditorStoreBridge.svelte';
import { idePreviewMode } from '../editor/idePreviewMode.svelte';
import { ide } from '../ideStore.svelte';
import {
  openGoToFile,
  openGoToLine,
  openGoToSymbol,
  openRecentFiles,
  openSearchEverywhereAll,
} from '../search/ideQuickOpen.svelte';
import { ideAgent } from '../store/ideAgentController.svelte';
import { ideKnownProjects, ideProjectPaletteCommands, openIdeProjectSwitcher } from './ideProjectRecents.svelte';
import { dispatchOpenLocalHistory } from './ideShellCommands';
import { runTargetMenuItems, runTargetRowLabel } from './ideRunTargetMenu';
import { ideKeymapPresetOf, ideShortcutLabel, type IdeShortcutId } from './ideShortcuts';

/** What the page hands the actions that need it (IdePage owns these). */
export interface IdeActionCtx {
  onAttachFolder?: () => void;
  onTriggerAgent?: () => void;
  onToggleExplorer?: () => void;
  explorerVisible?: boolean;
  /** The stacked (phone) layout: tool windows open as panes. */
  stack?: boolean;
  onShowPane?: (pane: 'tools' | 'agent') => void;
  onOpenScratches?: () => void;
}

/** palette = ⌘K · search = the quick-open's Actions tab · more = the header ⋯ */
export type IdeActionHome = 'palette' | 'search' | 'more';

export type IdeActionGroup = 'Search' | 'Run' | 'Code' | 'Navigate' | 'Tabs' | 'Tool windows' | 'Add to project' | 'Project' | 'Settings';

export interface IdeAction {
  id: string;
  /** "Verb object"; the palette shows it as "IDE: <title>". */
  title: string;
  group: IdeActionGroup;
  icon: IconName;
  homes: readonly IdeActionHome[];
  shortcut?: IdeShortcutId;
  keywords?: string;
  /** The ⋯ / Actions row's text when it says more than the title: a toggle says
   *  what it will do now ("Hide project explorer"), Maven vs Gradle. */
  label?: (ctx: IdeActionCtx) => string;
  /** Shown at all in this home and context (phone-only rows, a callback the page passed). */
  when?: (ctx: IdeActionCtx, home: IdeActionHome) => boolean;
  disabled?: (ctx: IdeActionCtx) => boolean;
  /** A ⋯ row that opens a submenu instead of running (the phone's run target). */
  children?: (ctx: IdeActionCtx) => MenuItem[];
  run: (ctx: IdeActionCtx) => void;
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

const noTarget = (): boolean => !ide.hasTarget;
const noFile = (): boolean => !ide.activePath;
const desktop = (c: IdeActionCtx): boolean => !c.stack;
const PS: readonly IdeActionHome[] = ['palette', 'search'];
/** In the phone ⋯ only when not stacked: the pane switcher already has it (I4-21). */
const notPaneInMore = (c: IdeActionCtx, h: IdeActionHome): boolean => h !== 'more' || !c.stack;

/** Stacked: a tool window is a pane; pick its tab without touching the workbench's
 *  persisted "dock open" preference. */
function showTool(ctx: IdeActionCtx, tab: 'git' | 'dbwatch' | 'terminal' | 'problems' | 'debug'): void {
  if (ctx.stack) {
    ide.activeBottomTab = tab;
    ctx.onShowPane?.('tools');
  } else ide.openBottomTab(tab);
}

export const IDE_ACTIONS: readonly IdeAction[] = [
  // Search & go to
  { id: 'searchEverywhere', title: 'Search everywhere', group: 'Search', icon: 'search', homes: ['palette', 'more'], shortcut: 'searchEverywhere', keywords: 'find symbol file action', when: (c, h) => h !== 'more' || c.stack === true, run: () => openSearchEverywhereAll() },
  { id: 'goToFile', title: 'Go to file', group: 'Search', icon: 'file', homes: PS, shortcut: 'goToFile', keywords: 'open quick open', run: () => openGoToFile() },
  { id: 'recentFiles', title: 'Open recent files', group: 'Search', icon: 'clock', homes: PS, shortcut: 'recentFiles', run: () => openRecentFiles() },
  { id: 'goToSymbol', title: 'Go to symbol', group: 'Search', icon: 'function', homes: PS, shortcut: 'goToSymbol', keywords: 'class function type struct', run: () => openGoToSymbol() },
  { id: 'goToLine', title: 'Go to line', group: 'Search', icon: 'step', homes: PS, shortcut: 'goToLine', keywords: 'jump line number column cursor', disabled: noFile, run: () => openGoToLine() },
  { id: 'findInPath', title: 'Find in path', group: 'Search', icon: 'search', homes: PS, shortcut: 'findInPath', keywords: 'grep text', disabled: noTarget, run: () => ide.openFindInPath({ replace: false }) },
  { id: 'replaceInPath', title: 'Replace in path', group: 'Search', icon: 'swap', homes: PS, shortcut: 'replaceInPath', disabled: noTarget, run: () => ide.openFindInPath({ replace: true }) },
  // Run
  { id: 'runTarget', title: 'Choose run target', group: 'Run', icon: 'target', homes: ['more'], label: () => runTargetRowLabel(), when: (c) => c.stack === true, disabled: noTarget, children: () => runTargetMenuItems(), run: () => {} },
  { id: 'debug', title: 'Debug current configuration', group: 'Run', icon: 'bug', homes: ['palette', 'search', 'more'], shortcut: 'debug', disabled: noTarget, when: (c, h) => h !== 'more' || c.stack === true, run: () => void ide.startDebug() },
  { id: 'run', title: 'Run current configuration', group: 'Run', icon: 'play', homes: PS, shortcut: 'run', disabled: noTarget, run: () => void ide.startRun() },
  { id: 'stop', title: 'Stop run or debug', group: 'Run', icon: 'stop', homes: PS, shortcut: 'stop', run: () => void ide.stopActiveExecution() },
  { id: 'runCoverage', title: 'Run with coverage', group: 'Run', icon: 'shield', homes: ['palette', 'search', 'more'], keywords: 'tests coverage', disabled: noTarget, run: () => void ide.startRun({ coverage: true }) },
  { id: 'editConfigs', title: 'Edit run configurations…', group: 'Run', icon: 'play', homes: ['palette', 'search', 'more'], keywords: 'launch debug environments', run: () => ide.openEnvironmentsModal('configs') },
  // Code
  { id: 'reformat', title: 'Reformat code', group: 'Code', icon: 'format', homes: PS, shortcut: 'reformat', keywords: 'format gofmt prettier rustfmt', disabled: noFile, run: () => void ide.formatActiveFile() },
  { id: 'optimizeImports', title: 'Optimize imports', group: 'Code', icon: 'layers', homes: PS, shortcut: 'optimizeImports', keywords: 'organize imports unused goimports', disabled: noFile, run: () => void ide.organizeImportsActiveFile() },
  { id: 'rename', title: 'Rename symbol…', group: 'Code', icon: 'edit', homes: PS, shortcut: 'rename', keywords: 'refactor', disabled: noFile, run: () => { if (ide.activePath) void ide.openRenameModal(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col); } },
  { id: 'findUsages', title: 'Find usages', group: 'Code', icon: 'search', homes: PS, shortcut: 'findUsages', keywords: 'references', disabled: noFile, run: () => { if (ide.activePath) void ide.findReferences(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col); } },
  { id: 'inlineEdit', title: 'Edit selection with AI…', group: 'Code', icon: 'sparkle', homes: PS, shortcut: 'inlineEdit', keywords: 'ai refactor rewrite inline', disabled: noFile, run: () => ideAgent.openInlineEdit() },
  { id: 'addSelectionToAgent', title: 'Add selection to agent', group: 'Code', icon: 'assistant', homes: PS, shortcut: 'addSelectionToAgent', keywords: 'ai context chat', disabled: noFile, run: () => void ideAgent.markSelectionFromEditor({ openPanel: true }) },
  { id: 'splitLines', title: 'Split selection into cursors', group: 'Code', icon: 'layers', homes: ['search'], shortcut: 'splitSelection', disabled: noFile, run: () => ide.splitSelectionToLines() },
  { id: 'prefixSuffix', title: 'Toggle multi-line edit (prefix / suffix)', group: 'Code', icon: 'edit', homes: PS, disabled: noFile, run: () => (ide.prefixSuffixOpen = !ide.prefixSuffixOpen) },
  { id: 'formatOnSave', title: 'Toggle format on save', group: 'Code', icon: 'check', homes: PS, run: () => toggleFormatOnSave() },
  { id: 'muteBreakpoints', title: 'Toggle breakpoint muting', group: 'Code', icon: 'shieldOff', homes: PS, label: () => (areIdeBreakpointsMuted() ? 'Unmute breakpoints' : 'Mute all breakpoints'), run: () => toggleIdeBreakpointsMuted() },
  { id: 'preview', title: 'Open preview', group: 'Code', icon: 'image', homes: PS, keywords: 'markdown readme html render image svg json mermaid', run: () => runIdePreview(false) },
  { id: 'previewSide', title: 'Open preview to the side', group: 'Code', icon: 'editorSplit', homes: PS, keywords: 'markdown readme html render split', when: desktop, run: () => runIdePreview(true) },
  // Navigate & tabs
  { id: 'back', title: 'Navigate back', group: 'Navigate', icon: 'chevronLeft', homes: ['search'], shortcut: 'navigateBack', run: () => void navigateIdeBack() },
  { id: 'forward', title: 'Navigate forward', group: 'Navigate', icon: 'chevronRight', homes: ['search'], shortcut: 'navigateForward', run: () => void navigateIdeForward() },
  { id: 'reveal', title: 'Reveal active file in project tree', group: 'Navigate', icon: 'target', homes: PS, disabled: noFile, run: () => void ide.revealInProjectTree() },
  { id: 'save', title: 'Save file', group: 'Tabs', icon: 'download', homes: ['search'], shortcut: 'save', disabled: noFile, run: () => void ide.saveFile() },
  { id: 'saveAll', title: 'Save all files', group: 'Tabs', icon: 'download', homes: PS, shortcut: 'saveAll', run: () => void ide.saveAll() },
  { id: 'reopenTab', title: 'Reopen closed tab', group: 'Tabs', icon: 'undo', homes: PS, shortcut: 'reopenTab', run: () => void ide.reopenClosedTab() },
  { id: 'closeUnmodified', title: 'Close unmodified tabs', group: 'Tabs', icon: 'x', homes: PS, run: () => void closeUnmodifiedIdeTabs() },
  { id: 'closeAll', title: 'Close all tabs', group: 'Tabs', icon: 'x', homes: PS, run: () => void closeAllIdeTabs() },
  // Project
  { id: 'switchProject', title: 'Switch project…', group: 'Project', icon: 'folder', homes: PS, keywords: 'open recent workspace folder repository ssh', run: () => { if (!openIdeProjectSwitcher()) toasts.info('Project switcher isn’t on screen', 'Open the IDE page first.'); } },
  { id: 'localHistory', title: 'Show local history…', group: 'Project', icon: 'clock', homes: ['palette', 'search', 'more'], keywords: 'revisions timeline restore diff', disabled: noTarget, run: () => dispatchOpenLocalHistory() },
  { id: 'addFolder', title: 'Add folder to project…', group: 'Add to project', icon: 'folder', homes: ['palette', 'search', 'more'], label: () => 'Folder…', when: (c) => Boolean(c.onAttachFolder), disabled: () => !ide.hasTarget || ide.target.kind !== 'local', run: (c) => c.onAttachFolder?.() },
  { id: 'addRepos', title: 'Add repositories…', group: 'Add to project', icon: 'layers', homes: ['palette', 'search', 'more'], keywords: 'multi-repo microservices folders bulk', label: () => 'Repositories…', disabled: () => ide.target.kind !== 'local' && ide.hasTarget, run: () => (ide.bulkRepoPickerMode = ide.hasTarget ? 'attach' : 'create') },
  { id: 'ssh', title: 'Connect remote SSH…', group: 'Project', icon: 'server', homes: ['palette', 'search', 'more'], keywords: 'remote host server', run: () => ide.openSettings('build.ssh') },
  { id: 'share', title: 'Share live session…', group: 'Project', icon: 'share', homes: ['palette', 'search', 'more'], keywords: 'collaborate pair', run: () => (ide.shareOpen = true) },
  // Tool windows (one ⋯ submenu)
  { id: 'toggleExplorer', title: 'Toggle project explorer', group: 'Tool windows', icon: 'sidebar', homes: ['more'], shortcut: 'toggleExplorer', label: (c) => (c.explorerVisible ? 'Hide project explorer' : 'Show project explorer'), when: (c) => desktop(c) && Boolean(c.onToggleExplorer), run: (c) => c.onToggleExplorer?.() },
  { id: 'toggleDock', title: 'Toggle tool dock', group: 'Tool windows', icon: 'terminal', homes: ['palette', 'search', 'more'], shortcut: 'toggleDock', label: () => (ide.bottomDockOpen ? 'Hide tool dock' : 'Show tool dock'), when: desktop, run: () => ide.toggleBottomDock() },
  { id: 'toggleBuildTool', title: 'Toggle Maven / Gradle tool window', group: 'Tool windows', icon: 'layers', homes: ['palette', 'search', 'more'], keywords: 'pom lifecycle spring profiles', label: () => `${ide.buildToolOpen ? 'Hide' : 'Show'} ${ide.buildToolKind === 'gradle' ? 'Gradle' : 'Maven'} tool window`, when: (c, h) => !c.stack && (h === 'palette' || Boolean(ide.buildToolKind)), run: () => ide.toggleBuildTool() },
  { id: 'toggleAgent', title: 'Toggle AI agent', group: 'Tool windows', icon: 'assistant', homes: ['palette', 'search', 'more'], shortcut: 'toggleAgent', keywords: 'ai assistant chat context', label: (c) => (c.stack ? 'AI agent' : ideAgent.panelOpen ? 'Hide AI agent' : 'Show AI agent'), when: (c, h) => notPaneInMore(c, h) && Boolean(c.stack ? c.onShowPane : c.onTriggerAgent), run: (c) => (c.stack ? c.onShowPane?.('agent') : c.onTriggerAgent?.()) },
  { id: 'gitChanges', title: 'Open Git changes', group: 'Tool windows', icon: 'branch', homes: ['palette', 'search', 'more'], shortcut: 'gitPanel', keywords: 'vcs commit diff branches', label: () => 'Git changes', when: notPaneInMore, disabled: noTarget, run: (c) => showTool(c, 'git') },
  { id: 'terminal', title: 'Open terminal', group: 'Tool windows', icon: 'terminal', homes: PS, shortcut: 'terminal', disabled: noTarget, run: (c) => showTool(c, 'terminal') },
  { id: 'debugger', title: 'Open debugger', group: 'Tool windows', icon: 'bug', homes: PS, shortcut: 'debugPanel', keywords: 'debug variables frames console breakpoints', disabled: noTarget, run: (c) => showTool(c, 'debug') },
  { id: 'problems', title: 'Open problems', group: 'Tool windows', icon: 'warning', homes: PS, shortcut: 'problemsPanel', disabled: noTarget, run: (c) => showTool(c, 'problems') },
  { id: 'dbWatch', title: 'Open live DB watch', group: 'Tool windows', icon: 'db', homes: ['palette', 'search', 'more'], keywords: 'database testcontainers mysql postgres mongo redis clickhouse diff', label: () => 'Live DB watch', disabled: noTarget, run: (c) => showTool(c, 'dbwatch') },
  { id: 'scratches', title: 'Open scratches', group: 'Tool windows', icon: 'workbench', homes: ['more'], label: () => 'Scratches', when: (c) => c.stack === true && Boolean(c.onOpenScratches), run: (c) => c.onOpenScratches?.() },
  // Settings
  { id: 'settings', title: 'Open settings…', group: 'Settings', icon: 'gear', homes: ['palette', 'search', 'more'], shortcut: 'settings', keywords: 'preferences project structure goroot jdk sdk', run: () => ide.openSettings('project.project') },
  { id: 'keymap', title: 'Show keyboard shortcuts', group: 'Settings', icon: 'command', homes: PS, keywords: 'keymap hotkeys keys jetbrains vscode', run: () => ide.openSettings('keymap') },
  { id: 'switchKeymap', title: 'Switch keymap preset', group: 'Settings', icon: 'command', homes: PS, keywords: 'jetbrains vscode standard', label: () => (ide.keymapPreset === 'jetbrains' ? 'Switch keymap to Standard' : 'Switch keymap to JetBrains'), run: () => ide.setKeymapPreset(ide.keymapPreset === 'jetbrains' ? 'standard' : 'jetbrains') },
  { id: 'sdks', title: 'Open SDKs & toolchains…', group: 'Settings', icon: 'box', homes: PS, keywords: 'goroot jdk node python rust toolchain download', run: () => ide.openSettings('project.sdks') },
  { id: 'shellEnv', title: 'Edit shell environment…', group: 'Settings', icon: 'terminal', homes: PS, keywords: 'environment variables zshrc path goprivate', run: () => ide.openSettings('build.env') },
  { id: 'plugins', title: 'Manage plugins & language servers…', group: 'Settings', icon: 'plug', homes: PS, keywords: 'lsp gopls pyright jdtls', run: () => ide.openSettings('plugins') },
  { id: 'colorScheme', title: 'Choose color scheme…', group: 'Settings', icon: 'palette', homes: PS, keywords: 'darcula islands light dark rainbow brackets theme', run: () => ide.openSettings('editor.colors') },
];

/** Touch screens get no keyboard-shortcut hints: there is no ⌘ to press. */
function touchScreen(): boolean {
  return typeof window !== 'undefined' && Boolean(window.matchMedia?.('(pointer: coarse)').matches);
}

/** The action's key in the active preset, or undefined (none there, or a touch screen). */
export function ideActionHint(a: IdeAction): string | undefined {
  if (!a.shortcut || touchScreen()) return undefined;
  return ideShortcutLabel(a.shortcut, ideKeymapPresetOf(ide.keymapPreset));
}

export function ideActionsFor(home: IdeActionHome, ctx: IdeActionCtx): IdeAction[] {
  return IDE_ACTIONS.filter((a) => a.homes.includes(home) && (!a.when || a.when(ctx, home)));
}

// The page's callbacks, remembered when it registers its palette commands, so the
// quick-open (mounted on its own) can run "Add folder…" or "Toggle AI agent" too.
let shellCtx: IdeActionCtx = {};
export function currentIdeShellCtx(): IdeActionCtx {
  return shellCtx;
}

/** ⌘K: every palette action as "IDE: <title>", then one "IDE: Open project <name>" per
 *  project you can reopen (named, folder, recent, SSH — I3-06). */
export function ideActionPaletteCommands(ctx: IdeActionCtx): Command[] {
  shellCtx = { ...shellCtx, ...ctx };
  const projects = ideProjectPaletteCommands(ideKnownProjects(ide.namedProjects, ws.workspaces.filter((w) => w.id !== SCRATCH_WORKSPACE_ID)), async () => ide);
  return [...ideActionsFor('palette', ctx).map((a): Command => ({
    id: `ide.${a.id}`,
    title: `IDE: ${a.title}`,
    group: 'IDE',
    shortcut: ideActionHint(a),
    keywords: a.keywords,
    run: () => {
      if (a.disabled?.(shellCtx)) toasts.info(`${a.title.replace(/…$/, '')} isn't available here`, 'Open a project or a file first.');
      else a.run(shellCtx);
    },
  })), ...projects];
}

function menuRow(a: IdeAction, ctx: IdeActionCtx): MenuItem {
  const disabled = a.disabled?.(ctx) ?? false;
  return {
    label: a.label?.(ctx) ?? a.title,
    icon: a.icon,
    hint: ideActionHint(a),
    disabled,
    ...(a.children && !disabled ? { children: a.children(ctx) } : { action: () => a.run(ctx) }),
  };
}

const MORE_SUBMENU: Partial<Record<IdeActionGroup, { label: string; icon: IconName }>> = {
  'Tool windows': { label: 'Tool windows', icon: 'panel' },
  'Add to project': { label: 'Add to project', icon: 'plus' },
};

/** The header ⋯: the `more` actions in group order, a separator between groups,
 *  "Tool windows" and "Add to project" as submenus. */
export function ideMoreMenuFromActions(ctx: IdeActionCtx): MenuItem[] {
  shellCtx = { ...shellCtx, ...ctx };
  const out: MenuItem[] = [];
  let lastGroup: string | null = null;
  const submenus = new Map<IdeActionGroup, MenuItem>();
  for (const a of ideActionsFor('more', ctx)) {
    // Stacked: what is left of "Tool windows" (DB watch, scratches) sits at the top level.
    const sub = ctx.stack && a.group === 'Tool windows' ? undefined : MORE_SUBMENU[a.group];
    const groupKey = a.group === 'Add to project' ? 'Project' : a.group;
    if (lastGroup !== null && lastGroup !== groupKey) out.push({ separator: true });
    lastGroup = groupKey;
    if (!sub) {
      out.push(menuRow(a, ctx));
      continue;
    }
    let parent = submenus.get(a.group);
    if (!parent) {
      parent = { label: sub.label, icon: sub.icon, children: [] };
      submenus.set(a.group, parent);
      out.push(parent);
    }
    parent.children!.push(menuRow(a, ctx));
  }
  return out;
}
