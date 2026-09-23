<script lang="ts">
  // Right panel → References: search the team library (shipped work first) and
  // borrow from it — Add as reference (an explicit, pinned `references` link),
  // Start from this (a new draft forked from that version, `derived_from`),
  // Compare, Open. Below: the provenance of this design as a lineage list.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { isAbortError } from '../../lib/api/client';
  import { createArtifact, createLink, search } from '../../lib/api/design';
  import type { DesignArtifact, DesignArtifactFormat, DesignSearchHit, DesignStatus, DesignStudio } from '../../lib/api/types';
  import ArtifactThumb from './ArtifactThumb.svelte';
  import StudioBadge from './StudioBadge.svelte';
  import StatusPill from './StatusPill.svelte';
  import { STUDIOS, buildLineage, statusLabel, type LinkRow } from './model';
  import { library } from './library.svelte';
  import { openArtifact } from './nav';

  interface Props {
    artifact: DesignArtifact;
    uses: LinkRow[];
    readonly: boolean;
    seqOf: (versionId: string) => number | null;
    onreload: () => void;
    oncompare: (ref: DesignArtifact) => void;
  }
  let { artifact, uses, readonly, seqOf, onreload, oncompare }: Props = $props();

  let q = $state('');
  let studio = $state<'' | DesignStudio>('');
  let status = $state<'' | DesignStatus>('');
  let storyId = $state('');
  let since = $state<'' | '30' | '90' | '365'>('');
  let hits = $state<DesignSearchHit[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let ctl: AbortController | null = null;

  const stories = $derived(Object.values(library.stories).sort((a, b) => a.source_key.localeCompare(b.source_key)));

  async function run(): Promise<void> {
    ctl?.abort();
    const mine = new AbortController();
    ctl = mine;
    loading = true;
    try {
      const sinceIso = since ? new Date(Date.now() - Number(since) * 86_400_000).toISOString() : undefined;
      const r = await search(
        q.trim(),
        { studio: studio || undefined, status: status || undefined, story_id: storyId || undefined, since: sinceIso, limit: 30 },
        mine.signal,
      );
      if (ctl !== mine) return;
      hits = r.filter((h) => h.artifact.id !== artifact.id);
      error = null;
    } catch (e) {
      if (isAbortError(e) || ctl !== mine) return;
      error = e instanceof Error ? e.message : String(e);
    } finally {
      if (ctl === mine) loading = false;
    }
  }

  let timer: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    void q;
    void studio;
    void status;
    void storyId;
    void since;
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => untrack(() => void run()), 200);
    return () => {
      if (timer) clearTimeout(timer);
    };
  });

  const lineage = $derived(buildLineage(uses, seqOf));
  /** artifact id → its citation label (R1…) when this design already borrows from it. */
  const cited = $derived(new Map(lineage.filter((l) => l.cite && l.artifactId).map((l) => [l.artifactId!, l.cite!])));
  const referenced = (id: string) =>
    cited.has(id) ||
    uses.some((u) => u.other?.id === id && (u.link.rel === 'references' || u.link.rel === 'derived_from'));

  let busyId = $state<string | null>(null);

  async function addReference(ref: DesignArtifact): Promise<void> {
    busyId = ref.id;
    try {
      await createLink(artifact.id, { rel: 'references', dst_kind: 'artifact', dst_id: ref.id });
      toasts.success('Added as reference', `${ref.title} is pinned at v${ref.head_seq ?? '?'}.`);
      onreload();
    } catch (e) {
      toasts.error('Couldn’t add the reference', e instanceof Error ? e.message : String(e));
    } finally {
      busyId = null;
    }
  }

  async function startFrom(ref: DesignArtifact): Promise<void> {
    busyId = ref.id;
    try {
      const res = await createArtifact({
        workspace_id: ws.currentId ?? artifact.workspace_id,
        format: ref.format as DesignArtifactFormat,
        studio: ref.studio,
        title: `${artifact.title} (from ${ref.title})`,
        project_id: artifact.project_id ?? undefined,
        derived_from: { artifact_id: ref.id },
        message: `Started from ${ref.title}`,
      });
      toasts.success('Draft created', `“${res.artifact.title}” keeps a pinned link to ${ref.title}.`);
      openArtifact(res.artifact.id);
    } catch (e) {
      toasts.error('Couldn’t start from this design', e instanceof Error ? e.message : String(e));
    } finally {
      busyId = null;
    }
  }

  function more(e: MouseEvent, ref: DesignArtifact): void {
    ctxMenu.show(e, [
      { label: 'Compare with this design', icon: 'columns', action: () => oncompare(ref) },
      { label: 'Open', icon: 'external', action: () => openArtifact(ref.id) },
    ]);
  }

  const monthYear = (iso: string) => new Date(iso).toLocaleDateString(undefined, { month: 'short', year: 'numeric' });
