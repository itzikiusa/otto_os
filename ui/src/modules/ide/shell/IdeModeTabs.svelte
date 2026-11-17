<script lang="ts">
  // Optional Project | Scratches tabs in the IDE header (Settings › Appearance
  // › "Show Scratches as a tab"). Project returns to the last real project;
  // Scratches opens the default project. A switch can be refused (unsaved
  // edits), so the tabs click rather than focus-then-select.
  import { tabKeys } from '../../../lib/tabKeys';
  import { onScratches, openReturnProject, openScratches, returnProject } from './ideScratches.svelte';

  const active = $derived(onScratches() ? 'scratches' : 'project');
  // Nothing to go back to yet (no named project, no workspace folder).
  const noProject = $derived(onScratches() && returnProject() === null);

  async function pick(e: MouseEvent, id: 'project' | 'scratches'): Promise<void> {
    if (id === active) return;
    const btn = e.currentTarget as HTMLButtonElement;
    const ok = id === 'scratches' ? await openScratches() : await openReturnProject();
    if (ok) btn.focus({ preventScroll: true });
  }
</script>

<div class="segmented" role="tablist" aria-label="IDE mode" tabindex="-1" onkeydown={tabKeys({ activate: 'click' })} data-testid="ide-mode-tabs">
  <button
    type="button"
    role="tab"
    data-testid="ide-mode-project"
    class:active={active === 'project'}
    aria-selected={active === 'project'}
    tabindex={active === 'project' ? 0 : -1}
    disabled={noProject}
    title={noProject ? 'No other project yet — open a folder or create a project' : undefined}
    onclick={(e) => void pick(e, 'project')}
  >Project</button>
  <button
    type="button"
    role="tab"
    data-testid="ide-mode-scratches"
    class:active={active === 'scratches'}
    aria-selected={active === 'scratches'}
    tabindex={active === 'scratches' ? 0 : -1}
    onclick={(e) => void pick(e, 'scratches')}
  >Scratches</button>
</div>
