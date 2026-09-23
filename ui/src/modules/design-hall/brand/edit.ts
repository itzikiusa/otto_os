// Brand Kit editor — the small interactive helpers the sections share: the
// per-token "⋯" menu (rename, describe, copy reference / CSS variable, delete),
// naming prompts and copy-with-toast. Everything goes through the shared
// confirmer / ctxMenu / toasts (no native dialogs, no hand-rolled popups).

import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { copyText } from '../../../lib/clipboard';
import type { BrandTokenGroup } from '../../../lib/api/types';
import { cssVarName, isValidTokenName, tokenLabel, tokenRef } from './tokens';

export async function copyWithToast(text: string, what: string): Promise<void> {
  if (await copyText(text)) toasts.success(`Copied ${what}`, text.length > 80 ? undefined : text);
  else toasts.error('Couldn’t copy', 'The clipboard isn’t available here.');
}

/** Ask for a token name; null when cancelled, invalid (explained) or taken. */
export async function askTokenName(
  title: string,
  taken: Iterable<string>,
  opts: { initial?: string; placeholder?: string; confirmLabel?: string } = {},
): Promise<string | null> {
  const name = await confirmer.promptText('Letters, digits, - or _ (e.g. primary, surface-alt, 2xl). Studios reference it by this name.', {
    title,
    confirmLabel: opts.confirmLabel ?? 'Save',
    initial: opts.initial ?? '',
    placeholder: opts.placeholder ?? '',
  });
  if (!name) return null;
  const n = name.trim();
  if (n === opts.initial) return null;
  if (!isValidTokenName(n)) {
    toasts.warn('That name won’t work', 'Use letters, digits, - or _, starting with a letter or digit (at most 64).');
    return null;
  }
  if (new Set(taken).has(n)) {
    toasts.warn('That name is taken', `There is already a token called “${n}”.`);
    return null;
  }
  return n;
}

export interface TokenMenuActions {
  rename?: () => void;
  describe?: () => void;
  remove?: () => void;
}

/** The "⋯" menu of one token (viewport-clamped via the global ctxMenu). */
export function tokenMenu(e: MouseEvent, group: BrandTokenGroup, name: string, actions: TokenMenuActions, readonly: boolean): void {
  const items: MenuItem[] = [
    { label: 'Copy token reference', icon: 'copy', action: () => void copyWithToast(tokenRef(group, name), 'token reference') },
    {
      label: 'Copy CSS variable',
      icon: 'copy',
      action: () => void copyWithToast(`var(${cssVarName(group, name)}${group === 'type' ? '-size' : ''})`, 'CSS variable'),
    },
  ];
  if (!readonly && (actions.rename || actions.describe || actions.remove)) items.push({ separator: true });
  if (!readonly && actions.rename) items.push({ label: 'Rename…', icon: 'edit', action: actions.rename });
  if (!readonly && actions.describe) items.push({ label: 'Describe…', icon: 'note', action: actions.describe });
  if (!readonly && actions.remove) items.push({ label: `Delete ${tokenLabel(name).toLowerCase()}`, icon: 'trash', danger: true, action: actions.remove });
  ctxMenu.show(e, items);
}

/** Ask for a token description ("" clears it); null when cancelled. */
export async function askDescription(name: string, current: string | undefined): Promise<string | null> {
  const v = await confirmer.promptText(`What is “${name}” for? Agents and people read this when they pick a token.`, {
    title: 'Describe token',
    confirmLabel: 'Save',
    initial: current ?? '',
    placeholder: 'Buttons, links and the one thing to click',
  });
  return v === null ? null : v.trim();
}
