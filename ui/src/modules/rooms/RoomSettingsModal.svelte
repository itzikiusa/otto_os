<script lang="ts">
  import { onMount } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { api } from '../../lib/api/client';
  import type { RoomSettings, RoomSettingsUpdate } from '../../lib/api/room-types';
  let {onclose}: {onclose: () => void} = $props();
  let loading = $state(true), busy = $state(false), error = $state(''), loaded = $state(false), loadError = $state('');
  let origin = $state(''), stun = $state(''), turn = $state(''), secret = $state(''), configured = $state(false), relayOnly = $state(false), clearSecret = $state(false);
  onMount(() => { void load(); });
  async function load() {
    loading = true; error = ''; loadError = '';
    try { const settings = await api.get<RoomSettings>('/room-settings'); origin = settings.public_origin; stun = settings.stun_urls.join('\n'); turn = settings.turn_urls.join('\n'); configured = settings.turn_secret_configured; relayOnly = settings.relay_only; loaded = true; }
    catch (e) { loadError = loadErrorText(e); }
    finally { loading = false; }
  }
  async function save() {
    busy = true; error = '';
    const lines = (value: string) => value.split(/\r?\n/).map(v => v.trim()).filter(Boolean);
    const settings: RoomSettingsUpdate = {public_origin: origin.trim(), stun_urls: lines(stun), turn_urls: lines(turn), relay_only: relayOnly};
    if (secret || clearSecret) settings.turn_secret = clearSecret ? '' : secret;
    try { await api.put('/room-settings', settings); secret = ''; onclose(); }
    catch (e) { error = e instanceof Error ? e.message : 'Could not save room settings.'; }
    finally { busy = false; }
  }
</script>
<Modal title="Room connection settings" width={580} {onclose}>
  {#if !loaded}
    <!-- Never show the form until the saved values arrived: an empty form's Save would overwrite STUN/TURN with blanks. -->
    <LoadState what="room settings" variant="compact" {loading} error={loadError} empty={true} onretry={load} />
  {:else}
    <p>To invite people on other computers, configure an HTTPS address that reaches this Otto daemon. An address beginning with localhost or 127.0.0.1 only works on this computer.</p>
    <div class="fields">
      <label>Public HTTPS origin <input type="url" bind:value={origin} placeholder="https://otto.example.com" /></label>
      <label>STUN servers, one per line <textarea rows="2" bind:value={stun} placeholder="stun:stun.example.com:3478"></textarea></label>
      <label>TURN servers, one per line <textarea rows="2" bind:value={turn} placeholder="turns:turn.example.com:5349"></textarea></label>
      <label>TURN shared secret <input type="password" bind:value={secret} autocomplete="new-password" placeholder={configured ? 'Saved in Keychain — leave empty to keep' : 'Not configured'} disabled={clearSecret} /></label>
      {#if configured}<label class="check"><input type="checkbox" bind:checked={clearSecret} /> Remove the saved TURN secret</label>{/if}
      <label class="check"><input type="checkbox" bind:checked={relayOnly} /> Require the TURN relay for voice and screens</label>
    </div>
    <p>TURN helps voice and screen sharing cross different networks. The shared secret stays in the host’s Keychain; guests receive short-lived credentials.</p>
  {/if}
  {#if error}<p role="alert">{error}</p>{/if}
  {#snippet footer()}<button class="btn" onclick={onclose} disabled={busy}>Cancel</button><button class="btn primary" disabled={!loaded || loading || busy || !origin.trim()} onclick={save}>{busy ? 'Saving…' : 'Save settings'}</button>{/snippet}
</Modal>
<style>
  p { line-height: 1.5; color: var(--text-dim); } .fields, label { display: grid; gap: 8px; } .fields { gap: 16px; } input, textarea { width: 100%; } textarea { resize: vertical; } .check { display: flex; align-items: center; gap: 8px; } .check input { width: auto; } [role='alert'] { color: var(--danger); }
</style>
