// The open design's version history, paged: `GET …/versions?offset=&limit=`
// answers newest first with the full count in `X-Total-Count`. The artifact
// view holds one of these (the Otto panel, links and Compare read `items`);
// the Versions panel pages it with "Load older". Every loader is
// sequence-guarded so a stale answer never replaces a newer one.
import { authedResponse } from '../../lib/api/client';
import { listVersions, qs } from '../../lib/api/design';
import type { DesignVersion } from '../../lib/api/types';
import { loadErrorText } from '../../lib/loadError';
import { mergeOlder, totalCountOf } from './versionRows';

export const VERSIONS_PAGE = 200;

/** One page of versions plus the server's total (null when not sent). */
export async function listVersionsPage(id: string, offset: number, limit = VERSIONS_PAGE): Promise<{ items: DesignVersion[]; total: number | null }> {
  const resp = await authedResponse(`/design/artifacts/${encodeURIComponent(id)}/versions${qs({ offset, limit })}`, { accept: 'application/json' });
  const items = (await resp.json()) as DesignVersion[];
  return { items, total: totalCountOf(resp.headers.get('X-Total-Count')) };
}

export class VersionList {
  items = $state<DesignVersion[]>([]);
  total = $state<number | null>(null);
  loading = $state(false);
  loadingOlder = $state(false);
  error = $state<string | null>(null);
  #id = '';
  #seq = 0;

  /** More versions exist on the server than are held. */
  get hasOlder(): boolean {
    return this.total != null && this.items.length < this.total;
  }

  reset(id: string): void {
    this.#id = id;
    this.#seq++;
    this.items = [];
    this.total = null;
    this.error = null;
    this.loading = false;
    this.loadingOlder = false;
  }

  /** (Re)load the newest page. */
  async load(): Promise<void> {
    const my = ++this.#seq;
    const id = this.#id;
    this.loading = true;
    try {
      const page = await listVersionsPage(id, 0);
      if (my !== this.#seq) return;
      this.items = page.items;
      this.total = page.total;
      this.error = null;
    } catch (e) {
      if (my === this.#seq) this.error = loadErrorText(e);
    } finally {
      if (my === this.#seq) this.loading = false;
    }
  }

  /**
   * A live `content` event only ADDS versions at the head: fetch the newest
   * few and prepend the unseen ones instead of re-listing the whole history.
   * No overlap with what is held (a gap, or nothing loaded yet) → full reload.
   */
  async loadNew(): Promise<void> {
    if (this.items.length === 0) return this.load();
    const my = ++this.#seq;
    try {
      const v = await listVersions(this.#id, { limit: 10 });
      if (my !== this.#seq) return;
      const have = new Set(this.items.map((x) => x.id));
      const fresh = v.filter((x) => !have.has(x.id));
      if (fresh.length === v.length) return this.load();
      if (fresh.length) {
        this.items = [...fresh, ...this.items];
        if (this.total != null) this.total += fresh.length;
      }
      this.error = null;
    } catch {
      if (my === this.#seq) return this.load();
    } finally {
      // This bumped the sequence, so a superseded full load won't clear it.
      if (my === this.#seq) this.loading = false;
    }
  }

  /** "Load older": the next page after the ones held. */
  async loadOlder(): Promise<void> {
    if (this.loadingOlder) return;
    const my = this.#seq;
    this.loadingOlder = true;
    try {
      const page = await listVersionsPage(this.#id, this.items.length);
      if (my !== this.#seq) return;
      this.items = mergeOlder(this.items, page.items);
      if (page.total != null) this.total = page.total;
      this.error = null;
    } catch (e) {
      if (my === this.#seq) this.error = loadErrorText(e);
    } finally {
      this.loadingOlder = false;
    }
  }

  /** A save just made this version: hold it at the top without a refetch. */
  prepend(v: DesignVersion): void {
    if (this.items.some((x) => x.id === v.id)) return;
    this.items = [v, ...this.items];
    if (this.total != null) this.total += 1;
  }
}
