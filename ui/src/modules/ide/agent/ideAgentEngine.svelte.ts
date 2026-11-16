// Lazily loaded Otto Threads / structured-runtime engine for the IDE AI Coding
// Agent dock (`IdeAgentPanel.svelte`) and `⌘I` inline edit (`IdeInlinePromptBar.svelte`).
// Kept out of `IdePage.svelte`'s static import closure via `ideAgentController.svelte.ts`.

export * from '../store/ideDiffApplier';
export type { IdeThreadEntry } from './ideContextBuilders';

import { untrack } from 'svelte';
import type { ApprovalDecision, RuntimeAccessMode, RuntimeProviderOrSlug, RuntimeSession } from '../../../lib/api/types';
import { loadErrorText } from '../../../lib/loadError';
import { normalizeEffort, supportsEffortSelection, type EffortLevel, type InteractionMode } from '../../../lib/providerModels';
import { agentProviders, attachmentSupport, capsOf, defaultAgentProvider, isRuntimeProvider, runtimeBaselineCaps, sessionProviderSlug, takesFile, type TakesCaps } from '../../../lib/providers';
import { getIdeRuntimeComposerUi, type RuntimeProjectionView, type RuntimeSessionPort } from '../../../lib/runtimeThreadPort';
import { auth } from '../../../lib/stores/auth.svelte';
import { runtimeDrafts, type ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { ide, type IdeDebugSessionSnapshot, type IdeDiagnostic, type IdeSymbolNode, type IdeTestResult, type IdeVariable } from '../ideStore.svelte';
import { matchesIdeProjectThread, resolveIdeHarnesses, resolveIdeInlineProviders, type IdeCodeProposal, type IdeContextChip, type IdeInlinePromptState } from '../store/ideDiffApplier';
import { applyIdeCodeProposal, computeProposalsMap, createDebugFrameChipEntry, createDiagnosticChipEntry, createFileChipFromPath, createSymbolChipFromPath, createTestFailureChipEntry, undoIdeCodeProposal } from './ideAgentRuntimeOps.svelte';
import { IDE_OPEN_THREAD_EVENT, ideAgentState, type IdeAgentEngineBridge, type IdeShellSelectionInput } from './ideAgentState.svelte';
import { interruptIdeSessionTurn, isIdeSessionWorking, loadIdeWorkspaceSessions, loadSelectedIdeSessionMeta, openIdeThreadInThreadsView, rememberIdeProjectRoot, runIdeComposerStateTurn, runIdeInlineEditStateTurn } from './ideAgentTurnRunner.svelte';
import { buildFailedRunDraftChip, collectIdeProjectRoots, collectIdeThreadEntries, currentIdeTargetKey, resolveIdeProjectCwd, resolveIdeWorkspaceId, type IdeThreadEntry } from './ideContextBuilders';

function toIdeInteractionMode(planMode: boolean, runtimeAccessMode: RuntimeAccessMode): InteractionMode {
  if (planMode) return 'plan';
  return runtimeAccessMode === 'full-access' ? 'full' : 'accept-edits';
}

function fromIdeInteractionMode(
  mode: InteractionMode,
  prevRuntimeAccessMode: RuntimeAccessMode,
): { planMode: boolean; runtimeAccessMode: RuntimeAccessMode } {
  if (mode === 'plan') return { planMode: true, runtimeAccessMode: prevRuntimeAccessMode };
  return {
    planMode: false,
    runtimeAccessMode: mode === 'full' ? 'full-access' : 'auto-accept-edits',
  };
}

class IdeAgentEngine implements IdeAgentEngineBridge {
  private stopThreadsMount: (() => void) | null = null;
  private subscribedSessionId: string | null = null;
  private stopViewOpen: (() => void) | null = null;
  private stopThreadWatch: (() => void) | null = null;
  private stopTargetWatch: (() => void) | null = null;
  private lastTargetKey = '';
  private inflightWsLoads = new Map<string, Promise<void>>();

  constructor() {
    void ideAgentState.ensureSessionPort().then((b) => {
      if (b) this.syncSubscriptions();
    });
  }

  bindRuntimeBridge(bridge: RuntimeSessionPort): void {
    ideAgentState.runtimeSession = bridge;
    this.syncSubscriptions();
  }

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
    return sid && ideAgentState.runtimeBridge?.runtimeThreads.byId[sid]
      ? runtimeDrafts.for(sid).text
      : ideAgentState.composerText;
  }
  set composerText(next: string) {
    const sid = ideAgentState.sessionId;
    if (sid && ideAgentState.runtimeBridge?.runtimeThreads.byId[sid]) runtimeDrafts.for(sid).text = next;
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

  get effectiveRuntimeAccessMode(): RuntimeAccessMode {
    return this.planMode ? 'approval-required' : this.runtimeAccessMode;
  }

  get interactionMode(): InteractionMode {
    return toIdeInteractionMode(this.planMode, this.runtimeAccessMode);
  }

  setInteractionMode(mode: InteractionMode): void {
    const next = fromIdeInteractionMode(mode, this.runtimeAccessMode);
    this.planMode = next.planMode;
    this.runtimeAccessMode = next.runtimeAccessMode;
  }

  setRuntimeAccessMode(mode: RuntimeAccessMode): void {
    this.planMode = false;
    this.runtimeAccessMode = mode;
  }

  readonly projectRoots = $derived.by((): string[] => collectIdeProjectRoots());
  readonly activeWorkspaceId = $derived.by((): string => resolveIdeWorkspaceId());

  readonly sessions = $derived.by((): RuntimeSession[] => {
    const bridge = ideAgentState.runtimeBridge;
    if (!bridge) return [];
    const wsId = this.activeWorkspaceId;
    const roots = this.projectRoots;
    return bridge.runtimeThreads.live(false).filter((s) =>
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
      runtimeAll: Object.values(ideAgentState.runtimeBridge?.runtimeThreads.byId ?? {}),
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
    ideAgentState.sessionId
      ? (ideAgentState.runtimeBridge?.runtimeThreads.get(ideAgentState.sessionId) ?? null)
      : null,
  );

  readonly activeThreadKind = $derived<'runtime' | 'pty' | 'draft'>(
    !ideAgentState.sessionId
      ? 'draft'
      : ideAgentState.runtimeBridge?.runtimeThreads.get(ideAgentState.sessionId)
        ? 'runtime'
        : ws.getSession(ideAgentState.sessionId)
          ? 'pty'
          : 'runtime',
  );

  readonly availableProviders = $derived.by((): string[] => {
    const activeOpen =
      (this.activeRuntimeSession ? sessionProviderSlug(this.activeRuntimeSession) : null) ??
      this.activeThread?.provider ??
      null;
    return resolveIdeHarnesses(agentProviders(), activeOpen ? [activeOpen] : []);
  });

  readonly inlineProviders = $derived.by((): RuntimeProviderOrSlug[] =>
    resolveIdeInlineProviders(this.availableProviders, agentProviders(), auth.meta),
  );

  readonly effectiveModel = $derived<string | null>(
    ideAgentState.model ?? this.activeRuntimeSession?.model ?? this.activeThread?.model ?? null,
  );

  /** What the agent takes: its session's own report, else the provider baseline. */
  readonly caps = $derived.by(
    (): TakesCaps =>
      capsOf(
        this.activeRuntimeSession?.capabilities,
        runtimeBaselineCaps(
          this.activeRuntimeSession ? sessionProviderSlug(this.activeRuntimeSession) : ideAgentState.provider,
          auth.meta,
        ),
      ),
  );

  readonly canAttachImages = $derived(attachmentSupport(this.caps).images);

  readonly canAttachFiles = $derived(attachmentSupport(this.caps).files);

  readonly projection = $derived<RuntimeProjectionView | null>(
    ideAgentState.sessionId
      ? (ideAgentState.runtimeBridge?.runtimeViews.projections[ideAgentState.sessionId] ?? null)
      : null,
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

  applyPendingHandoff(): void {
    const handoff = ideAgentState.takePendingHandoff();
    if (!handoff) return;
    this.panelOpen = true;
    if (handoff.sessionId) void this.selectSession(handoff.sessionId);
    else this.startNewChat();
  }

  syncTargetRoot(nextKey = currentIdeTargetKey()): void {
    const localRoot = ide.target.kind === 'local' ? (ide.target.root ?? '').trim().replace(/\/+$/, '') : '';
    if (localRoot && localRoot !== '~') rememberIdeProjectRoot(localRoot);
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
    const stopProviderGuard = $effect.root(() => {
      $effect(() => {
        const list = this.availableProviders;
        const curSid = ideAgentState.sessionId;
        const curProv = ideAgentState.provider;
        if (!curSid && list.length > 0 && !list.includes(curProv)) {
          untrack(() => {
            const def = defaultAgentProvider();
            this.setProvider(list.includes(def) ? def : list[0]!);
          });
        }
      });
    });
    return () => {
      stopProviderGuard();
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
          const nextKey = currentIdeTargetKey();
          untrack(() => this.syncTargetRoot(nextKey));
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
    const bridge = ideAgentState.runtimeBridge;
    if (!bridge) {
      void ideAgentState.ensureBridge().then((b) => {
        if (b) this.syncSubscriptions();
      });
      return;
    }
    const { runtimeThreads, runtimeViews } = bridge;
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
      if (!supportsEffortSelection(clean, auth.meta)) this.effort = null;
    }
  }

  cycleProvider(): void {
    const list = this.availableProviders;
    if (list.length > 0) this.setProvider(list[(list.indexOf(ideAgentState.provider) + 1) % list.length]);
  }

  startNewChat(): void {
    this.error = null;
    this.sessionId = null;
    ideAgentState.composerText = '';
    const composerUi = ideAgentState.composerUi ?? getIdeRuntimeComposerUi();
    if (composerUi) {
      composerUi.council.on = false;
      composerUi.council.seats = [];
    }
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
    const caps = this.caps;
    if (!attachmentSupport(caps).images) return;
    const bridge = ideAgentState.runtimeBridge ?? (await ideAgentState.ensureBridge());
    if (!bridge) return;
    const read = await bridge.readAttachments(files, (f) => takesFile(caps, f));
    if (read.length > 0) {
      const combined = [...this.attachments, ...read];
      if (combined.length > 20) {
        toasts.error('Too many attachments', 'At most 20 images or PDFs per message.');
        this.attachments = combined.slice(0, 20);
      } else {
        this.attachments = combined;
      }
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
      const bridge = ideAgentState.runtimeBridge ?? (await ideAgentState.ensureBridge());
      if (!bridge) throw new Error('Couldn’t load runtime threads in this window.');
      await bridge.runtimeThreads.reopen(sid);
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
    if (!st || !isRuntimeProvider(clean, auth.meta) || st.provider === clean) return;
    st.provider = clean;
    st.model = null;
    if (!supportsEffortSelection(clean, auth.meta)) st.effort = null;
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
      accessMode: this.effectiveRuntimeAccessMode,
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
      accessMode: this.effectiveRuntimeAccessMode,
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
      const bridge = ideAgentState.runtimeBridge ?? (await ideAgentState.ensureBridge());
      if (!bridge) throw new Error('Couldn’t load runtime threads in this window.');
      await bridge.runtimeThreads.respond(ideAgentState.sessionId, requestId, decision);
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
