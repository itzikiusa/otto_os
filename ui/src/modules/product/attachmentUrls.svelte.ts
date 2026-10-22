// Authed object-URL cache for product attachments (`/product/attachments/{id}`),
// shared by the Overview body preview (pasted `attachment:<id>` screenshots)
// and AttachmentsPanel's thumbnails — one in-flight map, one generation guard
// and one revoke path instead of a hand-rolled copy in each component.
//
//   const atts = new AttachmentUrls();
//   const url = await atts.load(id);   // cached, de-duplicated; null on failure
//   atts.reset();                      // story switch: revoke + drop everything
//   atts.dispose();                    // unmount
import { authedBlobUrl } from '../../lib/api/client';
import { loadErrorText } from '../../lib/loadError';

export class AttachmentUrls {
  /** id → object URL, for templates. */
  urls = $state<Record<string, string>>({});
  /** id → a fetch is in flight. */
  loading = $state<Record<string, boolean>>({});
  /** id → why the last fetch failed. */
  errors = $state<Record<string, string>>({});
  #created: string[] = [];
  #inflight = new Map<string, Promise<string | null>>();
  /** Bumped by reset(): a fetch that settles after it frees its blob. */
  #gen = 0;

  /** The object URL for `id`, fetched once (concurrent callers share the
   *  request). `null` when the fetch failed or a reset() superseded it. */
  load(id: string): Promise<string | null> {
    const have = this.urls[id];
    if (have) return Promise.resolve(have);
    const pending = this.#inflight.get(id);
    if (pending) return pending;
    const gen = this.#gen;
    this.loading = { ...this.loading, [id]: true };
    const { [id]: _cleared, ...restErr } = this.errors;
    this.errors = restErr;
    const run = (async (): Promise<string | null> => {
      try {
        const url = await authedBlobUrl(`/product/attachments/${id}`);
        if (gen !== this.#gen) {
          URL.revokeObjectURL(url); // stale — never cached, so free it now
          return null;
        }
        this.#created.push(url);
        this.urls = { ...this.urls, [id]: url };
        return url;
      } catch (e) {
        if (gen === this.#gen) this.errors = { ...this.errors, [id]: loadErrorText(e) };
        return null;
      } finally {
        if (gen === this.#gen) {
          this.#inflight.delete(id);
          this.loading = { ...this.loading, [id]: false };
        }
      }
    })();
    this.#inflight.set(id, run);
    return run;
  }

  /** Drop (and revoke) one cached URL — the attachment was deleted. */
  forget(id: string): void {
    const url = this.urls[id];
    if (url) {
      URL.revokeObjectURL(url);
      this.#created = this.#created.filter((u) => u !== url);
    }
    const { [id]: _removed, ...rest } = this.urls;
    this.urls = rest;
  }

  /** Revoke every URL and forget all state; in-flight fetches are dropped. */
  reset(): void {
    ++this.#gen;
    const stale = this.#created.splice(0);
    for (const url of stale) URL.revokeObjectURL(url);
    this.#inflight.clear();
    this.urls = {};
    this.loading = {};
    this.errors = {};
  }

  /** Unmount: the same as reset(). */
  dispose(): void {
    this.reset();
  }
}
