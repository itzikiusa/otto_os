// Synchronous IDE keyboard shortcuts and Color Scheme preference helpers
// for `IdePage.svelte`. Command palette & menus live in `./ideShellMenus.ts`.
//
// WHICH key does WHAT comes from one table, `./ideShortcuts.ts` (per keymap preset);
// this file only says how each command runs and when it may (`WINDOW_COMMANDS`).

import {
  readIdePreferences,
  writeIdePreferences,
  type ConcreteIdeColorSchemeId,
  type IdeColorSchemeId,
} from '../../../lib/idePreferences.svelte';
import { ide } from '../ideStore.svelte';
import { openGoToFile, openGoToLine, openGoToSymbol, openRecentFiles } from '../search/ideQuickOpen.svelte';
import { ideAgent } from '../store/ideAgentController.svelte';
import { ideKeymapPresetOf, matchIdeShortcuts, type IdeChord, type IdeShortcutId } from './ideShortcuts';

export type IdeQuickSchemeId = IdeColorSchemeId;
export type IdeDarkSchemeId = Extract<
  ConcreteIdeColorSchemeId,
  'jetbrains-islands-dark' | 'jetbrains-darcula' | 'vscode-dark-modern'
>;
export type IdeLightSchemeId = Extract<
  ConcreteIdeColorSchemeId,
  'jetbrains-islands-light' | 'vscode-light-modern'
>;

export interface QuickSchemePrefs {
  colorScheme: IdeQuickSchemeId;
  preferredDarkScheme: IdeDarkSchemeId;
  preferredLightScheme: IdeLightSchemeId;
  rainbowBrackets: boolean;
}

export function readQuickSchemePrefs(): QuickSchemePrefs {
  const p = readIdePreferences();
  return {
    colorScheme: p.colorScheme,
    preferredDarkScheme: p.preferredDarkScheme as IdeDarkSchemeId,
    preferredLightScheme: p.preferredLightScheme as IdeLightSchemeId,
    rainbowBrackets: p.rainbowBrackets,
  };
}

export function writeQuickSchemePref(patch: {
  colorScheme?: IdeQuickSchemeId;
  rainbowBrackets?: boolean;
}): QuickSchemePrefs {
  writeIdePreferences(patch);
  return readQuickSchemePrefs();
}

export function dispatchOpenLocalHistory(): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent('otto:ide-open-local-history', { detail: { path: ide.activePath ?? ide.target.root } }));
}

/** Go to Line is the quick-open's `:` mode now (`search/ideQuickOpen.svelte.ts`
 *  `openGoToLine`); this name stays for IdePage's existing import. */
export function promptIdeGoToLine(): void {
  openGoToLine();
}

const inXterm = (t: EventTarget | null): boolean => t instanceof Element && Boolean(t.closest('.xterm'));

/** A text field outside the code editor: the agent composer, the commit box, a filter or
 *  the editor's own find/replace inputs. Bare ⌃ keys are text editing there (⌃D is
 *  forward-delete on macOS), so the IDE's bare ⌃R / ⌃D / F-key commands stay out. */
export function inTextField(t: EventTarget | null): boolean {
  if (!(t instanceof Element)) return false;
  const tag = t.tagName;
  if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
  return (t as HTMLElement).isContentEditable === true && !t.closest('.cm-content');
}

/** The focused editor has its find panel open: ⌘G is "find next" there, not Go to Line. */
function inEditorWithFindPanel(t: EventTarget | null): boolean {
  return t instanceof Element && Boolean(t.closest('.cm-editor')?.querySelector('.cm-search'));
}

function hasActiveDebugSession(): boolean {
  return Boolean(ide.activeDebug && ide.activeDebug.state !== 'terminated');
}

function closeOpenIdeOverlays(): boolean {
  let closed = false;
  if (ide.searchEverywhereOpen) { ide.searchEverywhereOpen = false; closed = true; }
  if (ide.bulkRepoPickerMode !== null) { ide.bulkRepoPickerMode = null; closed = true; }
  if (ide.environmentsOpen) { ide.environmentsOpen = false; closed = true; }
  if (ide.toolchainsOpen) { ide.toolchainsOpen = false; closed = true; }
  if (ide.sshOpen) { ide.sshOpen = false; closed = true; }
  if (ide.shareOpen) { ide.shareOpen = false; closed = true; }
  if (ide.pluginsOpen) { ide.pluginsOpen = false; closed = true; }
  if (ide.settingsOpen) { ide.settingsOpen = false; closed = true; }
  return closed;
}

