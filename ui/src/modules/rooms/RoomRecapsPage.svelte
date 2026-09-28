<script lang="ts">
  import {onMount} from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import RecapPanel from './RecapPanel.svelte';
  import RecapSettingsModal from './RecapSettingsModal.svelte';
  import {recapRequest} from './recap-client';
  import {auth} from '../../lib/stores/auth.svelte';
  import {router} from '../../lib/router.svelte';
  import type {RecapMetadata} from '../../lib/api/room-recap-types';
  let archives = $state<RecapMetadata[]>([]), selected = $state(''), loading = $state(true), error = $state(''), settings = $state(false);
  onMount(() => { void load(); });
  async function load() { loading = true; error = ''; try { archives = await recapRequest<RecapMetadata[]>('/room-recaps'); if (!archives.some(a => a.id === selected)) selected = archives[0]?.id ?? ''; } catch (e) { error = e instanceof Error ? e.message : 'Could not load recap archives.'; } finally { loading = false; } }
</script>
<div class="recaps-page"><PageHeader title="Room recaps" icon="people" subtitle="Your local archives">
  {#snippet actions()}<button class="btn" onclick={() => router.go('rooms')}>Rooms</button>{#if auth.isRoot}<button class="btn" onclick={() => settings = true}>Local engines…</button>{/if}<button class="btn" onclick={load}>Refresh</button>{/snippet}
</PageHeader><PageBody>
  {#if loading}<p role="status">Loading recaps…</p>{:else if error}<p role="alert">{error}</p><button class="btn" onclick={load}>Retry</button>
  {:else if !archives.length}<EmptyState variant="page" icon="people" title="No room recaps yet" body="Prepare a recap from a room, ask everyone for consent, then start capture. Archives remain here after a room ends." />
  {:else}<label>Archive <select bind:value={selected}>{#each archives as archive}<option value={archive.id}>{archive.session_title} · {new Date(archive.created_at).toLocaleString()}</option>{/each}</select></label>{#key selected}<RecapPanel recapId={selected} />{/key}{/if}
</PageBody></div>
{#if settings}<RecapSettingsModal onclose={() => settings = false} />{/if}
<style>.recaps-page { display: flex; flex-direction: column; height: 100%; min-width: 0; } label { display: grid; gap: 8px; margin-block-end: 16px; } select { max-width: 100%; } [role='alert'] { color: var(--danger); }</style>
