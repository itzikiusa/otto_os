<script lang="ts">
  // PR detail: meta, editable markdown description, diff with inline comment
  // threads, general comments, approve/merge/decline, "open as session".
  // Three tabs: Summary | Files | Review (AI agents).
  import { onDestroy, untrack } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { DiffResp, PrComment, PrCommit, PrDetail } from '../../lib/api/types';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmOutward } from '../../lib/confirmOutward';
  import { renderMarkdown } from '../../lib/md';
  import { openExternal } from '../../lib/external';
  import DiffViewer from './DiffViewer.svelte';
  import CommentThread from './CommentThread.svelte';
  import ReviewPanel from './ReviewPanel.svelte';
  import PrMergeModal from './PrMergeModal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Icon from '../../lib/components/Icon.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import { rel } from '../../lib/stores/now.svelte';

  interface Props {
    repoId: string;
    number: number;
  }
  let { repoId, number }: Props = $props();
  let disposed = false;
  onDestroy(() => { disposed = true; });

  type Tab = 'summary' | 'files' | 'commits' | 'review';
  const TABS: Tab[] = ['summary', 'files', 'commits', 'review'];
  let activeTab: Tab = $state('summary');

  // Remember the last-used tab per PR so returning to a PR with a running
  // review lands back on Review (not Summary). Restore on PR change.
  $effect(() => {
    const saved = localStorage.getItem(`otto_pr_tab_${repoId}_${number}`);
    activeTab = saved && TABS.includes(saved as Tab) ? (saved as Tab) : 'summary';
  });

  function selectTab(t: Tab): void {
    activeTab = t;
    localStorage.setItem(`otto_pr_tab_${repoId}_${number}`, t);
  }

  let pr: PrDetail | null = $state(null);
  let diff: DiffResp | null = $state(null);
  let commits: PrCommit[] | null = $state(null);
  let loading = $state(true);
  let diffLoading = $state(false);
  let commitsLoading = $state(false);
  let editMode = $state(false);
  let editTitle = $state('');
  let editDesc = $state('');
  let busy = $state('');
  let newComment = $state('');
  // Merge runs through PrMergeModal — it owns the strategy, the readiness
  // rows and the delete-source-branch choice.
  let mergeOpen = $state(false);
  let showRequestChanges = $state(false);
  let requestChangesBody = $state('');
  $effect(() => guardUnsaved(
    () => (editMode && !!pr && (editTitle !== pr.title || editDesc !== pr.description_md))
      || newComment.trim() !== '' || requestChangesBody.trim() !== '',
    { what: `pull request #${number}` },
  ));
  // Provider to spawn the "open as session" review agent on (registry-sourced,
  // defaults to the configured default agent — never a hardcoded 'claude').
  let reviewProvider = $state(defaultAgentProvider());

  const inlineComments = $derived.by(() => (pr?.comments ?? []).filter((c) => c.path !== null));
  const generalComments = $derived.by(() => (pr?.comments ?? []).filter((c) => c.path === null));

  // A different PR (back/forward, a notification deep link) reuses this
  // component: drop the previous PR's data first, so a slow or failed load
  // never shows PR A's title/diff (or offers its Approve…) under PR B's number.
  $effect(() => {
    const rid = repoId;
    const num = number;
    untrack(() => {
      pr = null;
      diff = null;
      commits = null;
      prError = null;
      diffError = null;
      commitsError = null;
    });
    void load(rid, num);
  });

  // Load failures render INLINE with Retry (a toast is for failed actions).
  let prError = $state<string | null>(null);

  // Lazy-load diff when switching to Files tab. `diffError` stops the effect
  // from re-firing after an error — diff stays null on failure, so without the
  // latch this loop hammered the daemon with retries forever.
  let diffError = $state<string | null>(null);
  $effect(() => {
    if (activeTab === 'files' && diff === null && !diffLoading && !diffError) {
      void loadDiff(repoId, number);
    }
  });

  // Lazy-load commits when switching to Commits tab (same failure latch; the
  // latch used to leave the tab on "Loading commits…" forever).
  let commitsError = $state<string | null>(null);
  $effect(() => {
    if (activeTab === 'commits' && commits === null && !commitsLoading && !commitsError) {
      void loadCommits(repoId, number);
    }
  });

  async function load(rid: string, num: number): Promise<void> {
    loading = true;
    try {
      const next = await api.get<PrDetail>(`/repos/${rid}/prs/${num}`);
      if (disposed || rid !== repoId || num !== number) return; // switched PRs mid-flight
      pr = next;
      prError = null;
    } catch (e) {
      if (!disposed && rid === repoId && num === number) prError = loadErrorText(e);
    } finally {
      if (!disposed) loading = false;
    }
  }

  async function loadDiff(rid: string, num: number): Promise<void> {
    diffLoading = true;
    try {
      const next = await api.get<DiffResp>(`/repos/${rid}/prs/${num}/diff`);
      if (disposed || rid !== repoId || num !== number) return;
      diff = next;
      diffError = null;
    } catch (e) {
      if (!disposed && rid === repoId && num === number) diffError = loadErrorText(e);
    } finally {
      if (!disposed) diffLoading = false;
    }
  }

  async function loadCommits(rid: string, num: number): Promise<void> {
    commitsLoading = true;
    try {
      const next = await api.get<PrCommit[]>(`/repos/${rid}/prs/${num}/commits`);
      if (disposed || rid !== repoId || num !== number) return;
      commits = next;
      commitsError = null;
    } catch (e) {
      if (!disposed && rid === repoId && num === number) commitsError = loadErrorText(e);
    } finally {
      if (!disposed) commitsLoading = false;
    }
  }

  async function requestChanges(): Promise<void> {
    if (busy !== '') return;
    busy = 'request-changes';
    try {
      await api.post(`/repos/${repoId}/prs/${number}/request-changes`, {
        body: requestChangesBody.trim() || null,
      });
      if (disposed) return;
      toasts.success('Changes requested', `PR #${number}`);
      showRequestChanges = false;
      requestChangesBody = '';
      await load(repoId, number);
    } catch (e) {
      toasts.error('Request changes failed', e instanceof Error ? e.message : String(e));
    } finally {
      if (!disposed) busy = '';
    }
  }

  // The shared, self-ticking relative time (lib/stores/now) — no local formatter.
  const formatRelativeDate = (iso: string): string => rel(iso);

  const PR_STATE_LABEL: Record<string, string> = { open: 'Open', merged: 'Merged', declined: 'Declined' };
  const PROVIDER_LABEL: Record<string, string> = { github: 'GitHub', bitbucket: 'Bitbucket', gitlab: 'GitLab' };
  const prRepo = $derived(git.allRepos.find((r) => r.id === repoId) ?? git.repos.find((r) => r.id === repoId) ?? null);
  const providerName = $derived(prRepo?.provider ? (PROVIDER_LABEL[prRepo.provider] ?? prRepo.provider) : 'the provider');

  /** ←/→ (Home/End) move between the PR tabs, like any tablist. */
  function onTabKey(e: KeyboardEvent): void {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(e.key)) return;
    const i = TABS.indexOf(activeTab);
    const rtl = getComputedStyle(e.currentTarget as HTMLElement).direction === 'rtl';
    const fwd = e.key === (rtl ? 'ArrowLeft' : 'ArrowRight');
    const next = e.key === 'Home' ? 0 : e.key === 'End' ? TABS.length - 1 : (i + (fwd ? 1 : -1) + TABS.length) % TABS.length;
    e.preventDefault();
    selectTab(TABS[next]);
    const list = (e.currentTarget as HTMLElement).closest('[role="tablist"]');
    list?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus();
  }

  function startEdit(): void {
    if (!pr) return;
    editTitle = pr.title;
    editDesc = pr.description_md;
    editMode = true;
  }

  async function saveEdit(): Promise<void> {
    if (busy !== '') return;
    busy = 'edit';
    try {
      await api.patch(`/repos/${repoId}/prs/${number}`, { title: editTitle, description: editDesc });
      if (disposed) return;
      editMode = false;
      await load(repoId, number);
      toasts.success('Pull request updated');
    } catch (e) {
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    } finally {
      if (!disposed) busy = '';
    }
  }

  async function postComment(body: string, path?: string, line?: number, inReplyTo?: string): Promise<void> {
    await api.post<PrComment>(`/repos/${repoId}/prs/${number}/comments`, {
      body,
      path: path ?? null,
      line: line ?? null,
      in_reply_to: inReplyTo ?? null,
    });
    if (!disposed) await load(repoId, number);
  }

  /** Resolve or reopen a review thread on the provider, then refresh statuses. */
  async function resolveThread(threadId: string, resolved: boolean): Promise<void> {
    try {
      await api.post(`/repos/${repoId}/prs/${number}/comments/${encodeURIComponent(threadId)}/resolve`, { resolved });
      if (disposed) return;
      toasts.success(resolved ? 'Thread resolved' : 'Thread reopened');
      await load(repoId, number);
    } catch (e) {
      toasts.error(resolved ? 'Resolve failed' : 'Reopen failed', e instanceof Error ? e.message : String(e));
    }
  }

  // On a phone the on-screen keyboard can cover the comment box (it's at the
  // bottom of the Summary tab). Scroll it into view once focus lands; deferred so
  // it runs after the keyboard starts animating up.
  function scrollIntoViewOnFocus(e: FocusEvent): void {
    const el = e.currentTarget as HTMLElement;
    setTimeout(() => el.scrollIntoView({ block: 'center', behavior: 'smooth' }), 250);
  }

  async function addGeneralComment(): Promise<void> {
    if (busy !== '' || newComment.trim() === '') return;
    busy = 'comment';
    try {
      await postComment(newComment.trim());
      if (!disposed) newComment = '';
    } catch (e) {
      toasts.error('Couldn’t post the comment', e instanceof Error ? e.message : String(e));
    } finally {
      if (!disposed) busy = '';
    }
  }

  // Approve and Decline are posted to the provider under the user's account and
  // notify the author + reviewers, so both confirm where / what / who first.
  const repoLabel = $derived(prRepo?.name ?? 'this repository');

  async function action(kind: 'approve' | 'decline'): Promise<void> {
    const approve = kind === 'approve';
    const ok = await confirmOutward({
      verb: approve ? 'Approve PR' : 'Decline PR',
      title: approve ? `Approve PR #${number}?` : `Decline PR #${number}?`,
      where: `${repoLabel} · PR #${number}${pr ? ` “${pr.title}”` : ''}`,
      what: approve
        ? 'Your approval, posted under your git account.'
        : 'The PR is closed without merging. It can be reopened on the provider.',
      who: `${pr?.author ? `${pr.author} (the author)` : 'The author'} and the PR's reviewers are notified.`,
      danger: !approve,
    });
    if (!ok || disposed || busy !== '') return;
    busy = kind;
    try {
      await api.post(`/repos/${repoId}/prs/${number}/${kind}`);
      if (disposed) return;
      toasts.success(`PR ${kind === 'approve' ? 'approved' : kind + 'd'}`, `#${number}`);
      await load(repoId, number);
    } catch (e) {
      toasts.error(approve ? "Couldn't approve the PR" : "Couldn't decline the PR", e instanceof Error ? e.message : String(e));
    } finally {
      if (!disposed) busy = '';
    }
  }

  function moreMenu(e: MouseEvent | KeyboardEvent): void {
    ctxMenu.show(e, [{ label: 'Decline PR…', icon: 'x', danger: true, action: () => void action('decline') }]);
  }

  async function openAsSession(): Promise<void> {
    if (!pr) return;
    busy = 'session';
    try {
      await ws.createSession({
        kind: 'agent',
        provider: reviewProvider,
        title: `review PR #${pr.number}`,
        meta: {
          pr_context: {
            repo_id: repoId,
            number: pr.number,
            title: pr.title,
            url: pr.url,
            source_branch: pr.source_branch,
            target_branch: pr.target_branch,
          },
        },
      });
      // createSession → addSession → navigateToSession handles routing.
    } catch (e) {
      toasts.error('Could not open session', e instanceof Error ? e.message : String(e));
    } finally {
      if (!disposed) busy = '';
    }
  }
