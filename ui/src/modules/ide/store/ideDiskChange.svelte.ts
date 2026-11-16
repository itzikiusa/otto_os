// A file you are EDITING can change on disk (agents write files all day). The git refresh
// reloads clean tabs silently and skips dirty ones; this watcher covers the dirty case: it
// re-reads dirty tabs when a repo reports a change, when the window regains focus, and every
// few seconds while something is unsaved, and raises a notice the editor shows as a banner
// ("<path> changed on disk." Compare / Reload from disk / Keep mine). While a notice is up
// autosave waits; if the disk returns to what the buffer was based on, the notice goes away.

import { api } from '../../../lib/api/client';
import { pollWhileVisible } from '../../../lib/poll';
import { findContainingRoot } from '../shell/ideProjectHelpers';
import { detectTextEol, diskEolsOf, normalizeEditorLf, type IdeFileReadResp, type IdeOpenTab, type IdeProjectRoot } from './ideTypes';

export interface IdeDiskNotice { diskContent: string; diskSha: string; raw: string }

export type IdeDiskVerdict =
  | { kind: 'unchanged' } // disk is still what the buffer was based on
  | { kind: 'matches-buffer' } // someone wrote exactly the unsaved text
  | { kind: 'changed'; notice: IdeDiskNotice };

/** What a fresh disk read means for a dirty tab whose buffer is `buffer`. */
export function classifyDiskRead(tab: Pick<IdeOpenTab, 'sha256' | 'savedContent'>, buffer: string, fresh: Pick<IdeFileReadResp, 'sha256' | 'content'>): IdeDiskVerdict {
  const lf = normalizeEditorLf(fresh.content);
  if (tab.sha256 ? fresh.sha256 === tab.sha256 : lf === tab.savedContent) return { kind: 'unchanged' };
  if (lf === buffer) return { kind: 'matches-buffer' };
  return { kind: 'changed', notice: { diskContent: lf, diskSha: fresh.sha256, raw: fresh.content } };
}

export interface IdeDiskWatchHost {
  tabs: IdeOpenTab[];
  readonly target: { kind: string; root: string };
  readonly projectRoots: IdeProjectRoot[];
  readonly collabIsGuest: boolean;
  getBuffer(path?: string): string;
  targetQuery(extra?: Record<string, string | boolean | null | undefined>): string;
}

/** Autosave and the save-conflict path share this set with the buffer controller. */
export type ConflictSet = Pick<Set<string>, 'add' | 'delete' | 'has'>;

const POLL_MS = 5000;

export class IdeDiskChangeWatch {
  notices = $state<Record<string, IdeDiskNotice>>({});
  private checking = false;

  has(path: string): boolean { return path in this.notices; }

  private set(path: string, notice: IdeDiskNotice | null, conflicts: ConflictSet | null): void {
    const had = path in this.notices;
    if (notice) {
      const cur = this.notices[path];
      if (!cur || cur.diskSha !== notice.diskSha) this.notices = { ...this.notices, [path]: notice };
      conflicts?.add(path);
    } else if (had) {
      const next = { ...this.notices }; delete next[path]; this.notices = next;
      conflicts?.delete(path);
    }
  }

  /** Dirty, editable, loaded tabs whose disk copy can change under them. */
  watched(host: IdeDiskWatchHost): IdeOpenTab[] {
    if (host.collabIsGuest) return [];
    return host.tabs.filter((t) => !t.loading && !t.error && !t.readOnly && !t.truncated && !t.path.startsWith('untitled:')
      && !/^(?:jdt|jar):/i.test(t.path) && host.getBuffer(t.path) !== t.savedContent);
  }

  async check(host: IdeDiskWatchHost, conflicts: ConflictSet | null, read = (url: string) => api.bg.get<IdeFileReadResp>(url)): Promise<void> {
    if (this.checking) return;
    this.checking = true;
    try {
      const watched = this.watched(host);
      const keep = new Set(watched.map((t) => t.path));
      for (const p of Object.keys(this.notices)) if (!keep.has(p)) this.set(p, null, conflicts);
      for (const tab of watched) {
        const root = findContainingRoot(tab.path, host.projectRoots)?.path ?? host.target.root;
        let fresh: IdeFileReadResp;
        try { fresh = await read(`/ide/fs/file?${host.targetQuery({ root, path: tab.path })}`); } catch { continue; }
        if (!host.tabs.includes(tab)) continue;
        const v = classifyDiskRead(tab, host.getBuffer(tab.path), fresh);
        if (v.kind === 'changed') this.set(tab.path, v.notice, conflicts);
        else {
          // Disk now holds exactly the unsaved text: the next save must not call it a conflict.
          if (v.kind === 'matches-buffer') tab.sha256 = fresh.sha256;
          this.set(tab.path, null, conflicts);
        }
      }
    } finally { this.checking = false; }
  }

  /** "Keep mine": the disk copy becomes the base, so the next save overwrites it knowingly
   *  (its expected sha is the one on disk now) instead of failing as a conflict. */
  keepMine(tab: IdeOpenTab, conflicts: ConflictSet | null): void {
    const n = this.notices[tab.path];
    if (!n) return;
    tab.sha256 = n.diskSha; tab.savedContent = n.diskContent;
    tab.eol = detectTextEol(n.raw); tab.diskEols = diskEolsOf(n.raw);
    this.set(tab.path, null, conflicts);
  }

  /** After "Reload from disk" (the reload marks the tab clean) or a closed tab. */
  forget(path: string, conflicts: ConflictSet | null): void { this.set(path, null, conflicts); }

  /** Starts the triggers; returns the stop function. `liveRev` is read reactively by the caller. */
  start(host: IdeDiskWatchHost, conflicts: () => ConflictSet | null): () => void {
    if (typeof window === 'undefined') return () => {};
    const run = (): Promise<void> | void => { if (this.watched(host).length || Object.keys(this.notices).length) return this.check(host, conflicts()); };
    const poller = pollWhileVisible(() => run(), { ms: POLL_MS, immediate: false });
    const onFocus = (): void => { void run(); };
    window.addEventListener('focus', onFocus);
    return () => { poller.stop(); window.removeEventListener('focus', onFocus); };
  }
}

export const ideDiskChange = new IdeDiskChangeWatch();
