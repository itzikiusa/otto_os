<script lang="ts">
  // Shared "+ New Device / AVD" creation form.
  // Strictly filters Android system images to `installed === true` and gates
  // any inline iOS runtime download CTA on `canOfferIosRuntimeDownload`.
  // Can be rendered inline (Browser SimulatorPanel) or inside a Modal (IDE).
  import { untrack } from 'svelte';
  import Icon from '../components/Icon.svelte';
  import LoadState from '../components/LoadState.svelte';
  import { toasts } from '../toast.svelte';
  import { toastError } from '../toastError';
  import { loadErrorText } from '../loadError';
  import { simulatorsApi } from '../api/simulators';
  import type {
    CreateSimDeviceReq,
    SimCatalogResp,
    SimDevice,
    SimPlatform,
    SimPlatformReadiness,
  } from '../api/types';
  import { canOfferAndroidInstall, canOfferIosRuntimeDownload } from './simInstaller.svelte';

  interface Props {
    initialPlatform?: SimPlatform;
    /** Optional readiness diagnostics to gate inline install CTAs when catalog is empty. */
    iosReadiness?: SimPlatformReadiness | null;
    androidReadiness?: SimPlatformReadiness | null;
    installDisabled?: boolean;
    /** Show the header bar with inline close X (true for inline cards; false inside Modal). */
    showHeader?: boolean;
    oncreated: (device: SimDevice) => void;
    oncancel: () => void;
    oninstallios?: () => void;
    oninstallandroid?: () => void;
  }

  let {
    initialPlatform = 'ios',
    iosReadiness = null,
    androidReadiness = null,
    installDisabled = false,
    showHeader = true,
    oncreated,
    oncancel,
    oninstallios,
    oninstallandroid,
  }: Props = $props();

  let platform = $state<SimPlatform>('ios');
  let catalog = $state<SimCatalogResp | null>(null);
  let loading = $state(true);
  let error = $state('');
  let creating = $state(false);

  let name = $state('');
  let iosDeviceTypeId = $state('');
  let iosRuntimeId = $state('');
  let androidImageId = $state('');
  let androidProfileId = $state('');

  const availableIosRuntimes = $derived(
    (catalog?.ios_runtimes ?? []).filter((r) => r.available),
  );
  const iosDeviceTypes = $derived(catalog?.ios_device_types ?? []);
  const installedAndroidImages = $derived(
    (catalog?.android_system_images ?? []).filter((img) => img.installed),
  );
  const androidProfiles = $derived(catalog?.android_device_profiles ?? []);

  const canSubmit = $derived.by(() => {
    if (creating || !name.trim()) return false;
    if (platform === 'ios') return !!iosDeviceTypeId && !!iosRuntimeId;
    return !!androidImageId;
  });

  async function loadCatalog(): Promise<void> {
    loading = true;
    error = '';
    try {
      const c = await simulatorsApi.catalog();
      catalog = c;
      const runtimes = c.ios_runtimes.filter((r) => r.available);
      if (!iosRuntimeId && runtimes.length > 0) iosRuntimeId = runtimes[0].id;
      if (!iosDeviceTypeId && c.ios_device_types.length > 0) {
        const preferred =
          c.ios_device_types.find((d) => /iPhone 16 Pro/i.test(d.name)) ??
          c.ios_device_types.find((d) => d.form_factor === 'phone') ??
          c.ios_device_types[0];
        iosDeviceTypeId = preferred.id;
      }
      const images = c.android_system_images.filter((i) => i.installed);
      if (!androidImageId && images.length > 0) androidImageId = images[0].id;
      if (!androidProfileId && c.android_device_profiles.length > 0) {
        const preferred =
          c.android_device_profiles.find((p) => /pixel/i.test(p.id) || /pixel/i.test(p.name)) ??
          c.android_device_profiles[0];
        androidProfileId = preferred.id;
      }
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    const init = initialPlatform;
    untrack(() => {
      platform = init;
      void loadCatalog();
    });
  });

  async function submit(e: SubmitEvent): Promise<void> {
    e.preventDefault();
    if (!canSubmit) return;
    creating = true;
    try {
      const req: CreateSimDeviceReq =
        platform === 'ios'
          ? {
              platform: 'ios',
              name: name.trim(),
              device_type_id: iosDeviceTypeId,
              runtime_id: iosRuntimeId,
            }
          : {
              platform: 'android',
              name: name.trim(),
              system_image_id: androidImageId,
              ...(androidProfileId ? { device_profile: androidProfileId } : {}),
            };
      const dev = await simulatorsApi.createDevice(req);
      toasts.success('Device created', dev.name);
      oncreated(dev);
    } catch (err) {
      toastError('Couldn’t create device', err);
    } finally {
      creating = false;
    }
  }
</script>

<form
  class="simc"
  class:framed={showHeader}
  onsubmit={submit}
  aria-label="Create new simulator or emulator"
  data-testid="sim-create-form"
