// Lazy-loaded command palette registration, Color Scheme picker menu, and overflow
// menu for `IdePage.svelte` (keeps `page:ide` first-paint bundle lean). The palette
// entries and the ⋯ rows both come from the one action list, `./ideActions.ts`.

import type { Command } from '../../../lib/commands.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { ide } from '../ideStore.svelte';
import { ideActionPaletteCommands, ideMoreMenuFromActions } from './ideActions';
import { type IdeQuickSchemeId, type QuickSchemePrefs, writeQuickSchemePref } from './ideShellCommands';

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
  return ideActionPaletteCommands(opts);
}

export interface IdeMoreMenuOpts {
  explorerVisible: boolean;
  onToggleExplorer: () => void;
  onTriggerAgent: () => void;
  onAttachFolder: () => void;
  /** The stacked (phone) layout: tool windows open as panes, desktop-only
   *  toggles (explorer rail, build tool window, dock) are left out, and the
   *  header's Debug / Scratches / search live here. */
  stack?: boolean;
  onShowPane?: (pane: 'tools' | 'agent') => void;
  onOpenScratches?: () => void;
  /** Kept for the header's call; the colour scheme lives in Settings and ⌘K now. */
  scheme?: { cur: QuickSchemePrefs; onChange: (next: QuickSchemePrefs) => void };
}

/** The IDE ⋯ menu: what has no other home on screen (G13, review I1-09). */
export function ideMoreMenuItems(opts: IdeMoreMenuOpts): MenuItem[] {
  return ideMoreMenuFromActions({ ...opts, stack: opts.stack === true });
}

export function showIdeMoreMenu(anchor: HTMLElement | MouseEvent, opts: IdeMoreMenuOpts): void {
  openAnchoredMenu(anchor, ideMoreMenuItems(opts));
}
