<script module lang="ts">
  import { api } from '../../../lib/api/client';
  // The two retention settings, read once (root only; anyone else sees the defaults).
  let retention: Promise<{ success: number; failed: number }> | null = null;
  const hours = (v: unknown, d: number): number => (Number.isFinite(Number(v)) && v !== null && v !== '' ? Number(v) : d);
  function retentionHours(): Promise<{ success: number; failed: number }> {
    retention ??= api.bg.get<Record<string, unknown>>('/settings').then(
      (s) => ({ success: hours(s.workflow_thread_retention_success_hours, 24), failed: hours(s.workflow_thread_retention_failed_hours, 168) }),
      () => ({ success: 24, failed: 168 }),
    );
    return retention;
  }
</script>

<script lang="ts">
  // A step's (or reviewer's) conversation (ux.md §4.4): the step's runtime
  // thread embedded READ-ONLY (no composer, no "Open in Threads"; its approval
  // and question cards stay answerable), or — once retention deleted the
  // thread, or the embed reports it gone or forbidden — the digest the run
  // kept. A step run on the PTY runner (an older run) lists its sessions,
  // which open in Agents.
  import type { Component } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { readRunContext } from '../../../lib/api/workflows';
  import { loadErrorText } from '../../../lib/loadError';
  import { loadRuntimeThreadView, type RuntimeThreadViewProps } from '../../../lib/runtimeThreadPort';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import type { DigestItem, StepThread, TranscriptDigest } from '../../../lib/api/types';
  import { retentionNote } from './runWords';
  import { keptSessions } from './stepTabs';

  interface Props {
    thread?: StepThread | null;
    /** PTY session ids (an older run). */
    sessions?: readonly string[];
    /** The kept digest (the full node's `thread.digest`). */
    digest?: TranscriptDigest | null;
    workspaceId: string;
    runId: string;
    nodeId: string;
    attempt: number;
    /** The step (or sub-thread) is still running or waiting on a person: show
     *  the normal Threads composer so a person can queue/steer a follow-up. */
    writable?: boolean;
    /** Switch to another thread (a sub-thread card or Back to parent) in the same pane. */
    onselectthread?: (threadId: string) => void;
  }
  let { thread = null, sessions = [], digest = null, workspaceId, runId, nodeId, attempt, writable = false, onselectthread }: Props = $props();

  let gone = $state<'deleted' | 'forbidden' | null>(null);
  // Another step (or attempt) in the same pane starts over: the last one's
  // "removed" state and digest never show for it. Derived primitives keep a
  // re-created `thread` or parent selection object from wiping local state.
  const threadId = $derived(thread?.thread_id ?? null);
  const threadRunner = $derived(thread?.runner ?? null);
  const currentRunId = $derived(runId);
  const currentNodeId = $derived(nodeId);
  const currentAttempt = $derived(attempt);
  $effect(() => {
    void threadId;
    void currentNodeId;
    void currentAttempt;
    gone = null;
    items = null;
    itemsError = null;
  });
  let note = $state(retentionNote(24, 168));
  $effect(() => void retentionHours().then((h) => (note = retentionNote(h.success, h.failed))));
  const live = $derived(thread?.runner === 'threads' && thread.available !== false && !gone);

  // An older step's terminal sessions: only the ones that still exist open.
  const ptyIds = $derived(sessions.length ? [...sessions] : thread?.runner === 'pty' ? [thread.thread_id] : []);
  let kept = $state<Record<string, boolean>>({});
  $effect(() => {
    for (const id of ptyIds) void ws.ensureSession(id).then((s) => (kept[id] = !!s));
  });
  const pty = $derived(keptSessions(ptyIds, kept));

  let View = $state<Component<RuntimeThreadViewProps> | null>(null);
  let viewError = $state<string | null>(null);
  let viewRetry = $state(0);
  $effect(() => {
    void viewRetry;
    if (!live || View) return;
    loadRuntimeThreadView().then((m) => (View = m.default), (e) => (viewError = loadErrorText(e)));
  });

  // The digest's timeline, kept beside the run's files.
  // (A reviewer or the summarizer has none: the host passes no nodeId.)
  let items = $state<DigestItem[] | null>(null);
  let itemsError = $state<string | null>(null);
  let itemsRetry = $state(0);
  $effect(() => {
    void itemsRetry;
    const id = currentRunId;
    const nid = currentNodeId;
    const att = currentAttempt;
    if (live || threadRunner !== 'threads' || !nid) return;
    const ac = new AbortController();
    readRunContext(id, `.threads/${nid}-${att}.json`, ac.signal).then(
      (f) => {
        try {
          const v = JSON.parse(f.content) as { items?: DigestItem[] } | DigestItem[];
          items = Array.isArray(v) ? v : (v.items ?? []);
        } catch (e) {
          itemsError = loadErrorText(e);
        }
      },
      (e) => {
        if (!ac.signal.aborted) itemsError = loadErrorText(e);
      },
    );
    return () => ac.abort();
  });
