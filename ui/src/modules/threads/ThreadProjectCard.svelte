<script lang="ts">
  // The right "project card" (T3 Code's floating panel): where the thread runs,
  // how to open it, project scripts, then the git trio — branch, Create PR,
  // Changes +N −M. Floats over the new-thread stage and heads the inspector.
  // Every row is a door into an existing Otto surface (git store, Create-PR
  // sheet, Changes tab); nothing here owns state beyond the diff totals.
  import { api, isAbortError } from '../../lib/api/client';
  import type { DiffResp } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import {
    openBranchMenu,
    openOpenInMenu,
    promptNewScript,
    runInShell,
    showScriptMenu,
  } from './threadGitMenus';
  import { compactPath, forgeLabel } from './threadsHelpers';
  import { threadRepos } from './threadRepos.svelte';
  import { threads } from './threadsStore.svelte';

  // The Create-PR sheet (outward confirm lives there) loads on first open, not with the page.
  const CreatePrLazy = lazyComponent(() => import('../git/CreatePr.svelte')); // ui-guards: allow — Threads reuses the one Create-PR sheet

  interface Props {
    cwd: string;
    /** Floating card over the stage (shadowed) vs. flat inside the inspector. */
    floating?: boolean;
    /** When set, the folder row opens this (the stage's project menu). */
    onpickfolder?: (anchor: HTMLElement) => void;
  }
  let { cwd, floating = false, onpickfolder }: Props = $props();

  let openInEl = $state<HTMLButtonElement | null>(null);
  let branchEl = $state<HTMLButtonElement | null>(null);
  let prOpen = $state(false);
  let totals = $state<{ added: number; deleted: number } | null>(null);

  const homePath = $derived(ws.scratch?.root_path ?? null);
  const project = $derived(cwd.replace(/\/+$/, ''));
  const repo = $derived(threadRepos.repoFor(project));
  const repoChecked = $derived(threadRepos.checked(project));
  const status = $derived(repo ? (git.statusById[repo.id] ?? null) : null);
  const branch = $derived(status?.branch ?? null);
  const forge = $derived(forgeLabel(repo));
  const scripts = $derived(threads.scriptsFor(project));

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

  // Compact past 10k (T3: "1m", not "1000k") so huge diffs keep the row calm.
  const compactFmt = new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 });
  // A sheet that failed to load says so instead of silently not opening.
  $effect(() => {
    if (prOpen && CreatePrLazy.error) {
      toasts.error('Couldn’t open Create PR', CreatePrLazy.error);
      prOpen = false;
      CreatePrLazy.retry();
    }
  });

  const fmt = (n: number): string => (n >= 10_000 ? compactFmt.format(n) : n.toLocaleString());
</script>

