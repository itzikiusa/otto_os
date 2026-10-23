<script lang="ts">
  import {onDestroy} from 'svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import {loadErrorText} from '../../lib/loadError';
  import {recapBlob} from './recap-client';
  let {recapId, imageId, title}: {recapId: string; imageId: string; title: string} = $props();
  let url = $state(''), busy = $state(false), error = $state('');
  let live = true;
  onDestroy(() => { live = false; if (url) URL.revokeObjectURL(url); });
  // A failure stays on screen (LoadState's alert + Retry) until a retry answers,
  // so the focused Retry is not unmounted while the sample loads again.
  async function load() { busy = true; try { const blob = await recapBlob(`/room-recaps/${encodeURIComponent(recapId)}/images/${encodeURIComponent(imageId)}`); if (live) { url = URL.createObjectURL(blob); error = ''; } } catch (e) { if (live) error = loadErrorText(e); } finally { busy = false; } }
</script>
{#if url}<img src={url} alt={`Recorded sample: ${title}`} />
{:else if error}<LoadState what="this screen sample" variant="compact" loading={busy} {error} empty onretry={load} />
{:else}<button class="btn small" disabled={busy} onclick={load}>{busy ? 'Loading sample…' : 'Show screen sample'}</button>{/if}
<style>img { display: block; max-width: 100%; max-height: 420px; object-fit: contain; }</style>
