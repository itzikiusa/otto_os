<script lang="ts">
  import {onMount} from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import {loadErrorText} from '../../lib/loadError';
  import {recapRequest} from './recap-client';
  import type {RecapCapabilities, RecapEngineSettings} from '../../lib/api/room-recap-types';
  let {onclose}: {onclose: () => void} = $props();
  let config = $state<RecapEngineSettings>({whisper_executable: '', whisper_model: '', language: 'auto', threads: 2});
  let capability = $state<RecapCapabilities | null>(null), loading = $state(true), busy = $state(false), error = $state(''), loaded = $state(false), loadError = $state('');
  onMount(() => { void load(); });
  async function load() { loading = true; error = ''; loadError = ''; try { [config, capability] = await Promise.all([recapRequest<RecapEngineSettings>('/room-recap-settings'), recapRequest<RecapCapabilities>('/room-recap-capabilities')]); loaded = true; } catch (e) { loadError = loadErrorText(e); if (loaded) error = loadError; } finally { loading = false; } }
  async function save() { busy = true; error = ''; try { await recapRequest('/room-recap-settings', config, undefined, 'PUT'); await load(); } catch (e) { error = e instanceof Error ? e.message : 'Could not save settings.'; } finally { busy = false; } }
</script>
<Modal title="Local recap engines" width={600} {onclose}>
  <p>Spoken transcription runs locally with whisper.cpp and a multilingual model. Summary generation uses your signed-in Codex subscription. No API key is needed.</p>
  {#if !loaded}
    <!-- Never offer the form before the saved values arrived: Save would overwrite them with defaults. -->
    <LoadState what="local engine settings" variant="compact" {loading} error={loadError} empty={true} onretry={load} />
  {:else}<div class="fields">
    <label>whisper.cpp executable <input dir="ltr" bind:value={config.whisper_executable} placeholder="/absolute/path/to/whisper-cli" /></label>
    <label>Multilingual model file <input dir="ltr" bind:value={config.whisper_model} placeholder="/absolute/path/to/ggml-model.bin" /></label>
    <label>Speech language <select bind:value={config.language}><option value="auto">Detect automatically</option><option value="en">English</option><option value="he">Hebrew</option></select></label>
    <label>Recognition threads <select bind:value={config.threads}>{#each [1, 2, 3, 4] as threads}<option value={threads}>{threads}</option>{/each}</select></label>
  </div>
  {#if capability}<dl><dt>Speech recognition</dt><dd>{capability.speech_ready ? 'Ready' : capability.configuration_error ?? 'Setup needed'}</dd><dt>Codex subscription</dt><dd>{capability.codex_subscription_ready ? 'Ready' : 'Sign in to the local Codex CLI'}</dd><dt>Screen text recognition</dt><dd>{capability.screen_text_ready ? 'Ready' : 'Unavailable on this computer'}</dd></dl><p>{capability.setup}</p>{/if}{/if}
  {#if error}<p role="alert">{error}</p>{/if}
  {#snippet footer()}<button class="btn" onclick={onclose}>Done</button><button class="btn primary" disabled={!loaded || loading || busy} onclick={save}>{busy ? 'Checking…' : 'Save and check'}</button>{/snippet}
</Modal>
<style>
  p { line-height: 1.5; color: var(--text-dim); } .fields, label { display: grid; gap: 8px; } .fields { gap: 16px; } input { width: 100%; } dl { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 2fr); gap: 8px; } dd { margin: 0; overflow-wrap: anywhere; } [role='alert'] { color: var(--danger); }
</style>
