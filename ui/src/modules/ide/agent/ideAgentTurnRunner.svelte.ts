// Turn orchestration helpers for the IDE AI Coding Agent dock and `⌘I` inline
// prompt: runtime session ensure/reopen, Council & Ask-another-agent delegation,
// PTY thread input fallback, and zero-polling inline-edit execution.

import { api } from '../../../lib/api/client';
import type { PromptImage, RuntimeAccessMode, RuntimeProviderOrSlug } from '../../../lib/api/types';
import { ideWorkspacePort } from '../../../lib/ideWorkspacePort.svelte';
import { loadErrorText } from '../../../lib/loadError';
import { normalizeEffort, type EffortLevel, type InteractionMode } from '../../../lib/providerModels';
import { harnessLabel, isRuntimeProvider, sessionProviderSlug } from '../../../lib/providers';
import { router } from '../../../lib/router.svelte';
import {
  getIdeRuntimeComposerUi,
  openInThreads,
  type RuntimeSessionPort,
} from '../../../lib/runtimeThreadPort';
import { auth } from '../../../lib/stores/auth.svelte';
import { runtimeDrafts, type ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
import { transcript } from '../../../lib/stores/transcript.svelte';
import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
import { ide } from '../ideStore.svelte';
import {
  buildIdeAgentPrompt,
  stripIdeContextBlocks,
  type IdeCodeProposal,
  type IdeContextChip,
  type IdeInlinePromptState,
} from '../store/ideDiffApplier';
import {
  dispatchPromptToRuntimeSession,
  isRuntimeTurnBusy,
  extractInlineEditProposal,
  waitForInlineTurnCompletion,
} from './ideAgentRuntimeOps.svelte';
import { ideAgentState } from './ideAgentState.svelte';
import { resolveIdeProjectCwd, resolveIdeWorkspaceId } from './ideContextBuilders';

async function requireIdeRuntimeSessionPort(): Promise<RuntimeSessionPort> {
  const bridge = ideAgentState.runtimeSession ?? (await ideAgentState.ensureSessionPort());
  if (!bridge) throw new Error('Couldn’t load runtime threads in this window.');
  return bridge;
}

export function summarizeIdeThreadTitle(text: string, provider = 'Agent'): string {
  const first =
    stripIdeContextBlocks(text)
      .split('\n')
      .find((l) => l.trim().length > 0)
      ?.trim() ?? `${harnessLabel(provider)} thread`;
  return first.length > 64 ? `${first.slice(0, 61)}…` : first;
}

export function rememberIdeProjectRoot(localCwd: string | null): void {
  const clean = (localCwd ?? '').trim().replace(/\/+$/, '');
  if (!clean || clean === '~') return;
  ideWorkspacePort.setLastRoot(clean);
  if (ideAgentState.runtimeBridge) {
    ideAgentState.runtimeBridge.threads.rememberProject(clean);
  } else {
    void ideAgentState.ensureBridge().then((b) => b?.threads.rememberProject(clean));
  }
}

export function openIdeThreadInThreadsView(id: string | null, workspaceId: string, cwd: string | null): void {
  rememberIdeProjectRoot(cwd);
  const bridge = ideAgentState.runtimeBridge;
  if (id) {
    if (bridge) bridge.threads.selectThread(id, { syncRoute: false });
    else void ideAgentState.ensureBridge().then((b) => b?.threads.selectThread(id, { syncRoute: false }));
    openInThreads(id);
    return;
  }
  if (bridge) {
    bridge.threads.startNewThread({ workspaceId, cwd });
    router.go('threads/new');
    return;
  }
  void ideAgentState.ensureBridge().then((b) => {
    b?.threads.startNewThread({ workspaceId, cwd });
    router.go('threads/new');
  });
}

export async function interruptIdeSessionTurn(sessionId: string): Promise<void> {
  const bridge = ideAgentState.runtimeBridge ?? (await ideAgentState.ensureBridge());
  if (bridge?.runtimeThreads.byId[sessionId]) await bridge.runtimeThreads.interrupt(sessionId);
  else await api.post<void>(`/sessions/${encodeURIComponent(sessionId)}/input`, { text: '\u001b', submit: false });
}

export async function loadIdeWorkspaceSessions(wsId: string, inflight: Map<string, Promise<void>>): Promise<void> {
  if (!wsId) return;
  const bridge = ideAgentState.runtimeBridge ?? (await ideAgentState.ensureBridge());
  if (!bridge) return;
  const key = `${wsId}:all`;
  let pending = inflight.get(key);
  if (!pending) {
    pending = Promise.allSettled([bridge.runtimeThreads.load(wsId), bridge.runtimeThreads.loadAll()])
      .then(() => undefined)
      .finally(() => {
        inflight.delete(key);
      });
    inflight.set(key, pending);
  }
  await pending;
}

export function isIdeSessionWorking(sending: boolean, sid: string | null): boolean {
  if (sending) return true;
  if (!sid) return false;
  const bridge = ideAgentState.runtimeBridge;
  const row = bridge?.runtimeThreads.byId[sid];
  if (row && (row.status === 'working' || row.status === 'starting')) return true;
  const proj = bridge?.runtimeViews.projections[sid];
  if (proj) {
    if (proj.session?.status === 'running' || proj.session?.status === 'starting') return true;
    if (proj.turns.some((t) => t.status === 'running' || t.status === 'pending')) return true;
  }
  const pty = ws.getSession(sid);
  if (pty && !row) {
    const st = ws.statusMap[sid] ?? pty.status;
    return st === 'working' || st === 'running';
  }
  return false;
}

export async function loadSelectedIdeSessionMeta(
  id: string,
): Promise<{ provider: string; model?: string | null; accessMode?: RuntimeAccessMode } | null> {
  const bridge = await requireIdeRuntimeSessionPort();
  bridge.threads.selectThread(id, { syncRoute: false });
  const pty = ws.getSession(id);
  if (pty && !bridge.runtimeThreads.byId[id]) {
    return {
      provider: pty.provider,
      model: typeof pty.meta?.model === 'string' && pty.meta.model.trim() ? pty.meta.model.trim() : null,
    };
  }
  const row = bridge.runtimeThreads.get(id) ?? (await bridge.runtimeThreads.loadOne(id));
  if (!row) return null;
  void bridge.runtimeThreads.loadLinked(id).catch(() => {});
  return {
    provider: sessionProviderSlug(row),
    ...(row.model ? { model: row.model } : {}),
    ...(row.access_mode ? { accessMode: row.access_mode } : {}),
  };
}

export async function ensureIdeRuntimeSession(opts: {
  existingId: string | null;
  targetProvider: RuntimeProviderOrSlug;
  seedTitle: string;
  wsId: string;
  localCwd: string | null;
  model: string | null;
  effort: EffortLevel | null;
  onSessionBound: (id: string) => void;
  syncSubscriptions: () => void;
}): Promise<string> {
  const bridge = await requireIdeRuntimeSessionPort();
  const { runtimeThreads, threads } = bridge;
  if (opts.existingId) {
    const row =
      runtimeThreads.get(opts.existingId) ??
      (ws.getSession(opts.existingId)
        ? undefined
        : await runtimeThreads.loadOne(opts.existingId).catch(() => undefined));
    if (row && (sessionProviderSlug(row) === opts.targetProvider || row.provider === opts.targetProvider)) {
      if (row.status === 'closed' || row.status === 'dormant') await runtimeThreads.reopen(opts.existingId);
      opts.syncSubscriptions();
      return opts.existingId;
    }
  }
  const created = await runtimeThreads.start(opts.wsId, {
    provider: opts.targetProvider,
    cwd: opts.localCwd,
    title: summarizeIdeThreadTitle(opts.seedTitle, opts.targetProvider),
    model: opts.model,
    ...(opts.effort ? { effort: opts.effort } : {}),
    prompt: '',
  });
  opts.onSessionBound(created.id);
  rememberIdeProjectRoot(opts.localCwd);
  threads.selectThread(created.id, { syncRoute: false });
  opts.syncSubscriptions();
  return created.id;
}

export async function executeIdeInlineEdit(opts: {
  state: IdeInlinePromptState;
  query: string;
  wsId: string;
  localCwd: string | null;
  planMode: boolean;
  accessMode?: RuntimeAccessMode;
  existingSessionId: string | null;
  onSessionBound: (id: string) => void;
  syncSubscriptions: () => void;
  isCancelled: () => boolean;
}): Promise<IdeCodeProposal | null> {
  const { state: st, query } = opts;
  const bridge = await requireIdeRuntimeSessionPort();
  const inlineChip: IdeContextChip = {
    id: 'inline-sel',
    key: `selection:${st.path}:${st.startLine}-${st.endLine}`,
    kind: 'selection',
    label: `@${st.relPath}:L${st.startLine}-L${st.endLine}`,
    path: st.path,
    startLine: st.startLine,
    endLine: st.endLine,
    language: st.language,
    snippet: st.selectedText,
  };
  const promptPayload = buildIdeAgentPrompt(query, [inlineChip], {
    workspaceRoot: opts.localCwd ?? undefined,
    activePath: st.path,
    planMode: opts.planMode,
  });
  const effort = normalizeEffort(st.effort);
  const sid = await ensureIdeRuntimeSession({
    existingId: opts.existingSessionId,
    targetProvider: st.provider,
    seedTitle: query,
    wsId: opts.wsId,
    localCwd: opts.localCwd,
    model: st.model,
    effort,
    onSessionBound: opts.onSessionBound,
    syncSubscriptions: opts.syncSubscriptions,
  });
  if (opts.isCancelled()) return null;
  // A queued prompt has no turn id yet, so its reply can't be told apart from the running chat turn's.
  if (isRuntimeTurnBusy(bridge, sid)) throw new Error('The agent is still answering in the chat. Try the inline edit when it finishes.');
  const knownIds = new Set((bridge.runtimeViews.projections[sid]?.items ?? []).map((i) => i.id));
  const resp = await dispatchPromptToRuntimeSession(sid, promptPayload, {
    model: st.model,
    effort,
    accessMode: opts.accessMode,
  });
  const reply = await waitForInlineTurnCompletion(sid, resp, knownIds, opts.isCancelled);
  if (opts.isCancelled()) return null;
  return extractInlineEditProposal(st, query, reply);
}

export async function executeIdePromptSend(opts: {
  seedText: string;
  fullPrompt: string;
  provider: string;
  model: string | null;
  effort: EffortLevel | null;
  interactionMode: InteractionMode;
  accessMode?: RuntimeAccessMode;
  freshContext: boolean;
  files: ComposerFile[];
  wsId: string;
  localCwd: string | null;
  currentSessionId: string | null;
  onSessionBound: (id: string) => void;
  selectSession: (id: string) => Promise<void>;
  syncSubscriptions: () => void;
}): Promise<void> {
  const bridge = await requireIdeRuntimeSessionPort();
  const { runtimeThreads } = bridge;
  const composerUi = ideAgentState.composerUi ?? getIdeRuntimeComposerUi();
  const pics: PromptImage[] = opts.files
    .filter((f) => f.mime.startsWith('image/'))
    .map(({ name, mime, data_base64 }) => ({ ...(name ? { name } : {}), mime, data_base64 }));

  if (
    opts.currentSessionId &&
    isRuntimeProvider(opts.provider, auth.meta) &&
    composerUi &&
    composerUi.askTargets.for(opts.currentSessionId).length > 0
  ) {
    const children = await runtimeThreads.askChildren(opts.currentSessionId, {
      prompt: opts.fullPrompt,
      targets: composerUi.askTargets.for(opts.currentSessionId),
      ...(opts.accessMode ? { access_mode: opts.accessMode } : {}),
      ...(pics.length > 0 ? { images: pics } : {}),
    });
    composerUi.askTargets.clear(opts.currentSessionId);
    void runtimeThreads.loadLinked(opts.currentSessionId).catch(() => {});
    if (children[0]) await opts.selectSession(children[0].id);
    return;
  }

  if (!opts.currentSessionId && composerUi?.council.ready) {
    const seats = await composerUi.council.start(opts.wsId, {
      prompt: opts.fullPrompt,
      title: summarizeIdeThreadTitle(opts.seedText, opts.provider),
      cwd: opts.localCwd,
      ...(pics.length > 0 ? { images: pics } : {}),
    });
    rememberIdeProjectRoot(opts.localCwd);
    if (seats[0]) await opts.selectSession(seats[0].id);
    return;
  }

  if (isRuntimeProvider(opts.provider, auth.meta)) {
    const sid = await ensureIdeRuntimeSession({
      existingId: opts.currentSessionId,
      targetProvider: opts.provider,
      seedTitle: opts.seedText,
      wsId: opts.wsId,
      localCwd: opts.localCwd,
      model: opts.model,
      effort: opts.effort,
      onSessionBound: opts.onSessionBound,
      syncSubscriptions: opts.syncSubscriptions,
    });
    await dispatchPromptToRuntimeSession(sid, opts.fullPrompt, {
      model: opts.model,
      effort: opts.effort,
      files: opts.files,
      accessMode: opts.accessMode,
      freshContext: opts.freshContext,
    });
    return;
  }

  if (
    opts.currentSessionId &&
    ws.getSession(opts.currentSessionId)?.provider === opts.provider &&
    !runtimeThreads.byId[opts.currentSessionId]
  ) {
    await api.post<void>(`/sessions/${encodeURIComponent(opts.currentSessionId)}/input`, {
      text: opts.fullPrompt,
      submit: true,
    });
    transcript.recordOptimisticPrompt(opts.currentSessionId, opts.fullPrompt);
    return;
  }

  const meta: Record<string, unknown> = {
    surface: 'threads',
    interaction_mode: opts.interactionMode,
    ...(opts.localCwd ? { ide_root: opts.localCwd } : {}),
    ...(opts.model ? { model: opts.model } : {}),
    ...(opts.effort ? { effort: opts.effort } : {}),
  };
  const created = await ws.openSessionWithPrompt(
    {
      provider: opts.provider,
      title: summarizeIdeThreadTitle(opts.seedText, opts.provider),
      cwd: opts.localCwd,
      prompt: opts.fullPrompt,
      meta,
    },
    { scratch: opts.wsId === SCRATCH_WORKSPACE_ID && ws.currentId !== SCRATCH_WORKSPACE_ID },
  );
  transcript.recordOptimisticPrompt(created.id, opts.fullPrompt);
  rememberIdeProjectRoot(opts.localCwd);
  await opts.selectSession(created.id);
}

export async function runIdeInlineEditStateTurn(opts: {
  query?: string;
  planMode: boolean;
  accessMode: RuntimeAccessMode;
  handoffAndSend: () => void;
  syncSubscriptions: () => void;
}): Promise<void> {
  const st = ideAgentState.inlinePrompt;
  if (!st || !st.open || st.busy) return;
  const q = (opts.query ?? st.query).trim();
  if (!q) return;
  if (!isRuntimeProvider(st.provider, auth.meta)) {
    opts.handoffAndSend();
    return;
  }
  st.query = q;
  st.busy = true;
  st.error = null;
  st.proposal = null;
  const reqSeq = ++ideAgentState.inlineReqSeq;
  try {
    const proposal = await executeIdeInlineEdit({
      state: st,
      query: q,
      wsId: resolveIdeWorkspaceId(),
      localCwd: resolveIdeProjectCwd(),
      planMode: opts.planMode,
      accessMode: opts.planMode ? 'approval-required' : opts.accessMode,
      existingSessionId: ideAgentState.sessionId,
      onSessionBound: (id) => {
        ideAgentState.sessionId = id;
      },
      syncSubscriptions: opts.syncSubscriptions,
      isCancelled: () => ideAgentState.inlineReqSeq !== reqSeq || !ideAgentState.inlinePrompt?.open,
    });
    if (ideAgentState.inlineReqSeq !== reqSeq || !ideAgentState.inlinePrompt?.open) return;
    if (!proposal) ideAgentState.inlinePrompt.error = 'Agent reply did not include a code change.';
    else ideAgentState.inlinePrompt.proposal = proposal;
  } catch (e) {
    if (ideAgentState.inlineReqSeq === reqSeq && ideAgentState.inlinePrompt?.open) {
      ideAgentState.inlinePrompt.error = loadErrorText(e);
    }
  } finally {
    if (ideAgentState.inlineReqSeq === reqSeq && ideAgentState.inlinePrompt?.open) {
      ideAgentState.inlinePrompt.busy = false;
    }
  }
}

export async function runIdeComposerStateTurn(opts: {
  overrideText?: string;
  interactionMode: InteractionMode;
  accessMode: RuntimeAccessMode;
  planMode: boolean;
  ensurePanelOpen: () => void;
  selectSession: (id: string) => Promise<void>;
  syncSubscriptions: () => void;
}): Promise<void> {
  const activeSid = ideAgentState.sessionId;
  const activeDraft =
    activeSid && ideAgentState.runtimeBridge?.runtimeThreads.byId[activeSid]
      ? runtimeDrafts.for(activeSid)
      : null;
  const rawText = opts.overrideText ?? (activeDraft ? activeDraft.text : ideAgentState.composerText);
  const userText = rawText.trim();
  if ((!userText && ideAgentState.contexts.length === 0 && ideAgentState.attachments.length === 0) || ideAgentState.sending) {
    return;
  }
  opts.ensurePanelOpen();
  const snapshotContexts = ideAgentState.contexts.slice();
  const snapshotFiles = ideAgentState.attachments.slice();
  const snapshotFresh = ideAgentState.freshContext;
  const seedText =
    userText ||
    (snapshotContexts.length > 0
      ? 'Inspect the attached context and propose improvements.'
      : 'Inspect the attached files.');
  const fullPrompt = buildIdeAgentPrompt(seedText, snapshotContexts, {
    workspaceRoot: resolveIdeProjectCwd() ?? undefined,
    activePath: ide.activePath,
    planMode: opts.planMode,
    omitWhenEmpty: true,
  });

  ideAgentState.sending = true;
  ideAgentState.error = null;
  try {
    await executeIdePromptSend({
      seedText,
      fullPrompt,
      provider: ideAgentState.provider,
      model: ideAgentState.model,
      effort: ideAgentState.effort,
      interactionMode: opts.interactionMode,
      accessMode: opts.planMode ? 'approval-required' : opts.accessMode,
      freshContext: snapshotFresh,
      files: snapshotFiles,
      wsId: resolveIdeWorkspaceId(),
      localCwd: resolveIdeProjectCwd(),
      currentSessionId: ideAgentState.sessionId,
      onSessionBound: (id) => {
        ideAgentState.sessionId = id;
      },
      selectSession: opts.selectSession,
      syncSubscriptions: opts.syncSubscriptions,
    });
    if (opts.overrideText === undefined) {
      if (activeDraft) activeDraft.text = '';
      ideAgentState.composerText = '';
    }
    ideAgentState.contexts = [];
    ideAgentState.attachments = [];
    ideAgentState.freshContext = false;
  } catch (e) {
    ideAgentState.error = loadErrorText(e);
  } finally {
    ideAgentState.sending = false;
  }
}
