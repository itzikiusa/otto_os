import type { PromptImage, RuntimeAccessMode, RuntimeComposerContext } from '../api/types';
import type { EffortLevel } from '../providerModels';

/** An attachment waiting in a runtime composer. */
export type ComposerFile = PromptImage & { name: string };

export interface RuntimeComposerDraft {
  text: string;
  freshContext: boolean;
  context: RuntimeComposerContext[];
  recovered: boolean;
  saveStatus: 'loading' | 'saving' | 'saved' | 'error';
  saveError: string | null;
  /** Why the last attachment(s) were not added (size or count), shown inline. */
  attachError: string | null;
  retrySave: () => void;
  flushSave: () => void;
  images: ComposerFile[];
  /** `undefined` = inherit `session.model`; `null` = explicit "Default model". */
  model: string | null | undefined;
  effort: EffortLevel | null;
  pickedMode: RuntimeAccessMode | null;
  sending: boolean;
  reopening: boolean;
  stopping: boolean;
  awaitSeq: number | null;
  startError: string | null;
}

export type ComposerContext = RuntimeComposerContext;

/** What the installed writer (runtimeDraftRecovery) gives back for one draft. */
export interface DraftRoot {
  /** Stop everything. `flush`: save unsaved content first (an idle draft
   *  leaving memory); the returned promise settles when that is done. */
  dispose(flush?: boolean): void | Promise<void>;
  /** A view shows this draft: start the live sync, returning its stop. */
  activate?(): () => void;
}

/** Drafts no view shows that stay in memory, so going back is instant. The
 *  rest are saved and dropped: the daemon has every draft. */
export const IDLE_DRAFTS_KEPT = 6;

/** The live draft owns action flags; the installed daemon writer persists content only. */
export class RuntimeDrafts {
  private byId = $state<Record<string, RuntimeComposerDraft>>({});
  private initialize: ((id: string, draft: RuntimeComposerDraft, after?: Promise<void>) => DraftRoot | (() => void)) | null = null;
  private roots = new Map<string, DraftRoot>();
  /** Views holding each draft (runtimeViews.open), and their live-sync stops. */
  private holds = new Map<string, number>();
  private live = new Map<string, () => void>();
  /** Drafts no view holds, least recently released first. */
  private idle: string[] = [];
  /** Evicted drafts still saving: a re-open loads after the save lands. */
  private settling = new Map<string, Promise<void>>();
  configure(initialize: (id: string, draft: RuntimeComposerDraft, after?: Promise<void>) => DraftRoot | (() => void)): void {
    this.initialize = initialize;
  }
  private identity: string | null = null;
  /** Bumped on every identity change: an evicted draft's last save stops with it. */
  epoch = 0;

  setIdentity(identity: string): void {
    if (this.identity === identity) return;
    this.identity = identity;
    this.clear();
  }

  owns(id: string, draft: RuntimeComposerDraft): boolean {
    return this.byId[id] === draft;
  }

  /** Whether a view shows `id` now (only those sync on focus and turn events). */
  held(id: string): boolean {
    return (this.holds.get(id) ?? 0) > 0;
  }

  clear(): void {
    this.epoch++;
    for (const stop of this.live.values()) stop();
    for (const root of this.roots.values()) void root.dispose();
    this.live.clear();
    this.roots.clear();
    this.idle = [];
    this.settling.clear();
    this.byId = {};
  }

  forget(id: string): void {
    this.live.get(id)?.();
    this.live.delete(id);
    void this.roots.get(id)?.dispose();
    this.roots.delete(id);
    this.idle = this.idle.filter((x) => x !== id);
    delete this.byId[id];
  }

  /** A view shows `id`'s thread: keep its draft live until the returned release. */
  hold(id: string): () => void {
    const n = (this.holds.get(id) ?? 0) + 1;
    this.holds.set(id, n);
    if (n === 1) {
      this.idle = this.idle.filter((x) => x !== id);
      this.startLive(id);
    }
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const left = (this.holds.get(id) ?? 1) - 1;
      if (left > 0) {
        this.holds.set(id, left);
        return;
      }
      this.holds.delete(id);
      this.live.get(id)?.();
      this.live.delete(id);
      if (this.byId[id]) this.touchIdle(id);
    };
  }

  private startLive(id: string): void {
    const stop = this.byId[id] ? this.roots.get(id)?.activate?.() : undefined;
    if (stop) this.live.set(id, stop);
  }

  private touchIdle(id: string): void {
    this.idle = [...this.idle.filter((x) => x !== id), id];
    this.evictIdle(id);
  }

  /** Drop the oldest idle drafts past {@link IDLE_DRAFTS_KEPT}, saving them
   *  first. One that is mid-send or failed to save stays: its content may
   *  exist nowhere else. */
  private evictIdle(keep: string): void {
    let excess = this.idle.length - IDLE_DRAFTS_KEPT;
    for (const id of [...this.idle]) {
      if (excess <= 0) break;
      const d = this.byId[id];
      if (id === keep) continue;
      if (d && (d.sending || d.reopening || d.stopping || d.saveStatus === 'error')) continue;
      excess--;
      this.idle = this.idle.filter((x) => x !== id);
      const done = this.roots.get(id)?.dispose(true);
      this.roots.delete(id);
      delete this.byId[id];
      if (done) {
        const settled = Promise.resolve(done).catch(() => {});
        this.settling.set(id, settled);
        void settled.then(() => {
          if (this.settling.get(id) === settled) this.settling.delete(id);
        });
      }
    }
  }

  for(id: string): RuntimeComposerDraft {
    if (this.byId[id]) return this.byId[id];
    this.byId[id] = {
      freshContext: false, context: [], recovered: false, saveStatus: 'loading', saveError: null, attachError: null, retrySave: () => {}, flushSave: () => {},
      text: '', images: [], model: undefined, effort: null, pickedMode: null,
      sending: false, reopening: false, stopping: false, awaitSeq: null, startError: null,
    };
    const root = this.initialize?.(id, this.byId[id], this.settling.get(id));
    if (root) this.roots.set(id, typeof root === 'function' ? { dispose: root } : root);
    if (this.held(id)) this.startLive(id);
    else this.touchIdle(id);
    return this.byId[id];
  }
}

export const runtimeDrafts = new RuntimeDrafts();
