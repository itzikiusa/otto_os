<script lang="ts">
  // Branch dropdown (switch/create) + push/pull/fetch + ahead/behind.
  import { api } from '../../lib/api/client';
  import type { BranchInfo, RepoStatusResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    repoId: string;
    status: RepoStatusResp;
    onstatus: (s: RepoStatusResp) => void;
  }
  let { repoId, status, onstatus }: Props = $props();

  let open = $state(false);
  let branchList: BranchInfo[] = $state([]);
  let filter = $state('');
  let busy = $state('');

  async function toggle(): Promise<void> {
    open = !open;
    if (open) {
      filter = '';
      try {
        branchList = await api.get<BranchInfo[]>(`/repos/${repoId}/branches`);
      } catch {
        branchList = [];
      }
    }
  }

  const filteredBranches = $derived(
    branchList.filter((b) => b.name.toLowerCase().includes(filter.toLowerCase())),
  );
  const canCreate = $derived(
    filter.trim() !== '' && !branchList.some((b) => b.name === filter.trim()),
  );

  async function checkout(branch: string, create: boolean): Promise<void> {
    busy = 'checkout';
    try {
      const s = await api.post<RepoStatusResp>(`/repos/${repoId}/checkout`, { branch, create });
      onstatus(s);
      open = false;
      toasts.success(create ? 'Branch created' : 'Switched branch', branch);
    } catch (e) {
      toasts.error('Checkout failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function gitOp(op: 'push' | 'pull'): Promise<void> {
    busy = op;
    try {
      const r = await api.post<{ output: string }>(`/repos/${repoId}/${op}`);
      toasts.success(`${op} complete`, r.output.split('\n')[0]);
      const s = await api.get<RepoStatusResp>(`/repos/${repoId}/status`);
      onstatus(s);
    } catch (e) {
      toasts.error(`${op} failed`, e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }
</script>

<div class="branchbar">
  <div class="branch-dd">
    <button class="btn branch-btn" onclick={toggle}>
      <Icon name="branch" size={12} />
      <span class="mono">{status.branch}</span>
      {#if status.ahead > 0}<span class="ab up">↑{status.ahead}</span>{/if}
      {#if status.behind > 0}<span class="ab down">↓{status.behind}</span>{/if}
      <Icon name="chevronDown" size={10} />
    </button>

    {#if open}
      <div class="dd card">
        <input
          class="input dd-filter"
          bind:value={filter}
          placeholder="Switch or create branch…"
          spellcheck="false"
          onkeydown={(e) => {
            if (e.key === 'Enter' && canCreate) void checkout(filter.trim(), true);
            if (e.key === 'Escape') open = false;
          }}
        />
        <div class="dd-list">
          {#each filteredBranches as b (b.name)}
            <button class="dd-item" onclick={() => checkout(b.name, false)} disabled={b.is_current}>
              <span class="mono grow">{b.name}</span>
              {#if b.is_current}<Icon name="check" size={11} />{/if}
              {#if b.upstream}<span class="dim dd-up">{b.upstream}</span>{/if}
            </button>
          {/each}
          {#if canCreate}
            <button class="dd-item create" onclick={() => checkout(filter.trim(), true)}>
              <Icon name="plus" size={11} />
              Create <span class="mono">{filter.trim()}</span>
            </button>
          {/if}
          {#if filteredBranches.length === 0 && !canCreate}
            <div class="dim" style="padding: 8px 10px; font-size: 12px">No branches</div>
          {/if}
        </div>
      </div>
    {/if}
  </div>

  <span class="grow"></span>

  <button class="btn small" disabled={busy !== ''} onclick={() => gitOp('pull')}>
    <Icon name="arrowDown" size={11} />
    {busy === 'pull' ? 'Pulling…' : 'Pull'}
  </button>
  <button class="btn small" disabled={busy !== ''} onclick={() => gitOp('push')}>
    <Icon name="arrowUp" size={11} />
    {busy === 'push' ? 'Pushing…' : 'Push'}
  </button>
</div>

{#if open}
  <!-- click-away -->
  <div class="dd-away" role="presentation" onclick={() => (open = false)}></div>
{/if}

<style>
  .branchbar {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .branch-dd {
    position: relative;
    z-index: 50;
  }
  .branch-btn {
    gap: 7px;
  }
  .ab {
    font-size: 10.5px;
    font-weight: 600;
  }
  .ab.up {
    color: var(--status-working);
  }
  .ab.down {
    color: #febc2e;
  }
  .dd {
    position: absolute;
    top: 30px;
    inset-inline-start: 0;
    width: 280px;
    padding: 8px;
    box-shadow: var(--shadow);
    z-index: 60;
  }
  .dd-filter {
    width: 100%;
    margin-bottom: 6px;
  }
  .dd-list {
    max-height: 240px;
    overflow-y: auto;
  }
  .dd-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 27px;
    padding: 0 8px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    font-size: 12px;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .dd-item:hover:not(:disabled) {
    background: var(--surface-2);
  }
  .dd-item:disabled {
    opacity: 0.7;
    cursor: default;
  }
  .dd-item.create {
    color: var(--accent);
  }
  .dd-up {
    font-size: 10px;
  }
  .dd-away {
    position: fixed;
    inset: 0;
    z-index: 40;
  }

  /* ── Mobile + tablet (≤1024px): keep the branch row on one line, bump the
     pull/push tap targets, and clamp the switch/create dropdown to the viewport
     so its fixed 280px width can't spill off a narrow phone. ── */
  @media (max-width: 1024px) {
    .branchbar {
      flex-wrap: nowrap;
      overflow-x: auto;
      scrollbar-width: none;
    }
    .branchbar::-webkit-scrollbar { display: none; }
    .branch-btn,
    .branchbar :global(.btn.small) {
      min-height: 34px;
      flex-shrink: 0;
    }
    .dd {
      width: min(280px, calc(100vw - 24px));
    }
    .dd-item { height: 34px; font-size: 13px; }
    .dd-filter { height: 34px; font-size: 14px; }
  }
</style>
