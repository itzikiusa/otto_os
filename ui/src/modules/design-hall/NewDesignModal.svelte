<script lang="ts">
  // "New design" sheet: title, studio (+ format where a studio has several),
  // an optional starter template, the project to file it in and the story it
  // implements. Creates a draft and hands its id back — nothing is generated.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { DesignStudio } from '../../lib/api/types';
  import { STUDIOS, formatLabel, studioInfo } from './model';
  import { createDesign, templatesFor } from './create';
  import { library } from './library.svelte';

  interface Props {
    studio?: DesignStudio;
    format?: string;
    templateId?: string;
    projectId?: string | null;
    storyId?: string | null;
    onclose: () => void;
    oncreated: (id: string) => void;
  }
  let {
    studio: initialStudio = 'frames',
    format: initialFormat,
    templateId: initialTemplate = '',
    projectId: initialProject = null,
    storyId: initialStory = null,
    onclose,
    oncreated,
  }: Props = $props();

  /** Studios a person can start in today (Site and Spatial are planned). */
  const creatable = STUDIOS.filter((s) => s.formats.length > 0);

  // The sheet is seeded once from its props; after that the fields are the user's.
  const seed = untrack(() => ({
    studio: initialStudio,
    format: initialFormat ?? studioInfo(initialStudio).formats[0] ?? 'html',
    templateId: initialTemplate,
    projectId: initialProject ?? '',
    storyId: initialStory ?? '',
  }));
  let title = $state('');
  let studio = $state<DesignStudio>(seed.studio);
  let format = $state(seed.format);
  let templateId = $state(seed.templateId);
  let projectId = $state(seed.projectId);
  let storyId = $state(seed.storyId);
  let busy = $state(false);
  let error = $state<string | null>(null);

  const formats = $derived(studioInfo(studio).formats);
  const templates = $derived(templatesFor(format));
  const stories = $derived(
    Object.values(library.stories).sort((a, b) => a.source_key.localeCompare(b.source_key)),
  );
  const projects = $derived(library.projects.filter((p) => !p.archived));

  function pickStudio(s: DesignStudio): void {
    studio = s;
    const f = studioInfo(s).formats;
    if (!f.includes(format)) format = f[0] ?? 'html';
    templateId = '';
  }

  async function create(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      error = 'Pick a workspace first — new designs are filed under a workspace.';
      return;
    }
    busy = true;
    error = null;
    try {
      const id = await createDesign({
        workspaceId: wsId,
        studio,
        format,
        title: title || (templateId ? (templates.find((t) => t.id === templateId)?.name ?? '') : ''),
        projectId: projectId || null,
        storyId: storyId || null,
        templateId: templateId || undefined,
      });
      toasts.success('Draft created');
      oncreated(id);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="New design" width={520} {onclose}>
  <form
    class="form"
    onsubmit={(e) => {
      e.preventDefault();
      void create();
    }}
  >
    <div class="field">
      <label for="dh-new-title">Title</label>
      <input id="dh-new-title" class="input" bind:value={title} placeholder="Rewards landing hero" />
    </div>
    <div class="field">
      <span class="lbl" id="dh-new-studio">Studio</span>
      <div class="studios" role="radiogroup" aria-labelledby="dh-new-studio">
        {#each creatable as s (s.id)}
          <button
            type="button"
            role="radio"
            aria-checked={studio === s.id}
            class="pill-toggle"
            class:on={studio === s.id}
            onclick={() => pickStudio(s.id)}>{s.name}</button
          >
        {/each}
      </div>
      <span class="hint">{studioInfo(studio).note}</span>
    </div>
    {#if formats.length > 1}
      <div class="field">
        <label for="dh-new-format">Format</label>
        <select id="dh-new-format" class="input" bind:value={format} onchange={() => (templateId = '')}>
          {#each formats as f (f)}<option value={f}>{formatLabel(f)}</option>{/each}
        </select>
      </div>
    {/if}
    {#if templates.length}
      <div class="field">
        <label for="dh-new-template">Start from</label>
        <select id="dh-new-template" class="input" bind:value={templateId}>
          <option value="">Blank</option>
          {#each templates as t (t.id)}<option value={t.id}>{t.name} — {t.description}</option>{/each}
        </select>
      </div>
    {/if}
    <div class="row2">
      <div class="field">
        <label for="dh-new-project">Project</label>
        <select id="dh-new-project" class="input" bind:value={projectId}>
          <option value="">No project</option>
          {#each projects as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
        </select>
      </div>
      <div class="field">
        <label for="dh-new-story">Implements story</label>
        <select id="dh-new-story" class="input" bind:value={storyId} disabled={!stories.length}
          title={stories.length ? undefined : 'No product stories are loaded for this workspace'}>
          <option value="">None</option>
          {#each stories as s (s.id)}<option value={s.id}>{s.source_key} · {s.title}</option>{/each}
        </select>
      </div>
    </div>
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </form>
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={create} disabled={busy} data-testid="design-new-create">
      {busy ? 'Creating…' : 'Create draft'}
    </button>
  {/snippet}
</Modal>

<style>
  .form {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .lbl {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .studios {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .row2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  @media (max-width: 640px) {
    .row2 {
      grid-template-columns: 1fr;
    }
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-s);
  }
</style>
