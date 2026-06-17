<script lang="ts">
  // Per-session agent activity: a normalized task tracker + a live trail of
  // what's going on (skills loaded, commands run, files touched, prompts,
  // notes) — by user and by agent. Fed by REST load + the events WS.
  // Supports source filtering, search, level coloring and expandable detail.
  import { ws } from '../../lib/stores/workspace.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import type { TaskStatus, TrailKind, TrailSource } from '../../lib/api/types';

  const session = $derived(ws.activeSession);
  const wsId = $derived(ws.currentId);

  const tasks = $derived(activity.tasks(session?.id ?? null));
  const trail = $derived(activity.trail(session?.id ?? null));

  const doneCount = $derived(tasks.filter((t) => t.status === 'completed').length);
  const progress = $derived(tasks.length ? Math.round((doneCount / tasks.length) * 100) : 0);

  // Filters: source + free-text search. Newest first for at-a-glance scanning.
  type SourceFilter = 'all' | 'agent' | 'user' | 'otto';
  let sourceFilter = $state<SourceFilter>('all');
  let query = $state('');

  const filtered = $derived(
    [...trail]
      .reverse()
      .filter((e) => sourceFilter === 'all' || e.source === sourceFilter)
      .filter((e) => {
        const q = query.trim().toLowerCase();
        return q === '' || e.summary.toLowerCase().includes(q);
      }),
  );

  // Load once whenever the focused session changes (live updates arrive via WS).
  $effect(() => {
    const sid = session?.id;
    const w = wsId;
    if (sid && w) void activity.load(w, sid);
  });

  let note = $state('');
  let adding = $state(false);
  let expanded = $state<Record<string, boolean>>({});

  function toggle(id: string): void {
    expanded[id] = !expanded[id];
  }

  async function addNote(): Promise<void> {
    const text = note.trim();
    if (!text || !session || !wsId || adding) return;
    adding = true;
    try {
      await activity.addNote(wsId, session.id, text);
      note = '';
    } catch (e) {
      toasts.error('Could not add note', e instanceof Error ? e.message : String(e));
    } finally {
      adding = false;
    }
  }

  function onNoteKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void addNote();
    }
  }

  const KIND_ICON: Record<TrailKind, string> = {
    session: 'play',
    prompt: 'comment',
    skill: 'zap',
    command: 'command',
    tool: 'box',
    file: 'file',
    web: 'globe',
    task: 'check',
    note: 'note',
    other: 'dot',
  };

  const TASK_GLYPH: Record<TaskStatus, string> = {
    pending: '○',
    in_progress: '◐',
    completed: '●',
    blocked: '▲',
    cancelled: '✕',
  };

  const SOURCE_TABS: { id: SourceFilter; label: string }[] = [
    { id: 'all', label: 'All' },
    { id: 'agent', label: 'Agent' },
    { id: 'user', label: 'You' },
    { id: 'otto', label: 'Otto' },
  ];

  function sourceLabel(s: TrailSource): string {
    return s === 'user' ? 'you' : s === 'otto' ? 'otto' : 'agent';
  }

  /** Relative time: "now", "3m", "2h", else a short date. */
  function relTime(iso: string): string {
    try {
      const then = new Date(iso).getTime();
      const secs = Math.max(0, Math.round((Date.now() - then) / 1000));
      if (secs < 10) return 'now';
      if (secs < 60) return `${secs}s`;
      const mins = Math.floor(secs / 60);
      if (mins < 60) return `${mins}m`;
      const hrs = Math.floor(mins / 60);
      if (hrs < 24) return `${hrs}h`;
      return new Date(iso).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
    } catch {
      return '';
    }
  }

  function pretty(detail: unknown): string {
    try {
      return JSON.stringify(detail, null, 2);
    } catch {
      return String(detail);
    }
  }
</script>

