// Turn orchestration helpers for the IDE AI Coding Agent dock and `⌘I` inline
// prompt: runtime session ensure/reopen, Council & Ask-another-agent delegation,
// PTY thread input fallback, and zero-polling inline-edit execution.

import { api } from '../../../lib/api/client';
import type { PromptImage, RuntimeAccessMode, RuntimeProvider } from '../../../lib/api/types';
import { normalizeEffort, type EffortLevel, type InteractionMode } from '../../../lib/providerModels';
import { harnessLabel } from '../../../lib/providers';
import { router } from '../../../lib/router.svelte';
import { openInThreads } from '../../../lib/runtimeThreadPort';
import type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
import { transcript } from '../../../lib/stores/transcript.svelte';
import { SCRATCH_WORKSPACE_ID, ws } from '../../../lib/stores/workspace.svelte';
import { askTargets } from '../../threads/runtime/askTargets.svelte'; // ui-guards: allow — IDE agent dock supports Ask-another-agent delegation
import { council } from '../../threads/runtime/councilStore.svelte'; // ui-guards: allow — IDE agent dock supports Council multi-agent threads
import { isRuntimeProvider, runtimeThreads } from '../../threads/runtime/runtimeThreads.svelte'; // ui-guards: allow — IDE agent dock drives real Otto Threads
import { runtimeViews } from '../../threads/runtime/runtimeView.svelte'; // ui-guards: allow — IDE agent dock streams structured runtime views
import { threads } from '../../threads/threadsStore.svelte'; // ui-guards: allow — IDE agent dock syncs shared thread focus and project drafts
import {
  buildIdeAgentPrompt,
  stripIdeContextBlocks,
  type IdeCodeProposal,
  type IdeContextChip,
  type IdeInlinePromptState,
} from '../store/ideDiffApplier';
import {
  dispatchPromptToRuntimeSession,
  extractInlineEditProposal,
  waitForInlineTurnCompletion,
} from './ideAgentRuntimeOps.svelte';

export function summarizeIdeThreadTitle(text: string, provider = 'Agent'): string {
  const first =
    stripIdeContextBlocks(text)
      .split('\n')
      .find((l) => l.trim().length > 0)
      ?.trim() ?? `${harnessLabel(provider)} thread`;
  return first.length > 64 ? `${first.slice(0, 61)}…` : first;
}

export function openIdeThreadInThreadsView(id: string | null, workspaceId: string, cwd: string | null): void {
  if (id) {
    threads.selectThread(id, { syncRoute: false });
    openInThreads(id);
    return;
  }
  threads.startNewThread({ workspaceId, cwd });
  router.go('threads/new');
}

export async function interruptIdeSessionTurn(sessionId: string): Promise<void> {
  if (runtimeThreads.byId[sessionId]) await runtimeThreads.interrupt(sessionId);
  else await api.post<void>(`/sessions/${encodeURIComponent(sessionId)}/input`, { text: '\u001b', submit: false });
}

export async function loadIdeWorkspaceSessions(wsId: string, inflight: Map<string, Promise<void>>): Promise<void> {
  if (!wsId) return;
  const key = `${wsId}:all`;
  let pending = inflight.get(key);
  if (!pending) {
    pending = Promise.allSettled([runtimeThreads.load(wsId), runtimeThreads.loadAll()])
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
  const row = runtimeThreads.byId[sid];
  if (row && (row.status === 'working' || row.status === 'starting')) return true;
  const proj = runtimeViews.projections[sid];
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
  threads.selectThread(id, { syncRoute: false });
  const pty = ws.getSession(id);
  if (pty && !runtimeThreads.byId[id]) {
    return {
      provider: pty.provider,
      model: typeof pty.meta?.model === 'string' && pty.meta.model.trim() ? pty.meta.model.trim() : null,
    };
  }
  const row = runtimeThreads.get(id) ?? (await runtimeThreads.loadOne(id));
  if (!row) return null;
  void runtimeThreads.loadLinked(id).catch(() => {});
  return {
    provider: row.provider,
    ...(row.model ? { model: row.model } : {}),
    ...(row.access_mode ? { accessMode: row.access_mode } : {}),
  };
}

export async function ensureIdeRuntimeSession(opts: {
  existingId: string | null;
  targetProvider: RuntimeProvider;
  seedTitle: string;
  wsId: string;
  localCwd: string | null;
  model: string | null;
  effort: EffortLevel | null;
  onSessionBound: (id: string) => void;
  syncSubscriptions: () => void;
}): Promise<string> {
  if (opts.existingId) {
    const row =
      runtimeThreads.get(opts.existingId) ??
      (ws.getSession(opts.existingId)
        ? undefined
        : await runtimeThreads.loadOne(opts.existingId).catch(() => undefined));
    if (row && row.provider === opts.targetProvider) {
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
  threads.rememberProject(opts.localCwd);
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
    targetProvider: st.provider as RuntimeProvider,
    seedTitle: query,
    wsId: opts.wsId,
    localCwd: opts.localCwd,
    model: st.model,
    effort,
    onSessionBound: opts.onSessionBound,
    syncSubscriptions: opts.syncSubscriptions,
  });
  if (opts.isCancelled()) return null;
  const knownIds = new Set((runtimeViews.projections[sid]?.items ?? []).map((i) => i.id));
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
  const pics: PromptImage[] = opts.files
    .filter((f) => f.mime.startsWith('image/'))
    .map(({ mime, data_base64 }) => ({ mime, data_base64 }));

  if (opts.currentSessionId && isRuntimeProvider(opts.provider) && askTargets.for(opts.currentSessionId).length > 0) {
    const children = await runtimeThreads.askChildren(opts.currentSessionId, {
      prompt: opts.fullPrompt,
      targets: askTargets.for(opts.currentSessionId),
      ...(opts.accessMode ? { access_mode: opts.accessMode } : {}),
      ...(pics.length > 0 ? { images: pics } : {}),
    });
    askTargets.clear(opts.currentSessionId);
    void runtimeThreads.loadLinked(opts.currentSessionId).catch(() => {});
    if (children[0]) await opts.selectSession(children[0].id);
    return;
  }

  if (!opts.currentSessionId && council.ready) {
    const seats = await council.start(opts.wsId, {
      prompt: opts.fullPrompt,
      title: summarizeIdeThreadTitle(opts.seedText, opts.provider),
      cwd: opts.localCwd,
      ...(pics.length > 0 ? { images: pics } : {}),
    });
    threads.rememberProject(opts.localCwd);
    if (seats[0]) await opts.selectSession(seats[0].id);
    return;
  }

  if (isRuntimeProvider(opts.provider)) {
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
  threads.rememberProject(opts.localCwd);
  await opts.selectSession(created.id);
}
