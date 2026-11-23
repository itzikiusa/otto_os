<script lang="ts">
  // Right detail pane for the selected container: header with lifecycle
  // controls, shared <Tabs> strip (Overview / Logs / Terminal / Stats), and
  // on-demand secret reveal loading for Overview.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import ContainerOverviewTab from './ContainerOverviewTab.svelte';
  import ContainerLogsTab from './ContainerLogsTab.svelte';
  import ContainerTerminalTab from './ContainerTerminalTab.svelte';
  import ContainerStatsTab from './ContainerStatsTab.svelte';
  import { containersApi } from '../../lib/api/containers';
  import { loadErrorText } from '../../lib/loadError';
  import { toastError } from '../../lib/toastError';
  import { copyWithToast } from '../../lib/clipboard';
  import type { ContainerDetail, ContainerSummary } from '../../lib/api/types/containers';
  import {
    containerHealthInfo,
    containerStateInfo,
    type ContainerDetailTab,
  } from './containers-util';

  interface Props {
    container: ContainerSummary;
    tab: ContainerDetailTab;
    busy: boolean;
    dockerCliAvailable: boolean;
    refreshTick: number;
    ontabchange: (t: ContainerDetailTab) => void;
    onaction: (c: ContainerSummary, action: 'start' | 'stop' | 'restart' | 'pause' | 'unpause' | 'remove') => void;
  }

  let {
    container,
    tab,
    busy,
    dockerCliAvailable,
    refreshTick,
    ontabchange,
    onaction,
  }: Props = $props();

  let detail = $state<ContainerDetail | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let secretsLoaded = $state(false);
  let revealingSecret = $state(false);
  let revealedKeys = $state<Set<string>>(new Set());
  let loadSeq = 0;
  let activeAbort: AbortController | null = null;

  const DETAIL_TABS: readonly TabItem<ContainerDetailTab>[] = [
    { id: 'overview', label: 'Overview', icon: 'info' },
    { id: 'logs', label: 'Logs', icon: 'editorLines' },
    { id: 'terminal', label: 'Terminal', icon: 'terminal' },
    { id: 'stats', label: 'Stats', icon: 'gauge' },
  ];

  async function loadDetail(revealSecrets = false): Promise<void> {
    activeAbort?.abort();
    const ac = new AbortController();
    activeAbort = ac;
    const seq = ++loadSeq;
    const targetId = container.id;
    loading = !detail;
    error = null;
    try {
      const d = revealSecrets
        ? await containersApi.revealSecrets(targetId)
        : await containersApi.inspect(targetId, ac.signal);
      if (seq !== loadSeq || ac.signal.aborted || container.id !== targetId) return;
      detail = d;
      if (revealSecrets) secretsLoaded = true;
    } catch (e) {
      if (seq !== loadSeq || ac.signal.aborted || container.id !== targetId) return;
      error = loadErrorText(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  $effect(() => {
    void container.id;
    void refreshTick;
    untrack(() => {
      if (detail?.id !== container.id) {
        detail = null;
        secretsLoaded = false;
        revealedKeys = new Set();
      }
      void loadDetail(secretsLoaded);
    });
    return () => {
      activeAbort?.abort();
      activeAbort = null;
    };
  });

  async function toggleSecret(key: string): Promise<void> {
    if (revealedKeys.has(key)) {
      const next = new Set(revealedKeys);
      next.delete(key);
      revealedKeys = next;
      return;
    }
    const targetId = container.id;
    const seq = loadSeq;
    if (!secretsLoaded) {
      revealingSecret = true;
      try {
        const d = await containersApi.revealSecrets(targetId);
        if (d.id !== container.id || seq !== loadSeq) return;
        detail = d;
        secretsLoaded = true;
      } catch (e) {
        if (seq === loadSeq && container.id === targetId) {
          toastError('Couldn’t reveal secret value', e);
        }
        return;
      } finally {
        if (seq === loadSeq) revealingSecret = false;
      }
    }
    if (container.id !== targetId || seq !== loadSeq) return;
    const next = new Set(revealedKeys);
    next.add(key);
    revealedKeys = next;
  }

  const stateBadge = $derived(containerStateInfo(detail?.state ?? container.state));
  const healthBadge = $derived(containerHealthInfo(detail?.health ?? container.health));
  const isRunning = $derived((detail?.state ?? container.state).toLowerCase() === 'running');
  const isPaused = $derived((detail?.state ?? container.state).toLowerCase() === 'paused');
</script>

<section class="ct-detail" aria-label={`Container ${container.name}`} data-testid="container-detail">
  <div class="ct-detail-head">
    <div class="ct-detail-title-wrap">
      <div class="ct-detail-title-row">
        <h2 class="ct-detail-name" title={container.name}>{container.name}</h2>
        <Badge tone={stateBadge.tone} label={stateBadge.label} dot live={stateBadge.live} />
        {#if healthBadge}
          <Badge tone={healthBadge.tone} label={healthBadge.label} variant="outline" />
        {/if}
      </div>
      <div class="ct-detail-sub">
        <button
          type="button"
          class="ct-id-btn"
          dir="ltr"
          title="Copy container ID"
          onclick={() => void copyWithToast(container.id, 'container ID')}
        >
          {container.short_id}
          <Icon name="copy" size={12} />
        </button>
        <span class="ct-sub-sep" aria-hidden="true">·</span>
        <span class="ct-sub-image" dir="ltr" title={container.image}>{container.image}</span>
        {#if container.compose_project}
          <span class="ct-sub-sep" aria-hidden="true">·</span>
          <span>Stack {container.compose_project}</span>
        {/if}
      </div>
    </div>

    <div class="ct-detail-actions">
      {#if isPaused}
        <button type="button" class="btn small" disabled={busy} onclick={() => onaction(container, 'unpause')}>
          <Icon name="play" size={12} /> Resume
        </button>
      {:else if isRunning}
        <button type="button" class="btn small" disabled={busy} onclick={() => onaction(container, 'restart')}>
          <Icon name="refresh" size={12} /> Restart
        </button>
        <button
          type="button"
          class="icon-btn"
          disabled={busy}
          aria-label="Pause container"
          title="Pause container"
          onclick={() => onaction(container, 'pause')}
        >
          <Icon name="pause" size={13} />
        </button>
        <button type="button" class="btn small" disabled={busy} onclick={() => onaction(container, 'stop')}>
          <Icon name="stop" size={12} /> Stop
        </button>
      {:else}
        <button type="button" class="btn small" disabled={busy} onclick={() => onaction(container, 'start')}>
          <Icon name="play" size={12} /> Start
        </button>
      {/if}
      <button
        type="button"
        class="icon-btn ct-danger-icon"
        disabled={busy}
        aria-label="Remove container"
        title="Remove container"
        onclick={() => onaction(container, 'remove')}
      >
        <Icon name="trash" size={14} />
      </button>
    </div>
  </div>

  <Tabs
    label="Container details"
    idBase="ct-detail"
    tabs={DETAIL_TABS}
    value={tab}
    onchange={ontabchange}
  />

  <div
    class="ct-detail-panel"
    role="tabpanel"
    id="ct-detail-panel-{tab}"
    aria-labelledby="ct-detail-tab-{tab}"
  >
    {#if tab === 'overview'}
      <ContainerOverviewTab
        {container}
        {detail}
        {loading}
        {error}
        {revealingSecret}
        {revealedKeys}
        onretry={() => void loadDetail(secretsLoaded)}
        ontogglesecret={(key) => void toggleSecret(key)}
        onhideallsecrets={() => (revealedKeys = new Set())}
      />
    {:else if tab === 'logs'}
      <ContainerLogsTab containerId={container.id} containerName={container.name} />
    {:else if tab === 'terminal'}
      <ContainerTerminalTab
        containerId={container.id}
        containerName={container.name}
        running={isRunning}
        {dockerCliAvailable}
      />
    {:else if tab === 'stats'}
      <ContainerStatsTab
        containerId={container.id}
        containerName={container.name}
        running={isRunning}
      />
    {/if}
  </div>
</section>

<style>
  .ct-detail {
    flex: 1;
    min-inline-size: 0;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .ct-detail-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 12px 16px 10px;
    flex-shrink: 0;
  }
  .ct-detail-title-wrap {
    min-inline-size: 0;
    flex: 1 1 240px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ct-detail-title-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    min-inline-size: 0;
  }
  .ct-detail-name {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
    letter-spacing: -0.01em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ct-detail-sub {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ct-id-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0 4px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .ct-id-btn:hover {
    color: var(--text);
    background: var(--hover);
  }
  .ct-sub-image {
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-inline-size: 320px;
  }
  .ct-sub-sep {
    opacity: 0.6;
  }
  .ct-detail-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .ct-danger-icon:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .ct-detail-panel {
    flex: 1;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
</style>
