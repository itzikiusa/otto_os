<script lang="ts">
  // Containers module: native Docker container management inside Otto backed
  // by `otto-containers` (`/api/v1/containers/*`).
  //
  // Single `PageHeader` with engine/context status pill, inline segmented tabs
  // (Containers · Images · Volumes · Networks), one `.primary` Run… action,
  // and secondary actions (Switch Context, Pull image, System Prune) in `⋯`.
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import ContainersList from './ContainersList.svelte';
  import ContainerDetail from './ContainerDetail.svelte';
  import ImagesTab from './ImagesTab.svelte';
  import VolumesTab from './VolumesTab.svelte';
  import NetworksTab from './NetworksTab.svelte';
  import { ContainersController } from './containersStore.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { buildContainerCommands } from '../../lib/uiCommands/containers';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { formatBytes } from '../../lib/metric-format';
  import { buildOfflineDetail, engineKindLabel, runtimeStateInfo } from './containers-util';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import type { RunContainerResp } from '../../lib/api/types/containers';

  // Loaded on first open: the dialog carries the image-hints table.
  const RunContainerModal = lazyComponent(() => import('./RunContainerModal.svelte'));

  const ctrl = new ContainersController();

  let runModalOpen = $state(false);
  let runInitialImage = $state('');

  const selectedContainer = $derived(ctrl.selectedContainer);
  const reachableFallback = $derived(ctrl.reachableFallback);
  const emptyContainersPage = $derived(
    !!ctrl.status?.available &&
      ctrl.activeTab === 'containers' &&
      !ctrl.containersLoading &&
      !ctrl.containersError &&
      ctrl.containers.length === 0,
  );
  const showPhoneDetail = $derived(
    viewport.isPhone && ctrl.activeTab === 'containers' && ctrl.phoneDetailOpen && !!selectedContainer,
  );

  function openRunModal(initialImage = ''): void {
    runInitialImage = initialImage;
    runModalOpen = true;
  }

  function closeRunModal(): void {
    runModalOpen = false;
    runInitialImage = '';
  }

  function onRunCreated(resp: RunContainerResp): void {
    closeRunModal();
    ctrl.activeTab = 'containers';
    ctrl.selectContainer(resp.id, viewport.isPhone);
    void Promise.all([ctrl.loadStatusAndContexts(), ctrl.loadContainers()]);
  }

  function moreItems(): MenuItem[] {
    const items: MenuItem[] = [];
    const rt = ctrl.runtime;
    if (rt?.installed) {
      const rtState = runtimeStateInfo(rt);
      const rtItems: MenuItem[] = [];
      if (!rt.running && rt.can_start) {
        rtItems.push({
          label: ctrl.runtimeBusy === 'start' ? 'Starting Kiri runtime…' : 'Start Kiri runtime',
          icon: 'play',
          disabled: ctrl.runtimeBusy !== null,
          hint: rtState.label,
          action: () => void ctrl.startRuntime(),
        });
      }
      if (rt.running || rt.daemon_reachable) {
        rtItems.push({
          label: ctrl.runtimeBusy === 'restart' ? 'Restarting Kiri runtime…' : 'Restart Kiri runtime',
          icon: 'refresh',
          disabled: ctrl.runtimeBusy !== null,
          action: () => void ctrl.restartRuntime(),
        });
      }
      if (rt.can_stop && (rt.running || rt.daemon_reachable)) {
        rtItems.push({
          label: 'Stop Kiri runtime…',
          icon: 'stop',
          danger: true,
          disabled: ctrl.runtimeBusy !== null,
          action: () => void ctrl.stopRuntime(),
        });
      }
      if (rtItems.length > 0) {
        items.push({
          label: 'Kiri runtime',
          icon: 'zap',
          hint: rtState.label,
          children: rtItems,
        });
      }
    } else if (rt?.can_install) {
      items.push({
        label: isInstallingKiri ? 'Installing Kiri runtime…' : 'Install Kiri runtime…',
        icon: 'download',
        disabled: ctrl.runtimeBusy !== null || isInstallingKiri,
        action: () => void ctrl.installRuntime(true),
      });
    }
    if (ctrl.contexts.length > 0) {
      items.push({
        label: 'Switch context',
        icon: 'server',
        children: ctrl.contexts.map((ctx) => ({
          label: ctx.name,
          checked: ctx.is_active,
          hint: ctx.reachable ? engineKindLabel(ctx.engine_kind) : 'offline',
          title: ctx.socket_path,
          action: () => void ctrl.handleSelectContext(ctx),
        })),
      });
    }
    if (ctrl.status?.available) {
      if (items.length > 0) items.push({ separator: true });
      items.push(
        { label: 'Pull image…', icon: 'download', action: () => void ctrl.pullImagePrompt() },
        { separator: true },
        { label: 'System prune…', icon: 'trash', danger: true, action: () => void ctrl.handleSystemPrune() },
      );
    }
    return items;
  }

  $effect(() => {
    untrack(() => {
      void ctrl.refreshActiveTab();
    });
  });

  $effect(() =>
    registry.register(
      'containers',
      buildContainerCommands({
        containers: ctrl.containers,
        contexts: ctrl.contexts,
        runtime: ctrl.runtime,
        available: !!ctrl.status?.available,
        openTab: (tab) => ctrl.switchTab(tab),
        selectContainer: (id, dt) => {
          ctrl.activeTab = 'containers';
          ctrl.selectContainer(id, viewport.isPhone, dt);
        },
        openRunModal: () => openRunModal(),
        pullImage: () => void ctrl.pullImagePrompt(),
        refresh: () => void ctrl.refreshActiveTab(),
        systemPrune: () => void ctrl.handleSystemPrune(),
        selectContext: (ctx) => void ctrl.handleSelectContext(ctx),
        startRuntime: () => void ctrl.startRuntime(),
        stopRuntime: () => void ctrl.stopRuntime(),
        restartRuntime: () => void ctrl.restartRuntime(),
      }),
    ),
  );

  const engineLabel = $derived(ctrl.status ? engineKindLabel(ctrl.status.engine_kind) : 'Docker');
  const canStartKiri = $derived(!!ctrl.runtime?.installed && ctrl.runtime.can_start && !ctrl.runtime.running);
  const canInstallKiri = $derived(!!ctrl.runtime && !ctrl.runtime.installed && !!ctrl.runtime.can_install);
  const isInstallingKiri = $derived(ctrl.runtimeBusy === 'install' || !!ctrl.runtime?.installing);
  const hasOfflinePrimaryKiri = $derived(canStartKiri || canInstallKiri || isInstallingKiri);
  const vmAsleep = $derived(ctrl.status?.vm?.state?.toLowerCase() === 'asleep');
  const badgeLabel = $derived.by(() => {
    const st = ctrl.status;
    if (!st) return '';
    if (vmAsleep) return `${engineLabel} · Asleep`;
    return `${engineLabel} · ${st.containers_running} running`;
  });
  const badgeTitle = $derived.by(() => {
    const st = ctrl.status;
    if (!st) return '';
    const parts = [`Context ${st.active_context} (${st.socket_path})`];
    if (st.engine_version) parts.push(`v${st.engine_version}`);
    if (st.vm?.memory_used_bytes != null) {
      const used = formatBytes(st.vm.memory_used_bytes);
      const ceil = st.vm.memory_ceiling_bytes != null ? ` / ${formatBytes(st.vm.memory_ceiling_bytes)}` : '';
      parts.push(`VM ${used}${ceil}`);
    }
    return parts.join(' · ');
  });
  const offlineDetail = $derived(
    ctrl.status ? buildOfflineDetail(ctrl.status, ctrl.runtime, ctrl.runtimeError) : '',
  );
