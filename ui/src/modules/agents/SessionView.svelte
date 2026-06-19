<script lang="ts">
  // One pane: session header (status, provider, restart/kill) + terminal.
  import Terminal from '../../lib/components/Terminal.svelte';
  import StatusDot from '../../lib/components/StatusDot.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import AttachIssue from './AttachIssue.svelte';
  import AttachProductStory from './AttachProductStory.svelte';
  import Handover from './Handover.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { activity } from '../../lib/stores/activity.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { AttachedIssue, SessionStatus } from '../../lib/api/types';

  interface Props {
    sessionId: string;
    focused: boolean;
    showClose: boolean;
    onfocus: () => void;
    onclosepane: () => void;
    /** Show a maximize/restore (zoom) control (tiled view). */
    showZoom?: boolean;
  }
  let { sessionId, focused, showClose, onfocus, onclosepane, showZoom = false }: Props = $props();

  const maximized = $derived(ws.maximizedId === sessionId);

  const session = $derived(ws.sessions.find((s) => s.id === sessionId) ?? null);
  const status = $derived(ws.statusMap[sessionId] ?? session?.status ?? 'idle');
  const readOnly = $derived(ws.myRole === 'viewer');

  // Live per-session activity roll-up (current in-progress task + done/total),
  // surfaced in the pane header so tiled/split panes show what each agent is on.
  const summary = $derived(activity.summary(sessionId));
  // Sticky "needs you" flag — the session is blocked on operator input. Distinct
  // from plain idle; cleared by the store when the user opens/inputs.
  const needsYou = $derived(ws.needsYou[sessionId] === true);
  /** True when this agent session can be resumed after exiting. */
  const resumable = $derived(
    session?.kind === 'agent' && session?.provider_session_id != null,
  );

  let menuOpen = $state(false);
  let renaming = $state(false);
  let draftTitle = $state('');
  let attachIssueOpen = $state(false);
  let attachProductOpen = $state(false);
  let handoverOpen = $state(false);

  const attachedIssue = $derived(
    (session?.meta?.issue as AttachedIssue | undefined) ?? null,
  );

  // Handover breadcrumb (source session) + live "preparing brief" badge.
  const handoverFromId = $derived(
    typeof session?.meta?.handover_from === 'string'
      ? (session.meta.handover_from as string)
      : null,
  );
  const handoverFrom = $derived(
    handoverFromId ? (ws.sessions.find((s) => s.id === handoverFromId) ?? null) : null,
  );
  const handoverPending = $derived(session?.meta?.handover_pending === true);

  // --- Additional directories editor (meta.extra_dirs → `--add-dir` args) -----
  // Only agent sessions launch a CLI that honors `--add-dir`.
  const isAgent = $derived(session?.kind === 'agent');
  let dirsOpen = $state(false);
  let dirsBusy = $state(false);
  let extraDirs = $state<string[]>([]);
  let dirDraft = $state('');

  function openDirs(): void {
    menuOpen = false;
    const seed = session?.meta?.extra_dirs;
    extraDirs = Array.isArray(seed) ? seed.filter((d): d is string => typeof d === 'string') : [];
    dirDraft = '';
    dirsOpen = true;
  }

  function addDir(): void {
    const path = dirDraft.trim();
    if (path === '' || extraDirs.includes(path)) {
      dirDraft = '';
      return;
    }
    extraDirs = [...extraDirs, path];
    dirDraft = '';
  }

  function removeDir(dir: string): void {
    extraDirs = extraDirs.filter((d) => d !== dir);
  }

  function onDirKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      addDir();
    }
  }

  /** Collect the list, folding any typed-but-unadded draft, trimming + de-duping. */
  function collectDirs(): string[] {
    const dirs = [...extraDirs];
    const pending = dirDraft.trim();
    if (pending !== '' && !dirs.includes(pending)) dirs.push(pending);
    return dirs;
  }

  async function saveDirs(alsoRestart: boolean): Promise<void> {
    if (dirsBusy) return;
    dirsBusy = true;
    try {
      const dirs = collectDirs();
      await ws.updateSessionMeta(sessionId, { extra_dirs: dirs });
      if (alsoRestart) {
        await ws.restartSession(sessionId);
        toasts.success('Directories saved', 'Session restarted with the new directories.');
      } else {
        toasts.success('Directories saved', 'Applies the next time this session restarts.');
      }
      dirsOpen = false;
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      dirsBusy = false;
    }
  }

  function onTermStatus(s: SessionStatus): void {
    ws.statusMap[sessionId] = s;
  }

  function startRename(): void {
    if (readOnly) return;
    draftTitle = session?.title ?? '';
    renaming = true;
  }

  async function commitRename(): Promise<void> {
    renaming = false;
    const next = draftTitle.trim();
    if (!next || next === session?.title) return;
    try {
      await ws.renameSession(sessionId, next);
    } catch (e) {
      toasts.error('Rename failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function restart(): Promise<void> {
    try {
      await ws.restartSession(sessionId);
    } catch (e) {
      toasts.error('Restart failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function archive(): Promise<void> {
    menuOpen = false;
    try {
      await ws.archiveSession(sessionId);
    } catch (e) {
      toasts.error('Archive failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function del(): Promise<void> {
    menuOpen = false;
    try {
      await ws.killSession(sessionId);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function detachIssue(): Promise<void> {
    menuOpen = false;
    try {
      await ws.detachIssue(sessionId);
      toasts.info('Issue detached');
    } catch (e) {
      toasts.error('Detach failed', e instanceof Error ? e.message : String(e));
    }
  }

  function openAttachIssue(): void {
    menuOpen = false;
    attachIssueOpen = true;
  }

  function openAttachProductStory(): void {
    menuOpen = false;
    attachProductOpen = true;
  }

  function openHandover(): void {
    menuOpen = false;
    handoverOpen = true;
  }
</script>

<svelte:window onclick={() => (menuOpen = false)} />

<!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
<section
  class="pane"
  class:focused
  onmousedown={() => {
    // Interacting with the pane attends to it — drop the "needs you" flag.
    ws.clearNeedsYou(sessionId);
    onfocus();
  }}
>
  <header class="pane-head">
    <StatusDot {status} />
    {#if renaming}
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="rename-input"
        bind:value={draftTitle}
        autofocus
        onblur={commitRename}
        onkeydown={(e) => {
          if (e.key === 'Enter') commitRename();
          else if (e.key === 'Escape') renaming = false;
        }}
        onmousedown={(e) => e.stopPropagation()}
      />
    {:else}
      <span
        class="pane-title"
        role="button"
        tabindex="0"
        title="Double-click to rename; right-click for options"
        ondblclick={startRename}
        oncontextmenu={!readOnly ? (e) => ctxMenu.show(e, [
          { label: 'Rename…', icon: 'edit', action: startRename },
          ...(isAgent ? [{ label: 'Additional directories…', icon: 'folder', action: openDirs }] : []),
          ...(isAgent ? [{ label: 'Hand over to…', icon: 'send', action: openHandover }] : []),
          { separator: true },
          { label: attachedIssue ? 'Change Jira issue…' : 'Attach Jira issue…', icon: 'ticket', action: openAttachIssue },
          ...(attachedIssue ? [{ label: 'Detach issue', icon: 'link', action: detachIssue }] : []),
          { label: 'Attach product story…', icon: 'file', action: openAttachProductStory },
          { separator: true },
          { label: 'Archive', icon: 'archive', action: archive },
          { label: 'Delete', icon: 'trash', danger: true as const, action: del },
        ]) : undefined}
      >{session?.title ?? sessionId}</span>
    {/if}
    <span class="chip provider-chip">{session?.provider ?? '?'}</span>
    {#if needsYou}
      <span class="needs-you-badge" title="This session is waiting on you (input or a permission)">
        <Icon name="bell" size={10} /> Needs you
      </span>
    {/if}
    {#if summary && summary.total > 0}
      <span
        class="task-chip"
        class:done={summary.done === summary.total}
        class:active={summary.in_progress != null}
        title={summary.in_progress ? `Now: ${summary.in_progress}` : `${summary.done}/${summary.total} tasks done`}
      >{summary.done}/{summary.total}</span>
    {/if}
    {#if summary?.in_progress}
      <span class="now-task" title="Current task: {summary.in_progress}">
        now: {summary.in_progress}
      </span>
    {/if}
    {#if handoverFromId}
      <button
        class="handover-crumb"
        title="Open the session this was handed over from"
        onmousedown={(e) => e.stopPropagation()}
        onclick={() => ws.openSession(handoverFromId)}
      >↰ {handoverFrom?.title ?? 'source'}</button>
    {/if}
    {#if handoverPending}
      <span class="handover-pending" title="Preparing the handover brief…">⏳ handover…</span>
    {/if}
    {#if session?.cwd}<span class="pane-cwd mono" title={session.cwd}>{session.cwd}</span>{/if}
    <span class="grow"></span>
    {#if showZoom}
      <button
        class="icon-btn"
        onmousedown={(e) => e.stopPropagation()}
        onclick={() => ws.toggleMaximize(sessionId)}
        title={maximized ? 'Restore tiled view' : 'Zoom in on this session'}
      >
        <Icon name={maximized ? 'minimize' : 'maximize'} size={13} />
      </button>
    {/if}
    {#if !readOnly}
      <button class="icon-btn" onclick={restart} title="Restart session"><Icon name="refresh" size={13} /></button>
      <div class="menu-wrap" onmousedown={(e) => e.stopPropagation()} role="presentation">
        <button
          class="icon-btn"
          onclick={(e) => { e.stopPropagation(); menuOpen = !menuOpen; }}
          title="More…"
        >⋯</button>
        {#if menuOpen}
          <div class="menu" role="menu">
            <button role="menuitem" onclick={startRename}>Rename…</button>
            {#if isAgent}
              <button role="menuitem" onclick={openDirs}>Additional directories…</button>
              <button role="menuitem" onclick={openHandover}>Hand over to…</button>
            {/if}
            <button role="menuitem" onclick={openAttachIssue}>
              {attachedIssue ? 'Change Jira issue…' : 'Attach Jira issue…'}
            </button>
            {#if attachedIssue}
              <button role="menuitem" onclick={detachIssue}>Detach issue</button>
            {/if}
            <button role="menuitem" onclick={openAttachProductStory}>Attach product story…</button>
            <button role="menuitem" onclick={archive}>Archive</button>
            <button role="menuitem" class="danger" onclick={del}>Delete</button>
          </div>
        {/if}
      </div>
    {/if}
    {#if showClose}
      <button class="icon-btn" onclick={onclosepane} title="Close pane (keeps running)"><Icon name="x" size={12} /></button>
    {/if}
  </header>
  <div class="pane-term">
    {#key sessionId}
      <Terminal {sessionId} {readOnly} {resumable} onstatus={onTermStatus} />
    {/key}
  </div>
</section>

{#if attachIssueOpen}
  <AttachIssue {sessionId} onclose={() => (attachIssueOpen = false)} />
{/if}

{#if attachProductOpen}
  <AttachProductStory {sessionId} onclose={() => (attachProductOpen = false)} />
{/if}

{#if handoverOpen}
  <Handover {sessionId} onclose={() => (handoverOpen = false)} />
{/if}

{#if dirsOpen}
  <Modal title="Additional directories" onclose={() => (dirsOpen = false)}>
    <div class="field">
      <label for="sv-extra-dir">Directories the agent may access <span class="dim">(beyond its working dir)</span></label>
      {#if extraDirs.length > 0}
        <ul class="dir-list">
          {#each extraDirs as dir (dir)}
            <li class="dir-row">
              <span class="dir-path mono" title={dir}>{dir}</span>
              <button
                type="button"
                class="dir-remove"
                title="Remove directory"
                onclick={() => removeDir(dir)}
              >✕</button>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="dir-add">
        <input
          id="sv-extra-dir"
          class="input mono"
          bind:value={dirDraft}
          spellcheck="false"
          placeholder="/absolute/path/to/repo"
          onkeydown={onDirKeydown}
        />
        <button type="button" class="btn" disabled={dirDraft.trim() === ''} onclick={addDir}>Add</button>
      </div>
      <span class="hint">Passed as <code>--add-dir</code>. Takes effect on the next session restart.</span>
    </div>

    {#snippet footer()}
      <button class="btn" onclick={() => (dirsOpen = false)}>Cancel</button>
      <button class="btn" disabled={dirsBusy} onclick={() => saveDirs(false)}>
        {dirsBusy ? 'Saving…' : 'Save'}
      </button>
      <button class="btn primary" disabled={dirsBusy} onclick={() => saveDirs(true)}>
        {dirsBusy ? 'Saving…' : 'Save & restart'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .pane {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--term-bg);
    transition: border-color 140ms ease-out;
  }
  .pane.focused {
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .pane-head {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 30px;
    padding: 0 8px 0 10px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .pane-title {
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 180px;
  }
  .provider-chip {
    height: 16px;
    font-size: 9.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  /* "Needs you" — session blocked on operator input. Amber, attention-grabbing
     but tasteful; distinct from the (calmer) status dot for idle/working. */
  .needs-you-badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex-shrink: 0;
    height: 16px;
    padding: 0 6px;
    border-radius: 99px;
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: #febc2e;
    background: color-mix(in srgb, #febc2e 16%, transparent);
    white-space: nowrap;
  }
  /* Per-session task roll-up "done/total" — matches the sidebar chip. */
  .task-chip {
    flex-shrink: 0;
    padding: 0 5px;
    height: 15px;
    line-height: 15px;
    border-radius: 999px;
    font-size: 9px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--text-dim) 16%, transparent);
  }
  .task-chip.active {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .task-chip.done {
    color: var(--status-working, #3fb950);
    background: color-mix(in srgb, var(--status-working, #3fb950) 16%, transparent);
  }
  /* "now: «task»" — what the agent is doing this moment. Truncates so it never
     pushes the header controls off-screen in a narrow tile. */
  .now-task {
    min-width: 0;
    flex: 0 1 auto;
    font-size: 10.5px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .handover-crumb {
    flex-shrink: 0;
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 10.5px;
    padding: 1px 7px;
    border-radius: 99px;
    cursor: pointer;
  }
  .handover-crumb:hover {
    color: var(--text);
    border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .handover-pending {
    flex-shrink: 0;
    font-size: 10.5px;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    padding: 1px 7px;
    border-radius: 99px;
    white-space: nowrap;
  }
  .pane-cwd {
    font-size: 10.5px;
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 220px;
  }
  .pane-term {
    flex: 1;
    min-height: 0;
  }
  .pane-title[role='button'] {
    cursor: text;
  }
  .rename-input {
    font-size: 12px;
    font-weight: 600;
    background: var(--surface-2);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    color: var(--text);
    padding: 1px 6px;
    max-width: 200px;
    outline: none;
  }
  .menu-wrap {
    position: relative;
    display: inline-flex;
  }
  .menu {
    position: absolute;
    top: 22px;
    right: 0;
    z-index: 30;
    min-width: 130px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
    padding: 4px;
    display: flex;
    flex-direction: column;
  }
  .menu button {
    text-align: left;
    background: transparent;
    border: none;
    color: var(--text);
    font-size: 12px;
    padding: 5px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .menu button:hover {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .menu button.danger {
    color: var(--status-exited);
  }

  /* Additional directories editor (mirrors New Session). */
  .dir-list {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .dir-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .dir-path {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .dir-remove {
    flex-shrink: 0;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 10px;
    padding: 2px 4px;
    border-radius: 3px;
    line-height: 1;
  }
  .dir-remove:hover {
    color: var(--danger, #e5534b);
    background: color-mix(in srgb, var(--danger, #e5534b) 12%, transparent);
  }
  .dir-add {
    display: flex;
    gap: 6px;
  }
  .dir-add .input {
    flex: 1;
    min-width: 0;
  }
</style>