>
  <div class="simc-head">
    {#if showHeader}
      <span class="simc-title">New device / AVD</span>
    {:else}
      <span class="simc-label">Platform</span>
    {/if}
    <div class="simc-seg" role="group" aria-label="Platform">
      <button
        type="button"
        class="simc-segbtn"
        class:active={platform === 'ios'}
        aria-pressed={platform === 'ios'}
        onclick={() => (platform = 'ios')}
      >
        iOS Simulator
      </button>
      <button
        type="button"
        class="simc-segbtn"
        class:active={platform === 'android'}
        aria-pressed={platform === 'android'}
        onclick={() => (platform = 'android')}
      >
        Android AVD
      </button>
    </div>
    {#if showHeader}
      <button
        type="button"
        class="icon-btn simc-close"
        onclick={oncancel}
        aria-label="Cancel new device"
        title="Close form"
      >
        <Icon name="x" size={13} />
      </button>
    {/if}
  </div>

  {#if loading || error}
    <LoadState what="device catalog" {loading} {error} onretry={() => void loadCatalog()} />
  {:else if platform === 'ios'}
    {#if availableIosRuntimes.length === 0 || iosDeviceTypes.length === 0}
      <div class="simc-empty-row">
        <p class="simc-empty">
          No available iOS runtimes found yet.
        </p>
        {#if oninstallios && canOfferIosRuntimeDownload(iosReadiness)}
          <button
            type="button"
            class="btn small primary"
            disabled={installDisabled}
            onclick={oninstallios}
          >
            <Icon name="download" size={12} /> Download iOS Runtime
          </button>
        {/if}
      </div>
    {:else}
      <div class="simc-grid">
        <label class="simc-field">
          <span class="simc-label">Name</span>
          <input
            class="simc-input"
            type="text"
            dir="auto"
            bind:value={name}
            placeholder="e.g. Otto iPhone 16 Pro"
            required
          />
        </label>
        <label class="simc-field">
          <span class="simc-label">Device type</span>
          <select class="simc-input" bind:value={iosDeviceTypeId}>
            {#each iosDeviceTypes as dt (dt.id)}
              <option value={dt.id}>{dt.name}</option>
            {/each}
          </select>
        </label>
        <label class="simc-field">
          <span class="simc-label">iOS runtime</span>
          <select class="simc-input" bind:value={iosRuntimeId}>
            {#each availableIosRuntimes as rt (rt.id)}
              <option value={rt.id}>{rt.name}</option>
            {/each}
          </select>
        </label>
      </div>
    {/if}
  {:else}
    {#if installedAndroidImages.length === 0}
      <div class="simc-empty-row">
        <p class="simc-empty">
          No installed Android system images found yet.
        </p>
        {#if oninstallandroid && (!androidReadiness || canOfferAndroidInstall(androidReadiness))}
          <button
            type="button"
            class="btn small primary"
            disabled={installDisabled}
            onclick={oninstallandroid}
          >
            <Icon name="download" size={12} /> Install Android Emulator
          </button>
        {/if}
      </div>
    {:else}
      <div class="simc-grid">
        <label class="simc-field">
          <span class="simc-label">AVD name</span>
          <input
            class="simc-input"
            type="text"
            dir="auto"
            bind:value={name}
            placeholder="e.g. Otto_Pixel_Phone"
            required
          />
        </label>
        <label class="simc-field">
          <span class="simc-label">System image</span>
          <select class="simc-input" bind:value={androidImageId}>
            {#each installedAndroidImages as img (img.id)}
              <option value={img.id}>{img.id}</option>
            {/each}
          </select>
        </label>
        {#if androidProfiles.length > 0}
          <label class="simc-field">
            <span class="simc-label">Device profile</span>
            <select class="simc-input" bind:value={androidProfileId}>
              <option value="">Default</option>
              {#each androidProfiles as prof (prof.id)}
                <option value={prof.id}>{prof.name}</option>
              {/each}
            </select>
          </label>
        {/if}
      </div>
    {/if}
  {/if}

  <div class="simc-foot">
    <button type="button" class="btn small" onclick={oncancel} disabled={creating}>Cancel</button>
    <button type="submit" class="btn small primary" disabled={!canSubmit}>
      <Icon name="plus" size={12} /> {creating ? 'Creating…' : 'Create device'}
    </button>
  </div>
</form>

<style>
  .simc {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .simc.framed {
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .simc-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .simc-title {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .simc-seg {
    display: inline-flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    margin-inline-start: auto;
  }
  .simc-segbtn {
    padding: 4px 10px;
    border: none;
    background: var(--bg);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .simc-segbtn + .simc-segbtn {
    border-inline-start: 1px solid var(--border);
  }
  .simc-segbtn.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .simc-close {
    width: 26px;
    height: 26px;
  }
  .simc-grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    gap: 8px;
  }
  .simc-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .simc-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .simc-input {
    height: 30px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .simc-input:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .simc-empty-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
  }
  .simc-empty {
    margin: 2px 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .simc-foot {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
</style>
