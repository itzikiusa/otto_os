<script lang="ts">
  // Native TTS Provider Manager: macOS Built-in Voices (always ready) +
  // OpenAI-compatible, ElevenLabs, Kokoro (local), and Custom HTTP endpoints.
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
  import type {
    ProviderTestResult,
    TtsProvider,
    TtsProviderKind,
  } from '../../../lib/api/types';

  interface Props {
    canAdmin: boolean;
    onclose: () => void;
    onchanged?: () => void;
  }
  let { canAdmin, onclose, onchanged }: Props = $props();

  let providers = $state<TtsProvider[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let editing = $state<TtsProvider | 'new' | null>(null);
  let testingId = $state<string | null>(null);
  let testResults = $state<Record<string, ProviderTestResult>>({});
  let saving = $state(false);
  let formError = $state<string | null>(null);

  let fName = $state('');
  let fKind = $state<Exclude<TtsProviderKind, 'macos_say'>>('openai_compat');
  let fEndpoint = $state('');
  let fModel = $state('');
  let fVoice = $state('');
  let fAuthHeader = $state('Authorization');
  let fApiKey = $state('');
  let fClearKey = $state(false);
  let fAllowLocal = $state(false);
  let fEnabled = $state(true);

  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      providers = await mapi.listTtsProviders();
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    untrack(() => void load());
  });

  function kindLabel(k: TtsProviderKind): string {
    switch (k) {
      case 'macos_say':
        return 'macOS Built-in';
      case 'openai_compat':
        return 'OpenAI-compatible';
      case 'elevenlabs':
        return 'ElevenLabs';
      case 'kokoro':
        return 'Kokoro Local';
      case 'custom_http':
      default:
        return 'Custom HTTP';
    }
  }

  function openNew(): void {
    editing = 'new';
    formError = null;
    fName = '';
    fKind = 'openai_compat';
    fEndpoint = 'https://api.openai.com/v1/audio/speech';
    fModel = 'gpt-4o-mini-tts';
    fVoice = 'alloy';
    fAuthHeader = 'Authorization';
    fApiKey = '';
    fClearKey = false;
    fAllowLocal = false;
    fEnabled = true;
  }

  function openEdit(p: TtsProvider): void {
    editing = p;
    formError = null;
    fName = p.name;
    fKind = p.kind === 'macos_say' ? 'openai_compat' : p.kind;
    fEndpoint = p.endpoint_url ?? '';
    fModel = p.default_model ?? '';
    fVoice = p.default_voice ?? '';
    fAuthHeader = p.auth_header ?? 'Authorization';
    fApiKey = '';
    fClearKey = false;
    fAllowLocal = p.allow_local_network;
    fEnabled = p.enabled;
  }

  function onKindChange(k: Exclude<TtsProviderKind, 'macos_say'>): void {
    fKind = k;
    if (editing !== 'new') return;
    if (k === 'kokoro') {
      fEndpoint = 'http://127.0.0.1:8880/v1/audio/speech';
      fModel = 'kokoro';
      fVoice = 'af_heart';
      fAllowLocal = true;
    } else if (k === 'elevenlabs') {
      fEndpoint = 'https://api.elevenlabs.io/v1';
      fModel = 'eleven_multilingual_v2';
      fVoice = '';
      fAuthHeader = 'xi-api-key';
      fAllowLocal = false;
    } else if (k === 'openai_compat') {
      fEndpoint = 'https://api.openai.com/v1/audio/speech';
      fModel = 'gpt-4o-mini-tts';
      fVoice = 'alloy';
      fAuthHeader = 'Authorization';
      fAllowLocal = false;
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
    try {
      if (editing === 'new') {
        await mapi.createTtsProvider({
          name,
          kind: fKind,
          enabled: fEnabled,
          endpoint_url: fEndpoint.trim() || null,
          default_model: fModel.trim() || null,
          default_voice: fVoice.trim() || null,
          auth_header: fAuthHeader.trim() || null,
          api_key: fApiKey.trim() || undefined,
          allow_local_network: fAllowLocal,
        });
        toasts.success('Added TTS provider', name);
      } else {
        const apiKeyPatch = fClearKey ? '' : fApiKey.trim() ? fApiKey.trim() : undefined;
        await mapi.updateTtsProvider(editing.id, {
          name,
          enabled: fEnabled,
          ...(editing.kind !== 'macos_say'
            ? {
                endpoint_url: fEndpoint.trim() || null,
                default_model: fModel.trim() || null,
                default_voice: fVoice.trim() || null,
                auth_header: fAuthHeader.trim() || null,
                allow_local_network: fAllowLocal,
              }
            : { default_voice: fVoice.trim() || null }),
          ...(apiKeyPatch !== undefined ? { api_key: apiKeyPatch } : {}),
        });
        toasts.success('Updated TTS provider', name);
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

  async function removeProvider(p: TtsProvider): Promise<void> {
    if (!canAdmin || p.builtin) return;
    const ok = await confirmer.ask(`Delete the TTS provider “${p.name}”? Its Keychain key will also be removed.`, {
      title: 'Delete TTS provider',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    try {
      await mapi.deleteTtsProvider(p.id);
      toasts.success('Deleted TTS provider', p.name);
      await load();
      onchanged?.();
    } catch (e) {
      toastError('Couldn’t delete the TTS provider', e);
    }
  }

  async function runTest(p: TtsProvider): Promise<void> {
    if (!canAdmin || testingId) return;
    testingId = p.id;
    try {
      const res = await mapi.testTtsProvider(p.id);
      testResults = { ...testResults, [p.id]: res };
    } catch (e) {
      testResults = { ...testResults, [p.id]: { ok: false, latency_ms: null, message: loadErrorText(e) } };
    } finally {
      testingId = null;
    }
  }
</script>

<Modal title="TTS voice providers" width={620} {onclose}>
  <div class="tpm" data-testid="tts-provider-manager">
    {#if editing}
      <form
        class="tpm-form"
        onsubmit={(e) => {
          e.preventDefault();
          void saveForm();
        }}
      >
        <div class="tpm-head">
          <strong>{editing === 'new' ? 'Add TTS provider' : `Configure ${editing.name}`}</strong>
          <button type="button" class="btn small ghost" onclick={() => (editing = null)}>Back to list</button>
        </div>

        <div class="tpm-grid">
          <div class="tpm-field">
            <label for="tts-p-name">Display name</label>
            <input id="tts-p-name" dir="auto" class="input" bind:value={fName} placeholder="Kokoro Local" required />
          </div>
          {#if editing === 'new'}
            <div class="tpm-field">
              <label for="tts-p-kind">Provider type</label>
              <select
                id="tts-p-kind"
                class="input"
                value={fKind}
                onchange={(e) => onKindChange((e.currentTarget as HTMLSelectElement).value as Exclude<TtsProviderKind, 'macos_say'>)}
              >
                <option value="openai_compat">OpenAI-compatible (/v1/audio/speech)</option>
                <option value="kokoro">Kokoro (local server)</option>
                <option value="elevenlabs">ElevenLabs</option>
                <option value="custom_http">Custom HTTP</option>
              </select>
            </div>
          {/if}
        </div>

        {#if editing === 'new' || editing.kind !== 'macos_say'}
          <div class="tpm-field">
            <label for="tts-p-url">Endpoint URL</label>
            <input id="tts-p-url" dir="ltr" type="url" class="input" bind:value={fEndpoint} placeholder="https://api.openai.com/v1/audio/speech" />
          </div>
          <div class="tpm-grid">
            <div class="tpm-field">
              <label for="tts-p-model">Default model</label>
              <input id="tts-p-model" dir="ltr" class="input" bind:value={fModel} placeholder="gpt-4o-mini-tts" />
            </div>
            <div class="tpm-field">
              <label for="tts-p-voice">Default voice ID</label>
              <input id="tts-p-voice" dir="ltr" class="input" bind:value={fVoice} placeholder="alloy" />
            </div>
          </div>
          <div class="tpm-grid">
            <div class="tpm-field">
              <label for="tts-p-auth">Auth header</label>
              <input id="tts-p-auth" dir="ltr" class="input" bind:value={fAuthHeader} placeholder="Authorization" />
            </div>
            <div class="tpm-field">
              <label for="tts-p-key">API key (Keychain)</label>
              <input
                id="tts-p-key"
                dir="ltr"
                type="password"
                class="input"
                bind:value={fApiKey}
                placeholder={editing !== 'new' && editing.has_api_key ? 'Saved in Keychain (blank keeps)' : 'API key'}
              />
            </div>
          </div>
          {#if editing !== 'new' && editing.has_api_key}
            <label class="tpm-check">
              <input type="checkbox" bind:checked={fClearKey} />
              <span>Remove saved Keychain API key</span>
            </label>
          {/if}
          <label class="tpm-check">
            <input type="checkbox" bind:checked={fAllowLocal} />
            <span>Allow local/LAN TTS server endpoints (e.g. local Kokoro)</span>
          </label>
        {:else}
          <div class="tpm-field">
            <label for="tts-p-defv">Default macOS voice</label>
            <input id="tts-p-defv" dir="auto" class="input" bind:value={fVoice} placeholder="Samantha" />
          </div>
        {/if}

        <label class="tpm-check">
          <input type="checkbox" bind:checked={fEnabled} />
          <span>Enabled in Voice Studio</span>
        </label>

        {#if formError}
          <p class="tpm-err" use:announceShown={{ text: formError, assertive: true }}>
            <Icon name="warning" size={12} /> {formError}
          </p>
        {/if}

        <div class="tpm-acts">
          <button type="button" class="btn" onclick={() => (editing = null)}>Cancel</button>
          <button type="submit" class="btn primary" disabled={saving || !canAdmin}>
            {saving ? 'Saving…' : 'Save provider'}
          </button>
        </div>
      </form>
    {:else}
      <div class="tpm-head">
        <p class="tpm-dim">macOS Built-in Voices work offline immediately. Add OpenAI-compatible, Kokoro, ElevenLabs, or custom HTTP engines.</p>
        {#if canAdmin}
          <button type="button" class="btn small primary" onclick={openNew} data-testid="tts-add-provider">
            <Icon name="plus" size={12} /> Add provider
          </button>
        {/if}
      </div>

      <LoadState what="TTS providers" {loading} {error} empty={providers.length === 0} onretry={() => void load()}>
        <ul class="tpm-list">
          {#each providers as p (p.id)}
            {@const tr = testResults[p.id]}
            <li class="tpm-card">
              <div class="tpm-head">
                <div class="tpm-title">
                  <strong>{p.name}</strong>
                  <Badge label={kindLabel(p.kind)} />
                  <Badge tone={p.ready && p.enabled ? 'ok' : 'warn'} label={p.ready && p.enabled ? 'Ready' : 'Needs setup'} />
                  {#if p.has_api_key}<Badge tone="info" label="Keychain key" />{/if}
                </div>
                <div class="tpm-acts">
                  {#if canAdmin}
                    <button type="button" class="btn small ghost" disabled={testingId === p.id} onclick={() => void runTest(p)}>
                      {testingId === p.id ? 'Testing…' : 'Test'}
                    </button>
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
              <p class="tpm-dim">{p.status_detail}</p>
              {#if tr}
                <p class="tpm-test" class:ok={tr.ok} class:bad={!tr.ok} use:announceShown={tr.message}>
                  <Icon name={tr.ok ? 'check' : 'warning'} size={12} /> {tr.message}
                </p>
              {/if}
            </li>
          {/each}
        </ul>
      </LoadState>
    {/if}
  </div>

  {#snippet footer()}
    <button type="button" class="btn" onclick={onclose}>Done</button>
  {/snippet}
</Modal>

<style>
  .tpm {
    display: flex;
    flex-direction: column;
    gap: 12px; /* the dialog body is the one scroll area */
  }
  .tpm-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    flex-wrap: wrap;
  }
  .tpm-dim {
    margin: 0;
    flex: 1;
    min-inline-size: 200px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .tpm-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .tpm-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
  }
  .tpm-title {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-m);
  }
  .tpm-acts {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
  }
  .tpm-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .tpm-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  @media (max-width: 640px) {
    .tpm-grid {
      grid-template-columns: 1fr;
    }
  }
  .tpm-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .tpm-field label {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .tpm-check {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .tpm-err {
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
  .tpm-test {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
  }
  .tpm-test.ok {
    color: var(--success);
  }
  .tpm-test.bad {
    color: var(--danger);
  }
  @media (pointer: coarse) {
    .tpm :global(.input),
    .tpm select {
      min-block-size: 40px;
      font-size: 16px; /* ui-guards: allow */
    }
    .tpm :global(.btn),
    .tpm :global(.icon-btn) {
      min-block-size: 40px;
    }
    .tpm :global(.icon-btn) {
      min-inline-size: 40px;
    }
  }
</style>
