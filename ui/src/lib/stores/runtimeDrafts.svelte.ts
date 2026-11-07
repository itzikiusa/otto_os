import type { PromptImage, RuntimeAccessMode } from '../api/types';
import type { EffortLevel } from '../providerModels';

/** An attachment waiting in a runtime composer. */
export type ComposerFile = PromptImage & { name: string };

export interface RuntimeComposerDraft {
  text: string;
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

/** Unsent work and in-flight actions belong to a thread, across view remounts.
 * In-memory only: attachments and prompts must not enter browser storage. */
export class RuntimeDrafts {
  private byId = $state<Record<string, RuntimeComposerDraft>>({});
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
    this.byId = {};
  }

  forget(id: string): void {
    delete this.byId[id];
  }


  for(id: string): RuntimeComposerDraft {
    this.byId[id] ??= {
      text: '', images: [], model: null, effort: null, pickedMode: null,
      sending: false, reopening: false, stopping: false, awaitSeq: null, startError: null,
    };
    return this.byId[id];
  }
}

export const runtimeDrafts = new RuntimeDrafts();
