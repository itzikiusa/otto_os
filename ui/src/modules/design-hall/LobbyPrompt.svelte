<script lang="ts">
  // The lobby's one-line prompt: studio ▾ · the brief · ⋯ · Generate (the
  // page's ONE primary action). Generate creates the draft and hands the brief
  // to Otto (one turn, or a variants run), then opens it on the Otto tab.
  // The ⋯ popover holds the rest: "Draft only" (save the brief on an empty
  // draft, no agent), how many variants, whether to offer the team's closest
  // past designs as references, and the project to file it in. A small line
  // under the box says what non-default options are set, so nothing hides.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import type { DesignSearchHit, DesignStudio } from '../../lib/api/types';
  import StudioBadge from './StudioBadge.svelte';
  import { STUDIOS, studioInfo, titleFromPrompt } from './model';
  import { createDesign } from './create';
  import { generateFromBrief, referenceOf, suggestReferences } from './assist/handoff';
  import { library } from './library.svelte';

  interface Props {
    /** Disabled with a reason when the person can't create designs. */
    canEdit: boolean;
  }
  let { canEdit }: Props = $props();

  let prompt = $state('');
  let studio = $state<DesignStudio>('frames');
  let projectId = $state('');
  let variants = $state(1);
  let useRefs = $state(true);
  let creating = $state(false);
  const studios = STUDIOS.filter((s) => s.formats.length > 0);
  const projects = $derived(library.projects.filter((p) => !p.archived));
  const projectName = $derived(projects.find((p) => p.id === projectId)?.name ?? null);
  const ready = $derived(!!prompt.trim() && !creating && canEdit);

  /** The team's best matches for the brief — offered to Otto first as R1..R3. */
  let refHits = $state<DesignSearchHit[]>([]);
  let refCtl: AbortController | null = null;
  let refTimer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const p = prompt.trim();
    const on = useRefs;
    const wsId = ws.currentId;
    untrack(() => {
      if (refTimer) clearTimeout(refTimer);
      if (!on || p.length < 4 || !wsId) {
        refCtl?.abort();
        refHits = [];
        return;
      }
      refTimer = setTimeout(() => void loadRefs(p, wsId), 450);
    });
  });
  async function loadRefs(p: string, wsId: string): Promise<void> {
    refCtl?.abort();
    const mine = new AbortController();
    refCtl = mine;
    try {
      const hits = await suggestReferences(p, wsId, mine.signal);
      if (refCtl === mine) refHits = hits;
    } catch {
      /* no preview; Otto still searches the library itself */
    }
  }

  function workspace(): string | null {
    const wsId = ws.currentId;
    if (!wsId) toasts.warn('Pick a workspace first', 'New designs are filed under a workspace.');
    return wsId;
  }

  async function generate(): Promise<void> {
    const wsId = workspace();
    if (!wsId || !ready) return;
    creating = true;
    try {
      const r = await generateFromBrief({
        workspaceId: wsId,
        studio,
        format: studioInfo(studio).formats[0],
        title: titleFromPrompt(prompt),
        projectId: projectId || null,
        brief: prompt,
        variants,
        references: useRefs ? refHits.map(referenceOf) : [],
      });
      prompt = '';
      if (r.assistError) toasts.warn('Draft created, but Otto couldn’t start', r.assistError);
      router.go(`design/a/${encodeURIComponent(r.artifactId)}/otto`);
    } catch (e) {
      toastError('Couldn’t create the draft', e);
    } finally {
      creating = false;
    }
  }

  async function draftOnly(): Promise<void> {
    const wsId = workspace();
    if (!wsId || !ready) return;
    creating = true;
    try {
      const id = await createDesign({
        workspaceId: wsId,
        studio,
        format: studioInfo(studio).formats[0],
        title: titleFromPrompt(prompt),
        projectId: projectId || null,
        brief: prompt,
      });
      prompt = '';
      router.go(`design/a/${encodeURIComponent(id)}`);
    } catch (e) {
      toastError('Couldn’t create the draft', e);
    } finally {
      creating = false;
    }
  }

  function studioMenu(e: MouseEvent): void {
    ctxMenu.show(
      e,
      studios.map((s) => ({ label: s.name, icon: s.icon, checked: s.id === studio, hint: s.blurb, action: () => (studio = s.id) })),
    );
  }

  function projectMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'No project', pinned: true, checked: !projectId, action: () => (projectId = '') },
      ...projects.map((p) => ({ label: p.name, icon: 'folder', checked: p.id === projectId, action: () => (projectId = p.id) })),
    ];
    ctxMenu.show(e, items, { filter: projects.length > 8, filterPlaceholder: 'Find a project…', maxVisible: 12 });
  }

  function optionsMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      {
        label: 'Draft only',
        icon: 'file',
        hint: 'no agent',
        disabled: !ready,
        title: ready ? 'Save the brief on an empty draft, without asking Otto' : 'Describe the design first',
        action: () => void draftOnly(),
      },
      { separator: true },
      ...[1, 2, 3, 4].map((n) => ({
        label: n === 1 ? 'One direction' : `${n} variants to pick from`,
        checked: variants === n,
        action: () => (variants = n),
      })),
      { separator: true },
      {
        label: 'Use team references',
        checked: useRefs,
        title: 'Offer your team’s closest past designs to Otto first, as references it can cite',
        action: () => (useRefs = !useRefs),
      },
      {
        label: projectName ? `Project: ${projectName}…` : 'File in a project…',
        icon: 'folder',
        action: () => queueMicrotask(() => projectMenu(e)),
      },
    ]);
  }

  const summary = $derived(
    [variants > 1 ? `${variants} variants` : null, projectName ? `in ${projectName}` : null, useRefs ? null : 'no references']
      .filter(Boolean)
      .join(' · '),
  );
