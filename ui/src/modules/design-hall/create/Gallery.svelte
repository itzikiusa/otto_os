<script lang="ts">
  // My creations: everything made in this workspace, newest first — scenes,
  // characters and things, and voice lines — each opening its result. Things
  // still being made come first, with where they are ("Making 3 of 15"),
  // kept up while the page is open; "Nothing made yet" only when the daemon
  // really has nothing (a failed load says so, with Retry). The scenes of one
  // project stand together, after its name.
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { getProduction, listCreations } from '../../../lib/api/productions';
  import { pollWhileVisible } from '../../../lib/poll';
  import type { DesignArtifact, Production, ProductionSummary, TtsClip } from '../../../lib/api/types';
  import { loadErrorText } from '../../../lib/loadError';
  import { router } from '../../../lib/router.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import AudioLine from './AudioLine.svelte';
  import ItemThumb from './ItemThumb.svelte';
  import RigLine from './RigLine.svelte';
  import { firstPicture, stripState } from './progress';
  import { createStore, type CreationMeta } from './createStore.svelte';
  import { createHref, groupByProject, projectsOf, type ProjectChoice } from './plan';

  type Entry =
    | { key: string; at: string; type: 'scene'; p: Production }
    | { key: string; at: string; type: 'model'; a: DesignArtifact }
    | { key: string; at: string; type: 'clip'; c: TtsClip };

  let entries = $state<Entry[]>([]);
  let summaries = $state<ProductionSummary[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  const making = $derived(Object.values(createStore.jobs).filter((j) => j.status === 'queued' || j.status === 'running'));
  /** Creations started in this window that the last load hadn't seen yet (they always show). */
  const started = $derived(createStore.inProgress().filter((p) => !entries.some((e) => e.key === `p:${p.id}`)));
  // Ones still being worked on come first.
  const active = (e: Entry) => e.type === 'scene' && ['running', 'planning', 'ready'].includes((createStore.productions[e.p.id] ?? e.p).status);
  const shown = $derived.by<Entry[]>(() => {
    const all = [...started.map((p): Entry => ({ key: `p:${p.id}`, at: p.updated_at, type: 'scene', p })), ...entries];
    return [...all.filter(active), ...all.filter((e) => !active(e))];
  });
  const projects = $derived(projectsOf(summaries));
  const projectOf = (e: Entry): ProjectChoice | null => (e.type === 'scene' ? (projects.find((x) => x.id === (e.p.project_id ?? e.p.id)) ?? null) : null);
  const rows = $derived(groupByProject(shown, projectOf));

  $effect(() => {
    void ws.currentId;
    untrack(() => void load());
    // New and changing creations show up while the page is open (quietly: no loading state again).
    const poll = pollWhileVisible(() => load(true), { ms: 10_000, immediate: false });
    return () => poll.stop();
  });

  /** Only the latest load's answer is used (an older one that lands last is dropped). */
  let generation = 0;
  async function load(quiet = false): Promise<void> {
    const mine = ++generation;
    if (!quiet) loading = true;
    error = null;
    try {
      // Everything made in Create, whatever workspace is active (a failure is said, never an empty shelf).
      const { productions: listed, models: arts, clips } = await listCreations();
      if (mine !== generation) return;
      summaries = listed;
      // One card per scene: its pieces and lines live inside it, not loose on the shelf.
      // One that can't be read in full right now still shows, from its summary.
      const prods = (await Promise.all(listed.slice(0, 20).map((s) => getProduction(s.id).catch(() => s as unknown as Production)))).filter((p) => ['done', 'running', 'planning', 'ready', 'cancelled'].includes(p.status));
      // Creations being made are watched, so their cards (and the "In progress" strip) keep up.
      for (const p of prods) if (p.plan && (p.status === 'running' || p.status === 'planning')) createStore.watchProduction(p);
      const inside = new Set<string>();
      for (const p of prods) {
        if (p.scene_artifact_id) inside.add(p.scene_artifact_id);
        for (const i of p.plan?.items ?? []) for (const r of [i.result, i.previous_result]) for (const v of [r?.artifact_id, r?.clip_id]) if (v) inside.add(v);
      }
      if (mine !== generation) return;
      entries = [
        ...prods.map((p): Entry => ({ key: `p:${p.id}`, at: p.updated_at, type: 'scene', p })),
        ...arts.filter((a) => a.format === 'glb' && !inside.has(a.id)).map((a): Entry => ({ key: `a:${a.id}`, at: a.created_at, type: 'model', a })),
        ...clips.filter((c) => !inside.has(c.id)).map((c): Entry => ({ key: `c:${c.id}`, at: c.created_at, type: 'clip', c })),
      ].sort((x, y) => y.at.localeCompare(x.at));
    } catch (e) {
      // A quiet refresh that fails keeps what is shown.
      if (mine === generation && (!quiet || !entries.length)) error = loadErrorText(e);
    } finally {
      if (mine === generation) loading = false;
    }
  }

  const when = (iso: string): string => {
    const d = new Date(iso);
    const days = Math.floor((Date.now() - d.getTime()) / 86_400_000);
    return days <= 0 ? 'Today' : days === 1 ? 'Yesterday' : d.toLocaleDateString();
  };
  const kindWord = (a: DesignArtifact): string =>
    (a.meta as { create?: CreationMeta } | undefined)?.create?.kind === 'character' ? 'Person' : (a.meta as { create?: CreationMeta } | undefined)?.create?.kind === 'object' ? 'Thing' : 'Model';
</script>

{#snippet card(e: Entry)}
    {#if e.type === 'scene'}
      {@const live = createStore.productions[e.p.id] ?? e.p}
      {@const pic = firstPicture(live)}
      <button type="button" class="gl-card" onclick={() => router.go(createHref('p', e.p.id))} data-production={e.p.id}>
        <span class="gl-pic">{#if live.scene_artifact_id && !(active(e) && pic)}<ItemThumb artifactId={live.scene_artifact_id} icon="layers" size="fill" />{:else if pic}<ItemThumb artifactId={pic.artifact} preview={pic.preview} icon="layers" size="fill" />{:else}<Icon name="layers" size={24} />{/if}</span>
        <b>{e.p.plan?.title || e.p.description?.slice(0, 60)}</b>
        <small data-testid="create-gallery-state">{['running', 'planning', 'ready'].includes(live.status) ? stripState(live, createStore.soundReady === true) : when(e.at)}</small>
      </button>
    {:else if e.type === 'model'}
      <button type="button" class="gl-card" onclick={() => router.go(createHref('a', e.a.id))}>
        <span class="gl-pic"><ItemThumb artifactId={e.a.id} icon="box" size="fill" /></span>
        <b>{e.a.title}</b>
        <small>{kindWord(e.a)} · {when(e.at)}</small>
        {#if kindWord(e.a) === 'Person'}<RigLine artifactId={e.a.id} versionId={e.a.head_version_id} />{/if}
      </button>
    {:else}
      <div class="gl-card gl-clip">
        <AudioLine clipId={e.c.id} title={e.c.character_name || 'Voice'} detail={e.c.text.slice(0, 80)} />
        <small>{e.c.kind === 'music' ? 'Music' : e.c.kind === 'sound' ? 'Sound' : 'Voice'} · {when(e.at)}</small>
      </div>
    {/if}
{/snippet}

<h1 class="cr-h1">My creations</h1>
<p class="cr-sub">Everything you made, newest first.</p>

{#if making.length}
  <ul class="gl-grid gl-making">
    {#each making as j (j.id)}
      <li>
        <button type="button" class="gl-card" onclick={() => router.go(createHref('job', j.id))}>
          <span class="gl-pic"><span class="spinner" role="img" aria-label="Being made"></span></span>
          <b>{j.title}</b>
          <small>Still making…</small>
        </button>
      </li>
    {/each}
  </ul>
{/if}

<LoadState what="your creations" {loading} {error} empty={!shown.length && !making.length} onretry={() => void load()} variant="page">
  {#snippet emptyView()}
    <EmptyState variant="page" icon="folder" title="Nothing made yet" body="When you make something, it shows up here." actionLabel="Make something" onaction={() => router.go(createHref())} />
  {/snippet}
  <ul class="gl-grid" data-testid="create-gallery">
    {#each rows as e (e.key)}
      {#if 'label' in e}
        <li class="gl-group" data-testid="create-project-group">
          <h2 class="gl-label" data-testid="create-project-label"><Icon name="folder" size={14} /> {e.label}</h2>
          <ul class="gl-grid gl-inner">
            {#each e.items as x (x.key)}<li>{@render card(x)}</li>{/each}
          </ul>
        </li>
      {:else}
        <li>{@render card(e)}</li>
      {/if}
    {/each}
  </ul>
</LoadState>

<style>
  .gl-grid {
    list-style: none;
    margin: 0 0 var(--sp-4);
    padding: 0;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 170px), 1fr));
    align-items: start;
    gap: var(--sp-3);
  }
  /* A project's scenes: a whole row, its name over its own grid. */
  .gl-group {
    grid-column: 1 / -1;
  }
  .gl-label {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    margin: 0 0 var(--sp-2);
    color: var(--text-dim);
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .gl-inner {
    margin: 0;
  }
  .gl-card {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    inline-size: 100%;
    padding: var(--sp-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
    box-shadow: var(--shadow-card);
  }
  .gl-card:focus-visible {
    outline: 3px solid var(--accent-text);
    outline-offset: 2px;
  }
  .gl-clip {
    cursor: default;
  }
  .gl-pic {
    display: grid;
    place-items: center;
    aspect-ratio: 1;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text-dim);
    overflow: hidden;
  }
  .gl-card b {
    overflow-wrap: anywhere;
  }
  .gl-card small {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
