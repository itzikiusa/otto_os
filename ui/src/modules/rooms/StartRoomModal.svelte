<script lang="ts">
  import Modal from '../../lib/components/Modal.svelte';
  import { api, baseUrl } from '../../lib/api/client';
  import type { RoomCredential } from '../../lib/api/room-types';
  import { rememberRoom } from './room-access';
  import { router } from '../../lib/router.svelte';
  let { sessionId, onclose }: {sessionId: string; onclose: () => void} = $props();
  let name = $state('');
  let busy = $state(false);
  let error = $state('');
  async function start() {
    busy = true; error = '';
    try {
      const credential = await api.post<RoomCredential>(`/sessions/${encodeURIComponent(sessionId)}/room`, {name: name.trim()});
      rememberRoom(baseUrl(), credential);
      onclose(); router.go(`rooms/${credential.room_id}`);
    } catch (e) { error = e instanceof Error ? e.message : 'Could not start this room. Try again.'; }
    finally { busy = false; }
  }
</script>
<Modal title="Start a session room" {onclose}>
  <p>Invite up to three people to this terminal and its available history. You decide who enters and who can type.</p>
  <p>Anyone you give control can run commands with this session’s permissions. Chat stays separate from the agent.</p>
  <label>Your display name <input maxlength="80" bind:value={name} autocomplete="nickname" /></label>
  {#if error}<p role="alert">{error}</p>{/if}
  {#snippet footer()}
    <button class="btn" onclick={onclose} disabled={busy}>Cancel</button>
    <button class="btn primary" disabled={busy || !name.trim()} onclick={start}>{busy ? 'Starting…' : 'Start room'}</button>
  {/snippet}
</Modal>
<style>
  p { line-height: 1.5; margin-block: 0 16px; }
  label { display: grid; gap: 8px; }
  input { width: 100%; }
  [role='alert'] { color: var(--danger); margin-block-start: 12px; }
</style>
