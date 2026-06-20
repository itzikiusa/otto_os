<script lang="ts">
  // PR detail: meta, editable markdown description, diff with inline comment
  // threads, general comments, approve/merge/decline, "open as session".
  // Three tabs: Summary | Files | Review (AI agents).
  import { api } from '../../lib/api/client';
  import type { DiffResp, MergeStrategy, PrComment, PrCommit, PrDetail } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { renderMarkdown } from '../../lib/md';
  import { openExternal } from '../../lib/external';
  import DiffViewer from './DiffViewer.svelte';
  import CommentThread from './CommentThread.svelte';
  import ReviewPanel from './ReviewPanel.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    repoId: string;
    number: number;
  }
  let { repoId, number }: Props = $props();

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
  let mergeStrategy: MergeStrategy = $state('merge');
  let showRequestChanges = $state(false);
  let requestChangesBody = $state('');

  const inlineComments = $derived.by(() => (pr?.comments ?? []).filter((c) => c.path !== null));
  const generalComments = $derived.by(() => (pr?.comments ?? []).filter((c) => c.path === null));

  $effect(() => {
    void load(repoId, number);
  });

  // Lazy-load diff when switching to Files tab
  $effect(() => {
    if (activeTab === 'files' && diff === null && !diffLoading) {
      void loadDiff(repoId, number);
    }
  });

  // Lazy-load commits when switching to Commits tab
  $effect(() => {
    if (activeTab === 'commits' && commits === null && !commitsLoading) {
      void loadCommits(repoId, number);
    }
  });

  async function load(rid: string, num: number): Promise<void> {
    loading = true;
    try {
      pr = await api.get<PrDetail>(`/repos/${rid}/prs/${num}`);
    } catch (e) {
      toasts.error('Could not load PR', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  async function loadDiff(rid: string, num: number): Promise<void> {
    diffLoading = true;
    try {
      diff = await api.get<DiffResp>(`/repos/${rid}/prs/${num}/diff`);
    } catch (e) {
      toasts.error('Could not load diff', e instanceof Error ? e.message : String(e));
    } finally {
      diffLoading = false;
    }
  }

  async function loadCommits(rid: string, num: number): Promise<void> {
    commitsLoading = true;
    try {
      commits = await api.get<PrCommit[]>(`/repos/${rid}/prs/${num}/commits`);
    } catch (e) {
      toasts.error('Could not load commits', e instanceof Error ? e.message : String(e));
    } finally {
      commitsLoading = false;
    }
  }

  async function requestChanges(): Promise<void> {
    busy = 'request-changes';
    try {
      await api.post(`/repos/${repoId}/prs/${number}/request-changes`, {
        body: requestChangesBody.trim() || null,
      });
      toasts.success('Changes requested', `#${number}`);
      showRequestChanges = false;
      requestChangesBody = '';
      await load(repoId, number);
    } catch (e) {
      toasts.error('Request changes failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  function formatRelativeDate(iso: string): string {
    const diff = Date.now() - new Date(iso).getTime();
    const mins = Math.floor(diff / 60000);
    if (mins < 60) return `${mins}m ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24) return `${hours}h ago`;
    const days = Math.floor(hours / 24);
    if (days < 30) return `${days}d ago`;
    return new Date(iso).toLocaleDateString();
  }

  function startEdit(): void {
    if (!pr) return;
    editTitle = pr.title;
    editDesc = pr.description_md;
    editMode = true;
  }

  async function saveEdit(): Promise<void> {
    busy = 'edit';
    try {
      await api.patch(`/repos/${repoId}/prs/${number}`, { title: editTitle, description: editDesc });
      editMode = false;
      await load(repoId, number);
      toasts.success('Pull request updated');
    } catch (e) {
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function postComment(body: string, path?: string, line?: number, inReplyTo?: string): Promise<void> {
    await api.post<PrComment>(`/repos/${repoId}/prs/${number}/comments`, {
      body,
      path: path ?? null,
      line: line ?? null,
      in_reply_to: inReplyTo ?? null,
    });
    await load(repoId, number);
  }

  async function addGeneralComment(): Promise<void> {
    if (newComment.trim() === '') return;
    busy = 'comment';
    try {
      await postComment(newComment.trim());
      newComment = '';
    } catch (e) {
      toasts.error('Comment failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function action(kind: 'approve' | 'merge' | 'decline'): Promise<void> {
    busy = kind;
    try {
      if (kind === 'merge') {
        await api.post(`/repos/${repoId}/prs/${number}/merge`, { strategy: mergeStrategy });
      } else {
        await api.post(`/repos/${repoId}/prs/${number}/${kind}`);
      }
      toasts.success(`PR ${kind === 'approve' ? 'approved' : kind + 'd'}`, `#${number}`);
      await load(repoId, number);
    } catch (e) {
      toasts.error(`${kind} failed`, e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function openAsSession(): Promise<void> {
    if (!pr) return;
    busy = 'session';
    try {
      await ws.createSession({
        kind: 'agent',
        provider: 'claude',
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
      busy = '';
    }
  }
</script>

<div class="prd">
  {#if loading && !pr}
    <div style="padding: 16px"><Skeleton rows={5} height={40} /></div>
  {:else if pr}
    <header class="prd-head">
      <button class="btn ghost small" onclick={() => router.go(`git/${repoId}/prs`)}>
        ← Pull Requests
      </button>
      <span class="grow"></span>
      <button class="btn small" disabled={busy !== ''} onclick={openAsSession}>
        <Icon name="terminal" size={11} /> Open as session
      </button>
      <button class="btn small" onclick={() => openExternal(pr?.url)}>View on provider</button>
    </header>

    <div class="prd-title-block">
      {#if editMode}
        <input class="input prd-title-input" bind:value={editTitle} />
      {:else}
        <h1 class="prd-title">
          <span class="dim">#{pr.number}</span>
          {pr.title}
        </h1>
      {/if}
      <div class="prd-meta">
        <span class="chip {pr.state === 'open' ? 'ok' : pr.state === 'merged' ? 'accent' : 'bad'}">{pr.state}</span>
        <span class="dim">{pr.author}</span>
        <span class="mono dim">{pr.source_branch} → {pr.target_branch}</span>
        {#if pr.mergeable === false}<span class="chip bad">conflicts</span>{/if}
        {#if pr.approved_by.length > 0}
          <span class="chip ok" title={pr.approved_by.join(', ')}>
            ✓ {pr.approved_by.length} approval{pr.approved_by.length === 1 ? '' : 's'}
          </span>
        {/if}
      </div>
    </div>

    <!-- Tab bar -->
    <div class="prd-tabs">
      <button
        class="tab-btn"
        class:active={activeTab === 'summary'}
        onclick={() => selectTab('summary')}
      >
        <Icon name="comment" size={12} /> Summary
      </button>
      <button
        class="tab-btn"
        class:active={activeTab === 'files'}
        onclick={() => selectTab('files')}
      >
        <Icon name="file" size={12} /> Files
      </button>
      <button
        class="tab-btn"
        class:active={activeTab === 'commits'}
        onclick={() => selectTab('commits')}
      >
        <Icon name="git-commit" size={12} /> Commits
      </button>
      <button
        class="tab-btn"
        class:active={activeTab === 'review'}
        onclick={() => selectTab('review')}
      >
        <Icon name="zap" size={12} /> Review
      </button>
    </div>

    <!-- Summary tab -->
    {#if activeTab === 'summary'}
      <section class="prd-desc card">
        {#if editMode}
          <textarea class="input" rows="8" bind:value={editDesc}></textarea>
          <div class="row" style="justify-content: flex-end; margin-top: 8px">
            <button class="btn small" onclick={() => (editMode = false)}>Cancel</button>
            <button class="btn small primary" disabled={busy === 'edit'} onclick={saveEdit}>
              {busy === 'edit' ? 'Saving…' : 'Save'}
            </button>
          </div>
        {:else}
          <div class="md-body">
            <!-- renderMarkdown escapes input before transforming -->
            {@html renderMarkdown(pr.description_md || '_No description._')}
          </div>
          <button class="btn small ghost edit-btn" onclick={startEdit}>
            <Icon name="edit" size={11} /> Edit
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
                <span class="chip ok"><Icon name="check" size={11} /> APPROVED</span>
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
              <span class="chip ok"><Icon name="check" size={11} /> APPROVED</span>
            </div>
          {/each}
        </section>
      {/if}

      {#if pr.state === 'open'}
        <section class="prd-actions card">
          <button class="btn" disabled={busy !== ''} onclick={() => action('approve')}>
            <Icon name="check" size={12} />
            {busy === 'approve' ? 'Approving…' : 'Approve'}
          </button>
          <button
            class="btn warn"
            disabled={busy !== ''}
            onclick={() => (showRequestChanges = !showRequestChanges)}
          >
            <Icon name="alert-triangle" size={12} />
            Request changes
          </button>
          <div class="row merge-group">
            <select class="input" bind:value={mergeStrategy} style="width: 100px">
              <option value="merge">merge</option>
              <option value="squash">squash</option>
              <option value="rebase">rebase</option>
            </select>
            <button class="btn primary" disabled={busy !== ''} onclick={() => action('merge')}>
              <Icon name="merge" size={12} />
              {busy === 'merge' ? 'Merging…' : 'Merge'}
            </button>
          </div>
          <span class="grow"></span>
          <button class="btn danger" disabled={busy !== ''} onclick={() => action('decline')}>
            {busy === 'decline' ? 'Declining…' : 'Decline'}
          </button>
        </section>
        {#if showRequestChanges}
          <section class="prd-request-changes card">
            <div class="section-title" style="margin-bottom: 8px">Request changes</div>
            <textarea
              class="input"
              rows="3"
              bind:value={requestChangesBody}
              placeholder="Optional comment explaining what needs to change…"
            ></textarea>
            <div class="row" style="justify-content: flex-end; margin-top: 8px; gap: 8px">
              <button class="btn small ghost" onclick={() => (showRequestChanges = false)}>Cancel</button>
              <button
                class="btn small warn"
                disabled={busy === 'request-changes'}
                onclick={requestChanges}
              >
                {busy === 'request-changes' ? 'Requesting…' : 'Submit'}
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
            <CommentThread comment={c} onreply={(parentId, body) => postComment(body, undefined, undefined, parentId)} />
          </div>
        {:else}
          <p class="dim" style="font-size: 12px">No comments yet.</p>
        {/each}

        <div class="new-comment card">
          <textarea class="input" rows="3" bind:value={newComment} placeholder="Leave a comment…"></textarea>
          <div class="row" style="justify-content: flex-end; margin-top: 8px">
            <button
              class="btn primary small"
              disabled={busy === 'comment' || newComment.trim() === ''}
              onclick={addGeneralComment}
            >
              {busy === 'comment' ? 'Posting…' : 'Comment'}
            </button>
          </div>
        </div>
      </section>
    {/if}

    <!-- Files tab -->
    {#if activeTab === 'files'}
      <section class="prd-diff">
        {#if diffLoading || (!diff && !diffLoading)}
          {#if diffLoading}
            <Skeleton rows={4} height={30} />
          {:else}
            <p class="dim" style="font-size: 12px; padding: 12px 0">Loading diff…</p>
          {/if}
        {:else if diff}
          <DiffViewer
            {diff}
            prMode={true}
            showNav={true}
            comments={inlineComments}
            onAddComment={(path, line, body) => postComment(body, path, line)}
          />
        {/if}
      </section>
    {/if}

    <!-- Commits tab -->
    {#if activeTab === 'commits'}
      <section class="prd-commits">
        {#if commitsLoading}
          <Skeleton rows={4} height={28} />
        {:else if commits === null}
          <p class="dim" style="font-size: 12px; padding: 12px 0">Loading commits…</p>
        {:else if commits.length === 0}
          <p class="dim" style="font-size: 12px; padding: 12px 0">No commits found.</p>
        {:else}
          {#each commits as c (c.sha)}
            <div class="commit-row">
              <span class="commit-sha mono">{c.short_sha}</span>
              <span class="commit-subject">{c.subject}</span>
              <span class="commit-author dim">{c.author}</span>
              <span class="commit-date dim">{formatRelativeDate(c.date)}</span>
            </div>
          {/each}
        {/if}
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

<style>
  .prd {
    height: 100%;
    overflow-y: auto;
    padding: 12px 18px 48px;
  }
  .prd-head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
  }
  .prd-title {
    font-size: 17px;
    font-weight: 600;
    margin: 0;
    letter-spacing: -0.01em;
  }
  .prd-title-input {
    width: 100%;
    font-size: 15px;
    height: 32px;
  }
  .prd-meta {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-top: 6px;
    font-size: 12px;
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
    font-size: 12.5px;
    color: var(--text-dim);
    cursor: pointer;
    transition: color 120ms, border-color 120ms;
    margin-bottom: -1px;
  }
  .tab-btn:hover {
    color: var(--text);
  }
  .tab-btn.active {
    color: var(--accent);
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
    right: 8px;
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
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim, var(--dim));
    margin-bottom: 8px;
  }
  .reviewer-row {
    display: flex;
    align-items: center;
    gap: 9px;
    padding: 5px 0;
    font-size: 12.5px;
  }
  .reviewer-avatar {
    width: 22px;
    height: 22px;
    flex: none;
    border-radius: 50%;
    background: var(--accent);
    color: #fff;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    font-size: 11px;
    font-weight: 600;
  }
  .reviewer-img {
    width: 22px;
    height: 22px;
    flex: none;
    border-radius: 50%;
    object-fit: cover;
  }
  .reviewer-time {
    font-size: 11px;
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
  .btn.warn {
    background: var(--warn, #92400e);
    color: var(--warn-fg, #fef3c7);
    border-color: transparent;
  }
  .btn.warn:hover:not(:disabled) {
    filter: brightness(1.15);
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
    font-size: 12.5px;
  }
  .commit-row:last-child {
    border-bottom: none;
  }
  .commit-sha {
    font-size: 11.5px;
    color: var(--accent);
    white-space: nowrap;
  }
  .commit-subject {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .commit-author {
    white-space: nowrap;
    font-size: 11.5px;
  }
  .commit-date {
    white-space: nowrap;
    font-size: 11.5px;
  }
</style>
