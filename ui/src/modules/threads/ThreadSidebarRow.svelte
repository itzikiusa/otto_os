<script lang="ts">
  // One inbox row (T3 Code / Orca): three lines, the whole row one button —
  //   1. the project's badge and name … the status WORD, else the time;
  //   2. the title, bold while it needs you or finished unread;
  //   3. the branch (worktree icon in a worktree) … the agent's glyph.
  // A subagent (a thread working under this one, folded under it by default)
  // is two lines: title, then its word · glyph. The words are fixed
  // (threadInbox.ts): Approval · Question · Failed · Done · Working 18s for a
  // thread; Working · Needs you · Done · Failed · Stopped for a subagent — and
  // a thread whose subagent needs you says "Needs you" itself.
  // Pin / beside / ⋯ appear on hover or focus over line 3's end; on touch they
  // live in the row's long-press menu (use:rowMenu).
  import Icon from '../../lib/components/Icon.svelte';
  import { rowMenu } from '../../lib/rowMenu';
  import { git } from '../../lib/stores/git.svelte';
  import { now, rel } from '../../lib/stores/now.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { Session } from '../../lib/api/types';
  import { agentThreadBadge, personalAgentName } from '../../lib/personalAgentBadges';
  import { baseName, harnessLabel, projectName } from './threadsHelpers';
  import { worktreeProjects } from './worktreeProjects.svelte';
  import { threads } from './threadsStore.svelte';
  import { isRuntimeThread, runtimeThreads } from './runtime/runtimeThreads.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import ProjectBadge from './ProjectBadge.svelte';
  import { councilOf } from './runtime/council';
  import { inboxClass, statusWord, subagentWord } from './threadInbox';
  import { childRollup } from './runtime/childThreads';
  import { inboxLive } from './threadInbox.svelte';
  import { threadMeta } from './threadMeta.svelte';
  import { threadRepos } from './threadRepos.svelte';
  import { threadRowState, threadStateById } from './threadNav.svelte';

  interface Props {
    session: Session;
    variant: 'pinned' | 'active' | 'history';
    child?: boolean;
    onrowclick: (e: MouseEvent, s: Session) => void;
    onreopen: (s: Session, beside: boolean) => void;
    onmenu: (e: MouseEvent, s: Session) => void;
    onselect?: () => void;
    /** Threads nested under this one (threadFamilies.ts): fold, count, aggregate status; or detached from its parent. */
    family?: { kids: string[]; detached: boolean; folded: boolean; ontoggle: () => void };
  }

  let { session: s, variant, child = false, onrowclick, onreopen, onmenu, onselect, family }: Props = $props();

  const history = $derived(variant === 'history');
  const st = $derived(threadRowState(s));
  const pinned = $derived(threads.isPinned(s.id));
  const selected = $derived(threads.focusedSessionId === s.id && !threads.newDraftMode);
  const inPane = $derived(threads.paneIds.includes(s.id) && !threads.newDraftMode);
  /** Finished a turn while you looked elsewhere (cleared when focused). */
  const unread = $derived(!selected && st.key !== 'needs-you' && threadMeta.unread(s.id, ws.unread[s.id] === true || runtimeThreads.unread[s.id] === true));

  const kids = $derived(family ? family.kids : []);
  /** Its subagents rolled up (runtime/childThreads.ts `childRollup`, the one source). */
  const subs = $derived(
    kids.length === 0
      ? null
      : childRollup(kids.map((id) => ({ title: ws.getSession(id)?.title ?? runtimeThreads.get(id)?.title ?? '', word: subagentWord(threadStateById(id)?.key) }))),
  );
  const subNeeds = $derived(!history && !!subs && subs.needsYou > 0);
  const cls = $derived(history ? 'rest' : subNeeds ? 'needs' : inboxClass(st.key, unread));
  const runtime = $derived(isRuntimeThread(s));
  $effect(() => {
    // Approval vs Question needs the request's kind (one queued read); a
    // working row's start (one queued read; live events keep it after that).
    if (runtime && (cls === 'needs' || cls === 'working')) inboxLive.ensure(s.id);
  });
  const word = $derived(
    child ? subagentWord(st.key) : subNeeds ? 'Needs you' : statusWord(cls, {
      kinds: inboxLive.kindsOf(s.id),
      workingSince: cls === 'working' ? (inboxLive.since[s.id] ?? null) : null,
      now: cls === 'working' ? now() : 0,
    }),
  );
  const tone = $derived(
    word === 'Approval' || word === 'Question' || word === 'Needs you' ? 'warning' : word === 'Failed' ? 'danger' : word === 'Done' ? 'success' : '',
  );

  const meta = $derived(s.meta as Record<string, unknown> | undefined);
  /** A personal agent's thread is under the agent's name, never its folder's id. */
  const agentName = $derived(personalAgentName(meta, s.title));
  /** (Its folder is `…/personal/<agent id>`: never shown.) */
  const project = $derived(agentName ?? projectName(s.cwd));
  /** A personal agent's thread says what started it (lib/personalAgentBadges.ts). */
  const badge = $derived(agentThreadBadge(meta));
  // The branch: the thread's own (a worktree's), else its repo's checked-out
  // one. History rows ask for nothing.
  // A worktree (worktreeProjects) says its branch with the worktree icon.
  const linked = $derived(worktreeProjects.of(s.cwd));
  const worktreeBranch = $derived(typeof meta?.worktree_branch === 'string' && meta.worktree_branch.trim() ? meta.worktree_branch.trim() : (linked?.branch ?? null));
  const repo = $derived(history || worktreeBranch ? null : threadRepos.repoFor(s.cwd, s.workspace_id));
  $effect(() => {
    if (repo) git.ensureStatus(repo.id);
  });
  const branch = $derived(
    worktreeBranch ??
      (['git_branch', 'branch'].map((k) => meta?.[k]).find((v): v is string => typeof v === 'string' && v.trim() !== '') ||
        (repo ? (git.statusById[repo.id]?.branch ?? null) : null)),
  );

  // A council seat nested under seat 1 reads as its agent ("… · codex").
  const label = $derived(child && councilOf(runtimeThreads.get(s.id)) ? s.title.replace(/^.* · /, '') : s.title);
  const tip = $derived([s.title, project, harnessLabel(s.provider), branch ? `on ${branch}` : null, subNeeds ? `Needs you: ${subs?.needing || 'a subagent'}` : st.key !== 'idle' ? st.hint ?? st.label : null].filter(Boolean).join(' · '));

  const testid = $derived(
    variant === 'pinned' ? `thread-pinned-${s.id}` : variant === 'history' ? `thread-history-${s.id}` : `thread-row-${s.id}`,
  );
