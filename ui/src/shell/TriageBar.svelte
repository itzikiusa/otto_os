<script lang="ts">
  // Global `⌘⇧A` Cross-Workspace Triage Bar: keyboard-first inbox over every
  // pending MCP approval and waiting agent session/thread across all workspaces.
  import { untrack } from 'svelte';
  import { announceShown } from '../lib/announce.svelte';
  import ApprovalActions from '../lib/components/ApprovalActions.svelte';
  import Badge from '../lib/components/Badge.svelte';
  import DenySheet from '../lib/components/DenySheet.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import Icon from '../lib/components/Icon.svelte';
  import LoadState from '../lib/components/LoadState.svelte';
  import Modal from '../lib/components/Modal.svelte';
  import ProviderIcon from '../lib/components/ProviderIcon.svelte';
  import { liveQuery, EVENT_COMPLETE_SAFETY_MS } from '../lib/live';
  import { plural } from '../lib/plural';
  import { rel } from '../lib/stores/now.svelte';
  import type { McpApproval } from '../lib/api/types';
  import { triage } from './triage.svelte';
  import { triageKeyIntent, type TriageItem } from './triageModel';

  let selected = $state(0);
  let denyingApproval = $state<McpApproval | null>(null);
  let listEl = $state<HTMLUListElement | null>(null);

  const items = $derived(triage.items);
  const approvalCount = $derived(items.filter((i) => i.kind === 'approval').length);
  const waitingCount = $derived(items.length - approvalCount);

  $effect(() => {
    if (!triage.open) return;
    untrack(() => {
      selected = 0;
    });
    const q = liveQuery({
      run: () => triage.refresh(),
      on: ['mcp_approval_changed'],
      fallbackMs: 15_000,
      safetyMs: EVENT_COMPLETE_SAFETY_MS,
    });
    window.addEventListener('keydown', onKeydown);
    return () => {
      q.stop();
      window.removeEventListener('keydown', onKeydown);
      denyingApproval = null;
    };
  });

  $effect(() => {
    if (selected >= items.length) selected = Math.max(0, items.length - 1);
  });

  $effect(() => {
    const id = `triage-item-${selected}`;
    listEl?.querySelector(`#${id}`)?.scrollIntoView({ block: 'nearest' });
  });

  function onKeydown(e: KeyboardEvent): void {
    if (denyingApproval) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest('input, textarea, select, [contenteditable]')) return;
    const intent = triageKeyIntent(e);
    if (!intent) return;
    e.preventDefault();
    if (intent === 'refresh') {
      void triage.refresh();
      return;
    }
    if (items.length === 0) return;
    if (intent === 'next') {
      selected = Math.min(items.length - 1, selected + 1);
      return;
    }
    if (intent === 'prev') {
      selected = Math.max(0, selected - 1);
      return;
    }
    const cur: TriageItem | undefined = items[selected];
    if (!cur) return;
    if (intent === 'open') {
      void triage.jumpTo(cur);
      return;
    }
    if (cur.kind === 'approval' && cur.approval && triage.canDecide(cur.approval)) {
      if (intent === 'approve') void triage.decide(cur.approval, true);
      else if (intent === 'deny') denyingApproval = cur.approval;
    }
  }
</script>

