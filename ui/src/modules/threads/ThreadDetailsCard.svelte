<script lang="ts">
  // Thread details: ONE card for a thread's facts (T3 Code's details card) —
  // the folder (Open in ▾), the branch ▾, the one git action the checkout is
  // ready for (Commit → Push → Create PR → "PR #N", threadDetails.ts), Changes
  // +N −M, then its subagents (threadSubagents.ts): the thread it works
  // under, the ones running, the rest under Previous. It floats top-right of the conversation when there is room and
  // heads the right panel otherwise (ThreadsPage / ThreadInspector). Every
  // row is a door into an existing Otto surface (git store, the Git page's
  // commit composer, the Create-PR sheet and push flow — both confirm where
  // things go first —, the Changes and Pull request surfaces, the subagent's
  // own thread).
  import { api, isAbortError } from '../../lib/api/client';
  import type { DiffResp, Repo } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../lib/components/ProviderIcon.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { now } from '../../lib/stores/now.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { agentTasks, taskElapsed } from './runtime/agentTasks';
  import { openChildThread } from './runtime/childNav';
  import { councilOf, linkedChildren } from './runtime/council';
  import { mapChildThreadSummaries } from './runtime/runtimeComposer';
  import RuntimeTaskStop from './runtime/RuntimeTaskStop.svelte';
  import { runtimeThreads } from './runtime/runtimeThreads.svelte';
  import { runtimeViews } from './runtime/runtimeView.svelte';
  import { contextMeter } from './runtime/contextMeter';
  import { gitAction } from './threadDetails';
  import { anyThread } from './threadRelay';
  import { subagentSection, type SubagentRow } from './threadSubagents';
  import { delegatedParentId } from './threadsHelpers';
  import { openBranchMenu, openOpenInMenu } from './threadGitMenus';
  import { threadPrs } from './threadPrs.svelte';
  import { compactPath, forgeLabel } from './threadsHelpers';
  import { threadRepos } from './threadRepos.svelte';
  import { threads } from './threadsStore.svelte';

  // The Create-PR sheet (outward confirm lives there) loads on first open, not with the page.
  const CreatePrLazy = lazyComponent(() => import('../git/CreatePr.svelte')); // ui-guards: allow — Threads reuses the one Create-PR sheet

  interface Props {
    cwd: string;
    /** The thread the card describes; null on the new-thread stage. */
    threadId?: string | null;
    /** Floating beside the conversation (shadowed, dismissible) vs. flat heading the right panel. */
    floating?: boolean;
    /** Hides the floating card (remembered; ⋯ › View › Thread details brings it back). */
    ondismiss?: () => void;
  }
  let { cwd, threadId = null, floating = false, ondismiss }: Props = $props();
  // A thread stopped on its provider's usage limit, and when it resumes (RuntimeUsageLimit).
  const limitRow = $derived.by(() => {
    const rt = threadId ? runtimeThreads.get(threadId) : null;
    if (rt?.last_turn_status !== 'failed' || rt.last_error_class !== 'usage_limit') return null;
    const at = (rt.meta as Record<string, unknown> | null | undefined)?.resume_at;
    return typeof at === 'string' ? `resumes ${new Date(at).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}` : '';
  });
  const ctxMeter = $derived(contextMeter(threadId ? runtimeViews.projections[threadId]?.thread?.context_usage : null));

  let openInEl = $state<HTMLButtonElement | null>(null);
  let branchEl = $state<HTMLButtonElement | null>(null);
  let gitMenuEl = $state<HTMLButtonElement | null>(null);
  let prOpen = $state(false);
  let totals = $state<{ added: number; deleted: number } | null>(null);

  const homePath = $derived(ws.scratch?.root_path ?? null);
  const project = $derived(cwd.replace(/\/+$/, ''));
  const repo = $derived(threadRepos.repoFor(project));
  const repoChecked = $derived(threadRepos.checked(project));
  const status = $derived(repo ? (git.statusById[repo.id] ?? null) : null);
  const branch = $derived(status?.branch || null);
  const forge = $derived(forgeLabel(repo));
  const pr = $derived(threadPrs.prFor(repo, branch));
  const action = $derived(
    gitAction({
      changes: status?.changes.length ?? 0,
      branch,
      upstream: status?.upstream ?? null,
      ahead: status?.ahead ?? 0,
      remote: !!repo?.remote_url,
      forge: !!forge,
      prNumber: pr?.number ?? null,
    }),
  );

  $effect(() => {
    void git.loadAllRepos();
  });
  $effect(() => {
    if (repo) git.ensureStatus(repo.id);
  });

  // Working-tree totals for "Changes +N −M" — re-read whenever the repo's
  // status changes (the git store follows repo events), one summary pass only.
  $effect(() => {
    const id = repo?.id ?? null;
    void status;
    if (!id) {
      totals = null;
      return;
    }
    const ctl = new AbortController();
    void api
      .get<DiffResp>(`/repos/${encodeURIComponent(id)}/diff?target=working&summary=true`, ctl.signal)
      .then((r) => {
        if (ctl.signal.aborted) return;
        let added = r.total_added ?? 0;
        let deleted = r.total_deleted ?? 0;
        if (r.total_added == null) for (const f of r.files) added += f.added ?? 0;
        if (r.total_deleted == null) for (const f of r.files) deleted += f.deleted ?? 0;
        totals = { added, deleted };
      })
      .catch((e) => {
        if (!ctl.signal.aborted && !isAbortError(e)) totals = null;
      });
    return () => ctl.abort();
  });

  // Everything working under this thread (threadSubagents.ts): a runtime
  // thread's subagents and the threads opened from it, a terminal thread's
  // delegated threads, and the thread it works under itself.
  const subagents = $derived.by(() => {
    if (!threadId) return null;
    const rt = runtimeThreads.get(threadId);
    const p = rt ? (runtimeViews.projections[threadId] ?? null) : null;
    const tasks = rt
      ? agentTasks(p?.items ?? [], p?.session?.background_tasks ?? rt.background_tasks ?? [], {
          childThreads: mapChildThreadSummaries(linkedChildren(Object.values(runtimeThreads.byId), threadId)),
        })
      : { running: [], finished: [] };
    const parent = anyThread(rt?.parent_id ?? delegatedParentId(ws.getSession(threadId)));
    const s = subagentSection(tasks, {
      parent: parent ? { id: parent.id, title: parent.title } : null,
      // Delegated terminal threads, and a council's other seats (peers that
      // linkedChildren leaves out of the task list).
      delegated: [
        ...ws.sessions.filter((x) => !x.archived && delegatedParentId(x) === threadId),
        ...Object.values(runtimeThreads.byId).filter((x) => x.parent_id === threadId && councilOf(x)),
      ].map((x) => ({ id: x.id, title: x.title, provider: x.provider, status: ws.statusMap[x.id] ?? x.status })),
      providerOf: (id) => runtimeThreads.get(id)?.provider ?? null,
    });
    return s.parent || s.running.length || s.previous.length ? s : null;
  });
  let previousOpen = $state(false);
  const nowMs = $derived(subagents?.running.length ? now() : Date.now());
  const PREVIOUS_ROWS = 5;

  /** Opens a thread from the card; on a phone the details drawer gives way to it. */
  function openThread(id: string): void {
    if (threads.inspectorCompact) threads.setInspectorOpen(false);
    openChildThread(id);
  }

  function openSubagent(r: SubagentRow): void {
    if (r.threadId) openThread(r.threadId);
    else threads.showAgentsTasks();
  }

  // Compact past 10k (T3: "1m", not "1000k") so huge diffs keep the row calm.
  const compactFmt = new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 });
  const fmt = (n: number): string => (n >= 10_000 ? compactFmt.format(n) : n.toLocaleString());

  // A sheet that failed to load says so instead of silently not opening.
  $effect(() => {
    if (prOpen && CreatePrLazy.error) {
      toasts.error('Couldn’t open Create PR', CreatePrLazy.error);
      prOpen = false;
      CreatePrLazy.retry();
    }
  });

  function commit(r: Repo): void {
    git.requestWip(r.id);
    router.go(`git/${encodeURIComponent(r.id)}`);
  }

  function runAction(): void {
    if (!repo) return;
    if (action.kind === 'commit') commit(repo);
    else if (action.kind === 'push' || action.kind === 'publish') void pushBranch(repo);
    else if (action.kind === 'create-pr') prOpen = true;
    else if (action.kind === 'pr') threads.setInspectorTab('pr');
  }

  // The action's chevron: every git step, whatever the state; rows that can't
  // apply are disabled and say why.
  function openGitMenu(): void {
    if (!gitMenuEl || !repo) return;
    const r = repo;
    const dirty = (status?.changes.length ?? 0) > 0;
    const canPush = !!branch && !!r.remote_url && (!status?.upstream || (status?.ahead ?? 0) > 0);
    const items: MenuItem[] = [
      { label: 'Commit…', icon: 'commit', disabled: !dirty, title: dirty ? 'Open the commit composer in Git' : 'Nothing to commit', action: () => commit(r) },
      {
        label: status?.upstream ? 'Push…' : 'Publish branch…',
        icon: 'arrowUp',
        hint: status?.upstream && status.ahead ? `↑${status.ahead}` : undefined,
        disabled: !canPush,
        title: canPush ? undefined : !r.remote_url ? 'This repo has no remote' : branch ? 'Nothing to push' : 'Check out a branch first',
        action: () => void pushBranch(r),
      },
      pr
        ? { label: `View PR #${pr.number}`, icon: 'pr', action: () => threads.setInspectorTab('pr') }
        : { label: 'Create PR…', icon: 'pr', disabled: !forge, title: forge ? undefined : 'This repo has no supported remote', action: () => (prOpen = true) },
    ];
    ctxMenu.showAt(gitMenuEl, items, { align: 'end' });
  }

  async function pushBranch(r: Repo): Promise<void> {
    const st = git.statusById[r.id];
    if (!st?.branch) return;
    const dest = st.upstream ?? `origin/${st.branch}`;
    const n = st.ahead ?? 0;
    const ok = await confirmer.ask(
      st.upstream
        ? `Push ${n === 1 ? '1 commit' : `${n} commits`} on ${st.branch} to ${dest}${forge ? ` (${forge})` : ''}? Everyone with access to the remote can see them.`
        : `Publish ${st.branch} to ${dest}${forge ? ` (${forge})` : ''}? Everyone with access to the remote can see it.`,
      { title: st.upstream ? 'Push to remote' : 'Publish branch', confirmLabel: st.upstream ? 'Push' : 'Publish', danger: false },
    );
    if (!ok) return;
    const { runPush } = await import('../git/pushFlow'); // ui-guards: allow — the Git page's one push flow (rejected-push handling)
    await runPush(r.id, st, () => {});
  }

  const ACTION_ICON = { commit: 'commit', push: 'arrowUp', publish: 'arrowUp', 'create-pr': 'pr', pr: 'pr', none: 'pr' } as const;