interface ShellKeyCtx {
  e: KeyboardEvent;
  toggleExplorer?: () => void;
  triggerAgent?: () => void;
}

interface WindowCommand {
  /** false = not handled after all (the key goes on to the editor / the app). */
  run: (ctx: ShellKeyCtx) => boolean | void;
  /** Only while a debug session is live; tried before a command sharing its key (Standard F5). */
  session?: boolean;
  /** Needs an active file. */
  file?: boolean;
  /** Never from a focused terminal. */
  noTerm?: boolean;
  /** Its ⌘ chord also runs in the window phase (focus in a terminal included), as it always did. */
  window?: boolean;
  /** Still runs while the quick-open is the open overlay — it retargets the picker. */
  overSearch?: boolean;
  when?: (e: KeyboardEvent) => boolean;
}

const pos = (): { path: string; line: number; col: number } | null =>
  ide.activePath ? { path: ide.activePath, line: ide.cursorInfo.line, col: ide.cursorInfo.col } : null;

/** How each window-scope row of `IDE_SHORTCUTS` runs. A row without an entry here
 *  fails `unit/ideActionsShortcuts.test.ts`. */
export const WINDOW_COMMANDS: Record<IdeShortcutId, WindowCommand | undefined> = {
  settings: { window: true, run: () => ide.openSettings() },
  hardReload: { run: () => window.location.reload() },
  searchEverywhere: undefined,
  goToFile: { overSearch: true, run: () => openGoToFile() },
  goToSymbol: { overSearch: true, run: () => openGoToSymbol() },
  goToLine: { overSearch: true, file: true, when: (e) => !inEditorWithFindPanel(e.target), run: () => openGoToLine() },
  recentFiles: { overSearch: true, noTerm: true, run: () => openRecentFiles() },
  findInPath: { overSearch: true, run: () => ide.openSearchEverywhere('text', { replace: false }) },
  replaceInPath: { overSearch: true, window: true, run: () => ide.openSearchEverywhere('text', { replace: true }) },
  navigateBack: { run: () => (ide.canNavigateBack ? void ide.navigateBack() : false) },
  navigateForward: { run: () => (ide.canNavigateForward ? void ide.navigateForward() : false) },
  fileStructure: undefined,
  goToDefinition: undefined,
  findUsages: { file: true, run: () => { const p = pos(); if (p) void ide.findReferences(p.path, p.line, p.col); } },
  rename: { file: true, run: () => { const p = pos(); if (p) void ide.openRenameModal(p.path, p.line, p.col); } },
  quickFix: undefined,
  reformat: { window: true, noTerm: true, run: () => void ide.formatActiveFile() },
  optimizeImports: { window: true, noTerm: true, run: () => void ide.organizeImportsActiveFile() },
  toggleComment: undefined,
  duplicateLine: undefined,
  deleteLine: undefined,
  splitSelection: undefined,
  pasteHistory: undefined,
  save: { run: () => void ide.saveFile() },
  saveAll: { run: () => void ide.saveAll() },
  closeTab: { file: true, run: () => { if (ide.activePath) void ide.closeTab(ide.activePath); } },
  reopenTab: { run: () => void ide.reopenClosedTab() },
  toggleExplorer: { run: (c) => (c.toggleExplorer ? c.toggleExplorer() : false) },
  toggleDock: { run: () => ide.toggleBottomDock() },
  runPanel: { run: () => ide.openBottomTab('run') },
  debugPanel: { run: () => ide.openBottomTab('debug') },
  problemsPanel: { run: () => ide.openBottomTab('problems') },
  gitPanel: { run: () => ide.openBottomTab('git') },
  terminal: { run: () => ide.openBottomTab('terminal') },
  // ⌘L: add the editor selection (or the current line) to the agent, as the editor's
  // context menu and the docs say. ⇧⌘L toggles the agent dock; Go to Line is ⌘G.
  addSelectionToAgent: { window: true, run: () => void ideAgent.markSelectionFromEditor({ openPanel: true }) },
  toggleAgent: { window: true, run: (c) => (c.triggerAgent ? c.triggerAgent() : false) },
  inlineEdit: { window: true, run: () => ideAgent.openInlineEdit() },
  run: { noTerm: true, run: () => void ide.startRun() },
  debug: { noTerm: true, run: () => void ide.startDebug() },
  stop: { window: true, run: () => void ide.stopActiveExecution() },
  restartDebug: { session: true, run: () => void ide.restartDebug() },
  toggleBreakpoint: { window: true, file: true, run: () => { if (ide.activePath) ide.toggleBreakpoint(ide.activePath, ide.cursorInfo.line); } },
  resume: { session: true, window: true, run: () => void ide.debugCommand('continue') },
  stepOver: { session: true, run: () => void ide.stepOver() },
  stepInto: { session: true, run: () => void ide.stepInto() },
  stepOut: { session: true, run: () => void ide.stepOut() },
  runToCursor: { session: true, run: () => void ide.runToCursor() },
};