</script>

<div
  class="tr-row reveal-host"
  class:child
  class:tr-family={kids.length > 0}
  class:strong={cls === 'needs' || cls === 'done'}
  class:selected
  class:in-pane={inPane && !selected}
  data-testid={testid}
  data-state={history ? 'history' : st.key}
  data-class={cls}
>
  {#if kids.length > 0 && family}
    <button type="button" class="icon-btn tr-fold" aria-expanded={!family.folded} aria-label="{family.folded ? 'Show' : 'Fold'} the subagents of {s.title}" title={family.folded ? 'Show its subagents' : 'Fold its subagents'} data-testid="thread-family-fold-{s.id}" onclick={family.ontoggle}>
      <Icon name={family.folded ? 'chevronRight' : 'chevronDown'} size={12} />
    </button>
  {/if}
  <button
    type="button"
    class="tr-main"
    title={tip}
    aria-current={selected ? 'true' : undefined}
    onclick={(e) => (history ? onreopen(s, e.metaKey || e.ctrlKey) : onrowclick(e, s))}
    oncontextmenu={(e) => onmenu(e, s)}
    use:rowMenu
  >
    {#if !child}
      <span class="tr-line tr-l1">
        <ProjectBadge name={project} />
        <span class="tr-proj ellipsis" dir="auto">{project}</span>
        {#if badge}<span class="tr-badge tr-badge-{badge.kind} ellipsis" title={badge.title} data-testid="thread-badge-{s.id}">{badge.label}</span>{/if}
        {#if family?.detached}<span class="tr-badge" title="The thread that started it is closed or archived" data-testid="thread-detached-{s.id}">detached</span>{/if}
        <span class="tr-status {tone}" data-testid="thread-status-{s.id}">{word ?? rel(s.last_active_at)}</span>
      </span>
    {/if}
    <span class="tr-line tr-l2">
      <span class="tr-title ellipsis" dir="auto">{label}</span>
      {#if kids.length > 0}<span class="tr-kids" data-testid="thread-family-count-{s.id}">{kids.length} {kids.length === 1 ? 'subagent' : 'subagents'}</span>{/if}
    </span>
    <span class="tr-line tr-l3">
      {#if child}
        <span class="tr-status {tone}" data-testid="thread-status-{s.id}">{word ?? rel(s.last_active_at)}</span>
      {:else if branch}
        <Icon name={worktreeBranch ? 'worktree' : 'branch'} size={12} />
        <span class="tr-branch ellipsis" dir="ltr" data-worktree={worktreeBranch ? '' : undefined}>{branch}</span>
      {:else}
        <!-- No branch known: where the thread works, so line 3 always says it. -->
        <Icon name="folder" size={12} />
        <span class="tr-branch ellipsis" dir="auto">{history ? 'Closed · ' : ''}{agentName ?? (baseName(s.cwd ?? '') || '~')}</span>
      {/if}
      {#if hasProviderIcon(s.provider)}<span class="tr-prov" title={harnessLabel(s.provider)}><ProviderIcon provider={s.provider} size={12} /></span>{:else}<span class="tr-prov tr-prov-name">{harnessLabel(s.provider)}</span>{/if}
    </span>
  </button>
  <div class="tr-actions reveal-on-hover">
    {#if variant !== 'pinned'}
      <button
        type="button"
        class="icon-btn tr-act"
        class:tr-pin-on={pinned}
        aria-label="{pinned ? 'Unpin' : 'Pin'} {s.title}"
        title={pinned ? 'Unpin thread' : 'Pin thread'}
        data-testid="thread-pin-{s.id}"
        onclick={() => threads.togglePin(s.id)}
      >
        <Icon name="star" size={12} />
      </button>
    {/if}
    <button
      type="button"
      class="icon-btn tr-act"
      aria-label="Open {s.title} beside"
      title="Open beside (Split)"
      data-testid="thread-split-{s.id}"
      onclick={() => {
        if (history) onreopen(s, true);
        else {
          threads.openBeside(s.id);
          onselect?.();
        }
      }}
    >
      <Icon name="split" size={12} />
    </button>
    <button
      type="button"
      class="icon-btn tr-act"
      aria-label="More actions for {s.title}"
      title="More actions"
      aria-haspopup="menu"
      onclick={(e) => onmenu(e, s)}
    >
      <Icon name="more" size={12} />
    </button>
  </div>
</div>

<style>
  .tr-row {
    --tr-tint: transparent;
    position: relative;
    display: flex;
    align-items: flex-start;
    min-width: 0;
    border-radius: var(--radius-m);
    background: var(--tr-tint);
  }
  .tr-row:hover {
    --tr-tint: var(--hover);
  }
  .tr-row.in-pane {
    --tr-tint: var(--accent-faint);
  }
  /* Stronger than hover in light and dark alike. */
  .tr-row.selected {
    --tr-tint: color-mix(in srgb, var(--text) 12%, transparent);
  }
  .tr-row.child {
    margin-inline-start: var(--sp-6);
  }
  .tr-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    min-block-size: 64px;
    padding-block: var(--sp-3);
    padding-inline: var(--sp-5);
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .tr-row.child .tr-main {
    min-block-size: 46px;
  }
  .tr-line {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
    block-size: 18px;
  }
  .tr-l1,
  .tr-l3 {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .tr-l2 {
    block-size: 22px;
  }
  .tr-l3 :global(svg) {
    flex-shrink: 0;
  }
  .tr-proj,
  .tr-branch {
    min-width: 0;
  }
  .tr-title {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-l);
    font-weight: 500;
  }
  .tr-row.strong .tr-title {
    font-weight: 600;
  }
  .tr-status {
    flex-shrink: 0;
    margin-inline-start: auto;
    font-variant-numeric: tabular-nums;
  }
  .tr-row.child .tr-status {
    margin-inline-start: 0;
  }
  .tr-status.warning,
  .tr-status.danger,
  .tr-status.success {
    font-weight: 600;
  }
  .tr-status.warning {
    color: var(--warning);
  }
  .tr-status.danger {
    color: var(--danger);
  }
  .tr-status.success {
    color: var(--success);
  }
  .tr-prov {
    display: inline-flex;
    flex-shrink: 0;
    margin-inline-start: auto;
  }
  .tr-badge {
    flex-shrink: 1;
    min-width: 0;
    max-inline-size: 45%;
    padding: 0 var(--sp-3);
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: var(--fs-xs);
    line-height: 16px;
    color: var(--text-dim);
  }
  .tr-badge-chat {
    color: var(--accent-text);
    border-color: var(--accent-line);
  }
  .tr-badge-scheduled {
    color: var(--info);
  }
  .tr-badge-delegated {
    color: var(--success);
  }
  .tr-kids {
    flex: none;
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  /* Pin / beside / ⋯ over line 3's end (the branch and glyph), never over
     the status word or the title. Opaque so the line never shows through. */
  .tr-actions {
    position: absolute;
    inset-block-end: var(--sp-2);
    inset-inline-end: var(--sp-2);
    display: flex;
    align-items: center;
    padding-inline-start: var(--sp-3);
    border-radius: var(--radius-m);
    background: linear-gradient(var(--tr-tint), var(--tr-tint)), var(--bg-sidebar);
  }
  /* Line 3's height, so they never reach the title above it. */
  .tr-act {
    block-size: 20px;
    min-block-size: 20px;
  }
  .tr-fold {
    flex: none;
    margin-block-start: var(--sp-6);
    margin-inline-end: calc(-1 * var(--sp-4));
  }
  .tr-pin-on {
    color: var(--warning);
  }
  .tr-pin-on :global(svg path) {
    fill: currentColor;
  }
  /* Touch: the row is one tap target; pin / beside / ⋯ live in its
     long-press menu (use:rowMenu); the subagents chevron is finger-sized. */
  @media (hover: none), (pointer: coarse) {
    .tr-actions {
      display: none;
    }
    .tr-fold {
      inline-size: var(--hit-min);
      block-size: var(--hit-min);
      margin-block-start: var(--sp-5);
    }
  }
</style>