</script>

<section class="dc-card" class:floating aria-label="Thread details" data-testid="thread-details-card">
  <div class="dc-group">
    <div class="dc-line">
      <div class="dc-row dc-grow dc-static" title={project} data-testid="details-folder">
        <Icon name="folder" size={14} />
        <span class="dc-label mono" dir="ltr">{compactPath(project, homePath)}</span>
      </div>
      <button
        bind:this={openInEl}
        type="button"
        class="dc-end-btn"
        aria-haspopup="menu"
        title="Open the folder in Files, Git or a terminal; project scripts"
        data-testid="details-open-in"
        onclick={() => openInEl && openOpenInMenu(openInEl, { cwd: project, repo })}
      >Open in<Icon name="chevronDown" size={12} /></button>
      {#if ondismiss}
        <button type="button" class="icon-btn dc-x" aria-label="Hide thread details" title="Hide thread details (⋯ › View brings them back)" data-testid="details-dismiss" onclick={ondismiss}>
          <Icon name="x" size={12} />
        </button>
      {/if}
    </div>
  </div>

  <div class="dc-group">
    {#if repo}
      <button bind:this={branchEl} type="button" class="dc-row" aria-haspopup="menu" title="Switch branch" data-testid="details-branch" onclick={() => branchEl && void openBranchMenu(branchEl, repo)}>
        <Icon name="branch" size={14} />
        <span class="dc-label mono" dir="ltr">{branch ?? 'Detached HEAD'}</span>
        <Icon name="chevronDown" size={12} />
      </button>
      {#if action.kind !== 'none'}
      <div class="dc-line">
        <button
          type="button"
          class="dc-row dc-grow"
          title={action.kind === 'create-pr' ? `Open a pull request on ${forge} — you review it before it is sent` : action.kind === 'push' || action.kind === 'publish' ? 'Asks where it goes first' : action.kind === 'commit' ? 'Open the commit composer in Git' : undefined}
          data-testid="details-git-action"
          data-kind={action.kind}
          onpointerenter={() => action.kind === 'create-pr' && CreatePrLazy.prefetch()}
          onclick={runAction}
        >
          <Icon name={ACTION_ICON[action.kind]} size={14} />
          <span class="dc-label">{action.label}</span>
        </button>
        <button bind:this={gitMenuEl} type="button" class="icon-btn dc-more" aria-haspopup="menu" aria-label="Commit, push or open a pull request" title="Commit · Push · Pull request" data-testid="details-git-menu" onclick={openGitMenu}>
          <Icon name="chevronDown" size={12} />
        </button>
      </div>
      {/if}
      <button type="button" class="dc-row" title="Review the working-tree changes" data-testid="details-changes" onclick={() => threads.setInspectorTab('changes')}>
        <Icon name="file" size={14} />
        <span class="dc-label">{totals && totals.added + totals.deleted === 0 ? 'No changes' : 'Changes'}</span>
        {#if totals && totals.added + totals.deleted > 0}
          <span class="dc-stat mono" dir="ltr" title="{totals.added.toLocaleString()} added, {totals.deleted.toLocaleString()} deleted">
            <span class="dc-add">+{fmt(totals.added)}</span>
            <span class="dc-del">−{fmt(totals.deleted)}</span>
          </span>
        {/if}
      </button>
    {:else}
      <p class="dc-note">{!project ? 'Pick a project folder' : repoChecked ? 'Not a git repository' : 'Looking for a git repository…'}</p>
    {/if}
    {#if limitRow !== null}
      <div class="dc-row dc-static" data-testid="details-limit">
        <Icon name="clock" size={14} />
        <span class="dc-label">Limited</span>
        <span class="dc-stat dc-warn">{limitRow}</span>
      </div>
    {/if}
    {#if ctxMeter}
      <!-- How full the agent's context is (runtime/contextMeter.ts). -->
      <div class="dc-row dc-static" title="Context: {ctxMeter.text}" data-testid="details-context">
        <Icon name="gauge" size={14} />
        <span class="dc-label">Context</span>
        <span class="dc-stat mono" class:dc-warn={ctxMeter.tone === 'warning'} class:dc-danger={ctxMeter.tone === 'danger'} dir="ltr">{ctxMeter.pct} %</span>
      </div>
    {/if}
  </div>

  {#if subagents}
    {@const s = subagents}
    <div class="dc-group" data-testid="details-subagents">
      {#if s.parent}
        {@const parentId = s.parent.id}
        <button type="button" class="dc-row" title="The thread this one works under" data-testid="details-parent" onclick={() => openThread(parentId)}>
          <Icon name="arrowUp" size={14} />
          <span class="dc-label" dir="auto">{s.parent.title}</span>
        </button>
      {/if}
      {#if s.running.length || s.previous.length}
        <p class="dc-cap">Subagents{s.running.length ? ` · ${s.running.length} running` : ''}</p>
      {/if}
      {#each s.running as r (r.id)}
        {@render subagent(r)}
      {/each}
      {#if s.previous.length}
        <button type="button" class="dc-row dc-dim" aria-expanded={previousOpen} data-testid="details-previous" onclick={() => (previousOpen = !previousOpen)}>
          <Icon name={previousOpen ? 'chevronDown' : 'chevronRight'} size={12} />
          <span class="dc-label">Previous · {s.previous.length}</span>
        </button>
        {#if previousOpen}
          {#each s.previous.slice(0, PREVIOUS_ROWS) as r (r.id)}
            {@render subagent(r)}
          {/each}
          {#if s.previous.length > PREVIOUS_ROWS}
            <button type="button" class="dc-row dc-dim" onclick={() => threads.showAgentsTasks()}>
              <span class="dc-label">All in Agents & tasks…</span>
            </button>
          {/if}
        {/if}
      {/if}
    </div>
  {/if}
</section>

{#snippet subagent(r: SubagentRow)}
  <div class="dc-line" data-testid="details-subagent" data-word={r.word}>
    <button type="button" class="dc-row dc-grow" title={r.threadId ? `Open ${r.title}` : `${r.title}: Agents & tasks`} onclick={() => openSubagent(r)}>
      {#if r.provider && hasProviderIcon(r.provider)}
        <ProviderIcon provider={r.provider} size={14} />
      {:else}
        <Icon name="assistant" size={14} />
      {/if}
      <span class="dc-label" dir="auto">{r.title}</span>
      <span class="dc-word" data-word={r.word}>{r.word}</span>
      {#if r.word === 'Working' || r.word === 'Needs you'}
        <span class="dc-end mono">{taskElapsed(r, nowMs) ?? ''}</span>
      {/if}
    </button>
    {#if r.task && threadId && runtimeThreads.get(threadId)}<RuntimeTaskStop part="button" sessionId={threadId} task={r.task} />{/if}
  </div>
{/snippet}

{#if prOpen && repo && CreatePrLazy.component}
  {@const CreatePr = CreatePrLazy.component}
  <CreatePr
    repoId={repo.id}
    initialSource={branch ?? undefined}
    onclose={() => (prOpen = false)}
    oncreated={(created) => {
      prOpen = false;
      threadPrs.forget(repo, branch);
      router.go(`git/${encodeURIComponent(repo.id)}/pr/${created.number}`);
    }}
  />
{/if}

<style>
  .dc-card {
    display: flex;
    flex-direction: column;
    min-inline-size: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    font-size: var(--fs-m);
    overflow: hidden;
  }
  .dc-card.floating {
    box-shadow: var(--shadow-card);
  }
  .dc-group {
    display: flex;
    flex-direction: column;
    padding-block: var(--sp-3);
  }
  .dc-group + .dc-group {
    border-block-start: 1px solid var(--border);
  }
  .dc-line {
    display: flex;
    align-items: center;
    min-inline-size: 0;
  }
  .dc-row {
    display: flex;
    align-items: center;
    gap: var(--sp-5);
    min-inline-size: 0;
    min-block-size: 34px;
    padding-inline: var(--sp-6);
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .dc-grow {
    flex: 1;
  }
  .dc-row :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  button.dc-row:hover:not(:disabled) {
    background: var(--hover);
  }
  .dc-row:disabled {
    cursor: default;
    color: var(--text-dim);
  }
  .dc-static {
    cursor: default;
  }
  .dc-dim {
    color: var(--text-dim);
  }
  .dc-label {
    flex: 1;
    min-inline-size: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dc-end-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    flex: none;
    block-size: 28px;
    padding-inline: var(--sp-4);
    margin-inline-end: var(--sp-3);
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    cursor: pointer;
  }
  .dc-end-btn:hover {
    background: var(--hover);
    color: var(--text);
  }
  .dc-x,
  .dc-more {
    flex: none;
    margin-inline-end: var(--sp-3);
    color: var(--text-dim);
  }
  .dc-end {
    flex: none;
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .dc-warn {
    color: var(--warning);
  }
  .dc-danger {
    color: var(--danger);
  }
  .dc-stat {
    display: inline-flex;
    gap: var(--sp-3);
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .dc-add {
    color: var(--success);
  }
  .dc-del {
    color: var(--danger);
  }
  .dc-cap {
    padding-block: var(--sp-3);
    padding-inline: var(--sp-6);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .dc-cap {
    margin: 0;
  }
  .dc-word {
    flex: none;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .dc-word[data-word='Needs you'] {
    color: var(--warning);
    font-weight: 600;
  }
  .dc-word[data-word='Failed'] {
    color: var(--danger);
  }
  .dc-word[data-word='Done'] {
    color: var(--success);
  }
  .dc-note {
    margin: 0;
    padding-block: var(--sp-3);
    padding-inline: var(--sp-6);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  /* Touch: every control is a full finger-sized target (44 px, Apple's HIG). */
  @media (pointer: coarse) {
    .dc-row,
    .dc-end-btn,
    .dc-x,
    .dc-more {
      min-block-size: 44px;
    }
    .dc-x,
    .dc-more {
      min-inline-size: 44px;
    }
  }
</style>