</script>

<div class="refs" data-testid="design-references">
  <div class="search">
    <label class="box">
      <Icon name="search" size={14} />
      <input type="search" bind:value={q} placeholder="Search the team library" aria-label="Search references" data-testid="design-ref-search" />
    </label>
    <div class="filters">
      <select class="input" bind:value={studio} aria-label="Studio">
        <option value="">Studio: all</option>
        {#each STUDIOS as s (s.id)}<option value={s.id}>{s.name}</option>{/each}
      </select>
      <select class="input" bind:value={status} aria-label="Status">
        <option value="">Status: shipped first</option>
        {#each ['shipped', 'approved', 'review', 'draft'] as s (s)}<option value={s}>{statusLabel(s)}</option>{/each}
      </select>
      <select class="input" bind:value={storyId} aria-label="Linked story" disabled={!stories.length}>
        <option value="">Story: any</option>
        {#each stories as s (s.id)}<option value={s.id}>{s.source_key}</option>{/each}
      </select>
      <select class="input" bind:value={since} aria-label="Date">
        <option value="">Any time</option>
        <option value="30">Last 30 days</option>
        <option value="90">Last 90 days</option>
        <option value="365">Last year</option>
      </select>
    </div>
  </div>

  <div class="results">
    {#if error}
      <div class="err" role="alert"><Icon name="warning" size={14} /> Search failed. <button class="btn small ghost" onclick={() => void run()}>Retry</button></div>
    {:else if loading && !hits.length}
      <p class="dim" role="status">Searching the library…</p>
    {:else if !hits.length}
      <p class="dim">No designs match. Try fewer words or clear a filter.</p>
    {:else}
      <p class="count dim">{hits.length} result{hits.length === 1 ? '' : 's'} from the team library · shipped first</p>
      <ul>
        {#each hits as h (h.artifact.id)}
          {@const a = h.artifact}
          {@const isRef = referenced(a.id)}
          <li class="hit" class:cited={isRef} data-testid="design-ref-hit">
            <div class="top">
              <a class="pic" href={`#/design/a/${encodeURIComponent(a.id)}`} tabindex="-1" aria-hidden="true"><ArtifactThumb artifact={a} /></a>
              <div class="info">
                <div class="t">
                  <span class="name" title={a.title}>{a.title}</span>
                  {#if cited.get(a.id)}<span class="cite">{cited.get(a.id)}</span>{/if}
                </div>
                <div class="meta"><StudioBadge studio={a.studio} showName /> <span class="dim">· {monthYear(a.updated_at)}</span></div>
                <div class="meta">
                  {#each h.story_ids.slice(0, 2) as sid (sid)}
                    {#if library.stories[sid]}<span class="chip mono-chip">{library.stories[sid].source_key}</span>{/if}
                  {/each}
                  <StatusPill status={a.status} />
                </div>
                {#if h.reference_count}<div class="dim small">Used as reference in {h.reference_count} design{h.reference_count === 1 ? '' : 's'}</div>{/if}
                {#if h.snippet}<div class="snip dim small">{h.snippet.replace(/<\/?[^>]+>/g, '')}</div>{/if}
              </div>
            </div>
            <div class="acts">
              {#if isRef}
                <span class="done"><Icon name="check" size={12} /> Referenced</span>
              {:else if !readonly}
                <button class="btn small ghost" disabled={busyId === a.id} onclick={() => void addReference(a)} data-testid="design-ref-add">Add as reference</button>
              {/if}
              <button class="btn small ghost" disabled={busyId === a.id} onclick={() => void startFrom(a)}>Start from this</button>
              <span class="grow"></span>
              <button class="icon-btn" onclick={(e) => more(e, a)} aria-label={`More actions for ${a.title}`} title="More actions" aria-haspopup="menu">
                <Icon name="more" size={14} />
              </button>
            </div>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  <section class="prov" aria-labelledby="dh-prov-h">
    <h3 id="dh-prov-h">Provenance of this design</h3>
    <div class="prov-title">{artifact.title}{artifact.head_seq != null ? ` v${artifact.head_seq}` : ''}</div>
    {#if lineage.length}
      <ul class="lineage">
        {#each lineage as l, i (i)}
          <li>
            <span class="rel">{l.relText}</span>
            {#if l.artifactId}
              <a href={`#/design/a/${encodeURIComponent(l.artifactId)}`} class="lt">{l.title}</a>
            {:else}
              <span class="lt">{l.title}</span>
            {/if}
            <span class="dim small">{l.broken ? 'broken' : l.version}</span>
            {#if l.cite}<span class="cite">{l.cite}</span>{/if}
          </li>
        {/each}
      </ul>
    {:else}
      <p class="dim small">Made from scratch — no references, forks or embeds yet.</p>
    {/if}
  </section>
</div>

<style>
  .refs {
    display: flex;
    flex-direction: column;
    min-height: 100%;
  }
  .search {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-block-end: 1px solid var(--border);
    position: sticky;
    inset-block-start: 0;
    background: var(--surface);
    z-index: 2;
  }
  .box {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 27px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .box:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .box input {
    flex: 1;
    min-width: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    outline: none;
  }
  .filters {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
  }
  .filters select {
    height: 24px;
    padding-block: 0;
    font-size: var(--fs-xs);
  }
  .results {
    padding: 8px 14px 12px;
    flex: 1;
  }
  .count {
    margin: 0 0 6px;
    font-size: var(--fs-xs);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .hit {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    margin-block-end: 8px;
    overflow: hidden;
  }
  .hit.cited {
    border-color: var(--border-strong);
    border-inline-start: 3px solid var(--success);
  }
  .top {
    display: flex;
    gap: 10px;
    padding: 10px;
  }
  .pic {
    flex: none;
    width: 88px;
    height: 64px;
    border-radius: var(--radius-s);
    overflow: hidden;
    border: 1px solid var(--border);
  }
  .info {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .t {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .name {
    font-weight: 600;
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
    font-size: var(--fs-xs);
  }
  .mono-chip {
    font-family: var(--font-mono);
  }
  .snip {
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
  .cite {
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 0 5px;
    border-radius: var(--radius-s);
    color: var(--success);
    background: var(--success-soft);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 6px;
    border-block-start: 1px solid var(--border);
  }
  .done {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0 8px;
    font-size: var(--fs-s);
    color: var(--success);
  }
  .grow {
    flex: 1;
  }
  .err {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .err :global(svg) {
    color: var(--danger);
  }
  .prov {
    padding: 12px 14px 16px;
    border-block-start: 1px solid var(--border);
    background: var(--bg);
  }
  .prov h3 {
    margin: 0 0 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .prov-title {
    font-weight: 600;
    font-size: var(--fs-s);
    margin-block-end: 4px;
  }
  .lineage li {
    display: grid;
    grid-template-columns: 104px minmax(0, 1fr) auto auto;
    gap: 6px;
    align-items: baseline;
    padding: 3px 0 3px 12px;
    border-inline-start: 1px solid var(--border-strong);
    font-size: var(--fs-s);
  }
  .rel {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .lt {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-decoration: none;
  }
  a.lt:hover {
    color: var(--accent-text);
  }
  .dim {
    color: var(--text-dim);
  }
  p.dim {
    margin: 0;
    font-size: var(--fs-s);
  }
  .small {
    font-size: var(--fs-xs);
  }
</style>
