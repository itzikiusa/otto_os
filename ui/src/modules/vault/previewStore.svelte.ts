// Hover / focus page previews for vault links — one popover per note view,
// shared by the structured panel's link chips and the reading view's inline
// wikilinks. Previews are cached per (ws, vault, path); a busy daemon (409)
// or a missing note shows "Preview unavailable" instead of an error toast.

import { vaultNote } from '../../lib/api/vault';
import type { VaultNote } from '../../lib/api/types';
import { previewText } from './structuredNote';
import { vault } from './vault.svelte';

const cache = new Map<string, Promise<VaultNote>>();
const CACHE_MAX = 100;
const DELAY_MS = 300;

export interface PreviewState {
  path: string;
  /** Anchor rect (viewport coordinates) the popover hangs under. */
  x: number;
  y: number;
  top: number;
  title: string;
  type: string | null;
  text: string;
  state: 'loading' | 'ok' | 'error';
}

function load(path: string): Promise<VaultNote> {
  const v = vault.current!;
  const key = `${vault.wsId}:${v.id}:${vault.status?.generation ?? vault.status?.last_scan_at ?? ""}:${path}`;
  let p = cache.get(key);
  if (!p) {
    p = vaultNote(vault.wsId, v.id, path);
    p.catch(() => { if (cache.get(key) === p) cache.delete(key); });
    cache.set(key, p);
    while (cache.size > CACHE_MAX) cache.delete(cache.keys().next().value!);
  }
  return p;
}

class LinkPreview {
  current = $state<PreviewState | null>(null);
  #timer: ReturnType<typeof setTimeout> | undefined;
  #generation = 0;

  show(anchor: Element, path: string | null | undefined): void {
    clearTimeout(this.#timer);
    const request = ++this.#generation;
    this.current = null;
    if (!path || !/\.md$/i.test(path) || !vault.current || path === vault.notePath) return;
    const workspace = vault.wsId, id = vault.current.id;
    const current = () => request === this.#generation && vault.wsId === workspace && vault.current?.id === id;
    const r = anchor.getBoundingClientRect();
    this.#timer = setTimeout(() => {
      if (!current()) return;
      this.current = {
        path, x: r.left, y: r.bottom + 6, top: r.top,
        title: path.split('/').pop()!.replace(/\.md$/i, ''), type: null, text: '', state: 'loading',
      };
      load(path).then(
        (n) => {
          if (!current() || this.current?.path !== path) return;
          this.current = { ...this.current, title: n.meta.title, type: n.meta.okf_type, text: n.meta.description || previewText(n.raw), state: 'ok' };
        },
        () => {
          if (current() && this.current?.path === path) this.current = { ...this.current, state: 'error' };
        },
      );
    }, DELAY_MS);
  }

  hide(): void {
    this.#generation++;
    clearTimeout(this.#timer);
    this.current = null;
  }
}

export const linkPreview = new LinkPreview();