</script>

<div class="prompt" data-testid="design-prompt-box">
  <button class="studio-pick" onclick={studioMenu} aria-haspopup="menu" aria-label={`Studio: ${studioInfo(studio).name}`}
    title="The studio Otto drafts in" data-testid="design-prompt-studio">
    <StudioBadge {studio} /> <span class="sname">{studioInfo(studio).name}</span> <Icon name="chevronDown" size={12} />
  </button>
  <input
    dir="auto"
    class="brief"
    type="text"
    placeholder="Describe it… e.g. “a launch page for the loyalty programme”"
    aria-label="Describe the design"
    bind:value={prompt}
    onkeydown={(e) => {
      if (e.key === 'Enter' && !e.isComposing && ready) {
        e.preventDefault();
        void generate();
      }
    }}
    data-testid="design-prompt"
  />
  <span class="go">
    <button class="icon-btn" onclick={optionsMenu} aria-haspopup="menu" aria-label="Draft options" data-testid="design-prompt-more"
      title="Draft options — draft only, variants, references, project">
      <Icon name="more" size={14} />
    </button>
    <button class="btn primary" disabled={!ready} onclick={() => void generate()} data-testid="design-prompt-generate"
      title={canEdit ? 'Create the draft and ask Otto to design it (Enter)' : 'You can view designs but not create them'}>
      <Icon name="sparkle" size={13} /> {creating ? 'Starting…' : 'Generate'}
    </button>
  </span>
</div>
{#if summary || (useRefs && refHits.length)}
  <div class="opts" data-testid="design-prompt-ref-chips">
    {#if summary}<span class="dim">{summary}</span>{/if}
    {#if useRefs && refHits.length}
      <span class="dim">References</span>
      {#each refHits as h, i (h.artifact.id)}
        <a class="chip ref" href={`#/design/a/${encodeURIComponent(h.artifact.id)}`} title={`${h.artifact.title} (${h.artifact.status}) — offered as R${i + 1}`}>
          R{i + 1} · {h.artifact.title}
        </a>
      {/each}
    {/if}
  </div>
{/if}

<style>
  .prompt {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .prompt:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .studio-pick {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-block-size: 30px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .studio-pick:hover {
    border-color: var(--border-strong);
  }
  .brief {
    flex: 1 1 220px;
    min-width: 0;
    min-block-size: 30px;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    padding-inline: 4px;
    outline: none;
  }
  .go {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-inline-start: auto;
  }
  .opts {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-block-start: 8px;
    font-size: var(--fs-s);
  }
  .ref {
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    text-decoration: none;
  }
  .ref:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .dim {
    color: var(--text-dim);
  }
  /* A phone: the brief takes its own row, the controls share the next. */
  @media (max-width: 640px) {
    .brief {
      order: -1;
      flex-basis: 100%;
      min-block-size: var(--hit-min);
    }
    .studio-pick {
      min-block-size: var(--hit-min);
    }
  }
</style>