{#if triage.open}
<Modal
  title="Cross-workspace triage"
  width={680}
  onclose={() => triage.close()}
>
  <div class="triage" role="region" aria-label="Pending items across workspaces">
    <div class="triage-bar">
      <div class="triage-counts">
        {#if approvalCount > 0}
          <Badge tone="warn" label={plural(approvalCount, 'approval')} />
        {/if}
        {#if waitingCount > 0}
          <Badge tone="accent" label={`${waitingCount} waiting`} />
        {/if}
        {#if items.length === 0 && !triage.loading}
          <span class="triage-quiet">All clear across workspaces</span>
        {/if}
      </div>
      <div class="triage-tools">
        <span class="triage-keys" aria-hidden="true">
          <kbd>J</kbd>/<kbd>K</kbd> move · <kbd>A</kbd> approve · <kbd>D</kbd> deny · <kbd>⏎</kbd> open
        </span>
        <button
          type="button"
          class="btn small"
          disabled={triage.loading}
          aria-label="Refresh triage queue"
          title="Refresh (R)"
          onclick={() => void triage.refresh()}
        >
          <Icon name="refresh" size={12} />
          {triage.loading ? 'Refreshing…' : 'Refresh'}
        </button>
      </div>
    </div>

    <LoadState
      what="triage items"
      variant="compact"
      loading={triage.loading && items.length === 0}
      error={triage.error}
      empty={items.length === 0}
      onretry={() => void triage.refresh()}
    >
      {#snippet emptyView()}
        <EmptyState
          icon="check"
          title="Nothing needs you"
          body="Pending MCP approvals and waiting sessions across all your workspaces appear here."
        />
      {/snippet}

      <ul class="triage-list" bind:this={listEl} role="listbox" aria-label="Items needing attention">
        {#each items as item, idx (item.id)}
          {@const active = idx === selected}
          <li
            id="triage-item-{idx}"
            class="triage-row"
            class:selected={active}
            role="option"
            aria-selected={active}
            onmouseenter={() => (selected = idx)}
          >
            <div class="triage-main">
              <button
                type="button"
                class="triage-jump"
                onclick={() => void triage.jumpTo(item)}
                title="Jump to {item.title}"
              >
                <span class="triage-icon">
                  {#if item.kind === 'approval'}
                    <Icon name="shield" size={14} />
                  {:else if item.provider}
                    <ProviderIcon provider={item.provider} size={14} />
                  {:else}
                    <Icon name={item.kind === 'thread' ? 'threads' : 'terminal'} size={14} />
                  {/if}
                </span>
                <span class="triage-copy">
                  <span class="triage-title">{item.title}</span>
                  <span class="triage-sub">
                    {item.detail}{#if item.at && rel(item.at)}{` · ${rel(item.at)}`}{/if}
                  </span>
                </span>
              </button>
              <div class="triage-meta">
                <Badge tone="neutral" label={item.workspaceName} />
                {#if item.kind !== 'approval'}
                  <button
                    type="button"
                    class="btn small"
                    onclick={() => void triage.jumpTo(item)}
                  >
                    Open
                  </button>
                {/if}
              </div>
            </div>

            {#if item.approval}
              {@const appr = item.approval}
              {@const err = triage.decideErrors[appr.id]}
              <div class="triage-actions">
                <ApprovalActions
                  size="small"
                  busy={triage.deciding[appr.id] ?? null}
                  disabled={!triage.canDecide(appr)}
                  disabledReason={triage.blockedReason(appr)}
                  denyTarget={appr.title}
                  denyTitle="Deny approval request"
                  testid={`triage-approval-${appr.id}`}
                  onapprove={() => void triage.decide(appr, true)}
                  ondeny={(reason) => void triage.decide(appr, false, reason)}
                />
              </div>
              {#if err}
                <p class="triage-err" use:announceShown={{ text: err, assertive: true }}>{err}</p>
              {/if}
            {/if}
          </li>
        {/each}
      </ul>
    </LoadState>
  </div>
</Modal>
{/if}

{#if denyingApproval}
  {@const target = denyingApproval}
  <DenySheet
    action={target.title}
    title="Deny approval request"
    onclose={() => (denyingApproval = null)}
    ondeny={(reason) => {
      denyingApproval = null;
      void triage.decide(target, false, reason);
    }}
  />
{/if}

<style>
  .triage {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 220px;
    max-height: 70vh;
  }
  .triage-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--border);
  }
  .triage-counts,
  .triage-tools {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .triage-quiet {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .triage-keys {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .triage-keys kbd {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    padding: 0 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
  }
  .triage-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
    overflow-y: auto;
    min-height: 0;
  }
  .triage-row {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    transition: border-color var(--dur-fast) ease-out, background var(--dur-fast) ease-out;
  }
  .triage-row.selected {
    border-color: var(--accent);
    background: var(--hover);
  }
  .triage-main {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 10px;
    min-width: 0;
  }
  .triage-jump {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .triage-icon {
    flex: none;
    display: inline-grid;
    place-items: center;
    width: 20px;
    height: 20px;
    color: var(--warning);
  }
  .triage-copy {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .triage-title {
    font-size: var(--fs-m);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .triage-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .triage-meta {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .triage-actions {
    display: flex;
    justify-content: flex-end;
    padding-inline-start: 28px;
  }
  .triage-err {
    margin: 0;
    padding-inline-start: 28px;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
  @media (max-width: 640px) {
    .triage-keys {
      display: none;
    }
  }
</style>
