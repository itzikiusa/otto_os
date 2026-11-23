// Live project tree: the side bar follows the folder on disk by itself.
//
// The tree lists a folder once per expand, so a file a terminal, an agent or `git checkout`
// created stayed invisible until the folder was closed and re-opened. Each tree holds a lease
// on the daemon (`POST /ide/fs/watch`: its local project roots + the folders it shows); one
// OS watcher per root answers with `ide_fs_changed` naming the folders whose entries changed,
// and only those are re-listed — in place, so scroll, selection and expansion stay put. A
// folder open but scrolled off screen is re-listed when it comes back on screen.
// Window focus re-lists every shown folder when nothing watches them — a Remote SSH target
// (nothing watches a remote disk) or a root the daemon would not watch (`/`, `/Users`) — and a
// socket resync always does (events may have been missed).

import { api } from '../../../lib/api/client';
import type { IdeFsWatchReq, IdeFsWatchResp } from '../../../lib/api/types';
import { newId } from '../../../lib/id';

export interface IdeTreeLiveHost {
  readonly target: { kind: string; root: string };
  readonly projectRoots: readonly { path: string }[];
  readonly additionalRoots: readonly string[];
  readonly expandedDirs: Readonly<Record<string, boolean>>;
  readonly childrenByDir: Readonly<Record<string, unknown>>;
  readonly collabIsGuest: boolean;
  /** Re-read one folder's listing in place. */
  relistDir(dir: string): Promise<void>;
}

/** The daemon drops a lease not re-posted for 3 min. */
export const LEASE_HEARTBEAT_MS = 60_000;
/** Focus / resync re-lists at most this often. */
const REFRESH_GAP_MS = 2_000;

const trimSlash = (p: string): string => p.replace(/\/+$/, '') || '/';

/** Folders the tree shows a listing of: the primary root, expanded folders, and attached
 *  roots that are open (an unset flag on a loaded root means its default-open state). */
export function shownDirs(host: IdeTreeLiveHost): string[] {
  const out = new Set<string>();
  if (host.target.root) out.add(trimSlash(host.target.root));
  for (const [dir, open] of Object.entries(host.expandedDirs)) if (open) out.add(dir);
  for (const r of host.additionalRoots) if (host.expandedDirs[r] === undefined && host.childrenByDir[r]) out.add(r);
  return [...out].sort();
}

export class IdeTreeLive {
  readonly clientId: string;
  private posted = '';
  private postedDirs = new Set<string>();
  private inFlight = new Set<string>();
  private again = new Set<string>();
  private lastRefresh = 0;
  /** Every root of the posted lease has a live watcher. */
  private watching = false;
  /** Folders whose listing is on screen (null until the tree says: then all shown ones). */
  private screen: ReadonlySet<string> | null = null;
  /** Shown folders that may have changed off screen: re-listed when they come on screen. */
  private stale = new Set<string>();

  constructor(
    private host: IdeTreeLiveHost,
    private post: (body: IdeFsWatchReq) => Promise<IdeFsWatchResp | undefined> = (body) => api.bg.post<IdeFsWatchResp>('/ide/fs/watch', body),
    clientId: string = newId(),
  ) {
    this.clientId = clientId;
  }

  /** Post the lease when what the tree shows changed (`force`: the heartbeat). Only local
   *  targets are watched; leaving one releases its lease. */
  sync(force = false): void {
    const h = this.host;
    const local = h.target.kind === 'local' && Boolean(h.target.root) && !h.collabIsGuest;
    if (!local) return this.release();
    const body: IdeFsWatchReq = { client_id: this.clientId, roots: h.projectRoots.map((r) => trimSlash(r.path)), dirs: shownDirs(h) };
    const key = JSON.stringify(body);
    if (!force && key === this.posted) return;
    this.posted = key;
    const before = this.postedDirs;
    this.postedDirs = new Set(body.dirs);
    for (const d of this.stale) if (!this.postedDirs.has(d)) this.stale.delete(d);
    // A change between a folder's listing and the watcher learning of it is caught here (a
    // folder not listed yet is read fresh when it shows).
    const listed = body.dirs.filter((d) => !before.has(d) && h.childrenByDir[d]);
    this.post(body).then(
      (resp) => {
        if (this.posted === key) this.watching = (resp?.watched.length ?? 0) === body.roots.length;
        for (const d of listed) this.refresh(d);
      },
      () => { if (this.posted === key) { this.posted = ''; this.postedDirs = before; this.watching = false; } }, // retried on the next sync
    );
  }

  /** Drop this tree's lease (project closed, tree unmounted). */
  release(): void {
    if (!this.posted) return;
    this.posted = '';
    this.postedDirs = new Set();
    this.stale.clear();
    this.watching = false;
    this.post({ client_id: this.clientId, roots: [], dirs: [] }).catch(() => {}); // the daemon expires it anyway
  }

  /** An `ide_fs_changed` event: re-list the named folders this tree still shows. */
  apply(clientId: string, dirs: readonly string[]): void {
    if (clientId !== this.clientId) return;
    const shown = new Set(shownDirs(this.host));
    for (const d of dirs) if (shown.has(d)) this.refresh(d);
  }

  /** The tree's visible window: re-list the stale folders now on screen. */
  onScreen(dirs: ReadonlySet<string>): void {
    this.screen = dirs;
    for (const d of this.stale) if (dirs.has(d)) { this.stale.delete(d); this.relist(d); }
  }

  /** Back to the window: renew the lease; re-list everything when nothing watches it. */
  onFocus(now = Date.now()): void {
    this.sync(true);
    if (!this.watching) this.refreshAll(now);
  }

  /** Events may have been lost (socket reconnect / lag): renew the lease, re-list everything. */
  onResync(now = Date.now()): void {
    this.sync(true);
    this.refreshAll(now);
  }

  /** Re-list every shown folder (throttled). */
  refreshAll(now = Date.now()): void {
    if (now - this.lastRefresh < REFRESH_GAP_MS) return;
    this.lastRefresh = now;
    for (const d of shownDirs(this.host)) this.refresh(d);
  }

  /** Re-list `dir` now when it is on screen, else once it comes on screen. */
  private refresh(dir: string): void {
    if (!this.screen || this.screen.has(dir)) this.relist(dir);
    else this.stale.add(dir);
  }

  /** One listing per folder at a time; a change landing meanwhile re-reads once more after. */
  private relist(dir: string): void {
    if (this.inFlight.has(dir)) { this.again.add(dir); return; }
    this.inFlight.add(dir);
    void this.host.relistDir(dir).catch(() => {}).finally(() => {
      this.inFlight.delete(dir);
      if (this.again.delete(dir)) this.relist(dir);
    });
  }
}
