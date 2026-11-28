<script lang="ts">
  // IDE Bottom Dock "Devices" tool window: manage iOS Simulators & Android Emulators/AVDs,
  // pick the active Run Target device, inspect live screen frames, rotate, toggle light/dark,
  // forward Android ports (`adb reverse`), open local dev-server URLs, and view live logs.
  import { untrack } from 'svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import Tabs, { type TabItem } from '../../../lib/components/Tabs.svelte';
  import PaneDivider from '../../../lib/components/PaneDivider.svelte';
  import ListPane from '../../../lib/components/side/ListPane.svelte';
  import PhoneBack from '../../../lib/components/side/PhoneBack.svelte';
  import { LIST_PANE, loadPaneW, paneKey, paneStorage } from '../../../lib/components/side/paneKeys';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { rowMenu } from '../../../lib/rowMenu';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import SimCreateDeviceForm from '../../../lib/simulators/SimCreateDeviceForm.svelte';
  import SimInstallStatusCard from '../../../lib/simulators/SimInstallStatusCard.svelte';
  import SimReadinessCards from '../../../lib/simulators/SimReadinessCards.svelte';
  import { simInstaller } from '../../../lib/simulators/simInstaller.svelte';
  import type { SimDevice, SimPlatform, SimPlatformReadiness } from '../../../lib/api/types';
  import { ide } from '../ideStore.svelte';
  import {
    browserPresetForDevice,
    buildBrowserPreviewHash,
    detectDevServerUrls,
  } from './ideDevServerDetect';
  import { ideDevices } from './ideDevicesState.svelte';
  import IdeDeviceLogsView from './IdeDeviceLogsView.svelte';
  import IdeDevicePortsView from './IdeDevicePortsView.svelte';
  import IdeDevicePreviewStage from './IdeDevicePreviewStage.svelte';

  type PlatformFilter = 'all' | SimPlatform;
  type DetailTab = 'preview' | 'logs' | 'ports';

  let platformFilter = $state<PlatformFilter>('all');
  let detailTab = $state<DetailTab>('preview');
  let createModalOpen = $state(false);
  let setupOpen = $state(false);
  let urlInput = $state('');
  // The device list is a list pane in the tool window (shared range and key).
  let listW = $state(loadPaneW(paneStorage(), 'ide', 'devices', LIST_PANE, undefined, 280));
  const phone = $derived(viewport.isPhone);
  let phoneOpen = $state(false);

  $effect(() => {
    untrack(() => {
      void ideDevices.ensureLoaded();
      void simInstaller.load();
    });
    return simInstaller.onComplete((st) => {
      if (st.stage === 'done') void ideDevices.refresh(true);
    });
  });

  const filteredDevices = $derived.by(() => {
    const all = ideDevices.sortedDevices;
    if (platformFilter === 'all') return all;
    return all.filter((d) => d.platform === platformFilter);
  });

  const selected = $derived(ideDevices.selectedDevice);
  const selectedId = $derived(selected?.id ?? null);
  const selectedBooted = $derived(selected?.state === 'booted');
  const selectedCanForward = $derived(Boolean(selected?.capabilities.port_forward));
  const selectedPlatform = $derived(selected?.platform ?? null);

  const detectedServers = $derived(detectDevServerUrls(ide.activeRun?.output ?? []));

  const readinessList = $derived.by((): SimPlatformReadiness[] => {
    const st = ideDevices.status;
    if (!st) return [];
    const list = [st.ios, st.android];
    if (platformFilter === 'all') return list;
    return list.filter((p) => p.platform === platformFilter);
  });

  const degradedReadiness = $derived(readinessList.filter((r) => r.state !== 'ready'));

  const detailTabs = $derived.by((): readonly TabItem<DetailTab>[] => {
    const dev = selected;
    const portsCount = dev ? (ideDevices.portsByDevice[dev.id]?.length ?? 0) : 0;
    const items: TabItem<DetailTab>[] = [
      { id: 'preview', label: 'Preview', icon: 'image', testid: 'ide-sim-tab-preview' },
      { id: 'logs', label: 'Logs', icon: 'terminal', testid: 'ide-sim-tab-logs' },
    ];
    if (!dev || dev.platform === 'android' || dev.capabilities.port_forward) {
      items.push({
        id: 'ports',
        label: 'Ports',
        icon: 'link',
        count: portsCount > 0 ? portsCount : undefined,
        testid: 'ide-sim-tab-ports',
      });
    }
    return items;
  });

  // Track primitive identity/state so rotate/appearance updates do not trigger redundant resets.
  $effect(() => {
    const id = selectedId;
    const booted = selectedBooted;
    const canForward = selectedCanForward;
    const platform = selectedPlatform;
    if (id && booted && canForward) {
      untrack(() => void ideDevices.loadPorts(id));
    }
    if (platform === 'ios') {
      untrack(() => {
        if (detailTab === 'ports') detailTab = 'preview';
      });
    }
  });

  function openInBrowserEmulator(urlOverride?: string): void {
    const targetUrl =
      (urlOverride ?? urlInput).trim() || detectedServers[0]?.url || 'http://127.0.0.1:5173';
    const dev = selected;
    const deviceParam =
      dev && dev.state === 'booted'
        ? (`simulator:${dev.id}` as const)
        : browserPresetForDevice(dev);
    window.location.hash = buildBrowserPreviewHash({ url: targetUrl, device: deviceParam });
  }

  function toggleRotate(dev: SimDevice): void {
    const next = dev.orientation === 'landscape' ? 'portrait' : 'landscape';
    void ideDevices.setOrientation(dev, next);
  }

  function toggleAppearance(dev: SimDevice): void {
    const next = dev.appearance === 'dark' ? 'light' : 'dark';
    void ideDevices.setAppearance(dev, next);
  }

  // Boot / Shut down sit on the row; ⋯ (right-click, long-press) holds them and Restart.
  function deviceMenu(dev: SimDevice): MenuItem[] {
    const busy = Boolean(ideDevices.busyById[dev.id]);
    if (dev.state !== 'booted') return [{ label: 'Boot', icon: 'play', disabled: busy || !dev.capabilities.boot, action: () => void ideDevices.boot(dev) }];
    return [
      { label: 'Restart', icon: 'refresh', disabled: busy || !dev.capabilities.boot, action: () => void ideDevices.restart(dev) },
      { label: 'Shut Down', icon: 'stop', disabled: busy || !dev.capabilities.shutdown, action: () => void ideDevices.shutdown(dev) },
    ];
  }

  function stateTone(state: SimDevice['state']): 'ok' | 'info' | 'bad' | 'neutral' {
    if (state === 'booted') return 'ok';
    if (state === 'booting' || state === 'shutting_down') return 'info';
    if (state === 'offline') return 'bad';
    return 'neutral';
  }
