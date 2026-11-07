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
  retrySave: () => void;
  flushSave: () => void;
  images: ComposerFile[];
  model: string | null;
  effort: EffortLevel | null;
  pickedMode: RuntimeAccessMode | null;
  sending: boolean;
  reopening: boolean;
  stopping: boolean;
  awaitSeq: number | null;
  startError: string | null;
}

export type ComposerContext = RuntimeComposerContext;

/** The live draft owns action flags; the installed daemon writer persists content only. */
export class RuntimeDrafts {
  private byId = $state<Record<string, RuntimeComposerDraft>>({});
  private initialize: ((id: string, draft: RuntimeComposerDraft) => () => void) | null = null;
  private cleanups = new Map<string, () => void>();
  configure(initialize: (id: string, draft: RuntimeComposerDraft) => () => void): void {
    this.initialize = initialize;
  }
  private identity: string | null = null;

  setIdentity(identity: string): void {
    if (this.identity === identity) return;
    this.identity = identity;
    this.clear();
  }

  owns(id: string, draft: RuntimeComposerDraft): boolean {
    return this.byId[id] === draft;
  }

  clear(): void {
    for (const stop of this.cleanups.values()) stop();
    this.cleanups.clear();
    this.byId = {};
  }

  forget(id: string): void {
    this.cleanups.get(id)?.();
    this.cleanups.delete(id);
    delete this.byId[id];
  }


  for(id: string): RuntimeComposerDraft {
    if (this.byId[id]) return this.byId[id];
    this.byId[id] = {
      freshContext: false, context: [], recovered: false, saveStatus: 'loading', saveError: null, retrySave: () => {}, flushSave: () => {},
      text: '', images: [], model: null, effort: null, pickedMode: null,
      sending: false, reopening: false, stopping: false, awaitSeq: null, startError: null,
    };
    const stop = this.initialize?.(id, this.byId[id]);
    if (stop) this.cleanups.set(id, stop);
    return this.byId[id];
  }
}

export const runtimeDrafts = new RuntimeDrafts();