<section class="pc-card" class:floating aria-label="Project" data-testid="threads-project-card">
  <div class="pc-group">
    {#if onpickfolder}
      <button
        type="button"
        class="pc-row"
        aria-haspopup="menu"
        title={project || 'Choose a project folder'}
        data-testid="project-card-folder"
        onclick={(e) => onpickfolder?.(e.currentTarget)}
      >
        <Icon name="folder" size={14} />
        <span class="pc-label mono ellipsis" dir="ltr">{compactPath(project, homePath)}</span>
        <Icon name="chevronDown" size={12} />
      </button>
    {:else}
      <div class="pc-row pc-static" title={project}>
        <Icon name="folder" size={14} />
        <span class="pc-label mono ellipsis" dir="ltr">{compactPath(project, homePath)}</span>
      </div>
    {/if}
    <button
      bind:this={openInEl}
      type="button"
      class="pc-row"
      aria-haspopup="menu"
      data-testid="project-card-open-in"
      onclick={() => openInEl && openOpenInMenu(openInEl, { cwd: project, repo })}
    >
      <Icon name="external" size={14} />
      <span class="pc-label">Open in…</span>
      <Icon name="chevronDown" size={12} />
    </button>
    {#each scripts as s (s.name)}
      <button
        type="button"
        class="pc-row"
        title="Run “{s.command}” in a terminal thread (right-click for more)"
        oncontextmenu={(e) => showScriptMenu(e, project, s)}
        onclick={() => void runInShell(project, s)}
      >
        <Icon name="play" size={14} />
        <span class="pc-label mono ellipsis" dir="ltr">{s.name}</span>
      </button>
    {/each}
    <button
      type="button"
      class="pc-row pc-dim"
      disabled={!project}
      data-testid="project-card-add-script"
      onclick={() => void promptNewScript(project)}
    >
      <Icon name="plus" size={14} />
      <span class="pc-label">Add project script</span>
    </button>
  </div>

  <div class="pc-group">
    {#if repo}
      <button
        bind:this={branchEl}
        type="button"
        class="pc-row"
        aria-haspopup="menu"
        title="Switch branch"
        data-testid="project-card-branch"
        onclick={() => branchEl && void openBranchMenu(branchEl, repo)}
      >
        <Icon name="branch" size={14} />
        <span class="pc-label mono ellipsis" dir="ltr">{branch ?? 'Detached HEAD'}</span>
        <Icon name="chevronDown" size={12} />
      </button>
      <button
        type="button"
        class="pc-row"
        title={forge ? `Open a pull request on ${forge} — you review it before it is sent` : 'This repo has no supported remote'}
        disabled={!forge}
        data-testid="project-card-create-pr"
        onpointerenter={() => CreatePrLazy.prefetch()}
        onclick={() => (prOpen = true)}
      >
        <Icon name="pr" size={14} />
        <span class="pc-label">Create PR</span>
        {#if forge}<span class="pc-hint">{forge}</span>{/if}
      </button>
      <button
        type="button"
        class="pc-row"
        title="Review the working-tree changes"
        data-testid="project-card-changes"
        onclick={() => threads.setInspectorTab('changes')}
      >
        <Icon name="file" size={14} />
        <span class="pc-label">Changes</span>
        {#if totals}
          <span class="pc-stat mono" dir="ltr" title="{totals.added.toLocaleString()} added, {totals.deleted.toLocaleString()} deleted">
            <span class="pc-add">+{fmt(totals.added)}</span>
            <span class="pc-del">−{fmt(totals.deleted)}</span>
          </span>
        {/if}
      </button>
    {:else}
      <p class="pc-note">{!project ? 'Pick a project folder' : repoChecked ? 'Not a git repository' : 'Looking for a git repository…'}</p>
    {/if}
  </div>
</section>

{#if prOpen && repo && CreatePrLazy.component}
  {@const CreatePr = CreatePrLazy.component}
  <CreatePr
    repoId={repo.id}
    initialSource={branch ?? undefined}
    onclose={() => (prOpen = false)}
    oncreated={(pr) => {
      prOpen = false;
      router.go(`git/${encodeURIComponent(repo.id)}/pr/${pr.number}`);
    }}
  />
{/if}

<style>
  .pc-card {
    display: flex;
    flex-direction: column;
    min-width: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    overflow: hidden;
  }
  .pc-card.floating {
    box-shadow: var(--shadow-card);
  }
  .pc-group {
    display: flex;
    flex-direction: column;
    padding: 4px;
  }
  .pc-group + .pc-group {
    border-block-start: 1px solid var(--border);
  }
  .pc-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
    min-block-size: 32px;
    padding-inline: 10px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .pc-row :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .pc-row:hover:not(:disabled) {
    background: var(--hover);
  }
  .pc-row:disabled {
    cursor: default;
    opacity: var(--disabled-opacity);
  }
  .pc-static {
    cursor: default;
  }
  .pc-dim {
    color: var(--text-dim);
  }
  .pc-label {
    flex: 1;
    min-width: 0;
  }
  .pc-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .pc-stat {
    display: inline-flex;
    gap: 6px;
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .pc-add {
    color: var(--success);
  }
  .pc-del {
    color: var(--danger);
  }
  .pc-note {
    margin: 0;
    padding: 8px 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
