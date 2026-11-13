// Reactive Svelte 5 controller for the Otto IDE AI Coding Agent dock (`⌘L`)
// and inline editor prompt (`⌘I`). Zero listeners and zero projections are
// retained when both `panelOpen` and `inlinePrompt` are closed.

export * from './ideDiffApplier';

import { api } from '../../../lib/api/client';
import type { ApprovalDecision, RuntimeAccessMode, RuntimePromptResp, RuntimeProvider, RuntimeSession } from '../../../lib/api/types';
import { loadErrorText } from '../../../lib/loadError';
import { defaultAgentProvider } from '../../../lib/providers';
import { lsGet, lsSet } from '../../../lib/storage';
import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
import type { RuntimeProjection } from '../../threads/runtime/projection'; // ui-guards: allow — IDE agent dock drives structured runtime sessions
import { isRuntimeProvider, runtimeThreads } from '../../threads/runtime/runtimeThreads.svelte'; // ui-guards: allow — IDE agent dock drives structured runtime sessions
import { runtimeViews } from '../../threads/runtime/runtimeView.svelte'; // ui-guards: allow — IDE agent dock drives structured runtime sessions
import {
  ide, ideBasename, type IdeDebugSessionSnapshot, type IdeDiagnostic,
  type IdeFileReadResp, type IdeSymbolNode, type IdeTestResult, type IdeVariable,
} from '../ideStore.svelte';
import {
  buildIdeAgentPrompt, computeLineDiff, detectProposalLanguage, extractCodeProposals,
  lineRangeFromOffsets, MAX_CHIP_SNIPPET_CHARS, MAX_CONTEXT_CHIPS, offsetsForLineRange,
  parseIdeContextChipsFromPrompt, spliceCodeRange, stripIdeContextBlocks, truncateSnippet,
  type IdeCodeProposal, type IdeContextChip, type IdeInlinePromptState,
} from './ideDiffApplier';

