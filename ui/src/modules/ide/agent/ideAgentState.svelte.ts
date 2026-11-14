// Lightweight reactive state bag shared between the eager `ideAgent` shell
// (`store/ideAgentController.svelte.ts`, <180 LOC, zero Threads imports) and
// the lazily loaded `ideAgentEngine` (`agent/ideAgentEngine.svelte.ts`).

import type { RuntimeAccessMode } from '../../../lib/api/types';
import type { EffortLevel } from '../../../lib/providerModels';
import { defaultAgentProvider } from '../../../lib/providers';
import { lsGet, lsRemove, lsSet } from '../../../lib/storage';
import type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
import { ide, ideBasename } from '../ideStore.svelte';
import type {
  IdeCodeProposal,
  IdeContextChip,
  IdeInlinePromptState,
} from '../store/ideDiffApplier';

export const AGENT_OPEN_KEY = 'otto.ide.agentOpen';
export const AGENT_PROVIDER_KEY = 'otto.ide.agent.provider';
export const AGENT_MODEL_KEY = 'otto.ide.agent.model';
export const AGENT_EFFORT_KEY = 'otto.ide.agent.effort';
export const IDE_LAST_ROOT_KEY = 'otto.ide.lastRoot';
export const IDE_OPEN_HANDOFF_KEY = 'otto.ide.openThreadHandoff';
export const IDE_OPEN_THREAD_EVENT = 'otto:ide-open-thread';