/** ⌘ chords are the chord phase's (keys.ts → `keyContext.pageChords`, never from a
 *  terminal); every other chord is the window phase's. */
const isCmdChord = (c: IdeChord): boolean => Boolean(c.mod || c.meta);
/** A bare F-key or a literal ⌃ chord: text editing inside a text field. */
const isBareChord = (c: IdeChord): boolean => !c.mod && !c.meta && !c.alt;

function dispatchIdeShortcut(ctx: ShellKeyCtx, phase: 'chord' | 'window', overlayOpen: boolean): boolean {
  const { e } = ctx;
  const term = inXterm(e.target);
  // A terminal owns its ⌃ keys (⌃L clears, ⌃R searches history, ⌃` …).
  if (phase === 'window' && term && e.ctrlKey && !e.metaKey) return false;
  const preset = ideKeymapPresetOf(ide.keymapPreset);
  const hits = matchIdeShortcuts(e, preset, 'window')
    .map((h) => ({ ...h, cmd: WINDOW_COMMANDS[h.id] }))
    // A live-session command first: Standard F5 resumes a paused program, else starts Debug.
    .sort((a, b) => Number(Boolean(b.cmd?.session)) - Number(Boolean(a.cmd?.session)));
  for (const { chord, cmd } of hits) {
    if (!cmd) continue;
    const cmdChord = isCmdChord(chord);
    if (phase === 'chord' ? !cmdChord : cmdChord && !cmd.window) continue;
    if (overlayOpen && !(cmd.overSearch && ide.searchEverywhereOpen) && !(chord.meta && chord.ctrl)) continue;
    if (cmd.session && !hasActiveDebugSession()) continue;
    if (cmd.file && !ide.activePath) continue;
    if (cmd.noTerm && term) continue;
    if (isBareChord(chord) && inTextField(e.target)) continue;
    if (cmd.when && !cmd.when(e)) continue;
    if (cmd.run(ctx) === false) continue;
    e.preventDefault();
    return true;
  }
  return false;
}

/** The ⌘ chords, claimed through `keyContext.pageChords` before the app's own map. */
export function handleIdeChord(
  e: KeyboardEvent,
  overlayOpen: boolean,
  toggleExplorer: () => void,
  triggerAgent?: () => void,
): boolean {
  return dispatchIdeShortcut({ e, toggleExplorer, triggerAgent }, 'chord', overlayOpen);
}

export function handleIdeWindowKeyDown(e: KeyboardEvent, overlayOpen: boolean, triggerAgent: () => void): void {
  if (e.defaultPrevented) return;
  if (e.key === 'Escape' && !overlayOpen) {
    if (closeOpenIdeOverlays()) e.preventDefault();
    return;
  }
  if (!overlayOpen && (e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && (e.code === 'KeyV' || e.key.toLowerCase() === 'v') && e.target instanceof Element && Boolean(e.target.closest('.ide-editor-area'))) {
    e.preventDefault();
    window.dispatchEvent(new CustomEvent('otto:ide-paste-history'));
    return;
  }
  dispatchIdeShortcut({ e, triggerAgent }, 'window', overlayOpen);
}
