<script lang="ts">
  // Settings → Skills moved to Skills Lab, the one home for skills: the
  // bundled catalogue is the Lab's "Bundled" filter (install / update / remove
  // per skill, "Install all bundled skills" in the page ⋯). This route stays so
  // old links and bookmarks land there instead of on an unknown section.
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { router } from '../../lib/router.svelte';
  import { movedSectionRoute, sectionLabel } from './sections';

  const to = movedSectionRoute('skills') ?? 'skills-eval';
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
  <PageHeader title={sectionLabel('skills')} subtitle="Now in Skills Lab" />
  <PageBody width="readable">
    <EmptyState
      icon="box"
      title="Bundled skills live in Skills Lab"
      body="Install, update and remove the skills that ship with Otto from Skills Lab → Skills → Bundled."
      actionLabel="Open Skills Lab"
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
