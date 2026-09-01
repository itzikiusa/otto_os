<script lang="ts">
  // Branch dropdown (switch/create) + push/pull/fetch + ahead/behind.
  import { api, isDirtyGitRefusal } from '../../lib/api/client';
  import type { BranchInfo, PullResp, RepoStatusResp } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
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
      // A dirty tree blocked the switch — offer the daemon's stash → switch →
      // pull → pop gesture instead of dead-ending on the git message.
      if (!create && isDirtyGitRefusal(e)) {
        const ok = await confirmer.ask(
          `Your uncommitted changes are in the way of switching to "${branch}". Stash them, switch (pulling the upstream), then restore them?`,
          { title: 'Stash & switch', confirmLabel: 'Stash & switch' },
        );
        if (ok) {
          try {
            const resp = await api.post<{ status: RepoStatusResp; summary: string }>(
              `/repos/${repoId}/checkout-update`,
              { branch },
            );
            onstatus(resp.status);
            open = false;
            toasts.success(`Switched to ${branch}`, resp.summary);
          } catch (e2) {
            toasts.error('Stash & switch failed', e2 instanceof Error ? e2.message : String(e2));
          }
        }
      } else {
        toasts.error('Checkout failed', e instanceof Error ? e.message : String(e));
      }
    } finally {
      busy = '';
    }
  }

  function reportPull(s: RepoStatusResp, note?: string | null): void {
    onstatus(s);
    // A pull whose merge conflicted comes back 200 with unmerged paths — that
    // is not a failure, it needs the conflict resolver (same wording as the
    // graph toolbar).
    const conflicts = s.changes.filter((c) => c.kind === 'conflicted').length;
    if (conflicts > 0) {
      toasts.warn(
        'Pulled with conflicts',
        note ??
          `${conflicts} file${conflicts === 1 ? '' : 's'} need resolution — open "Resolve conflicts"`,
      );
    } else {
      toasts.success('Pulled', note ?? undefined);
    }
  }

  async function gitOp(op: 'push' | 'pull'): Promise<void> {
    busy = op;
    try {
      if (op === 'pull') {
        // Pull returns { status, note }; push returns the status itself.
        const r = await api.post<PullResp>(`/repos/${repoId}/pull`);
        reportPull(r.status, r.note);
      } else {
        const s = await api.post<RepoStatusResp>(`/repos/${repoId}/push`);
        onstatus(s);
        toasts.success('Pushed');
      }
    } catch (e) {
      // Dirty-tree pull refusal (409) → offer stash → pull → restore.
      if (op === 'pull' && isDirtyGitRefusal(e)) {
        const ok = await confirmer.ask(
          'Your uncommitted changes are in the way of the pull. Stash them, pull, then restore them?',
          { title: 'Stash, pull & restore', confirmLabel: 'Stash & pull' },
        );
        if (ok) {
          try {
            const r = await api.post<PullResp>(`/repos/${repoId}/pull`, { auto_stash: true });
            reportPull(r.status, r.note);
          } catch (e2) {
            toasts.error('Pull failed', e2 instanceof Error ? e2.message : String(e2));
          }
        }
      } else {
        toasts.error(`${op} failed`, e instanceof Error ? e.message : String(e));
      }
    } finally {
      busy = '';
    }
  }
</script>

<div class="branchbar" class:dd-open={open}>
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
    color: var(--status-warn);
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
  /* Long branch names ellipsize instead of widening the row / dropdown. */
  .dd-item .grow {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    /* overflow-x:auto forces overflow-y to compute to auto too, turning the
       ~34px bar into a clip container that swallows the dropdown. While the
       dropdown is open, let the bar overflow so the menu overlays the page. */
    .branchbar.dd-open {
      overflow-x: visible;
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
    .dd-item { height: 40px; font-size: 13px; }
    .dd-filter { height: 40px; font-size: 14px; }
  }
</style>
