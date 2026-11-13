// Reactive Svelte 5 controller for the Otto IDE AI Coding Agent dock (`⌘L`)
// and inline editor prompt (`⌘I`), unified with Otto's first-class Threads
// system (`/runtime-sessions` + PTY session threads). Zero listeners and zero
// projections are retained when both `panelOpen` and `inlinePrompt` are closed.

export * from './ideDiffApplier';
export type { IdeThreadEntry } from '../agent/ideContextBuilders';

import type { ApprovalDecision, RuntimeAccessMode, RuntimeProvider, RuntimeSession } from '../../../lib/api/types';
import { loadErrorText } from '../../../lib/loadError';
import { normalizeEffort, supportsEffortSelection, type EffortLevel, type InteractionMode } from '../../../lib/providerModels';
import { agentProviders, defaultAgentProvider } from '../../../lib/providers';
import { lsGet, lsSet } from '../../../lib/storage';
import type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { council } from '../../threads/runtime/councilStore.svelte'; // ui-guards: allow — IDE agent dock supports Council multi-agent threads
import type { RuntimeProjection } from '../../threads/runtime/projection'; // ui-guards: allow — IDE agent dock shares structured runtime projections
import { readAttachments } from '../../threads/runtime/runtimeComposer'; // ui-guards: allow — IDE agent dock shares image/PDF attachment reader
import { isRuntimeProvider, runtimeThreads } from '../../threads/runtime/runtimeThreads.svelte'; // ui-guards: allow — IDE agent dock drives real Otto Threads
import { runtimeViews } from '../../threads/runtime/runtimeView.svelte'; // ui-guards: allow — IDE agent dock streams structured runtime views
import {
  applyIdeCodeProposal,
  computeProposalsMap,
  readIdeFileBuffer,
  undoIdeCodeProposal,
} from '../agent/ideAgentRuntimeOps.svelte';
import {
  executeIdeInlineEdit,
  executeIdePromptSend,
  interruptIdeSessionTurn,
  isIdeSessionWorking,
  loadIdeWorkspaceSessions,
  loadSelectedIdeSessionMeta,
  openIdeThreadInThreadsView,
} from '../agent/ideAgentTurnRunner.svelte';
import {
  buildDebugFrameDraftChip,
  buildDiagnosticDraftChip,
  buildFailedRunDraftChip,
  buildFileDraftChip,
  buildIdeInlinePromptState,
  buildSelectionDraftChip,
  buildSymbolDraftChip,
  buildTestFailureDraftChip,
  collectIdeProjectRoots,
  collectIdeThreadEntries,
  currentIdeTargetKey,
  resolveIdeProjectCwd,
  resolveIdeWorkspaceId,
  upsertIdeContextChip,
  type IdeSelectionInput,
  type IdeThreadEntry,
} from '../agent/ideContextBuilders';
import {
  ide,
  ideBasename,
  type IdeDebugSessionSnapshot,
  type IdeDiagnostic,
  type IdeSymbolNode,
  type IdeTestResult,
  type IdeVariable,
} from '../ideStore.svelte';
import {
  buildIdeAgentPrompt,
  matchesIdeProjectThread,
  resolveIdeHarnesses,
  resolveIdeInlineProviders,
  type IdeCodeProposal,
  type IdeContextChip,
  type IdeInlinePromptState,
} from './ideDiffApplier';

const AGENT_OPEN_KEY = 'otto.ide.agentOpen';
const AGENT_PROVIDER_KEY = 'otto.ide.agent.provider';
const AGENT_MODEL_KEY = 'otto.ide.agent.model';
const AGENT_EFFORT_KEY = 'otto.ide.agent.effort';

function initialProvider(): string {
  const saved = (lsGet(AGENT_PROVIDER_KEY) ?? '').trim();
  return saved && saved !== 'shell' ? saved : defaultAgentProvider();
}

class IdeAgentController {
  #panelOpen = $state<boolean>(lsGet(AGENT_OPEN_KEY) === '1');
  #sessionId = $state<string | null>(null);
  #inlinePrompt = $state<IdeInlinePromptState | null>(null);
  #provider = $state<string>(initialProvider());
  #model = $state<string | null>((lsGet(AGENT_MODEL_KEY) ?? '').trim() || null);
  #effort = $state<EffortLevel | null>(normalizeEffort(lsGet(AGENT_EFFORT_KEY)));

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

