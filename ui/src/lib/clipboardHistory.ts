// Bridge to the desktop shell's clipboard history (apps/desktop/src-tauri/
// src/clipboard). Status + settings work from any Otto window; the content
// calls (list, text, image, icon, paste, pin, delete) are refused by the
// shell unless they come from the panel window itself (`otto-clipboard`).
// Not in lib/desktop.ts on purpose: only the panel and its Settings section
// load this, so the entry bundle doesn't grow.

import { isTauri } from './desktop';

export type ClipKind = 'text' | 'link' | 'file' | 'image';

export interface ClipEntry {
  id: string;
  kind: ClipKind;
  /** Text / link (first 300 characters), or file paths one per line. */
  text: string;
  width: number;
  height: number;
  size: number;
  source: { name: string; bundle_id: string | null } | null;
  first_ms: number;
  last_ms: number;
  count: number;
  pinned: boolean;
}

export interface ClipStatus {
  enabled: boolean;
  paused: boolean;
  /** macOS "Paste from Other Apps" for Otto: allowed (or never asked), ask, denied. */
  access: 'allowed' | 'ask' | 'denied';
  /** Accessibility granted: Return pastes; otherwise it copies only. */
  accessibility: boolean;
  count: number;
  /** The app Return pastes into (frontmost when the panel opened). */
  target: string | null;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri) throw new Error('Clipboard history needs the Otto desktop app.');
  const { invoke } = await import('@tauri-apps/api/core');
  return invoke<T>(cmd, args);
}

export const clip = {
  status: () => call<ClipStatus>('clipboard_status'),
  set: (o: { enabled?: boolean; paused?: boolean }) =>
    call<ClipStatus>('clipboard_set', { enabled: o.enabled ?? null, paused: o.paused ?? null }),
  clear: () => call<ClipStatus>('clipboard_clear'),
  openSettings: (pane: 'accessibility' | 'paste') => call<void>('clipboard_open_settings', { pane }),
  hide: () => call<void>('clipboard_hide'),
  list: (query: string, kind: ClipKind | null) => call<ClipEntry[]>('clipboard_list', { query, kind }),
  text: (id: string) => call<string>('clipboard_text', { id }),
  image: (id: string, thumb: boolean) => call<ArrayBuffer>('clipboard_image', { id, thumb }),
  icon: (bundleId: string) => call<ArrayBuffer>('clipboard_icon', { bundleId }),
  /** Resolves true when it pasted into the app in front, false when it only copied. */
  paste: (id: string, paste: boolean) => call<boolean>('clipboard_paste', { id, paste }),
  pin: (id: string, pinned: boolean) => call<void>('clipboard_pin', { id, pinned }),
  remove: (id: string) => call<void>('clipboard_delete', { id }),
};
