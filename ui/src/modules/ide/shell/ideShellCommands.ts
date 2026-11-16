// Synchronous IDE keyboard shortcuts and Color Scheme preference helpers
// for `IdePage.svelte`. Command palette & menus live in `./ideShellMenus.ts`.

import { confirmer } from '../../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import {
  readIdePreferences,
  writeIdePreferences,
  type ConcreteIdeColorSchemeId,
  type IdeColorSchemeId,
} from '../../../lib/idePreferences.svelte';
import { ide, ideBasename } from '../ideStore.svelte';
import { ideAgent } from '../store/ideAgentController.svelte';

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

export async function promptIdeGoToLine(): Promise<void> {
  if (!ide.activePath) return;
  ide.goToLineOpen = true;
  try {
    const raw = await confirmer.promptText('Go to line[:column]:', {
      title: 'Go to Line / Column',
      placeholder: `${ide.cursorInfo.line}:${ide.cursorInfo.col}`,
      confirmLabel: 'Go',
      validate: (v) => (/^\d+(:\d+)?$/.test(v) ? null : 'Enter a line number, or line:column (for example 42 or 42:8).'),
    });
    if (!raw?.trim() || !ide.activePath) return;
    const [lStr, cStr] = raw.trim().split(':');
    ide.jumpTo(
      ide.activePath,
      Math.max(1, Number.parseInt(lStr ?? '1', 10) || 1),
      Math.max(1, Number.parseInt(cStr ?? '1', 10) || 1),
    );
  } finally {
    ide.goToLineOpen = false;
  }
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

/** ⌘L: add the editor selection (or the current line) to the agent, as the editor's
 *  context menu and the docs say. ⇧⌘L toggles the agent dock; Go to Line is ⌘G. */
function addSelectionToAgent(): void {
  ideAgent.markSelectionFromEditor({ openPanel: true });
}

function matchesKey(e: KeyboardEvent, code: string, letter: string): boolean {
  return e.code === code || e.key.toLowerCase() === letter;
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

export function handleIdeChord(
  e: KeyboardEvent,
  overlayOpen: boolean,
  toggleExplorer: () => void,
  triggerAgent?: () => void,
): boolean {
  if (e.metaKey && e.ctrlKey && e.shiftKey && !e.altKey && matchesKey(e, 'KeyR', 'r')) {
    e.preventDefault();
    window.location.reload();
    return true;
  }
  const mod = e.metaKey || e.ctrlKey;
  const pureMod = mod && !(e.metaKey && e.ctrlKey);
  if (pureMod && e.shiftKey && !e.altKey && matchesKey(e, 'KeyR', 'r')) {
    if (!overlayOpen || ide.searchEverywhereOpen) {
      e.preventDefault();
      ide.openSearchEverywhere('text', { replace: true });
      return true;
    }
  }
  if (pureMod && e.shiftKey && !e.altKey && matchesKey(e, 'KeyF', 'f')) {
    if (!overlayOpen || ide.searchEverywhereOpen) {
      e.preventDefault();
      ide.openSearchEverywhere('text', { replace: false });
      return true;
    }
  }
  if (overlayOpen) return false;
  if (e.altKey && (e.ctrlKey || e.metaKey) && !e.shiftKey && matchesKey(e, 'KeyO', 'o')) {
    e.preventDefault();
    void ide.organizeImportsActiveFile();
    return true;
  }
  if (e.metaKey && e.altKey && !e.ctrlKey && !e.shiftKey && matchesKey(e, 'KeyR', 'r') && hasActiveDebugSession()) {
    e.preventDefault();
    void ide.debugCommand('continue');
    return true;
  }
  if ((e.metaKey || e.ctrlKey) && e.altKey && !e.shiftKey && matchesKey(e, 'KeyL', 'l')) {
    e.preventDefault();
    void ide.formatActiveFile();
    return true;
  }
  if (pureMod && !e.shiftKey && !e.altKey && (e.code === 'Comma' || e.key === ',')) { e.preventDefault(); ide.openSettings(); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && matchesKey(e, 'KeyS', 's')) { e.preventDefault(); void ide.saveFile(); return true; }
  if (mod && !e.shiftKey && e.altKey && matchesKey(e, 'KeyS', 's')) { e.preventDefault(); void ide.saveAll(); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && matchesKey(e, 'KeyW', 'w') && ide.activePath) { e.preventDefault(); if (!ide.activeTab?.pinned) void ide.closeTab(ide.activePath); return true; }
  if (pureMod && e.shiftKey && !e.altKey && matchesKey(e, 'KeyT', 't')) { e.preventDefault(); void ide.reopenClosedTab(); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && matchesKey(e, 'KeyE', 'e')) { e.preventDefault(); openIdeRecentFilesMenu(); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && matchesKey(e, 'KeyG', 'g') && !inEditorWithFindPanel(e.target)) { e.preventDefault(); void promptIdeGoToLine(); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && matchesKey(e, 'KeyL', 'l')) { e.preventDefault(); addSelectionToAgent(); return true; }
  if (pureMod && e.shiftKey && !e.altKey && matchesKey(e, 'KeyL', 'l') && triggerAgent) { e.preventDefault(); triggerAgent(); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && e.code === 'Digit1') { e.preventDefault(); toggleExplorer(); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && e.code === 'Digit4') { e.preventDefault(); ide.openBottomTab('run'); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && e.code === 'Digit5') { e.preventDefault(); ide.openBottomTab('debug'); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && e.code === 'Digit6') { e.preventDefault(); ide.openBottomTab('problems'); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && e.code === 'Digit9') { e.preventDefault(); ide.openBottomTab('git'); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && e.code === 'BracketLeft' && ide.canNavigateBack) { e.preventDefault(); void ide.navigateBack(); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && e.code === 'BracketRight' && ide.canNavigateForward) { e.preventDefault(); void ide.navigateForward(); return true; }
  if (pureMod && e.shiftKey && !e.altKey && matchesKey(e, 'KeyO', 'o')) {
    e.preventDefault(); ide.openSearchEverywhere('files'); return true;
  }
  if (pureMod && !e.shiftKey && !e.altKey && matchesKey(e, 'KeyO', 'o')) {
    e.preventDefault(); ide.openSearchEverywhere('symbols'); return true;
  }
  if (pureMod && !e.shiftKey && !e.altKey && matchesKey(e, 'KeyI', 'i') && !(e.ctrlKey && !e.metaKey && inXterm(e.target))) {
    e.preventDefault(); ideAgent.openInlineEdit(); return true;
  }
  if (pureMod && !e.shiftKey && !e.altKey && e.key === 'F8' && ide.activePath) { e.preventDefault(); ide.toggleBreakpoint(ide.activePath, ide.cursorInfo.line); return true; }
  if (pureMod && !e.shiftKey && !e.altKey && e.key === 'F2') { e.preventDefault(); void ide.stopActiveExecution(); return true; }
  return false;
}

export function handleIdeWindowKeyDown(e: KeyboardEvent, overlayOpen: boolean, triggerAgent: () => void): void {
  if (!e.defaultPrevented && e.metaKey && e.ctrlKey && e.shiftKey && !e.altKey && matchesKey(e, 'KeyR', 'r')) {
    e.preventDefault();
    window.location.reload();
    return;
  }
  if (!e.defaultPrevented && e.key === 'Escape' && !overlayOpen) {
    if (closeOpenIdeOverlays()) {
      e.preventDefault();
      return;
    }
  }
  if (!e.defaultPrevented && (e.metaKey || e.ctrlKey) && !(e.metaKey && e.ctrlKey) && e.shiftKey && !e.altKey && matchesKey(e, 'KeyR', 'r')) {
    if (!overlayOpen || ide.searchEverywhereOpen) {
      e.preventDefault();
      ide.openSearchEverywhere('text', { replace: true });
      return;
    }
  }
  if (overlayOpen) return;
  const term = inXterm(e.target);

  if (!e.defaultPrevented && !term && e.altKey && (e.ctrlKey || e.metaKey) && !e.shiftKey && matchesKey(e, 'KeyO', 'o')) {
    e.preventDefault();
    void ide.organizeImportsActiveFile();
    return;
  }
  if (!e.defaultPrevented && e.metaKey && e.altKey && !e.ctrlKey && !e.shiftKey && matchesKey(e, 'KeyR', 'r') && hasActiveDebugSession()) {
    e.preventDefault();
    void ide.debugCommand('continue');
    return;
  }
  if (!e.defaultPrevented && !term && (e.metaKey || e.ctrlKey) && e.altKey && !e.shiftKey && matchesKey(e, 'KeyL', 'l')) {
    e.preventDefault();
    void ide.formatActiveFile();
    return;
  }
  if (!e.defaultPrevented && (e.metaKey || e.ctrlKey) && !(e.metaKey && e.ctrlKey) && !e.altKey && !(e.ctrlKey && !e.metaKey && term)) {
    if (!e.shiftKey && matchesKey(e, 'KeyL', 'l')) { e.preventDefault(); addSelectionToAgent(); return; }
    if (e.shiftKey && matchesKey(e, 'KeyL', 'l')) { e.preventDefault(); triggerAgent(); return; }
    if (!e.shiftKey && matchesKey(e, 'KeyI', 'i')) { e.preventDefault(); ideAgent.openInlineEdit(); return; }
    if (!e.shiftKey && (e.code === 'Comma' || e.key === ',')) { e.preventDefault(); ide.openSettings(); return; }
    if (!e.shiftKey && e.key === 'F8' && ide.activePath) { e.preventDefault(); ide.toggleBreakpoint(ide.activePath, ide.cursorInfo.line); return; }
    if (!e.shiftKey && e.key === 'F2') { e.preventDefault(); void ide.stopActiveExecution(); return; }
  }
  if (!e.defaultPrevented && (e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && matchesKey(e, 'KeyV', 'v') && e.target instanceof Element && Boolean(e.target.closest('.ide-editor-area'))) {
    e.preventDefault();
    window.dispatchEvent(new CustomEvent('otto:ide-paste-history'));
    return;
  }
  if (!e.defaultPrevented && e.ctrlKey && e.altKey && !e.metaKey && !e.shiftKey && hasActiveDebugSession()) {
    if (e.code === 'ArrowDown' || e.key === 'ArrowDown') { e.preventDefault(); void ide.stepOver(); return; }
    if (e.code === 'ArrowRight' || e.key === 'ArrowRight') { e.preventDefault(); void ide.stepInto(); return; }
    if (e.code === 'ArrowUp' || e.key === 'ArrowUp') { e.preventDefault(); void ide.stepOut(); return; }
  }
  if (e.defaultPrevented) return;
  // Bare ⌃ letters and F-keys belong to the text field the person is typing in.
  if (inTextField(e.target) && !e.metaKey && !e.altKey && (e.ctrlKey || /^F\d+$/.test(e.key))) return;
  if (e.altKey && !e.metaKey && !e.ctrlKey && !e.shiftKey && e.key === 'F12') { e.preventDefault(); ide.openBottomTab('terminal'); }
  else if (e.altKey && !e.metaKey && !e.ctrlKey && !e.shiftKey && e.key === 'F7' && ide.activePath) { e.preventDefault(); void ide.findReferences(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col); }
  else if (!term && e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && matchesKey(e, 'KeyR', 'r')) { e.preventDefault(); void ide.startRun(); }
  else if (!term && e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && matchesKey(e, 'KeyD', 'd')) { e.preventDefault(); void ide.startDebug(); }
  else if (e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && e.key === 'F6' && ide.activePath) { e.preventDefault(); void ide.openRenameModal(ide.activePath, ide.cursorInfo.line, ide.cursorInfo.col); }
  else if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === 'F8' && hasActiveDebugSession()) { e.preventDefault(); void ide.stepOver(); }
  else if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === 'F7' && hasActiveDebugSession()) { e.preventDefault(); void ide.stepInto(); }
  else if (e.shiftKey && !e.metaKey && !e.ctrlKey && !e.altKey && e.key === 'F8' && hasActiveDebugSession()) { e.preventDefault(); void ide.stepOut(); }
  else if (!e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey && e.key === 'F9' && hasActiveDebugSession()) { e.preventDefault(); void ide.debugCommand('continue'); }
  else if (e.altKey && !e.metaKey && !e.ctrlKey && !e.shiftKey && e.key === 'F9' && hasActiveDebugSession()) { e.preventDefault(); void ide.runToCursor(); }
}