</script>

<div class="prd-page">
<PageHeader
  title={pr ? `Pull request #${pr.number}` : 'Pull request'}
  crumbs={[{ label: prRepo ? `${prRepo.name} · Pull requests` : 'Pull requests', onclick: () => router.go(`git/${repoId}/prs`) }]}
>
  {#snippet actions()}
    {#if pr}
      <select class="prd-provider-select" bind:value={reviewProvider} disabled={busy !== ''} title="Agent to open the review session on" aria-label="Review agent">
        {#each agentProviders() as p (p)}
          <option value={p}>{p}</option>
        {/each}
      </select>
      <button class="btn small" disabled={busy !== ''} onclick={openAsSession}>
        <Icon name="terminal" size={12} /> Open as session
      </button>
      <button class="btn small" data-icon="external" onclick={() => openExternal(pr?.url)} title="Open this pull request on {providerName}">
        <Icon name="external" size={12} /> Open on {providerName}
      </button>
    {/if}
  {/snippet}
</PageHeader>
<div class="prd">
  {#if !pr && (loading || prError)}
    <LoadState
      what="this pull request"
      variant="page"
      rows={5}
      {loading}
      error={prError}
      empty
      onretry={() => void load(repoId, number)}
    />
  {:else if pr}

    <div class="prd-title-block">
      {#if editMode}
        <input class="input prd-title-input" aria-label="Pull request title" bind:value={editTitle} disabled={busy === 'edit'} />
      {:else}
        <h2 class="prd-title">
          <span class="dim">#{pr.number}</span>
          {pr.title}
        </h2>
      {/if}
      <div class="prd-meta">
        <span class="chip {pr.state === 'open' ? 'ok' : pr.state === 'merged' ? 'accent' : 'bad'}">{PR_STATE_LABEL[pr.state] ?? pr.state}</span>
        <span class="dim">{pr.author}</span>
        <span class="mono dim">{pr.source_branch} <span class="dir-arrow">→</span> {pr.target_branch}</span>
        {#if pr.mergeable === false}<span class="chip bad"><Icon name="warning" size={12} /> Conflicts</span>{/if}
        {#if pr.approved_by.length > 0}
          <span class="chip ok" title={pr.approved_by.join(', ')}>
            <Icon name="check" size={12} /> {pr.approved_by.length} approval{pr.approved_by.length === 1 ? '' : 's'}
          </span>
        {/if}
      </div>
    </div>

    <!-- Tab bar -->
    <div class="prd-tabs" role="tablist" aria-label="Pull request views">
      <button
        class="tab-btn"
        role="tab"
        aria-selected={activeTab === 'summary'}
        tabindex={activeTab === 'summary' ? 0 : -1}
        class:active={activeTab === 'summary'}
        onclick={() => selectTab('summary')}
        onkeydown={onTabKey}
      >
        <Icon name="comment" size={12} /> Summary
      </button>
      <button
        class="tab-btn"
        role="tab"
        aria-selected={activeTab === 'files'}
        tabindex={activeTab === 'files' ? 0 : -1}
        class:active={activeTab === 'files'}
        onclick={() => selectTab('files')}
        onkeydown={onTabKey}
      >
        <Icon name="file" size={12} /> Files
      </button>
      <button
        class="tab-btn"
        role="tab"
        aria-selected={activeTab === 'commits'}
        tabindex={activeTab === 'commits' ? 0 : -1}
        class:active={activeTab === 'commits'}
        onclick={() => selectTab('commits')}
        onkeydown={onTabKey}
      >
        <Icon name="commit" size={12} /> Commits
      </button>
      <button
        class="tab-btn"
        role="tab"
        aria-selected={activeTab === 'review'}
        tabindex={activeTab === 'review' ? 0 : -1}
        class:active={activeTab === 'review'}
        onclick={() => selectTab('review')}
        onkeydown={onTabKey}
      >
        <Icon name="zap" size={12} /> Review
      </button>
    </div>

    <!-- Summary tab -->
    {#if activeTab === 'summary'}
      <section class="prd-desc card">
        {#if editMode}
          <textarea class="input" rows="8" bind:value={editDesc} aria-label="Pull request description" disabled={busy === 'edit'}></textarea>
          <div class="row prd-compose-foot">
            <span class="hint dim">Updates the pull request on {providerName}; everyone on it sees the change.</span>
            <button class="btn small" disabled={busy === 'edit'} onclick={() => (editMode = false)}>Cancel</button>
            <button class="btn small primary" disabled={busy === 'edit' || editTitle.trim() === ''} onclick={saveEdit}>
              {busy === 'edit' ? 'Saving…' : 'Save to ' + providerName}
            </button>
          </div>
        {:else}
          <div class="md-body">
            <!-- renderMarkdown escapes input before transforming -->
            {@html renderMarkdown(pr.description_md || '_No description._')}
          </div>
          <button class="btn small ghost edit-btn" onclick={startEdit} title="Edit the title and description on {providerName}">
            <Icon name="edit" size={12} /> Edit
          </button>
        {/if}
      </section>

      {#if pr.reviewers.length > 0}
        <section class="prd-reviewers card">
          <div class="prd-reviewers-title">Reviewers</div>
          {#each pr.reviewers as reviewer (reviewer.name)}
            <div class="reviewer-row">
              {#if reviewer.avatar_url}
                <img class="reviewer-img" src={reviewer.avatar_url} alt={reviewer.name} />
              {:else}
                <span class="reviewer-avatar">{reviewer.name.charAt(0).toUpperCase()}</span>
              {/if}
              <span class="reviewer-name">{reviewer.name}</span>
              {#if reviewer.reviewed_at}
                <span class="reviewer-time dim">{formatRelativeDate(reviewer.reviewed_at)}</span>
              {/if}
              <span class="reviewer-spacer"></span>
              {#if reviewer.approved}
                <span class="chip ok"><Icon name="check" size={12} /> Approved</span>
              {/if}
            </div>
          {/each}
        </section>
      {:else if pr.approved_by.length > 0}
        <section class="prd-reviewers card">
          <div class="prd-reviewers-title">Approvals</div>
          {#each pr.approved_by as name (name)}
            <div class="reviewer-row">
              <span class="reviewer-avatar">{name.charAt(0).toUpperCase()}</span>
              <span class="reviewer-name">{name}</span>
              <span class="reviewer-spacer"></span>
              <span class="chip ok"><Icon name="check" size={12} /> Approved</span>
            </div>
          {/each}
        </section>
      {/if}

      {#if pr.state === 'open'}
        <section class="prd-actions card">
          <button class="btn" disabled={busy !== ''} onclick={() => action('approve')}>
            <Icon name="check" size={12} />
            {busy === 'approve' ? 'Approving…' : 'Approve…'}
          </button>
          <button
            class="btn warn"
            disabled={busy !== ''}
            onclick={() => (showRequestChanges = !showRequestChanges)}
          >
            <Icon name="warning" size={12} />
            Request changes
          </button>
          <div class="row merge-group">
            <button class="btn primary" disabled={busy !== ''} onclick={() => (mergeOpen = true)}>
              <Icon name="merge" size={12} />
              Merge
            </button>
          </div>
          <span class="grow"></span>
          <!-- Decline is destructive + outward: kept out of the primary row
               (next to Merge) and behind a ⋯ menu that then confirms. -->
          <button
            class="icon-btn"
            disabled={busy !== ''}
            data-testid="prd-more"
            aria-label="More PR actions"
            title="More PR actions"
            onclick={moreMenu}
            onkeydown={(e) => (e.key === 'Enter' || e.key === ' ') && moreMenu(e)}
          ><Icon name="more" size={14} /></button>
        </section>
        {#if showRequestChanges}
          <section class="prd-request-changes card">
            <div class="section-title" style="margin-bottom: 8px">Request changes</div>
            <textarea
              class="input"
              rows="3"
              bind:value={requestChangesBody}
              disabled={busy === 'request-changes'}
              aria-label="What needs to change"
              placeholder="The retry loop needs a cap before this can merge."
            ></textarea>
            <div class="row prd-compose-foot">
              <span class="hint dim">Posted to {repoLabel} PR #{number} under your account; {pr.author || 'the author'} and the reviewers are notified.</span>
              <button class="btn small ghost" disabled={busy === 'request-changes'} onclick={() => (showRequestChanges = false)}>Cancel</button>
              <button
                class="btn small warn"
                disabled={busy === 'request-changes'}
                onclick={requestChanges}
              >
                {busy === 'request-changes' ? 'Requesting…' : 'Request changes'}
              </button>
            </div>
          </section>
        {/if}
      {/if}

      <section class="prd-comments">
        <div class="section-title">
          Conversation ({generalComments.length})
        </div>
        {#each generalComments as c (c.id)}
          <div class="card" style="padding: 4px 14px 8px; margin-bottom: 8px">
            <CommentThread comment={c} onreply={(parentId, body) => postComment(body, undefined, undefined, parentId)} onresolve={resolveThread} />
          </div>
        {:else}
          <p class="dim" style="font-size: var(--fs-s)">No comments yet.</p>
        {/each}

        <div class="new-comment card">
          <textarea class="input" rows="3" bind:value={newComment} disabled={busy === 'comment'} aria-label="New comment" placeholder="Leave a comment…" onfocus={scrollIntoViewOnFocus}></textarea>
          <div class="row prd-compose-foot">
            <span class="hint dim">Posted to {repoLabel} PR #{number} under your account; everyone on the pull request sees it.</span>
            <button
              class="btn small"
              disabled={busy === 'comment' || newComment.trim() === ''}
              onclick={addGeneralComment}
            >
              {busy === 'comment' ? 'Posting…' : 'Post comment'}
            </button>
          </div>
        </div>
      </section>
    {/if}

    <!-- Files tab -->
    {#if activeTab === 'files'}
      <section class="prd-diff">
        {#if diffError || !diff}
          <LoadState
            what="the diff"
            loading={diffLoading || !diffError}
            error={diffError}
            empty
            onretry={() => void loadDiff(repoId, number)}
          />
        {:else}
          <DiffViewer
            {diff}
            prMode={true}
            showNav={true}
            comments={inlineComments}
            onAddComment={(path, line, body) => postComment(body, path, line)}
            onReplyComment={(parentId, body) => postComment(body, undefined, undefined, parentId)}
            onResolveComment={resolveThread}
          />
        {/if}
      </section>
    {/if}

    <!-- Commits tab -->
    {#if activeTab === 'commits'}
      <section class="prd-commits">
        <LoadState
          what="commits"
          loading={commitsLoading || (commits === null && !commitsError)}
          error={commitsError}
          empty={!commits || commits.length === 0}
          onretry={() => void loadCommits(repoId, number)}
        >
          {#snippet emptyView()}<p class="dim" style="font-size: var(--fs-s); padding: 12px 0">No commits found.</p>{/snippet}
          {#each commits ?? [] as c (c.sha)}
            <div class="commit-row">
              <span class="commit-sha mono">{c.short_sha}</span>
              <span class="commit-subject">{c.subject}</span>
              <span class="commit-author dim">{c.author}</span>
              <span class="commit-date dim">{formatRelativeDate(c.date)}</span>
            </div>
          {/each}
        </LoadState>
      </section>
    {/if}

    <!-- Review tab -->
    {#if activeTab === 'review'}
      <section class="prd-review">
        <ReviewPanel repoId={repoId} prNumber={number} />
      </section>
    {/if}
  {/if}
</div>
</div>

{#if mergeOpen && pr}
  <PrMergeModal
    {repoId}
    {number}
    {pr}
    onclose={() => {
      mergeOpen = false;
      // The modal's "Open review" link persists the tab and routes here — we
      // are already on this route, so honour the stored choice on close.
      const saved = localStorage.getItem(`otto_pr_tab_${repoId}_${number}`);
      if (saved && TABS.includes(saved as Tab)) activeTab = saved as Tab;
    }}
    onmerged={() => {
      mergeOpen = false;
      void load(repoId, number);
    }}
  />
{/if}

<style>
  .prd-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .prd {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 14px 20px 48px;
  }
  .prd-provider-select {
    height: 28px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-xs);
  }
  .prd-title {
    font-size: var(--fs-xl);
    font-weight: 600;
    margin: 0;
    letter-spacing: -0.01em;
  }
  .prd-title-input {
    width: 100%;
    font-size: var(--fs-l);
    height: 32px;
  }
  .prd-meta {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 6px;
    font-size: var(--fs-s);
  }

  /* Tabs */
  .prd-tabs {
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--border);
    margin: 14px 0 0;
  }
  .tab-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 7px 14px;
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
    transition: color 120ms, border-color 120ms;
    margin-bottom: -1px;
  }
  .tab-btn:hover {
    color: var(--text);
  }
  .tab-btn.active {
    color: var(--accent-text);
    border-bottom-color: var(--accent);
    font-weight: 600;
  }

  /* Summary tab */
  .prd-desc {
    position: relative;
    padding: 12px 16px;
    margin-top: 12px;
  }
  .prd-desc textarea {
    width: 100%;
  }
  .edit-btn {
    position: absolute;
    top: 8px;
    inset-inline-end: 8px;
    opacity: 0;
    transition: opacity 130ms ease-out;
  }
  .prd-desc:hover .edit-btn {
    opacity: 1;
  }
  .prd-reviewers {
    padding: 10px 16px 6px;
    margin-top: 10px;
  }
  .prd-reviewers-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    margin-bottom: 8px;
  }
  .reviewer-row {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 5px 0;
    font-size: var(--fs-s);
  }
  .reviewer-avatar {
    width: 22px;
    height: 22px;
    flex: none;
    border-radius: 50%;
    background: var(--accent);
    color: var(--accent-contrast);
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  /* A long reviewer name truncates instead of pushing the APPROVED chip off. */
  .reviewer-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .reviewer-img {
    width: 22px;
    height: 22px;
    flex: none;
    border-radius: 50%;
    object-fit: cover;
  }
  .reviewer-time {
    font-size: var(--fs-xs);
  }
  .reviewer-spacer {
    flex: 1;
  }
  .prd-actions {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    margin-top: 10px;
  }
  .merge-group {
    gap: 6px;
  }
  .prd-comments {
    margin-top: 16px;
    max-width: 760px;
  }
  .new-comment {
    padding: 12px;
  }
  .new-comment textarea {
    width: 100%;
  }

  /* Files tab */
  .prd-diff {
    margin-top: 12px;
  }

  /* Review tab */
  .prd-review {
    margin-top: 12px;
  }

  /* Request Changes inline panel */
  /* Composer footers: the "who sees it" hint on the leading side, actions at
     the trailing end. */
  .prd-compose-foot {
    justify-content: flex-end;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 8px;
  }
  .prd-compose-foot .hint {
    flex: 1 1 220px;
    min-width: 0;
    font-size: var(--fs-xs);
  }
  .btn.warn {
    background: var(--warning-soft);
    color: var(--warning);
    border-color: color-mix(in srgb, var(--warning) 45%, transparent);
  }
  .btn.warn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--warning) 24%, transparent);
  }
  .prd-request-changes {
    padding: 12px 16px;
    margin-top: 6px;
  }
  .prd-request-changes textarea {
    width: 100%;
  }

  /* Commits tab */
  .prd-commits {
    margin-top: 12px;
  }
  .commit-row {
    display: grid;
    grid-template-columns: 72px 1fr auto auto;
    align-items: center;
    gap: 10px;
    padding: 7px 12px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-s);
  }
  .commit-row:last-child {
    border-bottom: none;
  }
  .commit-sha {
    font-size: var(--fs-xs);
    color: var(--accent-text);
    white-space: nowrap;
  }
  .commit-subject {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .commit-author {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 140px;
    font-size: var(--fs-xs);
  }
  .commit-date {
    white-space: nowrap;
    font-size: var(--fs-xs);
  }

  /* Direction-aware arrow: the source→target separator mirrors in place under
     RTL so it points with the reading direction. */
  .dir-arrow {
    display: inline-block;
  }
  :global([dir='rtl']) .dir-arrow {
    transform: scaleX(-1);
  }

  /* ── Mobile + tablet (≤1024px): wrap dense rows, scroll the tab strip, legible
     text. 1024 so iPad portrait (834) + real-phone landscape (932) also wrap. ── */
  @media (max-width: 1024px) {
    .prd { padding: 12px 12px 48px; }
    /* Header actions wrap instead of overflowing; comfortable touch targets. */
    .prd-title { font-size: var(--fs-xl); overflow-wrap: anywhere; }
    .prd-title-input { height: 38px; font-size: 16px; }
    .prd-meta { flex-wrap: wrap; gap: 8px; font-size: var(--fs-m); min-width: 0; }
    /* Long branch names break instead of forcing horizontal overflow. */
    .prd-meta .mono { overflow-wrap: anywhere; min-width: 0; }

    /* Tab strip scrolls horizontally; bigger touch targets. */
    .prd-tabs {
      overflow-x: auto;
      scrollbar-width: none;
      flex-wrap: nowrap;
    }
    .prd-tabs::-webkit-scrollbar { display: none; }
    .tab-btn {
      font-size: var(--fs-l);
      padding: 10px 14px;
      white-space: nowrap;
      flex-shrink: 0;
    }

    /* Action bar wraps so every button stays reachable, full-height for touch. */
    .prd-actions { flex-wrap: wrap; gap: 8px; }
    .prd-actions .btn { height: 38px; }
    .prd-actions .grow { display: none; }
    .merge-group { flex: 1; gap: 8px; }
    .merge-group .btn { flex: 1; }

    /* Commit rows: drop the rigid 4-col grid for a wrapping two-line layout. */
    .commit-row {
      grid-template-columns: auto 1fr;
      gap: 4px 10px;
      font-size: var(--fs-m);
    }
    .commit-subject { grid-column: 1 / -1; white-space: normal; overflow-wrap: anywhere; }
    .commit-sha { font-size: var(--fs-s); }
    .commit-author, .commit-date { font-size: var(--fs-s); }

    /* Comment composer + edit areas: legible, comfortable, edit always shown. */
    .prd-comments { max-width: 100%; }
    .new-comment textarea,
    .prd-desc textarea,
    .prd-request-changes textarea { font-size: var(--fs-l); }
    .new-comment .btn,
    .prd-request-changes .btn { height: 34px; }
    .edit-btn { opacity: 1; }
    .md-body { font-size: var(--fs-l); overflow-wrap: anywhere; }
  }
</style>
