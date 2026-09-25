<script lang="ts">
  // Per-session agent activity: a normalized task tracker + a live trail of
  // what's going on (skills loaded, commands run, files touched, prompts,
  // notes) — by user and by agent. Fed by REST load + the events WS.
  // Supports source filtering, search, level coloring and expandable detail.
  //
  // The task tracker is two-way (docs/design/conversation-view.md §4.5/§5.4):
  // the agent's own plan (TodoWrite/TaskCreate, `source:'agent'`) is merged with
  // tasks pushed from here or the Mission Control board (`source:'user'`, shown
  // with a `from board` badge). A user task waits as *queued* until the nudge
  // sweep hands it to the agent's PTY (idle, or after the 120 s max defer).
  import { ws } from '../../lib/stores/workspace.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import Icon, { type IconName } from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import type { TaskStatus, TrailKind, TrailSource } from '../../lib/api/types';

  const session = $derived(ws.activeSession);
  const wsId = $derived(ws.currentId);

  const tasks = $derived(activity.tasks(session?.id ?? null));
  const trail = $derived(activity.trail(session?.id ?? null));

  const doneCount = $derived(tasks.filter((t) => t.status === 'completed').length);
  const progress = $derived(tasks.length ? Math.round((doneCount / tasks.length) * 100) : 0);
  const pendingNudges = $derived(tasks.filter((t) => t.nudge_pending).length);
  // Codex has no task tool, so its list is only ever what users add here.
  const isCodex = $derived(session?.provider === 'codex');
  const canEdit = $derived(ws.myRole !== 'viewer' && session?.kind === 'agent');
  const sessionEnded = $derived(
    !!session && (ws.statusMap[session.id] ?? session.status) === 'exited',
  );

  // "+ Add task" inline form.
  let addingTask = $state(false);
  let taskTitle = $state('');
  let taskDesc = $state('');
  let taskBusy = $state(false);
  let taskTitleEl = $state<HTMLInputElement | null>(null);

  function openAddTask(): void {
    addingTask = true;
    queueMicrotask(() => taskTitleEl?.focus());
  }

  function cancelAddTask(): void {
    addingTask = false;
    taskTitle = '';
    taskDesc = '';
  }

  async function submitTask(): Promise<void> {
    const title = taskTitle.trim();
    if (!title || !session || taskBusy) return;
    taskBusy = true;
    try {
      await activity.addTask(session.id, title, taskDesc.trim() || undefined);
      cancelAddTask();
    } catch (e) {
      toasts.error('Could not add task', e instanceof Error ? e.message : String(e));
    } finally {
      taskBusy = false;
    }
  }

  function onTaskKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void submitTask();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelAddTask();
    }
  }

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

  const loadError = $derived(session ? (activity.loadErrorBySession[session.id] ?? null) : null);

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

  const KIND_ICON: Record<TrailKind, IconName> = {
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

  function onRowKeydown(e: KeyboardEvent, id: string): void {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      toggle(id);
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
    {#if loadError}
      <!-- The trail + tasks didn't load: say so (an empty board would read as
           "the agent has done nothing"), with Retry. -->
      <div class="load-error" role="alert">
        <Icon name="warning" size={14} />
        <span class="grow">Couldn't load this session's activity. {loadError}</span>
        <button class="btn small" onclick={() => session && wsId && void activity.load(wsId, session.id, true)}>
          <Icon name="refresh" size={12} /> Retry
        </button>
      </div>
    {/if}
    <!-- Task tracker ------------------------------------------------------- -->
    <section class="section">
      <div class="section-title">
        <span>Task tracker</span>
        <span class="title-right">
          {#if pendingNudges > 0}
            <span class="count nudge" title="Waiting to be handed to the agent (delivered when idle, or after 120 s)">
              {pendingNudges} queued
            </span>
          {/if}
          {#if tasks.length > 0}<span class="count" data-testid="task-count">{doneCount}/{tasks.length}</span>{/if}
          {#if canEdit && !addingTask}
            <button
              class="add-task-btn"
              onclick={openAddTask}
              title={sessionEnded ? 'The session has exited — the task is kept and handed over on resume' : 'Push a task to this agent'}
              data-testid="add-task-btn"
            >
              <Icon name="plus" size={12} /> Add task
            </button>
          {/if}
        </span>
      </div>

      {#if addingTask}
        <form class="task-add" onsubmit={(e) => { e.preventDefault(); void submitTask(); }} data-testid="add-task-form">
          <input
            class="task-input"
            placeholder="Task title — the agent adds it to its list and does it next"
            bind:value={taskTitle}
            bind:this={taskTitleEl}
            onkeydown={onTaskKeydown}
            spellcheck="false"
            aria-label="Task title"
          />
          <textarea
            class="task-input desc"
            placeholder="Details (optional) — sent along with the nudge"
            bind:value={taskDesc}
            onkeydown={onTaskKeydown}
            rows="2"
            spellcheck="false"
            aria-label="Task description"
          ></textarea>
          <div class="task-add-row">
            <button type="button" class="tbtn" onclick={cancelAddTask}>Cancel</button>
            <button type="submit" class="tbtn primary" disabled={taskTitle.trim() === '' || taskBusy}>
              {taskBusy ? 'Adding…' : 'Add'}
            </button>
          </div>
        </form>
      {/if}

      {#if tasks.length === 0}
        {#if isCodex}
          <p class="empty-line dim">
            Codex does not publish a plan — only tasks you add here appear (Codex picks them up as prompts).
          </p>
        {:else}
          <p class="empty-line dim">No tasks yet. They appear when the agent plans its work.</p>
        {/if}
      {:else}
        <div class="progress-track" aria-hidden="true">
          <div class="progress-fill" style="width:{progress}%"></div>
        </div>
        <ul class="tasks" data-testid="task-list">
          {#each tasks as t (t.id)}
            <li
              class="task task-{t.status}"
              class:from-board={t.source === 'user'}
              class:nudge-pending={t.nudge_pending}
              title={t.description ? `${t.status.replace('_', ' ')} — ${t.description}` : t.status.replace('_', ' ')}
              data-testid="task-row"
              data-source={t.source}
            >
              <span class="task-glyph">{TASK_GLYPH[t.status]}</span>
              <span class="task-title">{t.title}</span>
              {#if t.source === 'user'}
                <span class="badge board" title="Added from the board / Activity panel">From board</span>
              {/if}
              {#if t.nudge_pending}
                <span class="badge queued" title="Waiting to be handed to the agent">Queued</span>
              {/if}
            </li>
          {/each}
        </ul>
        {#if isCodex}
          <p class="empty-line dim hint">Codex does not publish a plan — this list holds only the tasks you added.</p>
        {/if}
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
            <button
              class="tab"
              class:on={sourceFilter === t.id}
              aria-pressed={sourceFilter === t.id}
              onclick={() => (sourceFilter = t.id)}
            >
              {t.label}
            </button>
          {/each}
        </div>
        <input class="search" placeholder="Filter…" aria-label="Filter the trail" bind:value={query} spellcheck="false" />
      </div>

      <div class="note-add">
        <input
          class="note-input"
          placeholder="Add a note to this session…"
          aria-label="Note"
          bind:value={note}
          onkeydown={onNoteKeydown}
          spellcheck="false"
        />
        <button
          class="note-btn"
          title={note.trim() === '' ? 'Type a note first' : 'Add note'}
          aria-label="Add note"
          disabled={note.trim() === '' || adding}
          onclick={addNote}
        >
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
              <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events, a11y_no_noninteractive_tabindex -->
              <div
                class="row-main"
                class:clickable={e.detail != null}
                role={e.detail != null ? 'button' : undefined}
                tabindex={e.detail != null ? 0 : undefined}
                aria-expanded={e.detail != null ? !!expanded[e.id] : undefined}
                onclick={() => e.detail != null && toggle(e.id)}
                onkeydown={(ev) => e.detail != null && onRowKeydown(ev, e.id)}
              >
                <span class="row-icon"><Icon name={KIND_ICON[e.kind] ?? 'dot'} size={12} /></span>
                <div class="row-body">
                  <div class="row-summary">{e.summary}</div>
                  <div class="row-meta">
                    <span class="row-src">{sourceLabel(e.source)}</span>
                    <span class="row-time mono" title={new Date(e.ts).toLocaleString()}>{rel(e.ts)}</span>
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
    font-size: var(--fs-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-dim);
  }
  .count {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    background: var(--surface-2);
    border-radius: 999px;
    padding: 1px 7px;
  }
  .empty-line {
    font-size: var(--fs-xs);
    line-height: 1.4;
    margin: 2px 0;
  }

  /* Task tracker */
  .title-right {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .count.nudge {
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 14%, transparent);
  }
  .add-task-btn {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    height: 18px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0;
    text-transform: none;
    cursor: pointer;
  }
  .add-task-btn:hover {
    border-color: var(--accent);
    color: var(--accent-text);
  }
  .task-add {
    display: flex;
    flex-direction: column;
    gap: 5px;
    margin: 2px 0 4px;
  }
  .task-input {
    width: 100%;
    box-sizing: border-box;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    padding: 4px 8px;
    outline: none;
    resize: vertical;
  }
  .task-input:focus {
    border-color: var(--accent);
  }
  .task-input.desc {
    min-height: 34px;
  }
  .task-add-row {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .tbtn {
    height: 22px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .tbtn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--accent-contrast);
  }
  .tbtn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .badge {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border-radius: 999px;
    padding: 0 5px;
    line-height: 14px;
    align-self: center;
  }
  .badge.board {
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .badge.queued {
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 14%, transparent);
  }
  .task.nudge-pending .task-glyph {
    animation: pulse 1.4s ease-in-out infinite;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.35;
    }
  }
  .hint {
    margin-top: 4px;
  }
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
    /* A long plan scrolls on its own instead of squeezing the live trail
       below it down to nothing. */
    max-height: 240px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .task {
    display: flex;
    align-items: baseline;
    gap: 7px;
    font-size: var(--fs-s);
    line-height: 1.35;
  }
  .task-glyph {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    width: 12px;
    text-align: center;
    color: var(--text-dim);
  }
  .task-title {
    min-width: 0;
    word-break: break-word;
  }
  .task-in_progress .task-glyph {
    color: var(--accent-text);
  }
  .task-in_progress .task-title {
    color: var(--text);
    font-weight: 600;
  }
  .task-completed .task-glyph {
    color: var(--status-working);
  }
  .task-completed .task-title {
    color: var(--text-dim);
    text-decoration: line-through;
  }
  .task-blocked .task-glyph {
    color: var(--status-exited);
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
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
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
    color: var(--accent-text);
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
  .row-main.clickable:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
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
    color: var(--accent-text);
  }
  .lvl-warn .row-icon {
    color: var(--status-warn);
  }
  .lvl-error .row-icon {
    color: var(--status-exited);
  }
  .row-body {
    min-width: 0;
    flex: 1;
  }
  .row-summary {
    font-size: var(--fs-s);
    line-height: 1.35;
    color: var(--text);
    word-break: break-word;
  }
  .kind-command .row-summary {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  /* Text uses the text-safe semantic tokens (the --status-* ones are for dots). */
  .lvl-warn .row-summary {
    color: var(--warning);
  }
  .lvl-error .row-summary {
    color: var(--danger);
  }
  .row-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 1px;
  }
  .row-src {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .src-user .row-src {
    color: var(--accent-text);
  }
  .row-time {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .row-detail {
    margin: 2px 0 4px 28px;
    padding: 6px 8px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    line-height: 1.4;
    color: var(--text-dim);
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 220px;
    overflow: auto;
  }
  .load-error {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-block-end: 10px;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .load-error > :global(svg) {
    color: var(--danger);
    flex-shrink: 0;
  }
  .load-error .grow {
    flex: 1;
    min-width: 0;
  }
</style>
