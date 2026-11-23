<script lang="ts">
  // Simulators & Emulators picker modal body for Otto Browser.
  // Lists iOS Simulators & Android Emulators from `simulatorsApi.devices()`,
  // delegates readiness diagnostics, 1-click installation, and device creation
  // to the shared `lib/simulators/*` primitives, and lets the user boot,
  // shut down, mirror inside Browser, or open the active tab URL on a device.
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { loadErrorText } from '../../../lib/loadError';
  import { simulatorsApi } from '../../../lib/api/simulators';
  import type {
    SimDevice,
    SimPlatform,
    SimPlatformReadiness,
    SimulatorsStatus,
  } from '../../../lib/api/types';
  import SimCreateDeviceForm from '../../../lib/simulators/SimCreateDeviceForm.svelte';
  import SimInstallStatusCard from '../../../lib/simulators/SimInstallStatusCard.svelte';
  import SimReadinessCards from '../../../lib/simulators/SimReadinessCards.svelte';
  import { simInstaller } from '../../../lib/simulators/simInstaller.svelte';
  import { simDeviceSubtitle } from './simGeometry';

  interface Props {
    currentUrl?: string;
    onmirror: (device: SimDevice, split: boolean) => void;
  }
  let { currentUrl = '', onmirror }: Props = $props();

  let devices = $state<SimDevice[]>([]);
  let status = $state<SimulatorsStatus | null>(null);
  let loading = $state(true);
  let error = $state('');
  let busyId = $state<string | null>(null);

  let showCreate = $state(false);
  let createPlatform = $state<SimPlatform>('ios');

  const iosDevices = $derived(devices.filter((d) => d.platform === 'ios'));
  const androidDevices = $derived(devices.filter((d) => d.platform === 'android'));
  const unreadyPlatforms = $derived.by((): SimPlatformReadiness[] => {
    if (!status) return [];
    return [status.ios, status.android].filter((r) => r.state !== 'ready');
  });
  const installDisabled = $derived(simInstaller.busy || simInstaller.running);

  async function load(refresh = false): Promise<void> {
    loading = true;
    error = '';
    try {
      const resp = await simulatorsApi.devices({ refresh });
      devices = resp.devices;
      status = resp.status;
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load(false);
    void simInstaller.load();
    return simInstaller.onComplete(() => void load(true));
  });

  function openCreateForm(p: SimPlatform): void {
    createPlatform = p;
    showCreate = true;
  }

  function onDeviceCreated(dev: SimDevice): void {
    devices = [dev, ...devices.filter((d) => d.id !== dev.id)];
    showCreate = false;
    void load(true);
  }

  async function boot(d: SimDevice): Promise<void> {
    if (busyId) return;
    busyId = d.id;
    try {
      // Embedded-first: boot headlessly (no external emulator/Simulator window)
      // and open the in-app mirror stage automatically once booted.
      const updated = await simulatorsApi.boot(d.id, { wait: true });
      devices = devices.map((x) => (x.id === updated.id ? updated : x));
      toasts.success('Simulator ready', updated.name);
      onmirror(updated, true);
    } catch (e) {
      toastError(`Couldn’t boot ${d.name}`, e);
    } finally {
      busyId = null;
    }
  }

  async function shutdown(d: SimDevice): Promise<void> {
    if (busyId) return;
    busyId = d.id;
    try {
      const updated = await simulatorsApi.shutdown(d.id);
      devices = devices.map((x) => (x.id === updated.id ? updated : x));
      toasts.info('Simulator shut down', updated.name);
    } catch (e) {
      toastError(`Couldn’t shut down ${d.name}`, e);
    } finally {
      busyId = null;
    }
  }

  async function openUrl(d: SimDevice): Promise<void> {
    if (!currentUrl || busyId) return;
    busyId = d.id;
    try {
      let target = d;
      if (target.state !== 'booted' && target.capabilities.boot) {
        target = await simulatorsApi.boot(target.id, { wait: true });
        devices = devices.map((x) => (x.id === target.id ? target : x));
      }
      const resp = await simulatorsApi.openUrl(target.id, { url: currentUrl, auto_forward_localhost: true });
      const fwd = resp.forwarded_ports.length > 0 ? ` · forwarded port ${resp.forwarded_ports.join(', ')}` : '';
      toasts.success('Opened URL', `${target.name}${fwd}`);
      onmirror(target, true);
    } catch (e) {
      toastError(`Couldn’t open URL on ${d.name}`, e);
    } finally {
      busyId = null;
    }
  }

  function readinessTone(r: SimPlatformReadiness): 'ok' | 'warn' | 'off' {
    if (r.state === 'ready') return 'ok';
    if (r.state === 'degraded') return 'warn';
    return 'off';
  }
</script>

