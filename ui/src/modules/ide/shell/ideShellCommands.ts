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

const inXterm = (t: EventTarget | null): boolean => t instanceof Element && Boolean(t.closest('.xterm'));

export function handleIdeChord(e: KeyboardEvent, overlayOpen: boolean, toggleExplorer: () => void): boolean {
  if (overlayOpen) return false;
  const mod = e.metaKey || e.ctrlKey;
  if (mod && !e.shiftKey && !e.altKey && (e.code === 'Comma' || e.key === ',')) { e.preventDefault(); ide.openSettings(); return true; }
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
    if (e.code === 'Comma' || e.key === ',') { e.preventDefault(); ide.openSettings(); return; }
  }
  if (!e.defaultPrevented && (e.metaKey || e.ctrlKey) && e.shiftKey && !e.altKey && e.code === 'KeyV' && e.target instanceof Element && Boolean(e.target.closest('.ide-editor-area'))) {
    e.preventDefault();
    window.dispatchEvent(new CustomEvent('otto:ide-paste-history'));
    return;
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
