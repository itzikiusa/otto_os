// Lightweight reactive state bag shared between the eager `ideAgent` shell
// (`store/ideAgentController.svelte.ts`, <180 LOC, zero Threads imports) and
// the lazily loaded `ideAgentEngine` (`agent/ideAgentEngine.svelte.ts`).

import type { RuntimeAccessMode } from '../../../lib/api/types';
import type { EffortLevel } from '../../../lib/providerModels';
import { defaultAgentProvider } from '../../../lib/providers';
import { lsGet, lsSet } from '../../../lib/storage';
import type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
import { ide, ideBasename } from '../ideStore.svelte';
import {
  detectProposalLanguage,
  IDE_RUNTIME_PROVIDERS,
  lineRangeFromOffsets,
  MAX_CHIP_SNIPPET_CHARS,
  MAX_CONTEXT_CHIPS,
  offsetsForLineRange,
  truncateSnippet,
  type IdeCodeProposal,
  type IdeContextChip,
  type IdeInlinePromptState,
} from '../store/ideDiffApplier';

export const AGENT_OPEN_KEY = 'otto.ide.agentOpen';
export const AGENT_PROVIDER_KEY = 'otto.ide.agent.provider';
export const AGENT_MODEL_KEY = 'otto.ide.agent.model';
export const AGENT_EFFORT_KEY = 'otto.ide.agent.effort';

