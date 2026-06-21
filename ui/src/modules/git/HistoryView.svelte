<script lang="ts">
  // Commit log + click → commit diff.
  import { api } from '../../lib/api/client';
  import type { CommitInfo, DiffResp } from '../../lib/api/types';
  import DiffViewer from './DiffViewer.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    repoId: string;
  }
  let { repoId }: Props = $props();

  let commits: CommitInfo[] = $state([]);
  let loading = $state(true);
  let selected: string | null = $state(null);
  let diff: DiffResp | null = $state(null);
  let diffLoading = $state(false);
  let exhausted = $state(false);

  const PAGE = 50;

  $effect(() => {
    const id = repoId;
    loading = true;
    commits = [];
    selected = null;
    diff = null;
    exhausted = false;
    void api
      .get<CommitInfo[]>(`/repos/${id}/log?limit=${PAGE}&skip=0`)
      .then((c) => {
        commits = c;
        exhausted = c.length < PAGE;
      })
      .catch(() => (commits = []))
      .finally(() => (loading = false));
  });

  async function loadMore(): Promise<void> {
    const more = await api.get<CommitInfo[]>(
      `/repos/${repoId}/log?limit=${PAGE}&skip=${commits.length}`,
    );
    commits = [...commits, ...more];
    if (more.length < PAGE) exhausted = true;
  }

  async function select(sha: string): Promise<void> {
    selected = sha;
    diffLoading = true;
    try {
      diff = await api.get<DiffResp>(`/repos/${repoId}/diff?target=${encodeURIComponent(`commit:${sha}`)}`);
    } catch {
      diff = { files: [] };
    } finally {
      diffLoading = false;
    }
  }

  function fmtDate(iso: string): string {
    return new Date(iso).toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' });
  }

  // ── Mobile (phone) accordion ──────────────────────────────────────────────
  // Stack the commit list over the diff as collapsible, independently-scrollable
  // sections so picking a commit gives the diff the whole screen.
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });
  let secListOpen = $state(true);

  async function selectMobile(sha: string): Promise<void> {
    await select(sha);
    if (isMobile) secListOpen = false;
  }
</script>

<div class="history" class:mobile={isMobile}>
  {#if isMobile}
    <button
      class="mob-sec-head"
      onclick={() => (secListOpen = !secListOpen)}
      aria-expanded={secListOpen}
    >
      <Icon name={secListOpen ? 'chevronDown' : 'chevronRight'} size={13} />
      <Icon name="commit" size={13} />
      <span>Commits</span>
      <span class="grow"></span>
      {#if !loading}<span class="mob-sec-count">{commits.length}</span>{/if}
    </button>
  {/if}
  <div class="hist-side" class:mob-collapsed={isMobile && !secListOpen}>
    {#if loading}
      <div style="padding: 10px"><Skeleton rows={8} height={34} /></div>
    {:else}
      {#each commits as c (c.sha)}
        <button class="commit" class:selected={selected === c.sha} onclick={() => selectMobile(c.sha)}>
          <div class="c-subject">{c.subject}</div>
          <div class="c-meta">
            <span class="mono c-sha">{c.short_sha}</span>
            <span class="dim">{c.author}</span>
            <span class="grow"></span>
            <span class="dim">{fmtDate(c.date)}</span>
          </div>
        </button>
      {:else}
        <div class="dim" style="padding: 16px; font-size: 12px">No commits.</div>
      {/each}
      {#if !exhausted && commits.length > 0}
        <button class="btn ghost" style="margin: 8px" onclick={loadMore}>Load more</button>
      {/if}
    {/if}
  </div>

  {#if isMobile && selected !== null}
    <button
      class="mob-sec-head mob-diff-head"
      onclick={() => { secListOpen = true; }}
      aria-expanded="true"
    >
      <Icon name="chevronDown" size={13} />
      <Icon name="file" size={13} />
      <span class="mob-diff-title">Diff</span>
      <span class="grow"></span>
      <span class="mob-back">← Commits</span>
    </button>
  {/if}
  <div class="hist-diff" class:mob-hidden={isMobile && selected === null}>
    {#if selected === null}
      <EmptyState icon="commit" title="Select a commit" body="Pick a commit on the left to see its diff." />
    {:else if diffLoading}
      <Skeleton rows={6} height={28} />
    {:else if diff}
      <DiffViewer {diff} />
    {/if}
  </div>
</div>

<style>
  .history {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .hist-side {
    width: 320px;
    flex-shrink: 0;
    overflow-y: auto;
    border-inline-end: 1px solid var(--border);
    padding: 6px;
    display: flex;
    flex-direction: column;
  }
  .commit {
    text-align: start;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    padding: 7px 9px;
    cursor: pointer;
    color: var(--text);
    transition: background 120ms ease-out;
  }
  .commit:hover {
    background: var(--surface-2);
  }
  .commit.selected {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .c-subject {
    font-size: 12.5px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .c-meta {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 2px;
    font-size: 10.5px;
  }
  .c-sha {
    color: var(--accent);
    font-size: 10.5px;
  }
  .hist-diff {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 12px 14px;
  }

  /* ── Mobile accordion section headers ── */
  .mob-sec-head {
    display: none;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 11px 14px;
    border: none;
    border-bottom: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    text-align: start;
    flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .mob-sec-head:active { background: color-mix(in srgb, var(--accent) 10%, var(--surface-2)); }
  .mob-sec-count {
    font-size: 11px;
    font-weight: 700;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface);
    color: var(--text-dim);
  }
  .mob-diff-head { background: color-mix(in srgb, var(--accent) 12%, var(--surface-2)); }
  .mob-diff-title { font-size: 13px; color: var(--accent); font-weight: 700; }
  .mob-back { font-size: 12px; color: var(--text-dim); }

  /* ── Mobile + tablet (≤1024px): stack commit list over the diff, each collapsible ── */
  @media (max-width: 1024px) {
    .history.mobile { flex-direction: column; overflow-y: auto; -webkit-overflow-scrolling: touch; }
    .mobile .mob-sec-head { display: flex; }
    .mobile .hist-side {
      width: 100%;
      flex: 0 0 auto;
      max-height: 45vh;
      overflow-y: auto;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    .mobile .hist-diff {
      min-width: 0;
      width: 100%;
      flex: 1 1 auto;
      min-height: 50vh;
    }
    .mobile .mob-collapsed,
    .mobile .hist-diff.mob-hidden { display: none !important; }
    /* Bigger touch targets + legible commit rows. */
    .mobile .commit { padding: 10px 10px; }
    .mobile .c-subject { font-size: 14px; }
    .mobile .c-meta { font-size: 12px; }
    .mobile .c-sha { font-size: 12px; }
  }
</style>
