// One design's review comments, shared by the stage's pin layer (PinHost)
// and the details panel's Comments tab: the loaded rows, comment mode, the
// pin dropped while its text is typed, and the selected thread (a pin and its
// list entry highlight together). Loads are sequence-guarded; a write bumps
// the sequence so an older in-flight list can't drop the row just written.
// Other tabs' writes arrive as `design_artifact_updated {change:"comments"}`
// and coalesce into one reload (`follow`).

import * as api from '../../../lib/api/design';
import { loadErrorText } from '../../../lib/loadError';
import type { DesignComment, DesignCommentPin, UpdateDesignCommentReq } from '../../../lib/api/types';
import { threadsOf, type Thread } from './pins';

/** The slice of `designBus` the store reads (injected: no import cycle, testable). */
export interface CommentBus {
  seq: number;
  resyncTick: number;
  since(after: number): readonly { type: string; artifact_id?: string | null; change?: string }[];
}

const RELOAD_MS = 250;

export class CommentsStore {
  readonly artifactId: string;
  items: DesignComment[] = $state([]);
  loading = $state(false);
  loaded = $state(false);
  error: string | null = $state(null);
  /** Comment mode: a click on the stage drops a pin. */
  mode = $state(false);
  /** A pin dropped on the stage whose text is being typed. */
  draft: DesignCommentPin | null = $state(null);
  /** The highlighted thread (its pin and its list entry). */
  selectedId: string | null = $state(null);
  #seq = 0;
  #seen = -1;
  #tick = -1;
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(artifactId: string) {
    this.artifactId = artifactId;
  }

  get threads(): Thread[] {
    return threadsOf(this.items);
  }

  async load(): Promise<void> {
    const my = ++this.#seq;
    this.loading = true;
    try {
      const rows = await api.listComments(this.artifactId);
      if (my !== this.#seq) return;
      this.items = rows;
      this.loaded = true;
      this.error = null;
    } catch (e) {
      if (my === this.#seq) this.error = loadErrorText(e);
    } finally {
      if (my === this.#seq) this.loading = false;
    }
  }

  /** A view mounted on this design: load (or, for a cached store, refresh
   *  quietly — events may have passed while nothing followed). The stage
   *  passes `fresh` to start out of comment mode. */
  open(bus: CommentBus, fresh = false): void {
    if (fresh) this.exitMode();
    if (this.#seen < 0) return this.follow(bus);
    this.#seen = bus.seq;
    this.#tick = bus.resyncTick;
    this.reloadSoon();
  }

  /** First call: load once. Later: reload when the bus says this design's
   *  comments changed, or after a reconnect (events may have been lost). */
  follow(bus: CommentBus): void {
    const now = bus.seq;
    const tick = bus.resyncTick;
    if (this.#seen < 0) {
      this.#seen = now;
      this.#tick = tick;
      if (!this.loaded && !this.loading) void this.load();
      return;
    }
    let reload = tick !== this.#tick;
    this.#tick = tick;
    if (now > this.#seen) {
      const evs = bus.since(this.#seen);
      this.#seen = now;
      reload ||= evs.some((e) => e.type === 'design_artifact_updated' && e.artifact_id === this.artifactId && (e.change === 'comments' || e.change === 'deleted'));
    }
    if (reload) this.reloadSoon();
  }

  reloadSoon(): void {
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => void this.load(), RELOAD_MS);
  }

  #upsert(c: DesignComment): void {
    this.#seq++; // an older in-flight list must not undo this write
    this.loading = false;
    const i = this.items.findIndex((x) => x.id === c.id);
    this.items = i < 0 ? [...this.items, c] : this.items.map((x) => (x.id === c.id ? c : x));
  }

  /** A new thread (with an optional pin) on `versionId`, or a reply. */
  async create(body: string, opts: { pin?: DesignCommentPin | null; versionId?: string | null; parentId?: string } = {}): Promise<DesignComment> {
    const c = await api.createComment(this.artifactId, {
      body,
      ...(opts.parentId ? { parent_id: opts.parentId } : {}),
      ...(!opts.parentId && opts.versionId ? { version_id: opts.versionId } : {}),
      ...(!opts.parentId && opts.pin ? { pin: { x: opts.pin.x, y: opts.pin.y, ...(opts.pin.frame ? { frame: opts.pin.frame } : {}) } } : {}),
    });
    this.#upsert(c);
    return c;
  }

  async update(id: string, patch: UpdateDesignCommentReq): Promise<DesignComment> {
    const c = await api.updateComment(this.artifactId, id, patch);
    this.#upsert(c);
    return c;
  }

  /** Delete a comment; a thread takes its replies. */
  async remove(id: string): Promise<void> {
    await api.deleteComment(this.artifactId, id);
    this.#seq++;
    this.loading = false;
    this.items = this.items.filter((x) => x.id !== id && x.parent_id !== id);
    if (this.selectedId === id) this.selectedId = null;
  }

  /** Leave comment mode (and drop an untyped pin). */
  exitMode(): void {
    this.mode = false;
    this.draft = null;
  }

  dispose(): void {
    clearTimeout(this.#timer);
  }
}

// One store per open design, shared by the stage and the panel. A handful of
// recent designs stay cached so going back shows their threads at once.
const stores = new Map<string, CommentsStore>();
const KEEP = 6;

export function commentsFor(artifactId: string): CommentsStore {
  let s = stores.get(artifactId);
  if (s) {
    stores.delete(artifactId);
    stores.set(artifactId, s); // most recent last
    return s;
  }
  s = new CommentsStore(artifactId);
  stores.set(artifactId, s);
  while (stores.size > KEEP) {
    const [oldest, old] = stores.entries().next().value as [string, CommentsStore];
    old.dispose();
    stores.delete(oldest);
  }
  return s;
}