const AGENT_OPEN_KEY = 'otto.ide.agentOpen';
// eslint-disable-next-line no-control-regex
const ANSI_RE = /\x1b\[[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07/g;
const CYCLE_PROVIDERS: readonly RuntimeProvider[] = ['claude', 'codex', 'gemini'];
const trimSlash = (p: string): string => p.replace(/\/+$/, '');

function initialProvider(): RuntimeProvider {
  const def = defaultAgentProvider();
  return isRuntimeProvider(def) ? def : 'claude';
}

function summarizeTitle(text: string): string {
  const first = stripIdeContextBlocks(text).split('\n').find((l) => l.trim().length > 0)?.trim() ?? 'IDE Agent';
  return first.length > 64 ? `${first.slice(0, 61)}…` : first;
}

interface SelectionInput {
  path?: string; from: number; to: number; startLine: number; endLine: number; text: string;
}

class IdeAgentController {
  #panelOpen = $state<boolean>(lsGet(AGENT_OPEN_KEY) === '1');
  #sessionId = $state<string | null>(null);
  #inlinePrompt = $state<IdeInlinePromptState | null>(null);

  provider = $state<RuntimeProvider>(initialProvider());
  accessMode = $state<'default' | 'plan' | 'full'>('default');
  composerText = $state('');
  sending = $state(false);
  error = $state<string | null>(null);
  contexts = $state<IdeContextChip[]>([]);
  appliedUndoByProposalId = $state<Record<string, string>>({});

  private stopThreadsMount: (() => void) | null = null;
  private subscribedSessionId: string | null = null;
  private stopViewOpen: (() => void) | null = null;
  private stopThreadWatch: (() => void) | null = null;
  private chipSeq = 0;
  private inlineReqSeq = 0;
  private loadedWsIds = new Set<string>();
  private appliedPathByProposalId = new Map<string, string>();
  private proposalCache = new Map<string, { sig: string; proposals: IdeCodeProposal[] }>();

  get panelOpen(): boolean { return this.#panelOpen; }
  set panelOpen(next: boolean) {
    if (this.#panelOpen === next) return;
    this.#panelOpen = next;
    lsSet(AGENT_OPEN_KEY, next ? '1' : '0');
    this.syncSubscriptions();
    if (next) void this.ensureWorkspaceSessions();
  }

  get sessionId(): string | null { return this.#sessionId; }
  set sessionId(next: string | null) {
    if (this.#sessionId === next) return;
    this.#sessionId = next;
    this.syncSubscriptions();
  }

  get inlinePrompt(): IdeInlinePromptState | null { return this.#inlinePrompt; }
  set inlinePrompt(next: IdeInlinePromptState | null) {
    this.#inlinePrompt = next;
    this.syncSubscriptions();
  }

  readonly sessions = $derived.by((): RuntimeSession[] => {
    const wsId = this.resolveWorkspaceId();
    const localRoot = ide.target.kind === 'local' && ide.target.root ? trimSlash(ide.target.root) : '';
    return runtimeThreads.live(false).filter((s) => {
      if (s.id === this.#sessionId) return true;
      if (localRoot && s.cwd) {
        const cwd = trimSlash(s.cwd);
        if (cwd === localRoot || cwd.startsWith(`${localRoot}/`) || localRoot.startsWith(`${cwd}/`)) return true;
      }
      return s.workspace_id === wsId;
    });
  });

  readonly projection = $derived<RuntimeProjection | null>(
    this.#sessionId ? (runtimeViews.projections[this.#sessionId] ?? null) : null,
  );

  readonly isWorking = $derived.by((): boolean => {
    if (this.sending) return true;
    const sid = this.#sessionId;
    if (!sid) return false;
    const row = runtimeThreads.byId[sid];
    if (row && (row.status === 'working' || row.status === 'starting')) return true;
    const proj = runtimeViews.projections[sid];
    if (!proj) return false;
    if (proj.session?.status === 'running' || proj.session?.status === 'starting') return true;
    return proj.turns.some((t) => t.status === 'running' || t.status === 'pending');
  });

  readonly proposalsByMessageId = $derived.by((): Record<string, IdeCodeProposal[]> => {
    const proj = this.projection;
    if (!proj || proj.items.length === 0) return {};
    const appliedMap = this.appliedUndoByProposalId;
    const activeFallback = ide.activePath;
    const out: Record<string, IdeCodeProposal[]> = {};
    const seenIds = new Set<string>();
    const userByTurn = new Map<string, string>();
    let latestUserText = '';

    for (let i = 0; i < proj.items.length; i++) {
      const it = proj.items[i];
      if (it.type === 'user_message') {
        latestUserText = it.text;
        if (it.turn_id) userByTurn.set(it.turn_id, it.text);
        continue;
      }
      if (it.type !== 'assistant_message') continue;
      seenIds.add(it.id);
      const promptText = (it.turn_id ? userByTurn.get(it.turn_id) : undefined) ?? latestUserText;
      const chips = parseIdeContextChipsFromPrompt(promptText);
      const selChips = chips.filter((c) => c.kind === 'selection' && c.path && c.startLine);
      const firstPathChip = selChips[0] ?? chips.find((c) => Boolean(c.path));
      const fallbackPath = firstPathChip?.path ? ide.resolvePath(firstPathChip.path) : activeFallback;
      const fallbackRange = selChips.length === 1 && selChips[0].startLine
        ? { startLine: selChips[0].startLine, endLine: selChips[0].endLine ?? selChips[0].startLine }
        : null;
      const rangeKey = fallbackRange ? `${fallbackRange.startLine}-${fallbackRange.endLine}` : '';
      const fbContent = fallbackPath ? this.baselineForMessage(it.id, fallbackPath, appliedMap) : '';
      const sig = `${it.text}\0${fallbackPath ?? ''}\0${rangeKey}\0${fbContent.length}:${fbContent.slice(0, 48)}`;
      let cached = this.proposalCache.get(it.id);
      if (!cached || cached.sig !== sig) {
        const extracted = extractCodeProposals(it.text, fallbackPath, fallbackRange, {
          messageId: it.id,
          resolvePath: (p) => ide.resolvePath(p),
          relativePath: (p) => ide.relativePath(p),
          getFileContent: (fullPath) => this.baselineForMessage(it.id, fullPath, appliedMap),
        });
        cached = { sig, proposals: extracted };
        this.proposalCache.set(it.id, cached);
      }
      if (cached.proposals.length > 0) {
        const mapped = cached.proposals.map((p) => {
          const isApplied = Object.prototype.hasOwnProperty.call(appliedMap, p.id);
          return p.applied === isApplied ? p : { ...p, applied: isApplied };
        });
        out[it.id] = mapped;
        if (it.message_id && it.message_id !== it.id) out[it.message_id] = mapped;
      }
    }
    if (this.proposalCache.size > seenIds.size + 24) {
      for (const k of this.proposalCache.keys()) if (!seenIds.has(k)) this.proposalCache.delete(k);
    }
    return out;
  });

  private baselineForMessage(msgId: string, fullPath: string, appliedMap: Readonly<Record<string, string>>): string {
    for (const [propId, snap] of Object.entries(appliedMap)) {
      if (propId.startsWith(`${msgId}:`) && this.appliedPathByProposalId.get(propId) === fullPath) return snap;
    }
    return this.readFileBuffer(fullPath);
  }

  private resolveWorkspaceId(): string {
    if (ide.target.workspace_id) return ide.target.workspace_id;
    if (ide.target.kind === 'local' && ide.target.root) {
      const root = trimSlash(ide.target.root);
      const exact = ws.workspaces.find((w) => w.root_path && trimSlash(w.root_path) === root);
      if (exact) return exact.id;
      const parent = ws.workspaces.find((w) => w.root_path && root.startsWith(`${trimSlash(w.root_path)}/`));
      if (parent) return parent.id;
    }
    return ws.currentId ?? ws.workspaces[0]?.id ?? SCRATCH_WORKSPACE_ID;
  }

  private async ensureWorkspaceSessions(): Promise<void> {
    const wsId = this.resolveWorkspaceId();
    if (!wsId || this.loadedWsIds.has(wsId) || runtimeThreads.loaded[wsId]) return;
    this.loadedWsIds.add(wsId);
    try { await runtimeThreads.load(wsId); }
    catch { this.loadedWsIds.delete(wsId); }
  }

  private syncSubscriptions(forceRelease = false): void {
    const active = !forceRelease && (this.#panelOpen || Boolean(this.#inlinePrompt?.open));
    const targetSid = active ? this.#sessionId : null;
    if (!targetSid) {
      this.stopViewOpen?.(); this.stopViewOpen = null;
      this.stopThreadWatch?.(); this.stopThreadWatch = null;
      this.subscribedSessionId = null;
      this.stopThreadsMount?.(); this.stopThreadsMount = null;
      return;
    }
    if (!this.stopThreadsMount) this.stopThreadsMount = runtimeThreads.mount();
    if (this.subscribedSessionId !== targetSid) {
      this.stopViewOpen?.();
      this.stopThreadWatch?.();
      this.subscribedSessionId = targetSid;
      this.stopViewOpen = runtimeViews.open(targetSid);
      this.stopThreadWatch = runtimeThreads.watch(targetSid);
    }
  }

  private readFileBuffer(fullPath: string): string {
    const viaStore = ide.getBuffer?.(fullPath);
    return (typeof viaStore === 'string' && viaStore.length > 0 ? viaStore : ide.tabs.find((t) => t.path === fullPath)?.content) ?? '';
  }

  private resolveEditorSelection(override?: SelectionInput): (SelectionInput & { fullPath: string; relPath: string }) | null {
    const rawPath = override?.path ?? ide.activePath;
    if (!rawPath) return null;
    const fullPath = ide.resolvePath(rawPath), relPath = ide.relativePath(fullPath) || ideBasename(fullPath), buf = this.readFileBuffer(fullPath);
    if (override) return { ...override, fullPath, relPath };
    const fromHandle = ide.getSelectionRange?.() ?? null;
    if (fromHandle) {
      return fromHandle.from === fromHandle.to && !fromHandle.text && buf.length > 0
        ? { ...lineRangeFromOffsets(buf, fromHandle.from, fromHandle.to), fullPath, relPath }
        : { ...fromHandle, fullPath, relPath };
    }
    const sel0 = ide.cursorInfo.selections[0];
    if (sel0 && (sel0.anchor !== sel0.head || buf.length > 0)) return { ...lineRangeFromOffsets(buf, sel0.anchor, sel0.head), fullPath, relPath };
    const curLine = Math.max(1, ide.cursorInfo.line || 1);
    return { ...offsetsForLineRange(buf, curLine, curLine), fullPath, relPath };
  }

  private upsertContext(draft: Omit<IdeContextChip, 'id'> & { id?: string }): IdeContextChip {
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
    const trimmed = this.contexts.length >= MAX_CONTEXT_CHIPS
      ? this.contexts.slice(this.contexts.length - (MAX_CONTEXT_CHIPS - 1))
      : this.contexts;
    this.contexts = [...trimmed, created];
    return created;
  }

  togglePanel(open?: boolean): void { this.panelOpen = open ?? !this.#panelOpen; }
  setProvider(next: RuntimeProvider): void { this.provider = next; }
  cycleProvider(): void { this.provider = CYCLE_PROVIDERS[(CYCLE_PROVIDERS.indexOf(this.provider) + 1) % CYCLE_PROVIDERS.length]; }
  setAccessMode(mode: 'default' | 'plan' | 'full'): void { this.accessMode = mode; }
  startNewChat(): void { this.error = null; this.sessionId = null; }
  removeContext(id: string): void { this.contexts = this.contexts.filter((c) => c.id !== id); }
  clearContexts(): void { this.contexts = []; }

  async selectSession(id: string | null): Promise<void> {
    this.error = null;
    this.sessionId = id;
    this.syncSubscriptions();
    if (!id) return;
    let row = runtimeThreads.get(id);
    if (!row) {
      try { row = await runtimeThreads.loadOne(id); }
      catch (e) { this.error = loadErrorText(e); return; }
    }
    if (row?.provider) this.provider = row.provider;
  }

  markSelectionFromEditor(opts?: { openPanel?: boolean; selection?: SelectionInput }): IdeContextChip | null {
    const resolved = this.resolveEditorSelection(opts?.selection);
    if (!resolved) return null;
    const { fullPath, relPath, startLine, endLine, text } = resolved;
    const lineCount = Math.max(1, endLine - startLine + 1);
    const chip = this.upsertContext({
      key: `selection:${fullPath}:${startLine}-${endLine}`, kind: 'selection',
      label: `@${relPath}:L${startLine}-L${endLine}`, sublabel: `${lineCount} ${lineCount === 1 ? 'line' : 'lines'}`,
      path: fullPath, startLine, endLine, language: detectProposalLanguage(relPath, ide.activeTab?.language), snippet: text,
    });
    if (opts?.openPanel !== false) this.panelOpen = true;
    return chip;
  }

  openInlineEdit(sel?: SelectionInput): void {
    const r = this.resolveEditorSelection(sel);
    if (!r) return;
    const prev = this.#inlinePrompt;
    this.inlinePrompt = {
      open: true, path: r.fullPath, relPath: r.relPath, language: detectProposalLanguage(r.relPath, ide.activeTab?.language),
      from: r.from, to: r.to, startLine: r.startLine, endLine: r.endLine, selectedText: r.text,
      query: prev?.open && prev.path === r.fullPath ? prev.query : '', busy: false, error: null, proposal: null,
    };
  }

  closeInlineEdit(): void {
    this.inlineReqSeq++;
    if (this.#inlinePrompt) this.inlinePrompt = null;
  }

  handoffInlineToChat(): void {
    const st = this.#inlinePrompt;
    if (!st) return;
    const q = st.query.trim();
    this.markSelectionFromEditor({
      openPanel: true,
      selection: { path: st.path, from: st.from, to: st.to, startLine: st.startLine, endLine: st.endLine, text: st.selectedText },
    });
    if (q) this.composerText = this.composerText.trim() ? `${this.composerText.trim()}\n${q}` : q;
    this.closeInlineEdit();
  }

  async submitInlineEdit(query?: string): Promise<void> {
    const st = this.#inlinePrompt;
    if (!st || !st.open || st.busy) return;
    const q = (query ?? st.query).trim();
    if (!q) return;

    st.query = q; st.busy = true; st.error = null; st.proposal = null;
    const reqSeq = ++this.inlineReqSeq;
    const inlineChip: IdeContextChip = {
      id: 'inline-sel', key: `selection:${st.path}:${st.startLine}-${st.endLine}`, kind: 'selection',
      label: `@${st.relPath}:L${st.startLine}-L${st.endLine}`, path: st.path,
      startLine: st.startLine, endLine: st.endLine, language: st.language, snippet: st.selectedText,
    };
    const promptPayload = buildIdeAgentPrompt(q, [inlineChip], {
      workspaceRoot: ide.target.kind === 'local' ? ide.target.root : undefined,
      activePath: st.path, planMode: this.accessMode === 'plan',
    });

    try {
      const sid = await this.ensureLiveSession(q);
      if (this.inlineReqSeq !== reqSeq || !this.#inlinePrompt?.open) return;
      const knownIds = new Set((runtimeViews.projections[sid]?.items ?? []).map((i) => i.id));
      const resp = await this.dispatchPromptToSession(sid, promptPayload);
      const reply = await this.waitForInlineTurnReply(sid, resp, knownIds, reqSeq);
      if (this.inlineReqSeq !== reqSeq || !this.#inlinePrompt?.open) return;

      const range = { from: st.from, to: st.to, startLine: st.startLine, endLine: st.endLine };
      const extracted = extractCodeProposals(reply.text, st.path, range, {
        messageId: reply.messageId, preferRange: true,
        resolvePath: (p) => ide.resolvePath(p), relativePath: (p) => ide.relativePath(p),
        getFileContent: (fullPath) => this.readFileBuffer(fullPath),
      });

      let proposal: IdeCodeProposal | null = extracted[0] ?? null;
      if (!proposal) {
        const cleaned = stripIdeContextBlocks(reply.text).replace(/^echo:\s*/i, '').trim();
        if (cleaned && cleaned !== q) {
          const diffLines = computeLineDiff(st.selectedText, cleaned);
          proposal = {
            id: `${reply.messageId}:inline`, messageId: reply.messageId, path: st.path, relPath: st.relPath,
            language: st.language, code: cleaned, range, diffLines,
            addedCount: diffLines.filter((d) => d.kind === 'add').length,
            removedCount: diffLines.filter((d) => d.kind === 'del').length, applied: false,
          };
        }
      }
      if (!proposal) this.#inlinePrompt.error = 'Agent reply did not include a code change.';
      else this.#inlinePrompt.proposal = proposal;
    } catch (e) {
      if (this.inlineReqSeq === reqSeq && this.#inlinePrompt?.open) this.#inlinePrompt.error = loadErrorText(e);
    } finally {
      if (this.inlineReqSeq === reqSeq && this.#inlinePrompt?.open) this.#inlinePrompt.busy = false;
    }
  }

  async acceptInlineProposal(): Promise<boolean> {
    const proposal = this.#inlinePrompt?.proposal;
    if (!proposal) return false;
    const applied = await this.applyProposal(proposal);
    if (applied) this.closeInlineEdit();
    return applied;
  }

  async attachFile(path: string, opts?: { openPanel?: boolean }): Promise<IdeContextChip | null> {
    const fullPath = ide.resolvePath(path);
    const relPath = ide.relativePath(fullPath) || ideBasename(fullPath);
    const openTab = ide.tabs.find((t) => t.path === fullPath && !t.loading && !t.error);
    let content = '';
    let language = openTab?.language ?? '';
    if (openTab) {
      content = this.readFileBuffer(fullPath);
    } else {
      try {
        const res = await api.get<IdeFileReadResp>(`/ide/fs/file?${ide.targetQuery({ path: fullPath })}`);
        content = res.content ?? '';
        language = res.language ?? '';
      } catch (e) { this.error = loadErrorText(e); return null; }
    }
    const lines = content ? content.split('\n').length : 1;
    const chip = this.upsertContext({
      key: `file:${fullPath}`, kind: 'file', label: `@${relPath}`,
      sublabel: `${lines} ${lines === 1 ? 'line' : 'lines'}`, path: fullPath,
      startLine: 1, endLine: Math.max(1, lines), language: detectProposalLanguage(relPath, language), snippet: content,
    });
    if (opts?.openPanel !== false) this.panelOpen = true;
    return chip;
  }

  attachSymbol(path: string, sym: IdeSymbolNode, opts?: { openPanel?: boolean }): IdeContextChip | null {
    const fullPath = ide.resolvePath(path);
    const relPath = ide.relativePath(fullPath) || ideBasename(fullPath);
    const startLine = Math.max(1, sym.line || 1);
    const endLine = Math.max(startLine, sym.end_line || startLine);
    const slice = offsetsForLineRange(this.readFileBuffer(fullPath), startLine, endLine);
    const chip = this.upsertContext({
      key: `symbol:${fullPath}:${sym.name}:${startLine}`, kind: 'symbol', label: `@${relPath}#${sym.name}`,
      sublabel: `L${startLine}-L${endLine}`, path: fullPath, startLine, endLine, language: detectProposalLanguage(relPath),
      snippet: slice.text || `${sym.kind} ${sym.name} (L${startLine}-L${endLine})`,
    });
    if (opts?.openPanel !== false) this.panelOpen = true;
    return chip;
  }

  attachDiagnostic(diag: IdeDiagnostic, opts?: { openPanel?: boolean; autoPrompt?: boolean }): void {
    const fullPath = ide.resolvePath(diag.path);
    const relPath = ide.relativePath(fullPath) || ideBasename(fullPath);
    const buf = this.readFileBuffer(fullPath);
    const startLine = Math.max(1, diag.line - 5);
    const endLine = Math.max(startLine, (diag.end_line ?? diag.line) + 5);
    let contextFrame = '';
    if (buf) {
      const slice = offsetsForLineRange(buf, startLine, endLine);
      const numbered = slice.text.split('\n').map((ln, idx) => `${slice.startLine + idx === diag.line ? '>' : ' '} ${slice.startLine + idx} | ${ln}`).join('\n');
      contextFrame = `\n\n${numbered}`;
    }
    this.upsertContext({
      key: `diag:${fullPath}:${diag.line}:${diag.message}`, kind: 'diagnostic', label: `⚠ ${relPath}:${diag.line}`,
      sublabel: `${diag.severity}: ${diag.message}`, path: fullPath, startLine: diag.line, endLine: diag.end_line ?? diag.line,
      language: detectProposalLanguage(relPath),
      snippet: `[${diag.severity.toUpperCase()}] ${relPath}:${diag.line}${diag.column ? `:${diag.column}` : ''} — ${diag.message}${contextFrame}`,
    });
    if (opts?.openPanel !== false) this.panelOpen = true;
    if (!this.composerText.trim()) this.composerText = `Fix this ${diag.severity} in ${relPath}:${diag.line}: ${diag.message}`;
    if (opts?.autoPrompt) void this.sendPrompt();
  }

  attachTestFailure(test: IdeTestResult, outputLines: string[], opts?: { openPanel?: boolean; autoPrompt?: boolean }): void {
    const clean = outputLines.map((l) => l.replace(ANSI_RE, ''));
    const hitIdx = clean.findIndex((l) => l.includes(test.name));
    const winLines = hitIdx >= 0 ? clean.slice(Math.max(0, hitIdx - 4), Math.min(clean.length, hitIdx + 36)) : clean.slice(-40);
    const fullPath = test.file ? ide.resolvePath(test.file) : undefined;
    const header = [
      `Test: ${test.name}${test.suite ? ` (${test.suite})` : ''}`,
      fullPath ? `Location: ${ide.relativePath(fullPath)}${test.line ? `:${test.line}` : ''}` : '',
      test.message ? `Failure: ${test.message.replace(ANSI_RE, '').trim()}` : '',
    ].filter(Boolean).join('\n');
    this.upsertContext({
      key: `test:${test.suite ?? ''}:${test.name}`, kind: 'test_failure', label: `✗ ${test.name}`,
      sublabel: test.suite || 'test failure', path: fullPath, startLine: test.line ?? undefined,
      snippet: winLines.length > 0 ? `${header}\n\n${winLines.join('\n')}` : header,
    });
    if (opts?.openPanel !== false) this.panelOpen = true;
    if (!this.composerText.trim()) this.composerText = `Investigate and fix the failing test ${test.name}.`;
    if (opts?.autoPrompt) void this.sendPrompt();
  }

  attachDebugFrame(dbg: IdeDebugSessionSnapshot, expandedVars?: Record<number, IdeVariable[]>, opts?: { openPanel?: boolean; autoPrompt?: boolean }): void {
    const topFrame = dbg.stack_frames.find((f) => f.id === dbg.active_frame_id) ?? dbg.stack_frames[0];
    const rawFile = dbg.paused_file ?? topFrame?.path ?? '';
    const fullPath = rawFile ? ide.resolvePath(rawFile) : undefined;
    const shortFile = rawFile ? ideBasename(rawFile) : dbg.title || 'debug';
    const line = dbg.paused_line ?? topFrame?.line ?? 1;
    const fnName = topFrame?.name || 'paused';
    const stackText = dbg.stack_frames.slice(0, 8).map((f, i) => `  #${i} ${f.name}${f.path ? ` (${ide.relativePath(ide.resolvePath(f.path))}:${f.line})` : ''}`).join('\n');
    const scopeLines: string[] = [];
    for (let i = 0; i < dbg.scopes.length; i++) {
      const sc = dbg.scopes[i];
      const vars = sc.variables?.length ? sc.variables : (expandedVars?.[sc.variables_reference] ?? []);
      if (vars.length === 0) continue;
      scopeLines.push(`${sc.name}:`);
      for (let j = 0; j < Math.min(vars.length, 24); j++) scopeLines.push(`  ${vars[j].name}${vars[j].var_type ? ` (${vars[j].var_type})` : ''} = ${vars[j].value}`);
    }
    if (scopeLines.length === 0 && dbg.inline_values.length > 0) {
      scopeLines.push('Inline values:', ...dbg.inline_values.map((iv) => `  L${iv.line} ${iv.name} = ${iv.value}`));
    }
    const parts = [
      `Paused: ${fnName} @ ${fullPath ? ide.relativePath(fullPath) : shortFile}:${line}${dbg.stopped_reason ? ` (${dbg.stopped_reason})` : ''}`,
      stackText ? `Call stack:\n${stackText}` : '',
      scopeLines.length > 0 ? scopeLines.join('\n') : '',
    ].filter(Boolean);
    this.upsertContext({
      key: `debug:${dbg.id}:${fullPath ?? shortFile}:${line}`, kind: 'debug_frame', label: `⏸ ${fnName} @ ${shortFile}:${line}`,
      sublabel: dbg.stopped_reason || 'paused frame', path: fullPath, startLine: line, endLine: line,
      language: fullPath ? detectProposalLanguage(fullPath, dbg.language) : dbg.language, snippet: parts.join('\n\n'),
    });
    if (opts?.openPanel !== false) this.panelOpen = true;
    if (!this.composerText.trim()) this.composerText = `Inspect the paused debugger frame at ${shortFile}:${line} and help fix the issue.`;
    if (opts?.autoPrompt) void this.sendPrompt();
  }

  private mappedAccessMode(): RuntimeAccessMode | undefined {
    if (this.accessMode === 'plan') return 'approval-required';
    if (this.accessMode === 'full') return 'full-access';
    return undefined;
  }

  private async ensureLiveSession(seedTitle: string): Promise<string> {
    const existingId = this.#sessionId;
    if (existingId) {
      const row = runtimeThreads.get(existingId) ?? (await runtimeThreads.loadOne(existingId));
      if (row.status === 'closed' || row.status === 'dormant') await runtimeThreads.reopen(existingId);
      this.syncSubscriptions();
      return existingId;
    }
    const wsId = this.resolveWorkspaceId();
    const localCwd = ide.target.kind === 'local' && ide.target.root && ide.target.root !== '~' ? ide.target.root : null;
    const created = await runtimeThreads.start(wsId, {
      provider: this.provider, cwd: localCwd, title: summarizeTitle(seedTitle), model: null, prompt: '',
    });
    this.#sessionId = created.id;
    this.syncSubscriptions();
    return created.id;
  }

  private async dispatchPromptToSession(sid: string, text: string): Promise<RuntimePromptResp> {
    const row = runtimeThreads.get(sid);
    const proj = runtimeViews.projections[sid];
    const turnBusy = row?.status === 'working' || row?.status === 'starting' ||
      proj?.turns.some((t) => t.status === 'running' || t.status === 'pending');
    const mode = this.mappedAccessMode();
    return runtimeThreads.prompt(sid, {
      text, ...(mode ? { access_mode: mode } : {}), ...(turnBusy ? { delivery: 'queue' as const } : {}),
    });
  }

  private waitForInlineTurnReply(sid: string, resp: RuntimePromptResp, knownIds: ReadonlySet<string>, reqSeq: number): Promise<{ messageId: string; text: string }> {
    const startedAt = Date.now();
    return new Promise((resolve, reject) => {
      const check = (): void => {
        if (this.inlineReqSeq !== reqSeq || !this.#inlinePrompt?.open) { resolve({ messageId: 'cancelled', text: '' }); return; }
        const proj = runtimeViews.projections[sid];
        if (proj) {
          const errItem = proj.items.find((i) => !knownIds.has(i.id) && i.type === 'error' && (!resp.turn_id || i.turn_id === resp.turn_id));
          if (errItem && errItem.type === 'error') { reject(new Error(errItem.failure.message || 'Inline edit turn returned an error.')); return; }
          const assistants = proj.items.filter(
            (i): i is Extract<typeof proj.items[number], { type: 'assistant_message' }> =>
              i.type === 'assistant_message' && (!knownIds.has(i.id) || (Boolean(resp.turn_id) && i.turn_id === resp.turn_id)),
          );
          const turn = resp.turn_id ? proj.turns.find((t) => t.id === resp.turn_id) : proj.turns[proj.turns.length - 1];
          const turnDone = turn
            ? turn.status === 'completed' || turn.status === 'failed' || turn.status === 'interrupted' || turn.status === 'cancelled'
            : (assistants.length > 0 && assistants.every((a) => !a.streaming) && runtimeThreads.get(sid)?.status === 'idle');
          if (turnDone && assistants.length > 0) {
            resolve({ messageId: assistants[assistants.length - 1].id, text: assistants.map((a) => a.text).join('\n\n') });
            return;
          }
          if (turn && (turn.status === 'failed' || turn.status === 'cancelled' || turn.status === 'interrupted')) {
            reject(new Error(`Turn ${turn.status}.`));
            return;
          }
        }
        if (Date.now() - startedAt > 60_000) { reject(new Error('Timed out waiting for inline edit response.')); return; }
        setTimeout(check, 60);
      };
      setTimeout(check, 35);
    });
  }

  async sendPrompt(overrideText?: string): Promise<void> {
    const userText = (overrideText ?? this.composerText).trim();
    if ((!userText && this.contexts.length === 0) || this.sending) return;
    if (!this.#panelOpen) this.panelOpen = true;
    const snapshotContexts = this.contexts.slice();
    const fullPrompt = buildIdeAgentPrompt(userText || 'Inspect the attached context and propose improvements.', snapshotContexts, {
      workspaceRoot: ide.target.kind === 'local' ? ide.target.root : undefined,
      activePath: ide.activePath, planMode: this.accessMode === 'plan',
    });
    this.sending = true;
    this.error = null;
    try {
      const sid = await this.ensureLiveSession(userText || snapshotContexts[0]?.label || 'IDE Agent');
      await this.dispatchPromptToSession(sid, fullPrompt);
      if (overrideText === undefined) this.composerText = '';
      this.contexts = [];
    } catch (e) { this.error = loadErrorText(e); }
    finally { this.sending = false; }
  }

  async stopTurn(): Promise<void> {
    if (!this.#sessionId) return;
    try { await runtimeThreads.interrupt(this.#sessionId); } catch (e) { this.error = loadErrorText(e); }
  }

  async respondApproval(requestId: string, decision: ApprovalDecision): Promise<void> {
    if (!this.#sessionId) return;
    try { await runtimeThreads.respond(this.#sessionId, requestId, decision); } catch (e) { this.error = loadErrorText(e); }
  }

  private async writeEditorBuffer(fullPath: string, content: string, range?: { from: number; to: number }): Promise<boolean> {
    if (ide.applyAgentEdit) return ide.applyAgentEdit(fullPath, content, range);
    await ide.openFile(fullPath, { preview: false });
    ide.updateBuffer(fullPath, range ? spliceCodeRange(this.readFileBuffer(fullPath), content, range) : content);
    return true;
  }

  async applyProposal(proposal: IdeCodeProposal): Promise<boolean> {
    const fullPath = ide.resolvePath(proposal.path);
    if (!ide.tabs.some((t) => t.path === fullPath && !t.loading)) await ide.openFile(fullPath, { preview: false });
    const tab = ide.tabs.find((t) => t.path === fullPath);
    if (!tab || tab.readOnly || !ide.canEditCollab) return false;
    const before = this.readFileBuffer(fullPath), r = proposal.range;
    const resolved = r ? (r.from >= 0 && r.to >= r.from && r.to <= before.length ? { from: r.from, to: r.to } : offsetsForLineRange(before, r.startLine, r.endLine)) : null;
    const fullNext = r ? spliceCodeRange(before, proposal.code, resolved ?? r) : proposal.code;
    const chunk = resolved ? fullNext.slice(resolved.from, fullNext.length - (before.length - resolved.to)) : fullNext;
    if (!(await this.writeEditorBuffer(fullPath, chunk, resolved ?? undefined))) return false;
    this.appliedPathByProposalId.set(proposal.id, fullPath);
    this.appliedUndoByProposalId = { ...this.appliedUndoByProposalId, [proposal.id]: before };
    return true;
  }

  async undoProposal(proposalId: string): Promise<boolean> {
    if (!Object.prototype.hasOwnProperty.call(this.appliedUndoByProposalId, proposalId)) return false;
    const before = this.appliedUndoByProposalId[proposalId];
    let fullPath = this.appliedPathByProposalId.get(proposalId) ?? null;
    if (!fullPath) {
      for (const list of Object.values(this.proposalsByMessageId)) {
        const hit = list.find((p) => p.id === proposalId);
        if (hit) { fullPath = ide.resolvePath(hit.path); break; }
      }
    }
    if (!fullPath || !(await this.writeEditorBuffer(fullPath, before))) return false;
    this.appliedPathByProposalId.delete(proposalId);
    const rest = { ...this.appliedUndoByProposalId };
    delete rest[proposalId];
    this.appliedUndoByProposalId = rest;
    return true;
  }

  dispose(): void { this.inlineReqSeq++; this.#inlinePrompt = null; this.sending = false; this.syncSubscriptions(true); }
}

export const ideAgent = new IdeAgentController();