<div class="simp">
  <div class="simp-top">
    <p class="simp-lead">
      Boot iOS Simulators or Android Emulators directly inside Otto (embedded headless by default), open your current tab with automatic Android localhost forwarding, or switch devices.
    </p>
    <div class="simp-top-acts">
      <button
        type="button"
        class="btn small"
        onclick={() => openCreateForm(status?.ios.state === 'ready' ? 'ios' : 'android')}
        disabled={loading}
      >
        <Icon name="plus" size={12} /> New device
      </button>
      <button type="button" class="btn small" onclick={() => void load(true)} disabled={loading}>
        <Icon name="refresh" size={12} /> Refresh
      </button>
    </div>
  </div>

  {#if simInstaller.visible && simInstaller.status}
    <SimInstallStatusCard
      status={simInstaller.status}
      busy={simInstaller.busy}
      oncancel={() => void simInstaller.cancel()}
      ondismiss={() => simInstaller.dismiss()}
      onrefresh={() => void load(true)}
    />
  {/if}

  {#if showCreate}
    <SimCreateDeviceForm
      initialPlatform={createPlatform}
      iosReadiness={status?.ios ?? null}
      androidReadiness={status?.android ?? null}
      {installDisabled}
      showHeader={true}
      oncreated={onDeviceCreated}
      oncancel={() => (showCreate = false)}
      oninstallios={() => void simInstaller.start('ios_runtime')}
      oninstallandroid={() => void simInstaller.start('android_emulator')}
    />
  {/if}

  {#if error || (loading && !status)}
    <LoadState what="simulators" {loading} {error} onretry={() => void load(true)} />
  {:else if status}
    {#if unreadyPlatforms.length > 0}
      <SimReadinessCards
        items={unreadyPlatforms}
        {installDisabled}
        startingTarget={simInstaller.startingTarget}
        oninstallandroid={() => void simInstaller.start('android_emulator')}
        oninstallios={() => void simInstaller.start('ios_runtime')}
      />
    {/if}

    {#each [
      { key: 'ios' as const, label: 'iOS Simulator', r: status.ios, list: iosDevices },
      { key: 'android' as const, label: 'Android Emulator', r: status.android, list: androidDevices },
    ] as section (section.key)}
      {#if section.r.state === 'ready' || section.list.length > 0}
        <div class="simp-sec">
          <div class="simp-sec-head">
            <span class="simp-dot {readinessTone(section.r)}" aria-hidden="true"></span>
            <h3 class="simp-h3">{section.label}</h3>
            <span class="simp-summary">{section.r.summary}</span>
          </div>

          {#if section.list.length > 0}
            <ul class="simp-list">
              {#each section.list as d (d.id)}
                {@const isBooted = d.state === 'booted'}
                <li class="simp-item">
                  <span class="simp-dot" class:ok={isBooted} class:warn={d.state === 'booting'} aria-hidden="true"></span>
                  <div class="simp-meta">
                    <span class="simp-name">{d.name}</span>
                    <span class="simp-sub">{simDeviceSubtitle(d)} · {d.state.replace('_', ' ')}</span>
                  </div>
                  <div class="simp-acts">
                    {#if isBooted}
                      {#if currentUrl && d.capabilities.open_url}
                        <button
                          class="btn small"
                          onclick={() => void openUrl(d)}
                          disabled={busyId === d.id}
                          title="Open active browser tab URL on {d.name}"
                        >
                          <Icon name="external" size={12} /> Open URL
                        </button>
                      {/if}
                      <button
                        class="btn small primary"
                        onclick={() => onmirror(d, true)}
                        title="Mirror {d.name} alongside Browser"
                      >
                        <Icon name="columns" size={12} /> Mirror
                      </button>
                      {#if d.capabilities.shutdown}
                        <button
                          class="icon-btn simp-icon"
                          onclick={() => void shutdown(d)}
                          disabled={busyId === d.id}
                          aria-label="Shut down {d.name}"
                          title="Shut down"
                        >
                          <Icon name="stop" size={12} />
                        </button>
                      {/if}
                    {:else if d.capabilities.boot}
                      <button
                        class="btn small"
                        onclick={() => void boot(d)}
                        disabled={busyId === d.id}
                      >
                        <Icon name="play" size={12} /> {busyId === d.id ? 'Booting…' : 'Boot'}
                      </button>
                      {#if currentUrl && d.capabilities.open_url}
                        <button
                          class="btn small"
                          onclick={() => void openUrl(d)}
                          disabled={busyId === d.id}
                          title="Boot and open current tab URL"
                        >
                          Boot &amp; Open
                        </button>
                      {/if}
                    {/if}
                  </div>
                </li>
              {/each}
            </ul>
          {:else}
            <p class="simp-none">No devices found yet.</p>
          {/if}
        </div>
      {/if}
    {/each}
  {/if}
</div>

<style>
  .simp {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .simp-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
  }
  .simp-top-acts {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .simp-lead {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.45;
  }
  .simp-sec {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .simp-sec-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .simp-h3 {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .simp-summary {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .simp-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-dim);
    flex-shrink: 0;
  }
  .simp-dot.ok { background: var(--success); }
  .simp-dot.warn { background: var(--warning); }
  .simp-dot.off { background: var(--text-dim); }
  .simp-icon {
    width: 26px;
    height: 26px;
    flex-shrink: 0;
  }
  .simp-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .simp-item {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--bg);
    border: 1px solid var(--border);
    min-width: 0;
  }
  .simp-meta {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .simp-name {
    font-size: var(--fs-s);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .simp-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .simp-acts {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .simp-none {
    margin: 2px 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  @media (max-width: 640px) {
    .simp-item {
      flex-wrap: wrap;
    }
    .simp-acts {
      width: 100%;
      justify-content: flex-end;
    }
  }
</style>