</script>

<div class="ct-page" data-testid="containers-page">
  {#if ctrl.unavailable}
    <PageHeader title="Containers" />
    <EmptyState
      variant="page"
      icon="box"
      title="Containers console isn’t available"
      body="This daemon doesn’t serve /containers/* yet. Update Otto (or restart the daemon after upgrading) and reopen this page."
    />
  {:else if !ctrl.status && ctrl.statusError}
    <PageHeader title="Containers" />
    <LoadState
      variant="page"
      what="containers"
      error={ctrl.statusError}
      empty
      onretry={() => void ctrl.refreshActiveTab()}
    />
  {:else if !ctrl.status}
    <PageHeader title="Containers" />
    <div class="ct-boot">
      <Skeleton rows={5} height={44} />
    </div>
  {:else if !ctrl.status.available}
    <PageHeader
      title="Containers"
      more={ctrl.contexts.length > 0 || ctrl.runtime?.installed || ctrl.runtime?.can_install ? moreItems : undefined}
    >
      {#snippet badge()}
        <Badge tone="warn" label="Offline" dot title={ctrl.status?.socket_path} />
      {/snippet}
    </PageHeader>
    <LoadState
      variant="page"
      what="containers"
      errorTitle={`${engineLabel} isn’t running`}
      error={offlineDetail}
      empty
      onretry={() => void ctrl.refreshActiveTab()}
    >
      {#snippet errorActions()}
        {#if canStartKiri}
          <button
            type="button"
            class="btn primary"
            disabled={ctrl.runtimeBusy !== null}
            onclick={() => void ctrl.startRuntime()}
          >
            <Icon name="play" size={13} />
            {ctrl.runtimeBusy === 'start' ? 'Starting Kiri…' : 'Start Kiri runtime'}
          </button>
        {:else if canInstallKiri || isInstallingKiri}
          <button
            type="button"
            class="btn primary"
            disabled={ctrl.runtimeBusy !== null || isInstallingKiri}
            onclick={() => void ctrl.installRuntime(true)}
          >
            <Icon name="download" size={13} />
            {isInstallingKiri ? (ctrl.runtime?.install_progress || 'Installing Kiri…') : 'Install & Start Kiri'}
          </button>
        {/if}
        {#if reachableFallback}
          <button
            type="button"
            class="btn"
            class:primary={!hasOfflinePrimaryKiri}
            disabled={ctrl.switchingContext || ctrl.runtimeBusy !== null}
            onclick={() => void ctrl.handleSelectContext(reachableFallback)}
          >
            <Icon name="server" size={13} />
            Switch to {reachableFallback.name}
          </button>
        {/if}
      {/snippet}
    </LoadState>
  {:else}
    <PageHeader
      title={showPhoneDetail && selectedContainer ? selectedContainer.name : 'Containers'}
      subtitle={showPhoneDetail ? undefined : ctrl.status.active_context}
      more={moreItems}
    >
      {#snippet leading()}
        {#if showPhoneDetail}
          <button
            type="button"
            class="icon-btn"
            aria-label="Back to containers list"
            title="Back to containers list"
            onclick={() => (ctrl.phoneDetailOpen = false)}
          >
            <Icon name="chevronLeft" size={16} />
          </button>
        {/if}
      {/snippet}

      {#snippet badge()}
        {#if !showPhoneDetail && ctrl.status}
          <Badge
            tone={vmAsleep ? 'info' : 'ok'}
            label={badgeLabel}
            dot
            live={!vmAsleep && ctrl.status.containers_running > 0}
            title={badgeTitle}
          />
        {/if}
      {/snippet}

      {#snippet tabs()}
        {#if !showPhoneDetail}
          <div class="segmented" role="tablist" aria-label="Containers sections">
            <button
              type="button"
              role="tab"
              class:active={ctrl.activeTab === 'containers'}
              aria-selected={ctrl.activeTab === 'containers'}
              tabindex={ctrl.activeTab === 'containers' ? 0 : -1}
              onkeydown={onTabKey}
              onclick={() => ctrl.switchTab('containers')}
            >
              Containers
            </button>
            <button
              type="button"
              role="tab"
              class:active={ctrl.activeTab === 'images'}
              aria-selected={ctrl.activeTab === 'images'}
              tabindex={ctrl.activeTab === 'images' ? 0 : -1}
              onkeydown={onTabKey}
              onclick={() => ctrl.switchTab('images')}
            >
              Images
            </button>
            <button
              type="button"
              role="tab"
              class:active={ctrl.activeTab === 'volumes'}
              aria-selected={ctrl.activeTab === 'volumes'}
              tabindex={ctrl.activeTab === 'volumes' ? 0 : -1}
              onkeydown={onTabKey}
              onclick={() => ctrl.switchTab('volumes')}
            >
              Volumes
            </button>
            <button
              type="button"
              role="tab"
              class:active={ctrl.activeTab === 'networks'}
              aria-selected={ctrl.activeTab === 'networks'}
              tabindex={ctrl.activeTab === 'networks' ? 0 : -1}
              onkeydown={onTabKey}
              onclick={() => ctrl.switchTab('networks')}
            >
              Networks
            </button>
          </div>
        {/if}
      {/snippet}

      {#snippet actions()}
        <button
          type="button"
          class="icon-btn"
          data-overflow="-1"
          data-icon="refresh"
          aria-label="Refresh"
          title="Refresh"
          onclick={() => void ctrl.refreshActiveTab()}
        >
          <Icon name="refresh" size={14} />
        </button>
        {#if !emptyContainersPage}
          <button type="button" class="btn small primary" onclick={() => openRunModal()}>
            <Icon name="play" size={12} /> Run…
          </button>
        {/if}
      {/snippet}
    </PageHeader>

    {#if ctrl.activeTab === 'containers'}
      <PageBody width="full" padded={false} fill>
        <LoadState
          variant="page"
          what="containers"
          loading={ctrl.containersLoading}
          error={ctrl.containersError}
          empty={ctrl.containers.length === 0}
          onretry={() => void ctrl.loadContainers()}
        >
          {#snippet emptyView()}
            <EmptyState
              variant="page"
              icon="box"
              title="No containers yet"
              body="Run an image or start a Docker Compose project to manage containers, logs, shells, and live stats here."
              actionLabel="Run container…"
              actionIcon="play"
              onaction={() => openRunModal()}
            />
          {/snippet}

          <div class="ct-split">
            {#if !showPhoneDetail}
              <ContainersList
                containers={ctrl.containers}
                selectedId={ctrl.selectedId}
                busyIds={ctrl.busyIds}
                busyStacks={ctrl.busyStacks}
                onselect={(id) => ctrl.selectContainer(id, viewport.isPhone)}
                onaction={(c, act) => void ctrl.handleContainerAction(c, act)}
                onstackaction={(proj, act) => void ctrl.handleStackAction(proj, act)}
                onopenrun={() => openRunModal()}
              />
            {/if}

            {#if (!viewport.isPhone || showPhoneDetail) && selectedContainer}
              <ContainerDetail
                container={selectedContainer}
                tab={ctrl.detailTab}
                busy={ctrl.busyIds.has(selectedContainer.id)}
                dockerCliAvailable={ctrl.status.docker_cli_available}
                refreshTick={ctrl.detailRefreshTick}
                ontabchange={(t) => (ctrl.detailTab = t)}
                onaction={(c, act) => void ctrl.handleContainerAction(c, act)}
              />
            {/if}
          </div>
        </LoadState>
      </PageBody>
    {:else}
      <PageBody width="full">
        {#if ctrl.activeTab === 'images'}
          <ImagesTab
            images={ctrl.images}
            loading={ctrl.imagesLoading}
            error={ctrl.imagesError}
            pulling={ctrl.pulling}
            onrefresh={() => void ctrl.loadImages()}
            onpull={() => void ctrl.pullImagePrompt()}
            onrunimage={(ref) => openRunModal(ref)}
          />
        {:else if ctrl.activeTab === 'volumes'}
          <VolumesTab
            volumes={ctrl.volumes}
            loading={ctrl.volumesLoading}
            error={ctrl.volumesError}
            onrefresh={() => void ctrl.loadVolumes()}
          />
        {:else if ctrl.activeTab === 'networks'}
          <NetworksTab
            networks={ctrl.networks}
            loading={ctrl.networksLoading}
            error={ctrl.networksError}
            onrefresh={() => void ctrl.loadNetworks()}
          />
        {/if}
      </PageBody>
    {/if}
  {/if}
</div>

{#if runModalOpen}
  <LazyMount
    lazy={RunContainerModal}
    what="the Run dialog"
    quiet
    onerror={closeRunModal}
    props={{ initialImage: runInitialImage, onclose: closeRunModal, oncreated: onRunCreated }}
  />
{/if}

<style>
  .ct-page {
    block-size: 100%;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .ct-boot {
    padding: 18px 20px;
  }
  .ct-split {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    display: flex;
    overflow: hidden;
  }
</style>
