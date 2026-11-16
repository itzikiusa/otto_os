// The crumbs row's single ⋯ menu: the editor actions that used to be a strip of unlabelled
// icons (structure, ↑/↓ function, quick fix, reformat, blame, colour scheme, Send to), now
// labelled rows that only appear when they mean something for the file (ideStatusModel).

import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { ide, type IdeOpenTab } from '../ideStore.svelte';
import { flushAutoLocalHistorySnapshot, toggleFormatOnSave } from './ideEditorStoreBridge.svelte';
import { ideEditorActionsFor } from './ideStatusModel';

export interface IdeEditorMoreCtx {
  tab: IdeOpenTab;
  hasSymbols: boolean;
  hasSelection: boolean;
  blameActive: boolean;
  /** Shortcut hints are left out on touch devices. */
  coarse: boolean;
  isGo: boolean;
  onjumpsymbol: (delta: -1 | 1) => void;
  onopenstructure: () => void;
  ontoggleblame: () => void;
  onquickfix: () => void;
  onsetgotags: () => void;
  onsendsession: () => void;
}

async function appearanceItems(tab: IdeOpenTab): Promise<MenuItem[]> {
  const { IDE_COLOR_SCHEME_OPTIONS, readIdeEditorPreferences, writeIdeEditorPreferences } = await import('../../../lib/components/cm-ide-theme');
  const cur = readIdeEditorPreferences();
  const toggle = (label: string, on: boolean, patch: () => void): MenuItem => ({ label, checked: on, action: patch });
  const items: MenuItem[] = [
    ...IDE_COLOR_SCHEME_OPTIONS.map((opt): MenuItem => ({
      label: opt.label,
      checked: cur.colorScheme === opt.id,
      action: () => { writeIdeEditorPreferences({ colorScheme: opt.id }); },
    })),
    { separator: true },
    toggle('Path crumbs', cur.showBreadcrumbs, () => { writeIdeEditorPreferences({ showBreadcrumbs: !cur.showBreadcrumbs }); }),
    toggle('Rainbow brackets', cur.rainbowBrackets, () => { writeIdeEditorPreferences({ rainbowBrackets: !cur.rainbowBrackets }); }),
    toggle('Inlay parameter hints', cur.inlayHints, () => { writeIdeEditorPreferences({ inlayHints: !cur.inlayHints }); }),
    toggle('Usages above symbols', cur.codeVision, () => { writeIdeEditorPreferences({ codeVision: !cur.codeVision }); }),
    toggle('Fold imports', cur.autoFoldImports, () => { writeIdeEditorPreferences({ autoFoldImports: !cur.autoFoldImports }); }),
    toggle('Auto-save', cur.autoSave, () => { writeIdeEditorPreferences({ autoSave: !cur.autoSave }); }),
    toggle('Format on save', cur.formatOnSave, () => { toggleFormatOnSave(); writeIdeEditorPreferences({ formatOnSave: !cur.formatOnSave }); }),
    { separator: true },
    { label: 'Editor settings…', icon: 'gear', action: () => ide.openSettings() },
  ];
  return tab.readOnly ? items.filter((it) => it.label !== 'Format on save') : items;
}

function sendToItems(tab: IdeOpenTab, onsendsession: () => void): MenuItem[] {
  const ctx = { name: tab.name, content: tab.content, lang: tab.path.split('.').pop()?.toLowerCase() ?? '' };
  const empty = !tab.content.trim();
  const send = () => import('../filetools/send/sendTo');
  return [
    { label: 'Database — Run on…', icon: 'db', disabled: empty, title: 'Opens the DB Explorer’s Run on… sheet; nothing runs yet', action: () => void send().then((m) => m.sendToDatabase(ctx)) },
    { label: /^\s*curl\s/.test(tab.content) ? 'API client — import curl' : 'API client — new request', icon: 'send', disabled: empty, action: () => void send().then((m) => m.sendToApiClient(ctx)) },
    { label: 'Terminal / agent session…', icon: 'terminal', disabled: empty, title: 'Pastes into a live session; never executed', action: onsendsession },
    { label: 'Vault — save as note…', icon: 'book', disabled: empty, action: () => void send().then((m) => m.sendToVault(ctx)) },
  ];
}

/** Rows of the ⋯ menu for `c` (exported for unit tests). */
export function ideEditorMoreRows(c: IdeEditorMoreCtx, appearance: MenuItem[]): MenuItem[] {
  const acts = ideEditorActionsFor(c.tab.path, c.tab.language, c.hasSymbols);
  const k = (keys: string): string | undefined => (c.coarse ? undefined : keys);
  const ro = c.tab.readOnly;
  const rows: MenuItem[] = [];
  if (c.hasSelection) rows.push({ label: 'Split selection into cursors', icon: 'layers', hint: k('⌥⇧I'), action: () => ide.splitSelectionToLines() });
  if (acts.structure) rows.push({ label: 'File structure…', icon: 'book', hint: k('⌘F12'), action: c.onopenstructure });
  if (acts.jumpFunction) {
    rows.push(
      { label: 'Previous function', icon: 'arrowUp', hint: k('⌃↑'), disabled: !c.hasSymbols, action: () => c.onjumpsymbol(-1) },
      { label: 'Next function', icon: 'arrowDown', hint: k('⌃↓'), disabled: !c.hasSymbols, action: () => c.onjumpsymbol(1) },
    );
  }
  if (acts.quickFix) rows.push({ label: 'Context actions…', icon: 'bulb', hint: k('⌥⏎'), action: c.onquickfix });
  if (acts.reformat) rows.push({ label: 'Reformat code', icon: 'format', hint: k('⌘⌥L'), disabled: ro, action: () => void ide.formatActiveFile() });
  if (acts.optimizeImports) rows.push({ label: 'Optimize imports', icon: 'sparkle', hint: k('⌃⌥O'), disabled: ro, action: () => void ide.organizeImportsActiveFile() });
  if (c.isGo) rows.push({ label: 'Go build tags…', icon: 'tag', action: c.onsetgotags });
  rows.push(
    { separator: true },
    // A checkable row draws its check in the icon slot: no icon, or it would sit out of line.
    { label: 'Git blame', checked: c.blameActive, action: c.ontoggleblame },
    {
      label: 'Local history…',
      icon: 'clock',
      action: () => {
        flushAutoLocalHistorySnapshot(c.tab.path);
        window.dispatchEvent(new CustomEvent('otto:ide-open-local-history', { detail: { path: c.tab.path } }));
      },
    },
    { label: 'Send to', icon: 'share', children: sendToItems(c.tab, c.onsendsession) },
    { separator: true },
    { label: 'Appearance', icon: 'palette', children: appearance },
  );
  return rows;
}

export async function openIdeEditorMoreMenu(anchor: HTMLElement | null, c: IdeEditorMoreCtx): Promise<void> {
  const appearance = await appearanceItems(c.tab);
  ctxMenu.showAt(anchor, ideEditorMoreRows(c, appearance), { align: 'end' });
}
