<script lang="ts">
  import {onDestroy} from 'svelte';
  import {recapBlob} from './recap-client';
  let {recapId, imageId, title}: {recapId: string; imageId: string; title: string} = $props();
  let url = $state(''), busy = $state(false), error = $state('');
  let live = true;
  onDestroy(() => { live = false; if (url) URL.revokeObjectURL(url); });
  async function load() { busy = true; error = ''; try { const blob = await recapBlob(`/room-recaps/${encodeURIComponent(recapId)}/images/${encodeURIComponent(imageId)}`); if (live) url = URL.createObjectURL(blob); } catch { if (live) error = 'Could not load this screen sample. Try again.'; } finally { busy = false; } }
</script>
{#if url}<img src={url} alt={`Recorded sample: ${title}`} />{:else}<button class="btn small" disabled={busy} onclick={load}>{busy ? 'Loading sample…' : error ? 'Retry screen sample' : 'Show screen sample'}</button>{/if}
{#if error}<p role="alert">{error}</p>{/if}
<style>img { display: block; max-width: 100%; max-height: 420px; object-fit: contain; } p { color: var(--danger); }</style>
