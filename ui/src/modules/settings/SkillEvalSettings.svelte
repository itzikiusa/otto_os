<script lang="ts">
  // Settings → Skills evaluator moved to Skills Lab, the one home for skills:
  // the defaults are a sheet in the Evaluator tab (page ⋯ → Evaluator
  // defaults). This route stays so old links open that sheet.
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { router } from '../../lib/router.svelte';
  import { movedSectionRoute, sectionLabel } from './sections';

  const to = movedSectionRoute('skill-eval') ?? 'skills-eval/evaluator';
  // First move Settings' own resume point off this route (else the sidebar's
  // Settings entry would bounce here, and on to the Lab, every time), then go.
  function forward(): void {
    router.replace('settings');
    router.go(to);
  }
  $effect(() => {
    untrack(forward);
  });
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('skill-eval')} subtitle="Now in Skills Lab" />
  <PageBody width="readable">
    <EmptyState
      icon="target"
      title="Evaluator defaults live in Skills Lab"
      body="Open Skills Lab → Evaluator and choose Evaluator defaults in the page’s ⋯ menu."
      actionLabel="Open the defaults"
      onaction={forward}
    />
  </PageBody>
</div>

<style>
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
</style>