</script>

<div class="sim-root" data-testid="ide-devices-panel">
  <header class="sim-bar">
    {#if ideDevices.status}
      <div class="sim-status-pills">
        <Badge
          tone={ideDevices.status.ios.state === 'ready' ? 'ok' : ideDevices.status.ios.state === 'degraded' ? 'warn' : 'neutral'}
          dot
          label={`iOS: ${ideDevices.status.ios.state}`}
          title={ideDevices.status.ios.summary}
        />
        <Badge
          tone={ideDevices.status.android.state === 'ready' ? 'ok' : ideDevices.status.android.state === 'degraded' ? 'warn' : 'neutral'}
          dot
          label={`Android: ${ideDevices.status.android.state}`}
          title={ideDevices.status.android.summary}
        />
      </div>
    {/if}

    <div class="sim-bar-spacer"></div>

    {#if degradedReadiness.length > 0 && filteredDevices.length > 0}
      <button
        type="button"
        class="btn small ghost"
        aria-expanded={setupOpen}
        onclick={() => (setupOpen = !setupOpen)}
        title="Show mobile SDK setup and 1-click installers"
        data-testid="ide-sim-toggle-setup"
      >
        <Icon name="download" size={12} />
        Setup
      </button>
    {/if}
    <button
      type="button"
      class="btn small ghost"
      onclick={() => (createModalOpen = true)}
      title="Create a new iOS Simulator or Android AVD"
      data-testid="ide-sim-new-device"
    >
      <Icon name="plus" size={12} />
      New Device
    </button>
    <button
      type="button"
      class="btn small ghost"
      onclick={() => openInBrowserEmulator()}
      title="Open current URL in Otto Browser’s mobile device emulator"
      data-testid="ide-sim-open-browser"
    >
      <Icon name="compass" size={12} />
      Browser Emulator
    </button>
    <button
      type="button"
      class="icon-btn"
      aria-label="Refresh simulators and emulators"
      title="Refresh simulators and emulators"
      disabled={ideDevices.loading}
      onclick={() => void ideDevices.refresh(true)}
      data-testid="ide-sim-refresh"
    >
      <Icon name="refresh" size={13} />
    </button>
  </header>

  {#if simInstaller.visible}
    <div class="sim-inst-wrap">
      <SimInstallStatusCard
        status={simInstaller.status}
        busy={simInstaller.busy}
        oncancel={() => void simInstaller.cancel()}
        ondismiss={() => simInstaller.dismiss()}
        onrefresh={() => void ideDevices.refresh(true)}
        testid="ide-sim-install-banner"
      />
    </div>
  {/if}

  {#if ideDevices.error && !ideDevices.loadedOnce}
    <div class="sim-state-wrap">
      <LoadState
        what="mobile simulators"
        error={ideDevices.error}
        onretry={() => void ideDevices.refresh(true)}
      />
    </div>
  {:else if ideDevices.loading && !ideDevices.loadedOnce}
    <div class="sim-state-wrap">
      <LoadState what="mobile simulators" loading />
    </div>
  {:else}
    {#if degradedReadiness.length > 0 && (filteredDevices.length === 0 || setupOpen)}
      <div class="sim-diag-wrap">
        <SimReadinessCards
          items={degradedReadiness}
          installDisabled={simInstaller.running || simInstaller.busy}
          startingTarget={simInstaller.startingTarget}
          maxSteps={2}
          oninstallandroid={() => void simInstaller.start('android_emulator', { create_default_avds: true })}
          oninstallios={() => void simInstaller.start('ios_runtime')}
        />
      </div>
    {/if}

    <div class="sim-split">
      <!-- A list pane inside the tool window: a standalone ListPane (its own
           header; the page's list is the explorer). A phone pushes: a tapped
           device shows its inspector with "‹ Devices". -->
      <div class="sim-list-col" class:phone style:inline-size={phone ? undefined : `${listW}px`} hidden={phone && phoneOpen}>
        <ListPane name="Devices" count={filteredDevices.length}>
          {#snippet filterRow()}
            <div class="segmented" role="group" aria-label="Filter devices by platform">
              <button type="button" class:active={platformFilter === 'all'} aria-pressed={platformFilter === 'all'} onclick={() => (platformFilter = 'all')}>All</button>
              <button type="button" class:active={platformFilter === 'ios'} aria-pressed={platformFilter === 'ios'} onclick={() => (platformFilter = 'ios')}>iOS</button>
              <button type="button" class:active={platformFilter === 'android'} aria-pressed={platformFilter === 'android'} onclick={() => (platformFilter = 'android')}>Android</button>
            </div>
          {/snippet}
          {#if filteredDevices.length === 0}
            <EmptyState
              icon="phone"
              title="No simulators found"
              body="Install an iOS Simulator runtime or create an Android AVD to boot and test apps directly from Otto IDE."
              actionLabel="Refresh"
              actionIcon="refresh"
              actionKind="secondary"
              onaction={() => void ideDevices.refresh(true)}
            />
          {:else}
            {#each filteredDevices as dev (dev.id)}
              {@const isSel = selected?.id === dev.id}
              {@const busy = ideDevices.busyById[dev.id]}
              <div class="side-row two-line" class:selected={isSel} data-testid="ide-sim-device-row">
                <button
                  type="button"
                  class="side-row-main"
                  data-side-row
                  aria-current={isSel ? 'true' : undefined}
                  title={dev.name}
                  onclick={() => { ideDevices.selectDevice(dev.id); phoneOpen = true; }}
                  oncontextmenu={(e) => { e.preventDefault(); ctxMenu.show(e, deviceMenu(dev)); }}
                  use:rowMenu
                >
                  <Icon name="phone" size={14} />
                  <span class="side-row-text">
                    <span class="side-row-title" dir="ltr">{dev.name}</span>
                    <span class="side-row-meta" dir="ltr">{dev.platform === 'ios' ? 'iOS' : 'Android'} {dev.os_version} · {dev.form_factor}</span>
                  </span>
                  <span class="side-row-trailing"><Badge tone={stateTone(dev.state)} dot={dev.state === 'booted'} live={dev.state === 'booting'} label={busy || dev.state} /></span>
                </button>
                {#if dev.state === 'booted'}
                  <button type="button" class="icon-btn" aria-label={`Shut down ${dev.name}`} title={`Shut down ${dev.name}`} disabled={Boolean(busy) || !dev.capabilities.shutdown} onclick={() => void ideDevices.shutdown(dev)}>
                    <Icon name="stop" size={14} />
                  </button>
                {:else}
                  <button type="button" class="icon-btn" aria-label={`Boot ${dev.name}`} title={`Boot ${dev.name}`} disabled={Boolean(busy) || !dev.capabilities.boot} onclick={() => void ideDevices.boot(dev)}>
                    <Icon name="play" size={14} />
                  </button>
                {/if}
                <button type="button" class="icon-btn side-row-more" tabindex="-1" aria-label={`${dev.name} actions`} title="More" onclick={(e) => ctxMenu.showAt(e.currentTarget as HTMLElement, deviceMenu(dev), { align: 'end' })}>
                  <Icon name="more" size={14} />
                </button>
              </div>
            {/each}
          {/if}
        </ListPane>
      </div>
      {#if !phone}
        <PaneDivider bind:width={listW} storageKey={paneKey('ide', 'devices', 'w')} min={LIST_PANE.min} max={LIST_PANE.max} defaultWidth={280} label="Resize devices" />
      {/if}

      <section class="sim-detail-col" aria-label="Selected device inspector" hidden={phone && !phoneOpen}>
        {#if !selected}
          <div class="sim-detail-empty">
            <EmptyState
              icon="phone"
              title="Select a device"
              body="Pick an iOS Simulator or Android Emulator to preview its screen, stream logs, forward ports, or set it as your Run Target."
            />
          </div>
        {:else}
          <div class="sim-detail-head">
            {#if phone}<PhoneBack label="Devices" onclick={() => (phoneOpen = false)} testid="ide-sim-devices-back" />{/if}
            <Tabs
              tabs={detailTabs}
              value={detailTab}
              onchange={(t) => (detailTab = t)}
              label="Device inspector tabs"
              size="s"
            >
              {#snippet trailing()}
                <div class="sim-quick-controls">
                  <button
                    type="button"
                    class="icon-btn"
                    aria-label="Rotate device"
                    title={`Rotate (${selected.orientation === 'landscape' ? 'to portrait' : 'to landscape'})`}
                    disabled={selected.state !== 'booted' || !selected.capabilities.rotate}
                    onclick={() => toggleRotate(selected)}
                  >
                    <Icon name="swap" size={13} />
                  </button>
                  <button
                    type="button"
                    class="icon-btn"
                    aria-label="Toggle light or dark appearance"
                    title={`Switch to ${selected.appearance === 'dark' ? 'light' : 'dark'} appearance`}
                    disabled={selected.state !== 'booted' || !selected.capabilities.appearance}
                    onclick={() => toggleAppearance(selected)}
                  >
                    <Icon name="palette" size={13} />
                  </button>
                </div>
              {/snippet}
            </Tabs>
          </div>

          {#if detailTab === 'preview'}
            <IdeDevicePreviewStage
              device={selected}
              {detectedServers}
              {urlInput}
              onurlchange={(next) => (urlInput = next)}
              onopenbrowser={(override) => openInBrowserEmulator(override)}
            />
          {:else if detailTab === 'logs'}
            <IdeDeviceLogsView device={selected} />
          {:else if detailTab === 'ports'}
            <IdeDevicePortsView device={selected} {detectedServers} />
          {/if}
        {/if}
      </section>
    </div>
  {/if}

  {#if createModalOpen}
    <Modal title="New Simulator / AVD" width={480} onclose={() => (createModalOpen = false)}>
      <SimCreateDeviceForm
        initialPlatform={platformFilter === 'android' ? 'android' : 'ios'}
        iosReadiness={ideDevices.status?.ios ?? null}
        androidReadiness={ideDevices.status?.android ?? null}
        installDisabled={simInstaller.running || simInstaller.busy}
        showHeader={false}
        oncreated={(created) => {
          ideDevices.registerCreatedDevice(created);
          createModalOpen = false;
        }}
        oncancel={() => (createModalOpen = false)}
        oninstallios={() => {
          void simInstaller.start('ios_runtime');
          createModalOpen = false;
        }}
        oninstallandroid={() => {
          void simInstaller.start('android_emulator', { create_default_avds: true });
          createModalOpen = false;
        }}
      />
    </Modal>
  {/if}
</div>

<style>
  .sim-root {
    display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; min-inline-size: 0;
    background: var(--surface);
  }
  .sim-bar {
    display: flex; align-items: center; gap: 8px; padding-block: 4px; padding-inline: 8px;
    border-block-end: 1px solid var(--border); flex-shrink: 0; overflow-x: auto; min-inline-size: 0;
  }
  .sim-inst-wrap {
    padding: 6px 8px; border-block-end: 1px solid var(--border); flex-shrink: 0;
  }
  .sim-diag-wrap {
    padding: 8px; border-block-end: 1px solid var(--border); background: var(--surface-2); flex-shrink: 0;
  }
  .sim-status-pills { display: inline-flex; align-items: center; gap: 6px; flex-shrink: 0; }
  .sim-bar-spacer { flex: 1; min-inline-size: 4px; }
  .sim-state-wrap { flex: 1; display: flex; align-items: center; justify-content: center; padding: 16px; }
  .sim-split { flex: 1; min-block-size: 0; min-inline-size: 0; display: flex; overflow: hidden; }
  .sim-list-col { flex-shrink: 0; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; }
  .sim-list-col.phone { flex: 1; }
  .sim-list-col[hidden], .sim-detail-col[hidden] { display: none; }
  .sim-detail-col {
    flex: 1; min-inline-size: 0; min-block-size: 0; display: flex; flex-direction: column; overflow: hidden;
  }
  .sim-detail-empty { flex: 1; display: flex; align-items: center; justify-content: center; padding: 12px; }
  .sim-detail-head { flex-shrink: 0; display: flex; align-items: center; border-block-end: 1px solid var(--border); }
  .sim-detail-head :global(.otabs) { flex: 1; min-inline-size: 0; }
  .sim-quick-controls { display: inline-flex; align-items: center; gap: 2px; }

  @media (pointer: coarse) {
    .sim-bar :global(.icon-btn) { min-inline-size: 36px; min-block-size: 36px; }
  }
</style>