const EFFORT_LEVELS = new Set<string>(['low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
const RUNTIME_SET = new Set<string>(IDE_RUNTIME_PROVIDERS);

export interface IdeShellSelectionInput {
  path?: string;
  from: number;
  to: number;
  startLine: number;
  endLine: number;
  text: string;
}

export interface IdeAgentEngineBridge {
  syncSubscriptions(forceRelease?: boolean): void;
  refreshWorkspaceSessions(): Promise<void>;
  syncInlineOpen(): void;
  dispose(): void;
}

function initialProvider(): string {
  const saved = (lsGet(AGENT_PROVIDER_KEY) ?? '').trim();
  return saved && saved !== 'shell' ? saved : defaultAgentProvider();
}

function initialEffort(): EffortLevel | null {
  const raw = (lsGet(AGENT_EFFORT_KEY) ?? '').trim().toLowerCase();
  return EFFORT_LEVELS.has(raw) ? (raw as EffortLevel) : null;
}

export function readIdeBufferLite(fullPath: string): string {
  const viaStore = ide.getBuffer?.(fullPath);
  return (
    (typeof viaStore === 'string' && viaStore.length > 0
      ? viaStore
      : ide.tabs.find((t) => t.path === fullPath)?.content) ?? ''
  );
}

export function resolveSelectionLite(
  override?: IdeShellSelectionInput,
): (IdeShellSelectionInput & { fullPath: string; relPath: string }) | null {
  const rawPath = override?.path ?? ide.activePath;
  if (!rawPath) return null;
  const fullPath = ide.resolvePath(rawPath);
  const relPath = ide.relativePath(fullPath) || ideBasename(fullPath);
  const buf = readIdeBufferLite(fullPath);
  if (override) return { ...override, fullPath, relPath };
  const fromHandle = ide.getSelectionRange?.() ?? null;
  if (fromHandle) {
    return fromHandle.from === fromHandle.to && !fromHandle.text && buf.length > 0
      ? { ...lineRangeFromOffsets(buf, fromHandle.from, fromHandle.to), fullPath, relPath }
      : { ...fromHandle, fullPath, relPath };
  }
  const sel0 = ide.cursorInfo.selections[0];
  if (sel0 && (sel0.anchor !== sel0.head || buf.length > 0)) {
    return { ...lineRangeFromOffsets(buf, sel0.anchor, sel0.head), fullPath, relPath };
  }
  const curLine = Math.max(1, ide.cursorInfo.line || 1);
  return { ...offsetsForLineRange(buf, curLine, curLine), fullPath, relPath };
}

class IdeAgentSharedState {
  panelOpen = $state<boolean>(lsGet(AGENT_OPEN_KEY) === '1');
  sessionId = $state<string | null>(null);
  inlinePrompt = $state<IdeInlinePromptState | null>(null);
  provider = $state<string>(initialProvider());
  model = $state<string | null>((lsGet(AGENT_MODEL_KEY) ?? '').trim() || null);
  effort = $state<EffortLevel | null>(initialEffort());

  runtimeAccessMode = $state<RuntimeAccessMode>('approval-required');
  planMode = $state<boolean>(false);
  freshContext = $state<boolean>(false);
  reopening = $state<boolean>(false);
  composerText = $state('');
  attachments = $state<ComposerFile[]>([]);
  sending = $state(false);
  error = $state<string | null>(null);
  contexts = $state<IdeContextChip[]>([]);
  appliedUndoByProposalId = $state<Record<string, string>>({});

  chipSeq = 0;
  inlineReqSeq = 0;
  appliedPathByProposalId = new Map<string, string>();
  proposalCache = new Map<string, { sig: string; proposals: IdeCodeProposal[] }>();
  engine: IdeAgentEngineBridge | null = null;

  setPanelOpen(next: boolean): boolean {
    if (this.panelOpen === next) return false;
    this.panelOpen = next;
    lsSet(AGENT_OPEN_KEY, next ? '1' : '0');
    return true;
  }

  setProvider(next: string): void {
    const clean = next.trim();
    if (!clean || clean === 'shell' || this.provider === clean) return;
    this.provider = clean;
    lsSet(AGENT_PROVIDER_KEY, clean);
  }

  setModel(next: string | null): void {
    const clean = (next ?? '').trim() || null;
    if (this.model === clean) return;
    this.model = clean;
    lsSet(AGENT_MODEL_KEY, clean ?? '');
  }

  setEffort(next: EffortLevel | null): void {
    if (this.effort === next) return;
    this.effort = next;
    lsSet(AGENT_EFFORT_KEY, next ?? '');
  }

  upsertContext(draft: Omit<IdeContextChip, 'id'> & { id?: string }): IdeContextChip {
    const snippet = truncateSnippet(draft.snippet, MAX_CHIP_SNIPPET_CHARS);
    const idx = this.contexts.findIndex((c) => c.key === draft.key);
    if (idx >= 0) {
      const updated: IdeContextChip = { ...this.contexts[idx], ...draft, id: this.contexts[idx].id, snippet };
      const next = this.contexts.slice();
      next[idx] = updated;
      this.contexts = next;
      return updated;
    }
    const created: IdeContextChip = { ...draft, id: draft.id ?? `ctx-${++this.chipSeq}`, snippet };
    const trimmed =
      this.contexts.length >= MAX_CONTEXT_CHIPS
        ? this.contexts.slice(this.contexts.length - (MAX_CONTEXT_CHIPS - 1))
        : this.contexts;
    this.contexts = [...trimmed, created];
    return created;
  }

  markSelectionSync(selection?: IdeShellSelectionInput): IdeContextChip | null {
    const resolved = resolveSelectionLite(selection);
    if (!resolved) return null;
    const { fullPath, relPath, startLine, endLine, text } = resolved;
    const lineCount = Math.max(1, endLine - startLine + 1);
    return this.upsertContext({
      key: `selection:${fullPath}:${startLine}-${endLine}`,
      kind: 'selection',
      label: `@${relPath}:L${startLine}-L${endLine}`,
      sublabel: `${lineCount} ${lineCount === 1 ? 'line' : 'lines'}`,
      path: fullPath,
      startLine,
      endLine,
      language: detectProposalLanguage(relPath, ide.activeTab?.language),
      snippet: text,
    });
  }

  openInlinePromptSync(sel?: IdeShellSelectionInput, effectiveModelFallback?: string | null): boolean {
    const r = resolveSelectionLite(sel);
    if (!r) return false;
    const prev = this.inlinePrompt;
    const defProvider = RUNTIME_SET.has(this.provider)
      ? this.provider
      : RUNTIME_SET.has(defaultAgentProvider())
        ? defaultAgentProvider()
        : 'claude';
    this.inlinePrompt = {
      open: true,
      path: r.fullPath,
      relPath: r.relPath,
      language: detectProposalLanguage(r.relPath, ide.activeTab?.language),
      from: r.from,
      to: r.to,
      startLine: r.startLine,
      endLine: r.endLine,
      selectedText: r.text,
      query: prev?.open && prev.path === r.fullPath ? prev.query : '',
      provider: prev?.open ? prev.provider : defProvider,
      model: prev?.open ? prev.model : (effectiveModelFallback ?? this.model),
      effort: prev?.open ? prev.effort : this.effort,
      busy: false,
      error: null,
      proposal: null,
    };
    return true;
  }

  resetTargetState(): void {
    this.sessionId = null;
    this.freshContext = false;
    this.contexts = [];
    this.attachments = [];
    this.inlineReqSeq++;
    this.inlinePrompt = null;
    this.error = null;
    this.proposalCache.clear();
    this.appliedPathByProposalId.clear();
    this.appliedUndoByProposalId = {};
  }

  disposeState(): void {
    this.inlineReqSeq++;
    this.inlinePrompt = null;
    this.sending = false;
    this.proposalCache.clear();
    this.appliedPathByProposalId.clear();
    this.appliedUndoByProposalId = {};
  }
}

export const ideAgentState = new IdeAgentSharedState();
