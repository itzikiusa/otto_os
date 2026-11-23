// Lightweight workspace-scoped bookmarks / quick URLs for the Browser module.
// Stored in localStorage per workspace so dev URLs (docs, staging, previews)
// are one click away without adding database migrations or toolbar clutter.

export interface BrowserBookmark {
  url: string;
  title: string;
  addedAt: string;
}

const MAX_BOOKMARKS = 40;
const storageKey = (wsId: string) => `otto_browser_bookmarks_${wsId}`;

export function normalizeBookmarkUrl(raw: string): string | null {
  const t = raw.trim();
  if (!t) return null;
  const candidate = /^[a-z][a-z0-9+.-]*:\/\//i.test(t) ? t : `https://${t}`;
  try {
    const u = new URL(candidate);
    if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
    return u.toString();
  } catch {
    return null;
  }
}

export function shortHostLabel(url: string): string {
  try {
    const u = new URL(url);
    const path = u.pathname && u.pathname !== '/' ? u.pathname : '';
    return `${u.host}${path}`.slice(0, 64);
  } catch {
    return url.slice(0, 64);
  }
}

function readStored(wsId: string): BrowserBookmark[] {
  if (!wsId) return [];
  try {
    const raw = localStorage.getItem(storageKey(wsId));
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed
      .filter((x): x is BrowserBookmark => !!x && typeof x.url === 'string' && typeof x.title === 'string')
      .slice(0, MAX_BOOKMARKS);
  } catch {
    return [];
  }
}

function writeStored(wsId: string, list: BrowserBookmark[]): void {
  if (!wsId) return;
  try {
    localStorage.setItem(storageKey(wsId), JSON.stringify(list.slice(0, MAX_BOOKMARKS)));
  } catch {
    /* private mode / quota */
  }
}

class BookmarksStore {
  private wsId = '';
  items: BrowserBookmark[] = $state([]);

  load(wsId: string): void {
    if (this.wsId === wsId) return;
    this.wsId = wsId;
    this.items = readStored(wsId);
  }

  has(url: string | null | undefined): boolean {
    if (!url) return false;
    const norm = normalizeBookmarkUrl(url);
    if (!norm) return false;
    return this.items.some((b) => b.url === norm);
  }

  add(url: string, title?: string): boolean {
    const norm = normalizeBookmarkUrl(url);
    if (!norm || !this.wsId) return false;
    const cleanTitle = (title || '').trim() || shortHostLabel(norm);
    const next = [
      { url: norm, title: cleanTitle.slice(0, 120), addedAt: new Date().toISOString() },
      ...this.items.filter((b) => b.url !== norm),
    ].slice(0, MAX_BOOKMARKS);
    this.items = next;
    writeStored(this.wsId, next);
    return true;
  }

  remove(url: string): void {
    const norm = normalizeBookmarkUrl(url) ?? url;
    if (!this.wsId) return;
    this.items = this.items.filter((b) => b.url !== norm);
    writeStored(this.wsId, this.items);
  }

  toggle(url: string, title?: string): boolean {
    if (this.has(url)) {
      this.remove(url);
      return false;
    }
    return this.add(url, title);
  }
}

export const browserBookmarks = new BookmarksStore();
