<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import { radioKey } from '../../lib/radioKey';
  // "New design" sheet: title, studio (+ format where a studio has several),
  // an optional starter template, the project to file it in and the story it
  // implements. Creates a draft and hands its id back — nothing is generated.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { ApiError } from '../../lib/api/client';
  import { loadErrorText } from '../../lib/loadError';
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

  /** Studios a person can start a design in (every studio with a format). */
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
  let errorDetail = $state('');

  const formats = $derived(studioInfo(studio).formats);
  const templates = $derived(templatesFor(format));
  const stories = $derived(
    Object.values(library.stories).sort((a, b) => a.source_key.localeCompare(b.source_key)),
  );
  const storyOf = $derived(stories.find((s) => s.id === storyId));

  /** "Implements story": every story in the workspace, searched as you type. */
  function storyMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'None', pinned: true, checked: !storyId, action: () => (storyId = '') },
      ...stories.map((s) => ({ label: `${s.source_key} · ${s.title}`, icon: 'ticket', checked: s.id === storyId, action: () => (storyId = s.id) })),
    ];
    ctxMenu.show(e, items, { filter: true, filterPlaceholder: 'Find a story by key or title…', maxVisible: 12 });
  }

  /** A failed create, in words a person can act on (the raw text stays in the tooltip). */
  function createErrorText(e: unknown): string {
    if (e instanceof ApiError) {
      if (e.status === 403) return 'You can’t create designs in this workspace. Ask a workspace admin for Design Hall edit access.';
      if (e.status === 413) return 'That template or file is too large to save as a design.';
      if (e.status === 400 || e.status === 422) return 'Design Hall couldn’t accept this design. Check the title and format, then try again.';
    }
    return `Couldn’t create the draft. ${loadErrorText(e)}`;
  }
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
      error = createErrorText(e);
      errorDetail = e instanceof Error ? e.message : String(e);
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
      <input dir="auto" id="dh-new-title" class="input" bind:value={title} placeholder="Rewards landing hero" />
    </div>
    <div class="field">
      <span class="lbl" id="dh-new-studio">Studio</span>
      <div class="studios" role="radiogroup" aria-labelledby="dh-new-studio">
        {#each creatable as s (s.id)}
          <button
            type="button"
            role="radio"
            aria-checked={studio === s.id}
            tabindex={studio === s.id ? 0 : -1}
            onkeydown={radioKey}
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
        <span class="lbl" id="dh-new-story-l">Implements story</span>
        <button type="button" id="dh-new-story" class="input picker" onclick={storyMenu} aria-haspopup="menu"
          aria-labelledby="dh-new-story-l dh-new-story" disabled={!stories.length} data-testid="design-new-story"
          title={stories.length ? 'Search every story in this workspace' : 'No product stories are loaded for this workspace'}>
          <span class="pv">{storyOf ? `${storyOf.source_key} · ${storyOf.title}` : 'None'}</span>
          <Icon name="chevronDown" size={12} />
        </button>
      </div>
    </div>
    {#if error}
      <p class="err" use:announceShown={{ text: error, assertive: true }} title={errorDetail} data-testid="design-new-error">
        <Icon name="warning" size={13} /> {error}
      </p>
    {/if}
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
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .picker {
    display: flex;
    align-items: center;
    gap: 6px;
    text-align: start;
    cursor: pointer;
  }
  .pv {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
