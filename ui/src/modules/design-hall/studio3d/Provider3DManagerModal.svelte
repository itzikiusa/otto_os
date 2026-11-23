<script lang="ts">
  // Native 3D Generation Provider Manager (Hunyuan3D-Omni, TRELLIS.2, Custom HTTP/CLI).
  // API keys are sent write-only and stored in the macOS Keychain by ottod.
  import { untrack } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import * as mapi from '../../../lib/api/media';
  import { licenseWarningFor, split3dProviders } from './providers';
  import type {
    Media3dProvider,
    Media3dProviderKind,
    Media3dTransport,
    ProviderTestResult,
  } from '../../../lib/api/types';

  interface Props {
    canAdmin: boolean;
    onclose: () => void;
    onchanged?: () => void;
  }
  let { canAdmin, onclose, onchanged }: Props = $props();

  let providers = $state<Media3dProvider[]>([]);
  // Otto Forge (and any provider already set up) shows; the rest wait under Advanced.
  const split = $derived(split3dProviders(providers));
  let loading = $state(true);
  let error = $state<string | null>(null);
  let editing = $state<Media3dProvider | 'new' | null>(null);
  let testingId = $state<string | null>(null);
  let testResults = $state<Record<string, ProviderTestResult>>({});
  let saving = $state(false);
  let formError = $state<string | null>(null);

  // Form fields
  let fName = $state('');
  let fKind = $state<Media3dProviderKind>('custom_http');
  let fTransport = $state<Media3dTransport>('http');
  let fEndpoint = $state('');
  let fCliBinary = $state('');
  let fCliArgs = $state('');
  let fCliWorkdir = $state('');
  let fAuthHeader = $state('Authorization');
  let fApiKey = $state('');
  let fClearKey = $state(false);
  let fAllowLocal = $state(true);
  let fEnabled = $state(true);
  let fTimeout = $state(300);
  let fMaxMb = $state(128);

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      providers = await mapi.list3dProviders();
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    untrack(() => void load());
  });

  function openNew(): void {
    editing = 'new';
    formError = null;
    fName = '';
    fKind = 'custom_http';
    fTransport = 'http';
    fEndpoint = '';
    fCliBinary = '';
    fCliArgs = '';
    fCliWorkdir = '';
    fAuthHeader = 'Authorization';
    fApiKey = '';
    fClearKey = false;
    fAllowLocal = true;
    fEnabled = true;
    fTimeout = 300;
    fMaxMb = 128;
  }

  function openEdit(p: Media3dProvider): void {
    editing = p;
    formError = null;
    fName = p.name;
    fKind = p.kind;
    fTransport = p.transport;
    fEndpoint = p.endpoint_url ?? '';
    fCliBinary = p.cli_binary ?? '';
    fCliArgs = p.cli_args_template.join('\n');
    fCliWorkdir = p.cli_workdir ?? '';
    fAuthHeader = p.auth_header ?? 'Authorization';
    fApiKey = '';
    fClearKey = false;
    fAllowLocal = p.allow_local_network;
    fEnabled = p.enabled;
    fTimeout = p.timeout_secs || 300;
    fMaxMb = Math.max(16, Math.min(256, Math.round((p.max_output_bytes || 128 * 1024 * 1024) / (1024 * 1024))));
  }

  function kindLabel(k: Media3dProviderKind): string {
    switch (k) {
      case 'hunyuan3d_omni':
        return 'Hunyuan3D-Omni';
      case 'trellis_2':
        return 'TRELLIS.2';
      case 'custom_cli':
        return 'Custom CLI';
      case 'custom_http':
      default:
        return 'Custom HTTP';
    }
  }

  async function saveForm(): Promise<void> {
    if (!canAdmin || saving || !editing) return;
    const name = fName.trim();
    if (!name) {
      formError = 'Enter a provider name.';
      return;
    }
    saving = true;
    formError = null;
    const argsList = fCliArgs
      .split(/\r?\n/)
      .map((s) => s.trim())
      .filter(Boolean);
    try {
      if (editing === 'new') {
        await mapi.create3dProvider({
          name,
          kind: fKind,
          transport: fKind === 'custom_cli' ? 'local_cli' : fTransport,
          enabled: fEnabled,
          endpoint_url: fTransport === 'http' && fKind !== 'custom_cli' ? fEndpoint.trim() || null : null,
          cli_binary: fTransport === 'local_cli' || fKind === 'custom_cli' ? fCliBinary.trim() || null : null,
          cli_args_template: argsList,
          cli_workdir: fCliWorkdir.trim() || null,
          auth_header: fAuthHeader.trim() || null,
          api_key: fApiKey.trim() || undefined,
          allow_local_network: fAllowLocal,
          timeout_secs: Math.max(10, Math.min(3600, Math.round(fTimeout))),
          max_output_bytes: Math.max(16, Math.min(256, Math.round(fMaxMb))) * 1024 * 1024,
        });
        toasts.success('Added 3D provider', name);
      } else {
        const apiKeyPatch = fClearKey ? '' : fApiKey.trim() ? fApiKey.trim() : undefined;
        await mapi.update3dProvider(editing.id, {
          name,
          transport: fKind === 'custom_cli' ? 'local_cli' : fTransport,
          enabled: fEnabled,
          endpoint_url: fTransport === 'http' && fKind !== 'custom_cli' ? fEndpoint.trim() || null : null,
          cli_binary: fTransport === 'local_cli' || fKind === 'custom_cli' ? fCliBinary.trim() || null : null,
          cli_args_template: argsList,
          cli_workdir: fCliWorkdir.trim() || null,
          auth_header: fAuthHeader.trim() || null,
          ...(apiKeyPatch !== undefined ? { api_key: apiKeyPatch } : {}),
          allow_local_network: fAllowLocal,
          timeout_secs: Math.max(10, Math.min(3600, Math.round(fTimeout))),
          max_output_bytes: Math.max(16, Math.min(256, Math.round(fMaxMb))) * 1024 * 1024,
        });
        toasts.success('Updated 3D provider', name);
      }
      editing = null;
      await load();
      onchanged?.();
    } catch (e) {
      formError = loadErrorText(e);
    } finally {
      saving = false;
    }
  }

  async function removeProvider(p: Media3dProvider): Promise<void> {
    if (!canAdmin || p.builtin) return;
    const ok = await confirmer.ask(`Delete the 3D provider “${p.name}”? Its Keychain key will also be removed.`, {
      title: 'Delete 3D provider',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await mapi.delete3dProvider(p.id);
      toasts.success('Deleted provider', p.name);
      await load();
      onchanged?.();
    } catch (e) {
      toastError('Couldn’t delete the provider', e);
    }
  }

  async function runTest(p: Media3dProvider): Promise<void> {
    if (!canAdmin || testingId) return;
    testingId = p.id;
    try {
      const res = await mapi.test3dProvider(p.id);
      testResults = { ...testResults, [p.id]: res };
    } catch (e) {
      testResults = { ...testResults, [p.id]: { ok: false, latency_ms: null, message: loadErrorText(e) } };
    } finally {
      testingId = null;
    }
  }

  const showCliFields = $derived(fKind === 'custom_cli' || fTransport === 'local_cli');
</script>

{#snippet card(p: Media3dProvider)}
  {@const tr = testResults[p.id]}
  {@const licWarn = licenseWarningFor(p)}
  <li class="pm-card">
    <div class="pm-top">
      <div class="pm-title">
        <strong>{p.name}</strong>
        <Badge label={kindLabel(p.kind)} />
        <Badge
          tone={p.ready && p.enabled ? 'ok' : p.enabled ? 'warn' : 'neutral'}
          label={p.ready && p.enabled ? 'Ready' : p.enabled ? 'Needs setup' : 'Disabled'}
        />
        {#if licWarn}<Badge tone="warn" label="Territory restriction" />{/if}
        {#if p.has_api_key}<Badge tone="info" label="Keychain key" />{/if}
      </div>
      <div class="pm-acts">
        {#if canAdmin}
          <button
            type="button"
            class="btn small ghost"
            disabled={testingId === p.id}
            onclick={() => void runTest(p)}
          >{testingId === p.id ? 'Testing…' : 'Test'}</button>
          <button type="button" class="btn small" onclick={() => openEdit(p)}>Configure</button>
          {#if !p.builtin}
            <button
              type="button"
              class="icon-btn"
              aria-label={`Delete ${p.name}`}
              title={`Delete ${p.name}`}
              onclick={() => void removeProvider(p)}
            ><Icon name="trash" size={13} /></button>
          {/if}
        {/if}
      </div>
    </div>
    <p class="pm-detail">{p.status_detail}</p>
    {#if licWarn}
      <p class="pm-warn" data-testid="gen3d-license-warning"><Icon name="warning" size={12} /> {licWarn}</p>
    {/if}
    {#if p.capabilities?.supported_stages?.length}
      <span class="pm-mono">Stages: {p.capabilities.supported_stages.join(' · ')} · Max {Math.round((p.max_output_bytes || 128 * 1024 * 1024) / (1024 * 1024))} MiB</span>
    {/if}
    {#if p.endpoint_url}
      <span class="pm-mono">{p.endpoint_url}</span>
    {:else if p.cli_binary}
      <span class="pm-mono">{p.cli_binary} {p.cli_args_template.join(' ')}</span>
    {/if}
    {#if tr}
      <p class="pm-test" class:ok={tr.ok} class:bad={!tr.ok} use:announceShown={tr.message}>
        <Icon name={tr.ok ? 'check' : 'warning'} size={12} />
        <span>{tr.message}{tr.latency_ms != null ? ` (${tr.latency_ms} ms)` : ''}</span>
      </p>
    {/if}
  </li>
{/snippet}

<Modal title="3D generation providers" width={640} {onclose}>
  <div class="pm" data-testid="gen3d-provider-manager">
    {#if editing}
      <form
        class="pm-form"
        onsubmit={(e) => {
          e.preventDefault();
          void saveForm();
        }}
      >
        <div class="pm-fhead">
          <strong>{editing === 'new' ? 'Add 3D provider' : `Configure ${editing.name}`}</strong>
          <button type="button" class="btn small ghost" onclick={() => (editing = null)}>Back to list</button>
        </div>

        <div class="pm-grid">
          <div class="pm-field">
            <label for="p3d-name">Display name</label>
            <input id="p3d-name" dir="auto" class="input" bind:value={fName} placeholder="Local Hunyuan3D-Omni" required />
          </div>
          <div class="pm-field">
            <label for="p3d-kind">Provider type</label>
            <select id="p3d-kind" class="input" bind:value={fKind} disabled={editing !== 'new'}>
              <option value="hunyuan3d_omni">Tencent Hunyuan3D-Omni</option>
              <option value="trellis_2">Microsoft TRELLIS.2</option>
              <option value="custom_http">Custom HTTP API</option>
              <option value="custom_cli">Custom Local CLI</option>
            </select>
          </div>
        </div>

        {#if fKind !== 'custom_cli'}
          <div class="pm-field">
            <label for="p3d-transport">Execution transport</label>
            <select id="p3d-transport" class="input" bind:value={fTransport}>
              <option value="http">HTTP API server (local GPU or remote)</option>
              <option value="local_cli">Local CLI runner (argv-only, no shell)</option>
            </select>
          </div>
        {/if}

        {#if !showCliFields}
          <div class="pm-field">
            <label for="p3d-url">Endpoint URL</label>
            <input
              id="p3d-url"
              dir="ltr"
              type="url"
              class="input"
              bind:value={fEndpoint}
              placeholder="http://127.0.0.1:8080/generate"
            />
          </div>
          <div class="pm-grid">
            <div class="pm-field">
              <label for="p3d-auth">Auth header</label>
              <input id="p3d-auth" dir="ltr" class="input" bind:value={fAuthHeader} placeholder="Authorization" />
            </div>
            <div class="pm-field">
              <label for="p3d-key">API key (Keychain)</label>
              <input
                id="p3d-key"
                dir="ltr"
                type="password"
                class="input"
                bind:value={fApiKey}
                placeholder={editing !== 'new' && editing.has_api_key ? 'Saved in Keychain (leave blank to keep)' : 'Optional bearer/API key'}
              />
            </div>
          </div>
          {#if editing !== 'new' && editing.has_api_key}
            <label class="pm-check">
              <input type="checkbox" bind:checked={fClearKey} />
              <span>Remove saved Keychain API key</span>
            </label>
          {/if}
          <label class="pm-check">
            <input type="checkbox" bind:checked={fAllowLocal} />
            <span>Allow local/LAN GPU server endpoints (loopback &amp; private network; metadata always blocked)</span>
          </label>
        {:else}
          <div class="pm-field">
            <label for="p3d-bin">Executable path</label>
            <input id="p3d-bin" dir="ltr" class="input" bind:value={fCliBinary} placeholder="/opt/homebrew/bin/python3" />
          </div>
          <div class="pm-field">
            <label for="p3d-args">Arguments (one per line — supports &#123;stage&#125;, &#123;mesh&#125;, &#123;prompt&#125;, &#123;image&#125;, &#123;output&#125;, &#123;poly_budget&#125;, &#123;seed&#125;)</label>
            <textarea id="p3d-args" dir="ltr" rows="4" class="input" bind:value={fCliArgs} placeholder={'infer.py\n--image\n{image}\n--output\n{output}'}></textarea>
          </div>
          <div class="pm-field">
            <label for="p3d-cwd">Working directory (optional)</label>
            <input id="p3d-cwd" dir="ltr" class="input" bind:value={fCliWorkdir} placeholder="/Users/you/models/Hunyuan3D-Omni" />
          </div>
        {/if}

        <div class="pm-grid">
          <div class="pm-field">
            <label for="p3d-timeout">Timeout (seconds)</label>
            <input id="p3d-timeout" type="number" min="10" max="3600" class="input" bind:value={fTimeout} />
          </div>
          <div class="pm-field">
            <label for="p3d-maxmb">Max output size (MiB, up to 256)</label>
            <input id="p3d-maxmb" type="number" min="16" max="256" class="input" bind:value={fMaxMb} />
          </div>
        </div>
        <label class="pm-check">
          <input type="checkbox" bind:checked={fEnabled} />
          <span>Enabled in generator</span>
        </label>

        {#if formError}
          <p class="pm-err" use:announceShown={{ text: formError, assertive: true }}>
            <Icon name="warning" size={12} /> {formError}
          </p>
        {/if}

        <div class="pm-factions">
          <button type="button" class="btn" onclick={() => (editing = null)}>Cancel</button>
          <button type="submit" class="btn primary" disabled={saving || !canAdmin}>
            {saving ? 'Saving…' : 'Save provider'}
          </button>
        </div>
      </form>
    {:else}
      <div class="pm-bar">
        <p class="pm-dim">Otto Forge generates 3D models on this Mac. Other generators and custom HTTP / CLI pipelines are under Advanced; keys stay in Keychain.</p>
      </div>

      <LoadState what="3D providers" {loading} {error} empty={providers.length === 0} onretry={() => void load()}>
        <ul class="pm-list">
          {#each split.shown as p (p.id)}{@render card(p)}{/each}
        </ul>
        <details class="pm-adv" data-testid="gen3d-providers-advanced">
          <summary>Advanced</summary>
          <ul class="pm-list">
            {#each split.advanced as p (p.id)}{@render card(p)}{/each}
          </ul>
          {#if canAdmin}
            <button type="button" class="btn small" onclick={openNew} data-testid="gen3d-add-provider">
              <Icon name="plus" size={12} /> Add provider
            </button>
          {/if}
        </details>
      </LoadState>
    {/if}
  </div>

  {#snippet footer()}
    <button type="button" class="btn" onclick={onclose}>Done</button>
  {/snippet}
</Modal>

<style>
  .pm {
    display: flex;
    flex-direction: column;
    gap: 12px; /* the dialog body is the one scroll area */
  }
  .pm-bar,
  .pm-fhead,
  .pm-top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    flex-wrap: wrap;
  }
  .pm-dim {
    margin: 0;
    flex: 1;
    min-inline-size: 200px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .pm-adv {
    margin-block-start: 12px;
  }
  .pm-adv summary {
    cursor: pointer;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text-dim);
  }
  .pm-adv > * + * {
    margin-block-start: 8px;
  }
  .pm-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pm-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
  }
  .pm-title {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-m);
  }
  .pm-acts {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .pm-detail {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .pm-mono {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .pm-test {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
  }
  .pm-test.ok {
    color: var(--success);
  }
  .pm-test.bad {
    color: var(--danger);
  }
  .pm-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .pm-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  @media (max-width: 640px) {
    .pm-grid {
      grid-template-columns: 1fr;
    }
  }
  .pm-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .pm-field label {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .pm-check {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text);
    cursor: pointer;
  }
  .pm-warn {
    margin: 0;
    display: flex;
    align-items: flex-start;
    gap: 6px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--warning);
    font-size: var(--fs-xs);
  }
  .pm-err {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .pm-factions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  @media (pointer: coarse) {
    .pm :global(.input),
    .pm textarea,
    .pm select {
      min-block-size: 40px;
      font-size: 16px; /* ui-guards: allow */
    }
    .pm :global(.btn),
    .pm :global(.icon-btn) {
      min-block-size: 40px;
    }
    .pm :global(.icon-btn) {
      min-inline-size: 40px;
    }
  }
</style>