{#if !session}
  <EmptyState
    icon="zap"
    title="No session selected"
    body="Open or focus an agent session to follow its live trail and task tracker."
  />
{:else}
  <div class="activity">
    <!-- Task tracker ------------------------------------------------------- -->
    <section class="section">
      <div class="section-title">
        <span>Task tracker</span>
        {#if tasks.length > 0}<span class="count">{doneCount}/{tasks.length}</span>{/if}
      </div>

      {#if tasks.length === 0}
        <p class="empty-line dim">No tasks yet. They appear when the agent plans its work.</p>
      {:else}
        <div class="progress-track" aria-hidden="true">
          <div class="progress-fill" style="width:{progress}%"></div>
        </div>
        <ul class="tasks">
          {#each tasks as t (t.id)}
            <li class="task task-{t.status}" title={t.status.replace('_', ' ')}>
              <span class="task-glyph">{TASK_GLYPH[t.status]}</span>
              <span class="task-title">{t.title}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <!-- Live trail --------------------------------------------------------- -->
    <section class="section grow">
      <div class="section-title">
        <span>Live trail</span>
        {#if trail.length > 0}<span class="count">{trail.length}</span>{/if}
      </div>

      <div class="filters">
        <div class="tabs">
          {#each SOURCE_TABS as t (t.id)}
            <button class="tab" class:on={sourceFilter === t.id} onclick={() => (sourceFilter = t.id)}>
              {t.label}
            </button>
          {/each}
        </div>
        <input class="search" placeholder="Filter…" bind:value={query} spellcheck="false" />
      </div>

      <div class="note-add">
        <input
          class="note-input"
          placeholder="Add a note to this session…"
          bind:value={note}
          onkeydown={onNoteKeydown}
          spellcheck="false"
        />
        <button class="note-btn" title="Add note" disabled={note.trim() === '' || adding} onclick={addNote}>
          <Icon name="plus" size={13} />
        </button>
      </div>

      {#if filtered.length === 0}
        <p class="empty-line dim">
          {trail.length === 0 ? 'No activity recorded yet.' : 'Nothing matches this filter.'}
        </p>
      {:else}
        <ul class="trail">
          {#each filtered as e (e.id)}
            <li class="row src-{e.source} kind-{e.kind} lvl-{e.level}">
              <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
              <div class="row-main" class:clickable={e.detail != null} onclick={() => e.detail != null && toggle(e.id)}>
                <span class="row-icon"><Icon name={KIND_ICON[e.kind] ?? 'dot'} size={12} /></span>
                <div class="row-body">
                  <div class="row-summary">{e.summary}</div>
                  <div class="row-meta">
                    <span class="row-src">{sourceLabel(e.source)}</span>
                    <span class="row-time mono">{relTime(e.ts)}</span>
                    {#if e.detail != null}
                      <Icon name={expanded[e.id] ? 'chevronDown' : 'chevronRight'} size={10} />
                    {/if}
                  </div>
                </div>
              </div>
              {#if e.detail != null && expanded[e.id]}
                <pre class="row-detail mono">{pretty(e.detail)}</pre>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  </div>
{/if}

<style>
  .activity {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .section.grow {
    flex: 1;
    min-height: 0;
    border-bottom: none;
  }
  .section-title {
    display: flex;
    align-items: center;
    justify-content: space-between;
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-dim);
  }
  .count {
    font-size: 10px;
    font-weight: 600;
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 999px;
    padding: 1px 7px;
  }
  .empty-line {
    font-size: 11.5px;
    line-height: 1.4;
    margin: 2px 0;
  }

  /* Task tracker */
  .progress-track {
    height: 4px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .progress-fill {
    height: 100%;
    background: var(--accent);
    border-radius: 999px;
    transition: width 200ms ease-out;
  }
  .tasks {
    list-style: none;
    margin: 2px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .task {
    display: flex;
    align-items: baseline;
    gap: 7px;
    font-size: 12px;
    line-height: 1.35;
  }
  .task-glyph {
    flex-shrink: 0;
    font-size: 10px;
    width: 12px;
    text-align: center;
    color: var(--text-dim);
  }
  .task-title {
    min-width: 0;
    word-break: break-word;
  }
  .task-in_progress .task-glyph {
    color: var(--accent);
  }
  .task-in_progress .task-title {
    color: var(--text);
    font-weight: 600;
  }
  .task-completed .task-glyph {
    color: var(--status-working, #3fb950);
  }
  .task-completed .task-title {
    color: var(--text-dim);
    text-decoration: line-through;
  }
  .task-blocked .task-glyph {
    color: var(--status-exited, #e5534b);
  }
  .task-cancelled .task-title {
    color: var(--text-dim);
    text-decoration: line-through;
    opacity: 0.7;
  }

  /* Filters */
  .filters {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .tabs {
    display: flex;
    gap: 2px;
  }
  .tab {
    height: 20px;
    padding: 0 7px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
  }
  .tab:hover {
    background: var(--surface-2);
  }
  .tab.on {
    background: var(--surface-2);
    color: var(--text);
    font-weight: 600;
  }
  .search {
    flex: 1;
    min-width: 0;
    height: 22px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: 11px;
    padding: 0 7px;
    outline: none;
  }
  .search:focus {
    border-color: var(--accent);
  }

  /* Note input */
  .note-add {
    display: flex;
    gap: 6px;
  }
  .note-input {
    flex: 1;
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: 11.5px;
    padding: 4px 8px;
    outline: none;
  }
  .note-input:focus {
    border-color: var(--accent);
  }
  .note-btn {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }
  .note-btn:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .note-btn:not(:disabled):hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  /* Trail */
  .trail {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
    overflow-y: auto;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .row-main {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 4px;
    border-radius: var(--radius-s);
  }
  .row-main.clickable {
    cursor: pointer;
  }
  .row-main:hover {
    background: var(--surface-2);
  }
  .row-icon {
    flex-shrink: 0;
    margin-top: 1px;
    color: var(--text-dim);
    display: inline-flex;
  }
  .kind-command .row-icon,
  .kind-skill .row-icon,
  .src-user .row-icon {
    color: var(--accent);
  }
  .lvl-warn .row-icon {
    color: var(--status-warn, #d29922);
  }
  .lvl-error .row-icon {
    color: var(--status-exited, #e5534b);
  }
  .row-body {
    min-width: 0;
    flex: 1;
  }
  .row-summary {
    font-size: 12px;
    line-height: 1.35;
    color: var(--text);
    word-break: break-word;
  }
  .kind-command .row-summary {
    font-family: var(--font-mono);
    font-size: 11px;
  }
  .lvl-warn .row-summary {
    color: var(--status-warn, #d29922);
  }
  .lvl-error .row-summary {
    color: var(--status-exited, #e5534b);
  }
  .row-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 1px;
  }
  .row-src {
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .src-user .row-src {
    color: var(--accent);
  }
  .row-time {
    font-size: 9.5px;
    color: var(--text-dim);
  }
  .row-detail {
    margin: 2px 0 4px 28px;
    padding: 6px 8px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    font-size: 10.5px;
    line-height: 1.4;
    color: var(--text-dim);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 220px;
    overflow: auto;
  }
</style>