const MAX_CONTEXT_CHIPS = 12;
const MAX_CHIP_SNIPPET_CHARS = 8_000;
const EFFORT_LEVELS = new Set<string>(['low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
const RUNTIME_SET = new Set<string>(['claude', 'codex', 'gemini', 'agy', 'grok', 'opencode', 'pi']);
const EXT_LANG_MAP: Record<string, string> = {
  go: 'go', java: 'java', class: 'java', rs: 'rust', py: 'python', pyi: 'python', pyw: 'python',
  ts: 'typescript', tsx: 'tsx', mts: 'typescript', cts: 'typescript', js: 'javascript', mjs: 'javascript', cjs: 'javascript', jsx: 'jsx',
  json: 'json', jsonc: 'json', jsonl: 'jsonl', ndjson: 'jsonl', csv: 'csv', tsv: 'tsv',
  sh: 'shell', bash: 'shell', zsh: 'shell', bats: 'shell', yaml: 'yaml', yml: 'yaml', md: 'markdown', sql: 'sql', html: 'html', css: 'css', svelte: 'svelte', toml: 'toml',
};

function truncateSnippet(text: string, maxChars = MAX_CHIP_SNIPPET_CHARS): string {
  const cap = Math.max(0, maxChars);
  const norm = text.replace(/\r\n/g, '\n');
  if (norm.length <= cap) return norm;
  const suffix = '\n… [truncated]';
  return cap <= suffix.length ? norm.slice(0, cap) : `${norm.slice(0, cap - suffix.length)}${suffix}`;
}

function detectProposalLanguage(path: string, explicitLang = ''): string {
  const clean = explicitLang.trim().toLowerCase();
  if (clean && !clean.includes('/') && !clean.includes('.')) return clean;
  if (/^(?:jdt:\/\/|jar:)/i.test(path.trim())) return 'java';
  const base = (path.split(/[?#]/)[0] ?? path).split(/[\\/]/).pop() ?? path;
  const ext = base.includes('.') ? (base.split('.').pop()?.toLowerCase() ?? '') : '';
  return EXT_LANG_MAP[ext] ?? (clean || 'txt');
}

function offsetsForLineRange(content: string, startLine: number, endLine: number): { from: number; to: number; startLine: number; endLine: number; text: string } {
  if (!content) return { from: 0, to: 0, startLine: 1, endLine: 1, text: '' };
  const sTarget = Math.max(1, Math.min(startLine, endLine));
  const eTarget = Math.max(sTarget, endLine);
  let line = 1, from = 0, foundStart = sTarget === 1;
  for (let i = 0; i < content.length; i++) {
    if (content.charCodeAt(i) === 10) {
      if (line === eTarget) return { from, to: i, startLine: sTarget, endLine: line, text: content.slice(from, i) };
      if (++line === sTarget) { from = i + 1; foundStart = true; }
    }
  }
  if (!foundStart) from = content.length;
  return { from, to: content.length, startLine: Math.min(sTarget, line), endLine: line, text: content.slice(from) };
}

function lineRangeFromOffsets(content: string, anchor: number, head: number): { from: number; to: number; startLine: number; endLine: number; text: string } {
  const len = content.length;
  const lo = Math.max(0, Math.min(len, Math.min(anchor, head)));
  const hi = Math.max(0, Math.min(len, Math.max(anchor, head)));
  let startLine = 1;
  for (let i = 0; i < lo; i++) if (content.charCodeAt(i) === 10) startLine++;
  if (lo === hi) {
    const from = lo === 0 ? 0 : content.lastIndexOf('\n', Math.max(0, lo - 1)) + 1;
    const nextNl = content.indexOf('\n', lo);
    const to = nextNl === -1 ? len : nextNl;
    return { from, to, startLine, endLine: startLine, text: content.slice(from, to) };
  }
  let endLine = startLine;
  const scanEnd = hi > lo && content.charCodeAt(hi - 1) === 10 ? hi - 1 : hi;
  for (let i = lo; i < scanEnd; i++) if (content.charCodeAt(i) === 10) endLine++;
  return { from: lo, to: hi, startLine, endLine, text: content.slice(lo, hi) };
}

export interface IdeShellSelectionInput {
  path?: string;
  from: number;
  to: number;
  startLine: number;
  endLine: number;
  text: string;
}

export interface IdeThreadHandoff {
  root: string;
  sessionId: string | null;
}

export interface IdeAgentEngineBridge {
  syncSubscriptions(forceRelease?: boolean): void;
  refreshWorkspaceSessions(): Promise<void>;
  syncInlineOpen(): void;
  applyPendingHandoff?(): void;
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

function consumeStoredHandoff(): IdeThreadHandoff | null {
  const raw = lsGet(IDE_OPEN_HANDOFF_KEY);
  if (!raw) return null;
  lsRemove(IDE_OPEN_HANDOFF_KEY);
  try {
    const parsed = JSON.parse(raw) as { root?: unknown; sessionId?: unknown; ts?: unknown };
    const ts = typeof parsed.ts === 'number' ? parsed.ts : 0;
    if (Date.now() - ts > 30_000) return null;
    const root = typeof parsed.root === 'string' ? parsed.root.trim() : '';
    const sessionId = typeof parsed.sessionId === 'string' && parsed.sessionId.trim() ? parsed.sessionId.trim() : null;
    return root ? { root, sessionId } : null;
  } catch {
    return null;
  }
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
  pendingHandoff: IdeThreadHandoff | null = consumeStoredHandoff();
  appliedPathByProposalId = new Map<string, string>();
  proposalCache = new Map<string, { sig: string; proposals: IdeCodeProposal[] }>();
  engine: IdeAgentEngineBridge | null = null;

  constructor() {
    if (this.pendingHandoff?.sessionId) {
      this.setPanelOpen(true);
      this.sessionId = this.pendingHandoff.sessionId;
    }
    if (typeof window !== 'undefined') {
      window.addEventListener(IDE_OPEN_THREAD_EVENT, (ev: Event) => {
        const detail = (ev as CustomEvent<{ root?: string; sessionId?: string | null }>).detail;
        lsRemove(IDE_OPEN_HANDOFF_KEY);
        const root = (detail?.root ?? '').trim();
        const sid = typeof detail?.sessionId === 'string' && detail.sessionId.trim() ? detail.sessionId.trim() : null;
        this.pendingHandoff = root ? { root, sessionId: sid } : null;
        if (sid !== null) {
          this.setPanelOpen(true);
          this.sessionId = sid;
        }
        this.engine?.applyPendingHandoff?.();
      });
    }
  }

  takePendingHandoff(): IdeThreadHandoff | null {
    const h = this.pendingHandoff ?? consumeStoredHandoff();
    this.pendingHandoff = null;
    return h;
  }

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
    const handoffSid = this.pendingHandoff?.sessionId ?? null;
    this.sessionId = handoffSid;
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
