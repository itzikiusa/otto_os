<script lang="ts">
  // The one calm state when the maker (Otto Forge) isn't set up: what it
  // means, a way back, and a quiet re-check.
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import { router } from '../../../lib/router.svelte';
  import { createStore } from './createStore.svelte';
</script>

<div data-testid="create-kit-missing">
  <EmptyState
    variant="page"
    icon="box"
    title="Otto can’t make people and things yet"
    body="The maker isn’t set up yet. Ask the person who set up Otto to add it."
    actionLabel="Go back"
    actionIcon="chevronLeft"
    onaction={() => router.back()}
  >
    <button type="button" class="btn ghost" onclick={() => void createStore.loadEngine(true)}>{createStore.engine.loading ? 'Checking…' : 'Check again'}</button>
  </EmptyState>
</div>
