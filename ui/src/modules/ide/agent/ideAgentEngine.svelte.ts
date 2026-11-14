// Lazily loaded Otto Threads / structured-runtime engine for the IDE AI Coding
// Agent dock (`IdeAgentPanel.svelte`) and `⌘I` inline edit (`IdeInlinePromptBar.svelte`).
// Kept out of `IdePage.svelte`'s static import closure via `ideAgentController.svelte.ts`.

export * from '../store/ideDiffApplier';
export type { IdeThreadEntry } from './ideContextBuilders';

import type { ApprovalDecision, RuntimeAccessMode, RuntimeProvider, RuntimeSession } from '../../../lib/api/types';
import { loadErrorText } from '../../../lib/loadError';
import { normalizeEffort, supportsEffortSelection, type EffortLevel, type InteractionMode } from '../../../lib/providerModels';
import { agentProviders } from '../../../lib/providers';
import { runtimeDrafts, type ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { council } from '../../threads/runtime/councilStore.svelte'; // ui-guards: allow — IDE agent dock supports Council multi-agent threads
import type { RuntimeProjection } from '../../threads/runtime/projection'; // ui-guards: allow — IDE agent dock shares structured runtime projections
import { readAttachments } from '../../threads/runtime/runtimeComposer'; // ui-guards: allow — IDE agent dock shares image/PDF attachment reader
import { isRuntimeProvider, runtimeThreads } from '../../threads/runtime/runtimeThreads.svelte'; // ui-guards: allow — IDE agent dock drives real Otto Threads
import { runtimeViews } from '../../threads/runtime/runtimeView.svelte'; // ui-guards: allow — IDE agent dock streams structured runtime views
import {
  ide,
  type IdeDebugSessionSnapshot,
  type IdeDiagnostic,
  type IdeSymbolNode,
  type IdeTestResult,
  type IdeVariable,
} from '../ideStore.svelte';
import {
  matchesIdeProjectThread,
  resolveIdeHarnesses,
  resolveIdeInlineProviders,
  type IdeCodeProposal,
  type IdeContextChip,
  type IdeInlinePromptState,
} from '../store/ideDiffApplier';
import {
  applyIdeCodeProposal,
  computeProposalsMap,
  createDebugFrameChipEntry,
  createDiagnosticChipEntry,
  createFileChipFromPath,
  createSymbolChipFromPath,
  createTestFailureChipEntry,
  undoIdeCodeProposal,
} from './ideAgentRuntimeOps.svelte';
import {
  IDE_OPEN_THREAD_EVENT,
  ideAgentState,
  type IdeAgentEngineBridge,
  type IdeShellSelectionInput,
} from './ideAgentState.svelte';
import {
  interruptIdeSessionTurn,
  isIdeSessionWorking,
  loadIdeWorkspaceSessions,
  loadSelectedIdeSessionMeta,
  openIdeThreadInThreadsView,
  rememberIdeProjectRoot,
  runIdeComposerStateTurn,
  runIdeInlineEditStateTurn,
} from './ideAgentTurnRunner.svelte';
import {
  buildFailedRunDraftChip,
  collectIdeProjectRoots,
  collectIdeThreadEntries,
  currentIdeTargetKey,
  resolveIdeProjectCwd,
  resolveIdeWorkspaceId,
  type IdeThreadEntry,
} from './ideContextBuilders';

class IdeAgentEngine implements IdeAgentEngineBridge {
  private stopThreadsMount: (() => void) | null = null;
  private subscribedSessionId: string | null = null;
  private stopViewOpen: (() => void) | null = null;
  private stopThreadWatch: (() => void) | null = null;
  private stopTargetWatch: (() => void) | null = null;
  private lastTargetKey = '';
  private inflightWsLoads = new Map<string, Promise<void>>();

  get panelOpen(): boolean {
    return ideAgentState.panelOpen;
  }
  set panelOpen(next: boolean) {
    if (!ideAgentState.setPanelOpen(next)) return;
    this.syncSubscriptions();
    if (next) void this.refreshWorkspaceSessions();
  }

  get sessionId(): string | null {
    return ideAgentState.sessionId;
  }
  set sessionId(next: string | null) {
    if (ideAgentState.sessionId === next) return;
    ideAgentState.sessionId = next;
    ideAgentState.freshContext = false;
    ideAgentState.proposalCache.clear();
    this.syncSubscriptions();
  }

  get inlinePrompt(): IdeInlinePromptState | null {
    return ideAgentState.inlinePrompt;
  }
  set inlinePrompt(next: IdeInlinePromptState | null) {
    ideAgentState.inlinePrompt = next;
    this.syncSubscriptions();
  }

  get provider(): string { return ideAgentState.provider; }
  set provider(next: string) { ideAgentState.setProvider(next); }

  get model(): string | null { return ideAgentState.model; }
  set model(next: string | null) { ideAgentState.setModel(next); }

  get effort(): EffortLevel | null { return ideAgentState.effort; }
  set effort(next: EffortLevel | null) { ideAgentState.setEffort(next); }

  get runtimeAccessMode(): RuntimeAccessMode { return ideAgentState.runtimeAccessMode; }
  set runtimeAccessMode(next: RuntimeAccessMode) { ideAgentState.runtimeAccessMode = next; }

  get planMode(): boolean { return ideAgentState.planMode; }
  set planMode(next: boolean) { ideAgentState.planMode = next; }

  get freshContext(): boolean { return ideAgentState.freshContext; }
  set freshContext(next: boolean) { ideAgentState.freshContext = next; }

  get reopening(): boolean { return ideAgentState.reopening; }
  set reopening(next: boolean) { ideAgentState.reopening = next; }

  get composerText(): string {
    const sid = ideAgentState.sessionId;
    return sid && runtimeThreads.byId[sid] ? runtimeDrafts.for(sid).text : ideAgentState.composerText;
  }
  set composerText(next: string) {
    const sid = ideAgentState.sessionId;
    if (sid && runtimeThreads.byId[sid]) runtimeDrafts.for(sid).text = next;
    else ideAgentState.composerText = next;
  }

  get attachments(): ComposerFile[] { return ideAgentState.attachments; }
  set attachments(next: ComposerFile[]) { ideAgentState.attachments = next; }

  get sending(): boolean { return ideAgentState.sending; }
  set sending(next: boolean) { ideAgentState.sending = next; }

  get error(): string | null { return ideAgentState.error; }
  set error(next: string | null) { ideAgentState.error = next; }

  get contexts(): IdeContextChip[] { return ideAgentState.contexts; }
  set contexts(next: IdeContextChip[]) { ideAgentState.contexts = next; }

  get appliedUndoByProposalId(): Record<string, string> { return ideAgentState.appliedUndoByProposalId; }
  set appliedUndoByProposalId(next: Record<string, string>) { ideAgentState.appliedUndoByProposalId = next; }

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
        activeSessionId: ideAgentState.sessionId,
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
      activeSessionId: ideAgentState.sessionId,
      targetWorkspaceId: this.activeWorkspaceId,
      projectRoots: this.projectRoots,
    }),
  );

  readonly activeThread = $derived.by((): IdeThreadEntry | null =>
    ideAgentState.sessionId ? (this.threads.find((t) => t.id === ideAgentState.sessionId) ?? null) : null,
  );

  readonly activeRuntimeSession = $derived<RuntimeSession | null>(
    ideAgentState.sessionId ? (runtimeThreads.get(ideAgentState.sessionId) ?? null) : null,
  );

  readonly activeThreadKind = $derived<'runtime' | 'pty' | 'draft'>(
    !ideAgentState.sessionId
      ? 'draft'
      : runtimeThreads.get(ideAgentState.sessionId)
        ? 'runtime'
        : ws.getSession(ideAgentState.sessionId)
          ? 'pty'
          : 'runtime',
  );

  readonly availableProviders = $derived.by((): string[] =>
    resolveIdeHarnesses(agentProviders(), [ideAgentState.provider, ...this.threads.map((t) => t.provider)]),
  );

  readonly inlineProviders = $derived.by((): RuntimeProvider[] => resolveIdeInlineProviders(this.availableProviders));

  readonly effectiveModel = $derived<string | null>(
    ideAgentState.model ?? this.activeRuntimeSession?.model ?? this.activeThread?.model ?? null,
  );

  readonly canAttachImages = $derived.by((): boolean => {
    const p = this.activeRuntimeSession?.provider ?? ideAgentState.provider;
    return isRuntimeProvider(p) && p !== 'agy';
  });

  readonly canAttachFiles = $derived.by(
    (): boolean => (this.activeRuntimeSession?.provider ?? ideAgentState.provider) === 'claude',
  );

  readonly projection = $derived<RuntimeProjection | null>(
    ideAgentState.sessionId ? (runtimeViews.projections[ideAgentState.sessionId] ?? null) : null,
  );

  readonly isWorking = $derived.by((): boolean => isIdeSessionWorking(ideAgentState.sending, ideAgentState.sessionId));

  readonly proposalsByMessageId = $derived.by((): Record<string, IdeCodeProposal[]> =>
    computeProposalsMap({
      projection: this.projection,
      appliedMap: ideAgentState.appliedUndoByProposalId,
      appliedPathByProposalId: ideAgentState.appliedPathByProposalId,
      proposalCache: ideAgentState.proposalCache,
    }),
  );

  resolveWorkspaceId(): string {
    return resolveIdeWorkspaceId();
  }

  resolveProjectCwd(): string | null {
    return resolveIdeProjectCwd();
  }

  syncInlineOpen(): void {
    const st = ideAgentState.inlinePrompt;
    if (st && st.model === null && this.effectiveModel) st.model = this.effectiveModel;
    this.syncSubscriptions();
  }

  private publishLiveIdeContext(): void {
    if (typeof window === 'undefined') return;
    const root = ide.target.kind === 'local' ? (ide.target.root ?? '').trim().replace(/\/+$/, '') : '';
    (window as unknown as Record<string, unknown>).__ottoIdeLiveContext = {
      root,
      additionalRoots: [...ide.additionalRoots],
      activePath: ide.activePath,
      openTabs: ide.tabs.map((t) => ({ path: t.path, name: t.name })),
    };
    if (root && root !== '~') rememberIdeProjectRoot(root);
    window.dispatchEvent(new CustomEvent('otto:ide-context-changed'));
  }

  applyPendingHandoff(): void {
    const handoff = ideAgentState.takePendingHandoff();
    if (!handoff) return;
    this.panelOpen = true;
    if (handoff.sessionId) void this.selectSession(handoff.sessionId);
    else this.startNewChat();
  }

  syncTargetRoot(nextKey = currentIdeTargetKey()): void {
    this.publishLiveIdeContext();
    if (this.lastTargetKey && this.lastTargetKey !== nextKey) {
      ideAgentState.resetTargetState();
      this.lastTargetKey = nextKey;
      this.syncSubscriptions();
      if (nextKey && ideAgentState.panelOpen) void this.refreshWorkspaceSessions();
      return;
    }
    if (!this.lastTargetKey && nextKey) {
      this.lastTargetKey = nextKey;
      if (ideAgentState.panelOpen) void this.refreshWorkspaceSessions();
    }
  }

  ensureMounted(): () => void {
    this.syncTargetRoot();
    this.syncSubscriptions();
    this.applyPendingHandoff();
    void this.refreshWorkspaceSessions();
    const onHandoff = (): void => this.applyPendingHandoff();
    if (typeof window !== 'undefined') window.addEventListener(IDE_OPEN_THREAD_EVENT, onHandoff);
    const stopLiveCtx = $effect.root(() => {
      $effect(() => {
        void [ide.target.kind, ide.target.root, ide.additionalRoots.length, ide.activePath, ide.tabs.length];
        this.publishLiveIdeContext();
      });
    });
    return () => {
      stopLiveCtx();
      if (typeof window !== 'undefined') window.removeEventListener(IDE_OPEN_THREAD_EVENT, onHandoff);
      if (!ideAgentState.panelOpen && !ideAgentState.inlinePrompt?.open) this.syncSubscriptions(true);
    };
  }

  async refreshWorkspaceSessions(): Promise<void> {
    await loadIdeWorkspaceSessions(resolveIdeWorkspaceId(), this.inflightWsLoads);
  }

  syncSubscriptions(forceRelease = false): void {
    const watchNeeded =
      !forceRelease &&
      (ideAgentState.panelOpen ||
        Boolean(ideAgentState.inlinePrompt?.open) ||
        ideAgentState.sessionId !== null ||
        ideAgentState.contexts.length > 0 ||
        ideAgentState.attachments.length > 0);
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

    const active = !forceRelease && (ideAgentState.panelOpen || Boolean(ideAgentState.inlinePrompt?.open));
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
    const targetSid = ideAgentState.sessionId;
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
    const chip = ideAgentState.upsertContext(draft);
    this.syncSubscriptions();
    return chip;
  }

  togglePanel(open?: boolean): void {
    this.panelOpen = open ?? !ideAgentState.panelOpen;
  }

  setProvider(next: string): void {
    const clean = next.trim();
    if (!clean || clean === 'shell') return;
    const activeProv = this.activeThread?.provider ?? null;
    if (ideAgentState.sessionId && activeProv && activeProv !== clean) this.sessionId = null;
    if (ideAgentState.provider !== clean) {
      this.provider = clean;
      this.model = null;
      if (!supportsEffortSelection(clean)) this.effort = null;
    }
  }

  cycleProvider(): void {
    const list = this.availableProviders;
    if (list.length > 0) this.setProvider(list[(list.indexOf(ideAgentState.provider) + 1) % list.length]);
  }

  setAccessMode(mode: 'default' | 'plan' | 'full'): void {
    this.accessMode = mode;
  }

  startNewChat(): void {
    this.error = null;
    this.sessionId = null;
    ideAgentState.composerText = '';
    council.on = false;
    council.seats = [];
  }

  attachMentionedPath(opts: { path: string; display: string; isDir: boolean }): void {
    if (opts.isDir) {
      const clean = opts.display.replace(/\/+$/, '');
      this.upsertContext({
        key: `dir:${opts.path}`,
        kind: 'file',
        label: `@${clean}/`,
        path: opts.path,
        snippet: `Folder path: ${opts.path}`,
      });
      return;
    }
    void this.attachFile(opts.path, { openPanel: false });
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
    openIdeThreadInThreadsView(targetId ?? ideAgentState.sessionId, resolveIdeWorkspaceId(), resolveIdeProjectCwd());
  }

  async reopenActiveSession(): Promise<boolean> {
    const sid = ideAgentState.sessionId;
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

  markSelectionFromEditor(opts?: { openPanel?: boolean; selection?: IdeShellSelectionInput }): IdeContextChip | null {
    const chip = ideAgentState.markSelectionSync(opts?.selection);
    if (!chip) return null;
    if (opts?.openPanel !== false) this.panelOpen = true;
    else this.syncSubscriptions();
    return chip;
  }

  openInlineEdit(sel?: IdeShellSelectionInput): void {
    if (ideAgentState.openInlinePromptSync(sel, this.effectiveModel)) this.syncSubscriptions();
  }

  setInlineProvider(next: string): void {
    const st = ideAgentState.inlinePrompt;
    const clean = next.trim();
    if (!st || !isRuntimeProvider(clean) || st.provider === clean) return;
    st.provider = clean;
    st.model = null;
    if (!supportsEffortSelection(clean)) st.effort = null;
  }

  setInlineModel(next: string | null): void {
    if (ideAgentState.inlinePrompt) ideAgentState.inlinePrompt.model = (next ?? '').trim() || null;
  }

  setInlineEffort(next: EffortLevel | null): void {
    if (ideAgentState.inlinePrompt) ideAgentState.inlinePrompt.effort = next;
  }

  closeInlineEdit(): void {
    ideAgentState.inlineReqSeq++;
    if (ideAgentState.inlinePrompt) this.inlinePrompt = null;
  }

  handoffInlineToChat(): void {
    const st = ideAgentState.inlinePrompt;
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
    await runIdeInlineEditStateTurn({
      query,
      planMode: this.planMode,
      accessMode: this.runtimeAccessMode,
      handoffAndSend: () => {
        this.handoffInlineToChat();
        void this.sendPrompt();
      },
      syncSubscriptions: () => this.syncSubscriptions(),
    });
  }

  async acceptInlineProposal(): Promise<boolean> {
    const proposal = ideAgentState.inlinePrompt?.proposal;
    if (!proposal) return false;
    const applied = await this.applyProposal(proposal);
    if (applied) this.closeInlineEdit();
    return applied;
  }

  async attachFile(path: string, opts?: { openPanel?: boolean }): Promise<IdeContextChip | null> {
    try {
      const chip = this.upsertContext(await createFileChipFromPath(path));
      if (opts?.openPanel !== false) this.panelOpen = true;
      return chip;
    } catch (e) {
      this.error = loadErrorText(e);
      return null;
    }
  }

  attachSymbol(path: string, sym: IdeSymbolNode, opts?: { openPanel?: boolean }): IdeContextChip | null {
    const chip = this.upsertContext(createSymbolChipFromPath(path, sym));
    if (opts?.openPanel !== false) this.panelOpen = true;
    return chip;
  }

  private applyAttachedEntry(
    entry: { chip: Omit<IdeContextChip, 'id'>; prompt: string },
    opts?: { openPanel?: boolean; autoPrompt?: boolean; preserveComposer?: boolean },
  ): void {
    this.upsertContext(entry.chip);
    if (opts?.openPanel !== false) this.panelOpen = true;
    if (!opts?.preserveComposer && !this.composerText.trim()) this.composerText = entry.prompt;
    if (opts?.autoPrompt) void this.sendPrompt();
  }

  attachDiagnostic(
    diag: IdeDiagnostic,
    opts?: { openPanel?: boolean; autoPrompt?: boolean; preserveComposer?: boolean },
  ): void {
    this.applyAttachedEntry(createDiagnosticChipEntry(diag), opts);
  }

  attachTestFailure(
    test: IdeTestResult,
    outputLines: string[],
    opts?: { openPanel?: boolean; autoPrompt?: boolean; preserveComposer?: boolean },
  ): void {
    this.applyAttachedEntry(createTestFailureChipEntry(test, outputLines), opts);
  }

  attachDebugFrame(
    dbg: IdeDebugSessionSnapshot,
    expandedVars?: Record<number, IdeVariable[]>,
    opts?: { openPanel?: boolean; autoPrompt?: boolean; preserveComposer?: boolean },
  ): void {
    this.applyAttachedEntry(createDebugFrameChipEntry(dbg, expandedVars), opts);
  }

  async sendPrompt(overrideText?: string): Promise<void> {
    await runIdeComposerStateTurn({
      overrideText,
      interactionMode: this.interactionMode,
      accessMode: this.runtimeAccessMode,
      planMode: this.planMode,
      ensurePanelOpen: () => {
        if (!this.panelOpen) this.panelOpen = true;
      },
      selectSession: (id) => this.selectSession(id),
      syncSubscriptions: () => this.syncSubscriptions(),
    });
  }

  async stopTurn(): Promise<void> {
    if (!ideAgentState.sessionId) return;
    try {
      await interruptIdeSessionTurn(ideAgentState.sessionId);
    } catch (e) {
      this.error = loadErrorText(e);
    }
  }

  async respondApproval(requestId: string, decision: ApprovalDecision): Promise<void> {
    if (!ideAgentState.sessionId) return;
    try {
      await runtimeThreads.respond(ideAgentState.sessionId, requestId, decision);
    } catch (e) {
      this.error = loadErrorText(e);
    }
  }

  async applyProposal(proposal: IdeCodeProposal): Promise<boolean> {
    const next = await applyIdeCodeProposal(
      proposal,
      ideAgentState.appliedUndoByProposalId,
      ideAgentState.appliedPathByProposalId,
    );
    if (!next) return false;
    this.appliedUndoByProposalId = next;
    return true;
  }

  async undoProposal(proposalId: string): Promise<boolean> {
    const next = await undoIdeCodeProposal(
      proposalId,
      ideAgentState.appliedUndoByProposalId,
      ideAgentState.appliedPathByProposalId,
      this.proposalsByMessageId,
    );
    if (!next) return false;
    this.appliedUndoByProposalId = next;
    return true;
  }

  dispose(): void {
    ideAgentState.disposeState();
    this.syncSubscriptions(true);
  }
}

export type { IdeAgentEngine };
export const ideAgentEngine = new IdeAgentEngine();
ideAgentState.engine = ideAgentEngine;
