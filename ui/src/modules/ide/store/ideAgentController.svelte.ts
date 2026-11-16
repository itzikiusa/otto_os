// Lightweight reactive shell (<180 LOC) for the Otto IDE AI Coding Agent dock
// (`⌘L`) and inline editor prompt (`⌘I`). Keeps the entire Threads / structured
// runtime dependency tree out of `IdePage.svelte`'s initial static bundle,
// loading `../agent/ideAgentEngine.svelte.ts` dynamically only when opened or
// when editor context is attached.

export type * from './ideDiffApplier';
export type { IdeThreadEntry } from '../agent/ideContextBuilders';

import { loadIdeRuntimeBridge } from '../../../lib/runtimeThreadPort';
import type { IdeAgentEngine } from '../agent/ideAgentEngine.svelte';
import { ideAgentState, type IdeShellSelectionInput } from '../agent/ideAgentState.svelte';
import type {
  IdeDebugSessionSnapshot,
  IdeDiagnostic,
  IdeSymbolNode,
  IdeTestResult,
  IdeVariable,
} from '../ideStore.svelte';
import type { IdeContextChip, IdeInlinePromptState } from './ideDiffApplier';

let enginePromise: Promise<IdeAgentEngine> | null = null;

function loadIdeAgentEngine(): Promise<IdeAgentEngine> {
  if (!enginePromise) {
    enginePromise = Promise.all([
      loadIdeRuntimeBridge().catch(() => null),
      import('../agent/ideAgentEngine.svelte'),
    ]).then(([bridge, m]) => {
      if (bridge) m.ideAgentEngine.bindRuntimeBridge(bridge);
      return m.ideAgentEngine;
    });
  }
  return enginePromise;
}

class IdeAgentControllerShell {
  get panelOpen(): boolean {
    return ideAgentState.panelOpen;
  }
  set panelOpen(next: boolean) {
    if (!ideAgentState.setPanelOpen(next)) return;
    if (next) {
      void loadIdeAgentEngine().then((eng) => {
        eng.syncSubscriptions();
        void eng.refreshWorkspaceSessions();
      });
    } else {
      ideAgentState.engine?.syncSubscriptions();
    }
  }

  get contexts(): IdeContextChip[] {
    return ideAgentState.contexts;
  }
  set contexts(next: IdeContextChip[]) {
    ideAgentState.contexts = next;
    ideAgentState.engine?.syncSubscriptions();
  }

  get inlinePrompt(): IdeInlinePromptState | null {
    return ideAgentState.inlinePrompt;
  }
  set inlinePrompt(next: IdeInlinePromptState | null) {
    ideAgentState.inlinePrompt = next;
    ideAgentState.engine?.syncSubscriptions();
  }

  get sessionId(): string | null {
    return ideAgentState.sessionId;
  }
  get composerText(): string {
    return ideAgentState.composerText;
  }
  set composerText(next: string) {
    ideAgentState.composerText = next;
  }

  togglePanel(open?: boolean): void {
    this.panelOpen = open ?? !ideAgentState.panelOpen;
  }

  markSelectionFromEditor(opts?: { openPanel?: boolean; selection?: IdeShellSelectionInput }): IdeContextChip | null {
    const chip = ideAgentState.markSelectionSync(opts?.selection);
    if (!chip) return null;
    if (opts?.openPanel !== false) this.panelOpen = true;
    else void loadIdeAgentEngine().then((eng) => eng.syncSubscriptions());
    return chip;
  }

  openInlineEdit(sel?: IdeShellSelectionInput): void {
    if (!ideAgentState.openInlinePromptSync(sel)) return;
    void loadIdeAgentEngine().then((eng) => eng.syncInlineOpen());
  }

  closeInlineEdit(): void {
    ideAgentState.inlineReqSeq++;
    if (ideAgentState.inlinePrompt) ideAgentState.inlinePrompt = null;
    ideAgentState.engine?.syncSubscriptions();
  }

  async attachFile(path: string, opts?: { openPanel?: boolean }): Promise<IdeContextChip | null> {
    if (opts?.openPanel !== false) this.panelOpen = true;
    const eng = await loadIdeAgentEngine();
    return eng.attachFile(path, opts);
  }

  attachSymbol(path: string, sym: IdeSymbolNode, opts?: { openPanel?: boolean }): void {
    if (opts?.openPanel !== false) this.panelOpen = true;
    void loadIdeAgentEngine().then((eng) => eng.attachSymbol(path, sym, opts));
  }

  attachDiagnostic(
    diag: IdeDiagnostic,
    opts?: { openPanel?: boolean; autoPrompt?: boolean; preserveComposer?: boolean },
  ): void {
    if (opts?.openPanel !== false) this.panelOpen = true;
    void loadIdeAgentEngine().then((eng) => eng.attachDiagnostic(diag, opts));
  }

  attachTestFailure(
    test: IdeTestResult,
    outputLines: string[],
    opts?: { openPanel?: boolean; autoPrompt?: boolean; preserveComposer?: boolean },
  ): void {
    if (opts?.openPanel !== false) this.panelOpen = true;
    void loadIdeAgentEngine().then((eng) => eng.attachTestFailure(test, outputLines, opts));
  }

  attachDebugFrame(
    dbg: IdeDebugSessionSnapshot,
    expandedVars?: Record<number, IdeVariable[]>,
    opts?: { openPanel?: boolean; autoPrompt?: boolean; preserveComposer?: boolean },
  ): void {
    if (opts?.openPanel !== false) this.panelOpen = true;
    void loadIdeAgentEngine().then((eng) => eng.attachDebugFrame(dbg, expandedVars, opts));
  }

  dispose(): void {
    ideAgentState.disposeState();
    ideAgentState.engine?.dispose();
  }
}

export const ideAgent = new IdeAgentControllerShell();
