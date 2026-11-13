<script lang="ts">
  // An Antigravity swarm member's work, opened from its subagent card: its
  // live state, the board posts by or for it and its own steps. agy sends
  // none of it over ACP (only the coordinator's `swarm` calls), so the daemon
  // reads it from agy's files (GET /runtime-sessions/{id}/swarm?member=),
  // polled while the member runs. T3 Code's NativeChildWorkDialog
  // (MIT, Copyright (c) 2026 T3 Tools Inc.) shows the same per-child steps.
  import { onDestroy } from 'svelte';
  import { api } from '../../../lib/api/client';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import type { RuntimeSwarmMember, RuntimeSwarm } from '../../../lib/api/types';

  interface Props {
    sessionId: string;
    member: string;
    /** Poll while it runs. */
    live: boolean;
  }
  let { sessionId, member, live }: Props = $props();

  const POLL_MS = 4000;
  let data = $state<RuntimeSwarmMember | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let gone = false;

  async function load(): Promise<void> {
    try {
      const s = await api.get<RuntimeSwarm>(`/runtime-sessions/${encodeURIComponent(sessionId)}/swarm?member=${encodeURIComponent(member)}`);
      data = s.members.find((m) => m.name === member) ?? null;
      error = null;
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
    if (!gone && live) timer = setTimeout(() => void load(), POLL_MS);
  }
  $effect(() => {
    void sessionId;
    void member;
    loading = true;
    void load();
    return () => {
      if (timer) clearTimeout(timer);
    };
  });
  onDestroy(() => (gone = true));

  const STEP: Record<string, string> = { PLANNER_RESPONSE: 'Thought / tools', GENERIC: 'Result', USER_INPUT: 'Message' };
  const phaseText = $derived(data ? (data.killed ? 'Killed' : data.phase ? data.phase.charAt(0) + data.phase.slice(1).toLowerCase() : null) : null);
</script>

<div class="sw" data-testid="runtime-swarm-work">
  <LoadState what="its work" loading={loading && !data} error={data ? null : error} empty={!data} variant="compact" rows={2} onretry={() => void load()}>
    {#snippet emptyView()}<p class="sw-meta">Antigravity has no record of this member yet.</p>{/snippet}
    {#if data}
      <p class="sw-meta">
        {#if phaseText}{phaseText}{/if}{#if data.model} · {data.model}{/if}
        {#if data.workspace}<span class="sw-path mono" dir="ltr" title={data.workspace}>{data.workspace}</span>{/if}
      </p>
      {#if data.posts.length}
        <p class="sw-label">Board</p>
        <ol class="sw-list">
          {#each data.posts as post (post.seq)}
            <li class="sw-post"><span class="sw-who">#{post.seq} {post.agent}</span><span class="sw-text" dir="auto">{post.text}</span></li>
          {/each}
        </ol>
      {/if}
      {#if data.steps?.length}
        <p class="sw-label">Steps{#if data.truncated} (latest){/if}</p>
        <ol class="sw-list">
          {#each data.steps as step (step.index)}
            <li class="sw-step">
              <span class="sw-who">{STEP[step.type] ?? step.type}</span>
              {#each step.tools ?? [] as call, i (i)}<code class="sw-call" dir="ltr">{call}</code>{/each}
              {#if step.text}<pre class="sw-text" dir="auto">{step.text}</pre>{/if}
            </li>
          {/each}
        </ol>
      {:else if live}
        <p class="sw-meta">Working…</p>
      {/if}
    {/if}
  </LoadState>
</div>

<style>
  .sw {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  .sw-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .sw-path {
    min-inline-size: 0;
    max-inline-size: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sw-label {
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }
  .sw-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 0;
    padding-inline-start: 10px;
    border-inline-start: 1px solid var(--border);
    list-style: none;
  }
  .sw-post,
  .sw-step {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
  }
  .sw-who {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 500;
  }
  .sw-text {
    margin: 0;
    max-block-size: 12rem;
    overflow: auto;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .sw-call {
    color: var(--text);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
