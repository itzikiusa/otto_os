<script lang="ts">
  // The commit card — ONE component wherever a checkout's work is committed:
  // Git's Changes view, Threads' commit sheet and (later) the Agents details
  // card. Every change is included on open (staged when nothing is yet —
  // uncheck a file to leave it out), the message is drafted from the diff,
  // and ONE primary follows the checkout's state (lib/gitNextStep.ts):
  // Commit and push → Push (↑N) → Create pull request → Open pull request.
  // A clean tree shows just that next step.
  import { untrack } from 'svelte';
  import type { Repo } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { forgeLabel, primaryStep, stateWords } from '../../lib/gitNextStep';
  import { branchPrs } from '../../lib/stores/branchPrs.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import WipPanel from './WipPanel.svelte';
  import { runPush } from './pushFlow';

  // The Create-PR sheet (it confirms where the PR goes) loads on first use.
  const CreatePrLazy = lazyComponent(() => import('./CreatePr.svelte'));

  interface Props {
    repo: Repo;
    /** 'page': Git's Changes view (the first diff opens beside the list).
     *  'sheet': a dialog over another page (Threads). */
    layout?: 'page' | 'sheet';
    /** After a commit (the sheet closes on it). */
    oncommitted?: () => void;
    /** Open the branch's pull request (Git: its PR view; else the PR route). */
    onopenpr?: (n: number) => void;
    /** A clean tree's secondary: back to the history. */
    onhistory?: () => void;
    /** Open the conflict resolver. */
    onresolve?: () => void;
  }
  let { repo, layout = 'page', oncommitted, onopenpr, onhistory, onresolve }: Props = $props();

  const status = $derived(git.statusById[repo.id] ?? null);
  let error = $state<string | null>(null);

  async function load(): Promise<void> {
    error = null;
    try {
      await git.refreshStatus(repo.id);
      error = git.statusErrorById[repo.id] ?? null;
    } catch (e) {
      error = loadErrorText(e);
    }
  }

  // Read the status if nobody has yet.
  $effect(() => {
    if (!status) untrack(() => !error && void load());
  });

  const dirty = $derived((status?.changes.length ?? 0) > 0);
  const forge = $derived(forgeLabel(repo));
  const branch = $derived(status?.branch && status.branch !== '(detached)' ? status.branch : null);
  const pr = $derived(dirty ? null : branchPrs.prFor(repo, branch));
  const next = $derived(
    primaryStep({
      changes: status?.changes.length ?? 0,
      branch,
      upstream: status?.upstream ?? null,
      ahead: status?.ahead ?? 0,
      remote: !!repo.remote_url,
      forge: !!forge,
      prNumber: pr?.number ?? null,
    }),
  );
  const CLEAN_TITLE: Record<string, string> = {
    none: 'Nothing to commit',
    push: 'Ready to push',
    publish: 'Ready to publish',
    'create-pr': 'Pushed — ready for a pull request',
    pr: 'Up to date with its pull request',
    commit: 'Nothing to commit',
  };
  let busy = $state(false);
  let prOpen = $state(false);

  async function runNext(): Promise<void> {
    const s = status;
    if (!s) return;
    if (next.kind === 'push' || next.kind === 'publish') {
      if (!branch) return;
      busy = true;
      try {
        await runPush(repo.id, { branch, upstream: s.upstream }, (st) => git.setStatus(repo.id, st));
      } finally {
        busy = false;
      }
    } else if (next.kind === 'create-pr') {
      prOpen = true;
    } else if (next.kind === 'pr' && pr) {
      onopenpr?.(pr.number);
    }
  }
  $effect(() => {
    if (prOpen && CreatePrLazy.error) {
      toasts.error('Couldn’t open Create pull request', CreatePrLazy.error);
      prOpen = false;
      CreatePrLazy.retry();
    }
  });
</script>

<div class="commit-card-host" class:sheet={layout === 'sheet'} data-testid="commit-card-host">
  {#if status && dirty}
    <WipPanel
      repoId={repo.id}
      {status}
      remote={!!repo.remote_url}
      selectFirst={layout === 'page' && !viewport.isPhone}
      autoDraft
      includeAll
      onstatus={(st) => git.setStatus(repo.id, st)}
      oncommitted={() => oncommitted?.()}
      {onresolve}
    />
  {:else if status && !dirty}
    <!-- A clean tree: just the next step, in words. -->
    <div class="cc-clean">
      <div class="cc-card" data-testid="commit-card-next">
        <div class="cc-text">
          <p class="cc-title">{CLEAN_TITLE[next.kind]}</p>
          <p class="cc-sub">
            {#if branch}<span class="mono" dir="ltr">{branch}</span> · {/if}{stateWords({ changes: 0, ahead: status.ahead, behind: status.behind, upstream: status.upstream, branch })}{#if pr}<span> · pull request #{pr.number} is open</span>{/if}
          </p>
        </div>
        <div class="cc-actions">
          {#if onhistory}<button type="button" class="btn cc-btn" onclick={onhistory}>History</button>{/if}
          {#if next.kind !== 'none'}
            <button
              type="button"
              class="btn primary cc-btn"
              data-testid="commit-card-primary"
              data-kind={next.kind}
              disabled={busy}
              onpointerenter={() => next.kind === 'create-pr' && CreatePrLazy.prefetch()}
              onclick={() => void runNext()}
            >
              <Icon name={next.kind === 'push' || next.kind === 'publish' ? 'arrowUp' : 'pr'} size={14} />
              {busy ? 'Pushing…' : next.label}
            </button>
          {/if}
        </div>
      </div>
    </div>
  {:else}
    <LoadState loading={!error} {error} what="changes" onretry={() => void load()} />
  {/if}
</div>

{#if prOpen && CreatePrLazy.component}
  {@const CreatePr = CreatePrLazy.component}
  <CreatePr
    repoId={repo.id}
    initialSource={branch ?? undefined}
    onclose={() => (prOpen = false)}
    oncreated={(created) => {
      prOpen = false;
      branchPrs.forget(repo, branch);
      onopenpr?.(created.number);
    }}
  />
{/if}

<style>
  .commit-card-host {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .commit-card-host > :global(.wip-panel) {
    flex: 1;
    min-height: 0;
  }
  .cc-clean {
    padding: 12px;
  }
  .cc-card {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 12px;
    max-inline-size: 760px;
    margin-inline: auto;
    padding: 14px 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface-2);
  }
  .cc-text {
    flex: 1;
    min-width: 0;
  }
  .cc-title {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .cc-sub {
    margin: 4px 0 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .cc-actions {
    display: flex;
    gap: 8px;
  }
  @media (max-width: 1024px) {
    .cc-actions {
      flex: 1 1 100%;
    }
    .cc-btn {
      flex: 1;
      min-height: 44px;
      justify-content: center;
    }
  }
</style>
