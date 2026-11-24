<script lang="ts">
  // One inbox row (T3 Code / Orca) on the shared lib/components/InboxRow:
  // three lines, the whole row one button —
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
  import InboxRow from '../../lib/components/InboxRow.svelte';
  import { councilOf } from './runtime/council';
  import { inboxClass, statusWord, subagentWord } from './threadInbox';
  import { wordTone } from '../../lib/statusWords';
  import { childRollup, familyCountLabel } from './runtime/childThreads';
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
      : childRollup(kids.map((id) => ({ title: ws.getSession(id)?.title ?? runtimeThreads.get(id)?.title ?? '', word: subagentWord(threadStateById(id)?.key, { limited: threadStateById(id)?.label === 'Limited' }) }))),
  );
  const subNeeds = $derived(!history && !!subs && subs.needsYou > 0);
  const cls = $derived(history ? 'rest' : subNeeds ? 'needs' : inboxClass(st.key, unread, inboxLive.kindsOf(s.id).length));
  const runtime = $derived(isRuntimeThread(s));
  $effect(() => {
    // Approval vs Question needs the request's kind (one queued read); a
    // working row's start comes from the row itself (inboxLive.workingSince).
    if (runtime && cls === 'needs') inboxLive.ensure(s.id);
  });
  const word = $derived(
    child ? subagentWord(st.key, { limited: st.label === 'Limited' }) : subNeeds ? 'Needs you' : statusWord(cls, {
      kinds: inboxLive.kindsOf(s.id),
      workingSince: cls === 'working' ? inboxLive.workingSince(s.id, s.meta as Record<string, unknown> | null) : null,
      now: cls === 'working' ? now() : 0,
      limited: st.label === 'Limited',
    }),
  );
  const tone = $derived(wordTone(word));

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

<InboxRow
  title={label}
  project={child ? null : project}
  {word}
  {tone}
  time={rel(s.last_active_at)}
  {tip}
  {child}
  strong={cls === 'needs' || cls === 'done'}
  {selected}
  {inPane}
  {testid}
  statusTestid="thread-status-{s.id}"
  state={history ? 'history' : st.key}
  {cls}
  fold={kids.length > 0 && family ? { folded: family.folded, label: `the subagents of ${s.title}`, testid: `thread-family-fold-${s.id}`, ontoggle: family.ontoggle } : undefined}
  onclick={(e) => (history ? onreopen(s, e.metaKey || e.ctrlKey) : onrowclick(e, s))}
  oncontextmenu={(e) => onmenu(e, s)}
>
  {#snippet context()}
    {#if badge}<span class="tr-badge tr-badge-{badge.kind} ellipsis" title={badge.title} data-testid="thread-badge-{s.id}">{badge.label}</span>{/if}
    {#if family?.detached}<span class="tr-badge" title="The thread that started it is closed or archived" data-testid="thread-detached-{s.id}">detached</span>{/if}
  {/snippet}
  {#snippet titleEnd()}
    {#if kids.length > 0}<span class="tr-kids" dir="auto" data-testid="thread-family-count-{s.id}">{familyCountLabel(kids.map((id) => !!councilOf(runtimeThreads.get(id))))}</span>{/if}
  {/snippet}
  {#snippet line3()}
    {#if child}
      <!-- (the word leads a child row's line 3: InboxRow) -->
    {:else if branch}
      <Icon name={worktreeBranch ? 'worktree' : 'branch'} size={12} />
      <span class="tr-branch ellipsis" dir="ltr" data-worktree={worktreeBranch ? '' : undefined}>{branch}</span>
    {:else}
      <!-- No branch known: where the thread works, so line 3 always says it. -->
      <Icon name="folder" size={12} />
      <span class="tr-branch ellipsis" dir="auto">{history ? 'Closed · ' : ''}{agentName ?? (baseName(s.cwd ?? '') || '~')}</span>
    {/if}
    {#if hasProviderIcon(s.provider)}<span class="tr-prov" title={harnessLabel(s.provider)}><ProviderIcon provider={s.provider} size={12} /></span>{:else}<span class="tr-prov tr-prov-name">{harnessLabel(s.provider)}</span>{/if}
  {/snippet}
  {#snippet actions()}
    {#if variant !== 'pinned'}
      <button
        type="button"
        class="icon-btn"
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
      class="icon-btn"
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
      class="icon-btn"
      aria-label="More actions for {s.title}"
      title="More actions"
      aria-haspopup="menu"
      onclick={(e) => onmenu(e, s)}
    >
      <Icon name="more" size={12} />
    </button>
  {/snippet}
</InboxRow>

<style>
  .tr-branch {
    min-width: 0;
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
  .tr-pin-on {
    color: var(--warning);
  }
  .tr-pin-on :global(svg path) {
    fill: currentColor;
  }
</style>