  private stopThreadsMount: (() => void) | null = null;
  private subscribedSessionId: string | null = null;
  private stopViewOpen: (() => void) | null = null;
  private stopThreadWatch: (() => void) | null = null;
  private stopTargetWatch: (() => void) | null = null;
  private lastTargetKey = '';
  private chipSeq = 0;
  private inlineReqSeq = 0;
  private inflightWsLoads = new Map<string, Promise<void>>();
  private appliedPathByProposalId = new Map<string, string>();
  private proposalCache = new Map<string, { sig: string; proposals: IdeCodeProposal[] }>();

  get panelOpen(): boolean { return this.#panelOpen; }
  set panelOpen(next: boolean) {
    if (this.#panelOpen === next) return;
    this.#panelOpen = next;
    lsSet(AGENT_OPEN_KEY, next ? '1' : '0');
    this.syncSubscriptions();
    if (next) void this.refreshWorkspaceSessions();
  }

  get sessionId(): string | null { return this.#sessionId; }
  set sessionId(next: string | null) {
    if (this.#sessionId === next) return;
    this.#sessionId = next;
    this.freshContext = false;
    this.proposalCache.clear();
    this.syncSubscriptions();
  }

  get inlinePrompt(): IdeInlinePromptState | null { return this.#inlinePrompt; }
  set inlinePrompt(next: IdeInlinePromptState | null) {
    this.#inlinePrompt = next;
    this.syncSubscriptions();
  }

  get provider(): string { return this.#provider; }
  set provider(next: string) {
    const clean = next.trim();
    if (!clean || clean === 'shell' || this.#provider === clean) return;
    this.#provider = clean;
    lsSet(AGENT_PROVIDER_KEY, clean);
  }

  get model(): string | null { return this.#model; }
  set model(next: string | null) {
    const clean = (next ?? '').trim() || null;
    if (this.#model === clean) return;
    this.#model = clean;
    lsSet(AGENT_MODEL_KEY, clean ?? '');
  }

  get effort(): EffortLevel | null { return this.#effort; }
  set effort(next: EffortLevel | null) {
    if (this.#effort === next) return;
    this.#effort = next;
    lsSet(AGENT_EFFORT_KEY, next ?? '');
  }

  get accessMode(): 'default' | 'plan' | 'full' {
    if (this.planMode) return 'plan';
    return this.runtimeAccessMode === 'full-access' ? 'full' : 'default';
  }
  set accessMode(mode: 'default' | 'plan' | 'full') {
    this.planMode = mode === 'plan';
    if (mode === 'full') this.runtimeAccessMode = 'full-access';
    else if (mode === 'plan' || this.runtimeAccessMode === 'full-access') this.runtimeAccessMode = 'approval-required';
  }

  get interactionMode(): InteractionMode {
    if (this.planMode) return 'plan';
    return this.runtimeAccessMode === 'full-access' ? 'full' : 'accept-edits';
  }

  setInteractionMode(mode: InteractionMode): void {
    this.planMode = mode === 'plan';
    this.runtimeAccessMode =
      mode === 'full' ? 'full-access' : mode === 'plan' ? 'approval-required' : 'auto-accept-edits';
  }

  setRuntimeAccessMode(mode: RuntimeAccessMode): void {
    this.planMode = false;
    this.runtimeAccessMode = mode;
  }

  readonly projectRoots = $derived.by((): string[] => collectIdeProjectRoots());
  readonly activeWorkspaceId = $derived.by((): string => resolveIdeWorkspaceId());

  readonly sessions = $derived.by((): RuntimeSession[] => {
    const wsId = this.activeWorkspaceId;
    const roots = this.projectRoots;
    return runtimeThreads.live(false).filter((s) =>
      matchesIdeProjectThread({
        sessionId: s.id,
        activeSessionId: this.#sessionId,
        workspaceId: s.workspace_id,
        targetWorkspaceId: wsId,
        cwd: s.cwd,
        projectRoots: roots,
      }),
    );
  });

  readonly threads = $derived.by((): IdeThreadEntry[] =>
    collectIdeThreadEntries({
      runtimeAll: Object.values(runtimeThreads.byId),
      ptySessions: ws.sessions,
      statusMap: ws.statusMap,
      needsYouMap: ws.needsYou,
      activeSessionId: this.#sessionId,
      targetWorkspaceId: this.activeWorkspaceId,
      projectRoots: this.projectRoots,
    }),
  );

  readonly activeThread = $derived.by((): IdeThreadEntry | null =>
    this.#sessionId ? (this.threads.find((t) => t.id === this.#sessionId) ?? null) : null,
  );

  readonly activeRuntimeSession = $derived<RuntimeSession | null>(
    this.#sessionId ? (runtimeThreads.get(this.#sessionId) ?? null) : null,
  );

  readonly activeThreadKind = $derived<'runtime' | 'pty' | 'draft'>(
    !this.#sessionId
      ? 'draft'
      : runtimeThreads.get(this.#sessionId)
        ? 'runtime'
        : ws.getSession(this.#sessionId)
          ? 'pty'
          : 'runtime',
  );

  readonly availableProviders = $derived.by((): string[] =>
    resolveIdeHarnesses(agentProviders(), [this.#provider, ...this.threads.map((t) => t.provider)]),
  );

  readonly inlineProviders = $derived.by((): RuntimeProvider[] => resolveIdeInlineProviders(this.availableProviders));

  readonly effectiveModel = $derived<string | null>(
    this.#model ?? this.activeRuntimeSession?.model ?? this.activeThread?.model ?? null,
  );

  readonly canAttachImages = $derived.by((): boolean => {
    const p = this.activeRuntimeSession?.provider ?? this.#provider;
    return isRuntimeProvider(p) && p !== 'agy';
  });

  readonly canAttachFiles = $derived.by(
    (): boolean => (this.activeRuntimeSession?.provider ?? this.#provider) === 'claude',
  );

  readonly projection = $derived<RuntimeProjection | null>(
    this.#sessionId ? (runtimeViews.projections[this.#sessionId] ?? null) : null,
  );

  readonly isWorking = $derived.by((): boolean => isIdeSessionWorking(this.sending, this.#sessionId));

  readonly proposalsByMessageId = $derived.by((): Record<string, IdeCodeProposal[]> =>
    computeProposalsMap({
      projection: this.projection,
      appliedMap: this.appliedUndoByProposalId,
      appliedPathByProposalId: this.appliedPathByProposalId,
      proposalCache: this.proposalCache,
    }),
  );

  resolveWorkspaceId(): string { return resolveIdeWorkspaceId(); }
  resolveProjectCwd(): string | null { return resolveIdeProjectCwd(); }

  syncTargetRoot(nextKey = currentIdeTargetKey()): void {
    if (this.lastTargetKey && this.lastTargetKey !== nextKey) {
      this.#sessionId = null;
      this.freshContext = false;
      this.contexts = [];
      this.attachments = [];
      this.inlineReqSeq++;
      this.#inlinePrompt = null;
      this.error = null;
      this.proposalCache.clear();
      this.appliedPathByProposalId.clear();
      this.appliedUndoByProposalId = {};
      this.lastTargetKey = nextKey;
      this.syncSubscriptions();
      if (nextKey && this.#panelOpen) void this.refreshWorkspaceSessions();
      return;
    }
    if (!this.lastTargetKey && nextKey) {
      this.lastTargetKey = nextKey;
      if (this.#panelOpen) void this.refreshWorkspaceSessions();
    }
  }

  ensureMounted(): () => void {
    this.syncTargetRoot();
    this.syncSubscriptions();
    void this.refreshWorkspaceSessions();
    return () => {
      if (!this.#panelOpen && !this.#inlinePrompt?.open) this.syncSubscriptions(true);
    };
  }

  async refreshWorkspaceSessions(): Promise<void> {
    await loadIdeWorkspaceSessions(resolveIdeWorkspaceId(), this.inflightWsLoads);
  }

  private syncSubscriptions(forceRelease = false): void {
    const watchNeeded =
      !forceRelease &&
      (this.#panelOpen ||
        Boolean(this.#inlinePrompt?.open) ||
        this.#sessionId !== null ||
        this.contexts.length > 0 ||
        this.attachments.length > 0);
    if (watchNeeded && !this.stopTargetWatch) {
      this.lastTargetKey = currentIdeTargetKey();
      this.stopTargetWatch = $effect.root(() => {
        $effect(() => {
          this.syncTargetRoot(currentIdeTargetKey());
        });
      });
    } else if (!watchNeeded && this.stopTargetWatch) {
      this.stopTargetWatch();
      this.stopTargetWatch = null;
      if (forceRelease) this.lastTargetKey = '';
    }

    const active = !forceRelease && (this.#panelOpen || Boolean(this.#inlinePrompt?.open));
    if (!active) {
      this.stopViewOpen?.();
      this.stopViewOpen = null;
      this.stopThreadWatch?.();
      this.stopThreadWatch = null;
      this.subscribedSessionId = null;
      this.stopThreadsMount?.();
      this.stopThreadsMount = null;
      return;
    }
    if (!this.stopThreadsMount) this.stopThreadsMount = runtimeThreads.mount();
    const targetSid = this.#sessionId;
    const isPtyOnly = Boolean(targetSid && !runtimeThreads.byId[targetSid] && ws.getSession(targetSid));
    if (!targetSid || isPtyOnly) {
      this.stopViewOpen?.();
      this.stopViewOpen = null;
      this.stopThreadWatch?.();
      this.stopThreadWatch = null;
      this.subscribedSessionId = null;
      return;
    }
    if (this.subscribedSessionId !== targetSid) {
      this.stopViewOpen?.();
      this.stopThreadWatch?.();
      this.subscribedSessionId = targetSid;
      this.stopViewOpen = runtimeViews.open(targetSid);
      this.stopThreadWatch = runtimeThreads.watch(targetSid);
      void runtimeThreads.loadLinked(targetSid).catch(() => {});
    }
  }

  private upsertContext(draft: Omit<IdeContextChip, 'id'> & { id?: string }): IdeContextChip {
    const { chip, next } = upsertIdeContextChip(this.contexts, draft, () => `ctx-${++this.chipSeq}`);
    this.contexts = next;
    this.syncSubscriptions();
    return chip;
  }

  togglePanel(open?: boolean): void { this.panelOpen = open ?? !this.#panelOpen; }

  setProvider(next: string): void {
    const clean = next.trim();
    if (!clean || clean === 'shell') return;
    const activeProv = this.activeThread?.provider ?? null;
    if (this.#sessionId && activeProv && activeProv !== clean) this.sessionId = null;
    if (this.#provider !== clean) {
      this.provider = clean;
      this.model = null;
      if (!supportsEffortSelection(clean)) this.effort = null;
    }
  }

  cycleProvider(): void {
    const list = this.availableProviders;
    if (list.length > 0) this.setProvider(list[(list.indexOf(this.#provider) + 1) % list.length]);
  }

  setAccessMode(mode: 'default' | 'plan' | 'full'): void { this.accessMode = mode; }

  startNewChat(): void {
    this.error = null;
    this.sessionId = null;
    council.on = false;
    council.seats = [];
  }

  removeContext(id: string): void {
    this.contexts = this.contexts.filter((c) => c.id !== id);
    this.syncSubscriptions();
  }

  clearContexts(): void {
    this.contexts = [];
    this.syncSubscriptions();
  }

  async addAttachments(files: Iterable<File>): Promise<void> {
    if (!this.canAttachImages) return;
    const allowPdf = this.canAttachFiles;
    const read = await readAttachments(
      files,
      (f) => f.type.startsWith('image/') || (allowPdf && f.type === 'application/pdf'),
    );
    if (read.length > 0) {
      this.attachments = [...this.attachments, ...read];
      this.syncSubscriptions();
    }
  }

  removeAttachment(index: number): void {
    this.attachments = this.attachments.filter((_, i) => i !== index);
    this.syncSubscriptions();
  }

  openActiveInThreads(targetId?: string | null): void {
    openIdeThreadInThreadsView(targetId ?? this.#sessionId, resolveIdeWorkspaceId(), resolveIdeProjectCwd());
  }

  async reopenActiveSession(): Promise<boolean> {
    const sid = this.#sessionId;
    if (!sid || this.reopening) return false;
    this.reopening = true;
    try {
      await runtimeThreads.reopen(sid);
      this.error = null;
      return true;
    } catch (e) {
      this.error = loadErrorText(e);
      return false;
    } finally {
      this.reopening = false;
    }
  }

  async selectSession(id: string | null): Promise<void> {
    this.error = null;
    this.sessionId = id;
    this.syncSubscriptions();
    if (!id) return;
    try {
      const meta = await loadSelectedIdeSessionMeta(id);
      if (!meta) return;
      this.provider = meta.provider;
      if (meta.model !== undefined) this.model = meta.model;
      if (meta.accessMode) this.runtimeAccessMode = meta.accessMode;
    } catch (e) {
      this.error = loadErrorText(e);
    }
  }

  hasContextKind(kind: IdeContextChip['kind'], path?: string | null): boolean {
    if (!path) return this.contexts.some((c) => c.kind === kind);
    const full = ide.resolvePath(path);
    return this.contexts.some((c) => c.kind === kind && c.path === full);
  }

  async toggleActiveFilePill(): Promise<void> {
    if (!ide.activePath) return;
    const full = ide.resolvePath(ide.activePath);
    const existing = this.contexts.find((c) => c.kind === 'file' && c.path === full);
    if (existing) this.removeContext(existing.id);
    else await this.attachFile(full, { openPanel: false });
  }

  toggleSelectionPill(): void {
    const existing = this.contexts.find((c) => c.kind === 'selection');
    if (existing) this.removeContext(existing.id);
    else this.markSelectionFromEditor({ openPanel: false });
  }

  toggleProblemsPill(): void {
    if (this.contexts.some((c) => c.kind === 'diagnostic')) {
      this.contexts = this.contexts.filter((c) => c.kind !== 'diagnostic');
      this.syncSubscriptions();
      return;
    }
    for (const diag of ide.problems.slice(0, 3)) {
      this.attachDiagnostic(diag, { openPanel: false, preserveComposer: true });
    }
  }

  toggleDebugPill(): void {
    if (this.contexts.some((c) => c.kind === 'debug_frame')) {
      this.contexts = this.contexts.filter((c) => c.kind !== 'debug_frame');
      this.syncSubscriptions();
      return;
    }
    if (ide.activeDebug?.state === 'paused') {
      this.attachDebugFrame(ide.activeDebug, ide.expandedVariables, { openPanel: false, preserveComposer: true });
    }
  }

  toggleFailedRunPill(): void {
    if (this.contexts.some((c) => c.kind === 'test_failure' || c.kind === 'run_output')) {
      this.contexts = this.contexts.filter((c) => c.kind !== 'test_failure' && c.kind !== 'run_output');
      this.syncSubscriptions();
      return;
    }
    const run = ide.activeRun;
    if (!run) return;
    const failedTest = run.tests.find((t) => t.status === 'failed');
    if (failedTest) this.attachTestFailure(failedTest, run.output ?? [], { openPanel: false, preserveComposer: true });
    else this.upsertContext(buildFailedRunDraftChip(run));
  }

  markSelectionFromEditor(opts?: { openPanel?: boolean; selection?: IdeSelectionInput }): IdeContextChip | null {
    const draft = buildSelectionDraftChip(opts?.selection, readIdeFileBuffer);
    if (!draft) return null;
    const chip = this.upsertContext(draft);
    if (opts?.openPanel !== false) this.panelOpen = true;
    return chip;
  }

  openInlineEdit(sel?: IdeSelectionInput): void {
    const next = buildIdeInlinePromptState(
      sel,
      this.#inlinePrompt,
      this.#provider,
      this.effectiveModel,
      this.#effort,
      readIdeFileBuffer,
    );
    if (next) this.inlinePrompt = next;
  }

  setInlineProvider(next: string): void {
    const st = this.#inlinePrompt;
    const clean = next.trim();
    if (!st || !isRuntimeProvider(clean) || st.provider === clean) return;
    st.provider = clean;
    st.model = null;
    if (!supportsEffortSelection(clean)) st.effort = null;
  }

  setInlineModel(next: string | null): void {
    if (this.#inlinePrompt) this.#inlinePrompt.model = (next ?? '').trim() || null;
  }

  setInlineEffort(next: EffortLevel | null): void {
    if (this.#inlinePrompt) this.#inlinePrompt.effort = next;
  }

  closeInlineEdit(): void {
    this.inlineReqSeq++;
    if (this.#inlinePrompt) this.inlinePrompt = null;
  }

  handoffInlineToChat(): void {
    const st = this.#inlinePrompt;
    if (!st) return;
    const q = st.query.trim();
    if (st.provider) this.setProvider(st.provider);
    if (st.model !== undefined) this.model = st.model;
    this.effort = normalizeEffort(st.effort);
    this.markSelectionFromEditor({
      openPanel: true,
      selection: {
        path: st.path,
        from: st.from,
        to: st.to,
        startLine: st.startLine,
        endLine: st.endLine,
        text: st.selectedText,
      },
    });
    if (q) this.composerText = this.composerText.trim() ? `${this.composerText.trim()}\n${q}` : q;
    this.closeInlineEdit();
  }

  async submitInlineEdit(query?: string): Promise<void> {
    const st = this.#inlinePrompt;
    if (!st || !st.open || st.busy) return;
    const q = (query ?? st.query).trim();
    if (!q) return;
    if (!isRuntimeProvider(st.provider)) {
      this.handoffInlineToChat();
      void this.sendPrompt();
      return;
    }
    st.query = q;
    st.busy = true;
    st.error = null;
    st.proposal = null;
    const reqSeq = ++this.inlineReqSeq;
    try {
      const proposal = await executeIdeInlineEdit({
        state: st,
        query: q,
        wsId: resolveIdeWorkspaceId(),
        localCwd: resolveIdeProjectCwd(),
        planMode: this.planMode,
        accessMode: this.planMode ? 'approval-required' : this.runtimeAccessMode,
        existingSessionId: this.#sessionId,
        onSessionBound: (id) => {
          this.#sessionId = id;
        },
        syncSubscriptions: () => this.syncSubscriptions(),
        isCancelled: () => this.inlineReqSeq !== reqSeq || !this.#inlinePrompt?.open,
      });
      if (this.inlineReqSeq !== reqSeq || !this.#inlinePrompt?.open) return;
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
    try {
      const chip = this.upsertContext(await buildFileDraftChip(path, readIdeFileBuffer));
      if (opts?.openPanel !== false) this.panelOpen = true;
      return chip;
    } catch (e) {
      this.error = loadErrorText(e);
      return null;
    }
  }

  attachSymbol(path: string, sym: IdeSymbolNode, opts?: { openPanel?: boolean }): IdeContextChip | null {
    const fullPath = ide.resolvePath(path);
    const relPath = ide.relativePath(fullPath) || ideBasename(fullPath);
    const chip = this.upsertContext(buildSymbolDraftChip(fullPath, relPath, readIdeFileBuffer(fullPath), sym));
    if (opts?.openPanel !== false) this.panelOpen = true;
    return chip;
  }

  attachDiagnostic(
    diag: IdeDiagnostic,
    opts?: { openPanel?: boolean; autoPrompt?: boolean; preserveComposer?: boolean },
  ): void {
    const fullPath = ide.resolvePath(diag.path);
    const relPath = ide.relativePath(fullPath) || ideBasename(fullPath);
    this.upsertContext(buildDiagnosticDraftChip(fullPath, relPath, readIdeFileBuffer(fullPath), diag));
    if (opts?.openPanel !== false) this.panelOpen = true;
    if (!opts?.preserveComposer && !this.composerText.trim()) {
      this.composerText = `Fix this ${diag.severity} in ${relPath}:${diag.line}: ${diag.message}`;
    }
    if (opts?.autoPrompt) void this.sendPrompt();
  }

  attachTestFailure(
    test: IdeTestResult,
    outputLines: string[],
    opts?: { openPanel?: boolean; autoPrompt?: boolean; preserveComposer?: boolean },
  ): void {
    const fullPath = test.file ? ide.resolvePath(test.file) : undefined;
    const relPath = fullPath ? ide.relativePath(fullPath) : undefined;
    this.upsertContext(buildTestFailureDraftChip(test, outputLines, fullPath, relPath));
    if (opts?.openPanel !== false) this.panelOpen = true;
    if (!opts?.preserveComposer && !this.composerText.trim()) {
      this.composerText = `Investigate and fix the failing test ${test.name}.`;
    }
    if (opts?.autoPrompt) void this.sendPrompt();
  }

  attachDebugFrame(
    dbg: IdeDebugSessionSnapshot,
    expandedVars?: Record<number, IdeVariable[]>,
    opts?: { openPanel?: boolean; autoPrompt?: boolean; preserveComposer?: boolean },
  ): void {
    const { chip, shortFile, line } = buildDebugFrameDraftChip(
      dbg,
      expandedVars,
      (p) => ide.resolvePath(p),
      (p) => ide.relativePath(p),
    );
    this.upsertContext(chip);
    if (opts?.openPanel !== false) this.panelOpen = true;
    if (!opts?.preserveComposer && !this.composerText.trim()) {
      this.composerText = `Inspect the paused debugger frame at ${shortFile}:${line} and help fix the issue.`;
    }
    if (opts?.autoPrompt) void this.sendPrompt();
  }

  async sendPrompt(overrideText?: string): Promise<void> {
    const userText = (overrideText ?? this.composerText).trim();
    if ((!userText && this.contexts.length === 0 && this.attachments.length === 0) || this.sending) return;
    if (!this.#panelOpen) this.panelOpen = true;
    const snapshotContexts = this.contexts.slice();
    const snapshotFiles = this.attachments.slice();
    const snapshotFresh = this.freshContext;
    const seedText =
      userText ||
      (snapshotContexts.length > 0 ? 'Inspect the attached context and propose improvements.' : 'Inspect the attached files.');
    const fullPrompt = buildIdeAgentPrompt(seedText, snapshotContexts, {
      workspaceRoot: resolveIdeProjectCwd() ?? undefined,
      activePath: ide.activePath,
      planMode: this.planMode,
      omitWhenEmpty: true,
    });

    this.sending = true;
    this.error = null;
    try {
      await executeIdePromptSend({
        seedText,
        fullPrompt,
        provider: this.#provider,
        model: this.#model,
        effort: this.#effort,
        interactionMode: this.interactionMode,
        accessMode: this.planMode ? 'approval-required' : this.runtimeAccessMode,
        freshContext: snapshotFresh,
        files: snapshotFiles,
        wsId: resolveIdeWorkspaceId(),
        localCwd: resolveIdeProjectCwd(),
        currentSessionId: this.#sessionId,
        onSessionBound: (id) => {
          this.#sessionId = id;
        },
        selectSession: (id) => this.selectSession(id),
        syncSubscriptions: () => this.syncSubscriptions(),
      });
      if (overrideText === undefined) this.composerText = '';
      this.contexts = [];
      this.attachments = [];
      this.freshContext = false;
    } catch (e) {
      this.error = loadErrorText(e);
    } finally {
      this.sending = false;
    }
  }

  async stopTurn(): Promise<void> {
    if (!this.#sessionId) return;
    try {
      await interruptIdeSessionTurn(this.#sessionId);
    } catch (e) {
      this.error = loadErrorText(e);
    }
  }

  async respondApproval(requestId: string, decision: ApprovalDecision): Promise<void> {
    if (!this.#sessionId) return;
    try {
      await runtimeThreads.respond(this.#sessionId, requestId, decision);
    } catch (e) {
      this.error = loadErrorText(e);
    }
  }

  async applyProposal(proposal: IdeCodeProposal): Promise<boolean> {
    const next = await applyIdeCodeProposal(proposal, this.appliedUndoByProposalId, this.appliedPathByProposalId);
    if (!next) return false;
    this.appliedUndoByProposalId = next;
    return true;
  }

  async undoProposal(proposalId: string): Promise<boolean> {
    const next = await undoIdeCodeProposal(
      proposalId,
      this.appliedUndoByProposalId,
      this.appliedPathByProposalId,
      this.proposalsByMessageId,
    );
    if (!next) return false;
    this.appliedUndoByProposalId = next;
    return true;
  }

  dispose(): void {
    this.inlineReqSeq++;
    this.#inlinePrompt = null;
    this.sending = false;
    this.proposalCache.clear();
    this.appliedPathByProposalId.clear();
    this.appliedUndoByProposalId = {};
    this.syncSubscriptions(true);
  }
}

export const ideAgent = new IdeAgentController();