</script>

{#snippet liveNotice()}
  <p class="live-note" data-testid="wf-step-live-note">
    <Icon name="info" size={12} />This conversation belongs to the run. What you send is added to this step.
  </p>
{/snippet}

{#if live && View}
  <div class="embed" data-testid="wf-step-conversation">
    <View
      sessionId={thread!.thread_id}
      {workspaceId}
      readOnly={!writable}
      showHeader
      showOpenInThreads
      expandWork
      composerTopSlot={writable ? liveNotice : undefined}
      {onselectthread}
      onunavailable={(reason) => (gone = reason)}
    />
  </div>
  {#if !writable}<p class="note"><Icon name="lock" size={12} />{note}</p>{/if}
{:else if live}
  <LoadState what="the conversation" loading={!viewError} error={viewError} empty onretry={() => { viewError = null; viewRetry++; }} />
{:else if thread?.runner === 'threads'}
  <div class="digest" data-testid="wf-step-digest">
    <p class="note"><Icon name="info" size={12} />{gone === 'forbidden' ? 'You can’t open this conversation. The answer, files and usage are kept — summary below.' : 'The conversation was removed after the run ended. The answer, files and usage are kept — summary below.'}</p>
    {#if items?.length}
      <ol class="items">
        {#each items as it, i (i)}
          <li><span class="role">{it.tool ? `${it.role} · ${it.tool}` : it.role}</span><span class="text" dir="auto">{it.text}</span></li>
        {/each}
      </ol>
    {:else if itemsError && !digest}
      <LoadState what="the conversation summary" variant="compact" error={itemsError} empty onretry={() => { itemsError = null; itemsRetry++; }} />
    {/if}
    {#if digest}
      {#if digest.last_answer_excerpt}<h4>The last answer</h4><pre class="answer" dir="auto">{digest.last_answer_excerpt}</pre>{/if}
      {#if digest.files_changed.length}<h4>Files changed</h4><ul class="files">{#each digest.files_changed as f (f)}<li><code>{f}</code></li>{/each}</ul>{/if}
      {#if digest.errors.length}<h4>Errors</h4><ul class="files">{#each digest.errors as e, i (i)}<li dir="auto">{e}</li>{/each}</ul>{/if}
      <p class="note">{digest.turns} turns · {digest.tool_calls} tool calls</p>
    {/if}
  </div>
{:else if ptyIds.length}
  {#if pty.gone}
    <p class="note" data-testid="wf-step-session-gone"><Icon name="info" size={12} />This step ran in a terminal session that is no longer kept.</p>
  {:else}
    <ul class="sessions">
      {#each pty.open as id (id)}
        <li>
          <span>Session · {thread?.provider ?? 'agent'} · {id.slice(-6)}</span>
          <button type="button" class="btn small" onclick={() => ws.navigateToSession(id)}>Open in Agents</button>
        </li>
      {/each}
    </ul>
  {/if}
{:else}
  <p class="note">This step has no conversation.</p>
{/if}

<style>
  .embed {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-block-size: 420px;
    block-size: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  .note,
  .live-note {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .note {
    padding-block: var(--sp-4);
  }
  .live-note {
    padding-block: var(--sp-1);
    padding-inline: var(--sp-2);
  }
  .items {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .items li {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding: var(--sp-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .role {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .text {
    font-size: var(--fs-m);
    overflow-wrap: anywhere;
  }
  h4 {
    margin: 0;
    margin-block-start: var(--sp-5);
    font-size: var(--fs-m);
  }
  .answer {
    margin: 0;
    margin-block-start: var(--sp-3);
    padding: var(--sp-5);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-family: var(--font-ui);
    font-size: var(--fs-m);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .files {
    margin: 0;
    padding-inline-start: var(--sp-6);
    font-size: var(--fs-s);
  }
  .sessions {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .sessions li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-4);
    padding-block: var(--sp-3);
    font-size: var(--fs-m);
  }
</style>
