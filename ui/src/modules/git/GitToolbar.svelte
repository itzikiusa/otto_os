<script lang="ts">
  // Toolbar row: Fetch / Pull / Push / Branch / Stash / Pop + current branch chip.
  import { api } from '../../lib/api/client';
  import type { RepoStatusResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
    /** Called after an op that may have MOVED refs/history (fetch/pull/push/
     *  branch) so the parent can re-mount the graph — otherwise the commit graph
     *  shows stale data until the repo tab is reopened. */
    onrefresh?: () => void;
  }
  let { repoId, status, onstatus, onrefresh }: Props = $props();

  let busy = $state('');
  // Branch creation inline input state
  let branchOpen = $state(false);
  let branchName = $state('');

  function focusOnMount(node: HTMLElement): void {
    node.focus();
  }

  async function doFetch(): Promise<void> {
    busy = 'fetch';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/fetch`);
      onstatus(s);
      onrefresh?.();
      toasts.success('Fetched', `origin`);
    } catch (e) {
      toasts.error('Fetch failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function doPull(): Promise<void> {
    busy = 'pull';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/pull`);
      onstatus(s);
      onrefresh?.();
      toasts.success('Pulled');
    } catch (e) {
      toasts.error('Pull failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function doPush(): Promise<void> {
    busy = 'push';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/push`, {});
      onstatus(s);
      onrefresh?.();
      toasts.success('Pushed');
    } catch (e) {
      toasts.error('Push failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function doCreateBranch(): Promise<void> {
    const name = branchName.trim();
    if (!name) return;
    busy = 'branch';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/checkout`, { branch: name, create: true });
      onstatus(s);
      onrefresh?.();
      toasts.success('Branch created', name);
      branchOpen = false;
      branchName = '';
    } catch (e) {
      toasts.error('Branch failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function doStash(): Promise<void> {
    busy = 'stash';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/stash`, { op: 'save' });
      onstatus(s);
      toasts.success('Stashed');
    } catch (e) {
      toasts.error('Stash failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function doPop(): Promise<void> {
    busy = 'pop';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/stash`, { op: 'pop' });
      onstatus(s);
      toasts.success('Stash popped');
    } catch (e) {
      toasts.error('Pop failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }
</script>

<div class="toolbar">
  <!-- Branch chip -->
  <span class="branch-chip">
    <Icon name="branch" size={12} />
    <span class="mono">{status.branch}</span>
    {#if status.ahead > 0}<span class="ab up">↑{status.ahead}</span>{/if}
    {#if status.behind > 0}<span class="ab down">↓{status.behind}</span>{/if}
  </span>

  <span class="divider"></span>

  <!-- Fetch -->
  <button class="tbtn" disabled={busy !== ''} onclick={doFetch} title="Fetch from remote">
    <Icon name="fetch" size={13} />
    {busy === 'fetch' ? 'Fetching…' : 'Fetch'}
  </button>

  <!-- Pull -->
  <button class="tbtn" disabled={busy !== ''} onclick={doPull} title="Pull from upstream">
    <Icon name="arrowDown" size={13} />
    {busy === 'pull' ? 'Pulling…' : 'Pull'}
  </button>

  <!-- Push -->
  <button class="tbtn" disabled={busy !== ''} onclick={doPush} title="Push to upstream">
    <Icon name="arrowUp" size={13} />
    {busy === 'push' ? 'Pushing…' : 'Push'}
  </button>

  <span class="divider"></span>

  <!-- Branch (create) -->
  <div class="branch-wrap">
    <button
      class="tbtn"
      class:active={branchOpen}
      disabled={busy !== ''}
      onclick={() => { branchOpen = !branchOpen; branchName = ''; }}
      title="Create new branch"
    >
      <Icon name="plus" size={13} />
      Branch
    </button>
    {#if branchOpen}
      <form
        class="branch-form"
        onsubmit={(e) => { e.preventDefault(); void doCreateBranch(); }}
      >
        <input
          class="input branch-input"
          bind:value={branchName}
          placeholder="new-branch-name"
          spellcheck="false"
          use:focusOnMount
          onkeydown={(e) => { if (e.key === 'Escape') { branchOpen = false; branchName = ''; } }}
        />
        <button class="btn small primary" type="submit" disabled={busy === 'branch' || branchName.trim() === ''}>
          {busy === 'branch' ? '…' : 'Create'}
        </button>
        <button class="btn small ghost" type="button" onclick={() => { branchOpen = false; branchName = ''; }}>
          <Icon name="x" size={11} />
        </button>
      </form>
      <!-- click-away -->
      <div class="dd-away" role="presentation" onclick={() => { branchOpen = false; branchName = ''; }}></div>
    {/if}
  </div>

  <span class="divider"></span>

  <!-- Stash -->
  <button class="tbtn" disabled={busy !== ''} onclick={doStash} title="Stash working changes">
    <Icon name="stash" size={13} />
    {busy === 'stash' ? 'Stashing…' : 'Stash'}
  </button>

  <!-- Pop -->
  <button class="tbtn" disabled={busy !== ''} onclick={doPop} title="Pop stash">
    <Icon name="arrowDown" size={13} />
    {busy === 'pop' ? 'Popping…' : 'Pop'}
  </button>
</div>

<style>
  .toolbar {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-wrap: nowrap;
  }
  .branch-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    border-radius: var(--radius-s);
    padding: 2px 8px;
    font-size: 11.5px;
    font-weight: 500;
    flex-shrink: 0;
  }
  .ab {
    font-size: 10px;
    font-weight: 700;
  }
  .ab.up { color: var(--status-working); }
  .ab.down { color: #febc2e; }
  .divider {
    display: inline-block;
    width: 1px;
    height: 16px;
    background: var(--border);
    margin: 0 4px;
    flex-shrink: 0;
  }
  .tbtn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 26px;
    padding: 0 9px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    cursor: pointer;
    white-space: nowrap;
    transition: background 120ms ease-out, color 120ms ease-out;
  }
  .tbtn:hover:not(:disabled) {
    background: var(--surface-2);
    color: var(--text);
  }
  .tbtn.active {
    background: var(--surface-2);
    color: var(--text);
  }
  .tbtn:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .branch-wrap {
    position: relative;
  }
  .branch-form {
    position: absolute;
    top: 32px;
    left: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 6px 8px;
    box-shadow: var(--shadow);
    z-index: 60;
    min-width: 300px;
  }
  .branch-input {
    flex: 1;
    height: 26px;
    font-size: 12px;
  }
  .dd-away {
    position: fixed;
    inset: 0;
    z-index: 50;
  }
</style>
