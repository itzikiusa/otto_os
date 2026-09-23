<script lang="ts">
  // Product → story → Design tab: the Design Hall view of this story. A small
  // graph — the story, the designs that implement it (an epic also shows its
  // children's), and what each of those designs uses — with "Open in Design
  // Hall". It sits above the unchanged Design Arena and stays out of the way:
  // collapsible (remembered per device) and silent when Design Hall isn't
  // available (no feature grant, or a daemon without the graph routes).
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { router } from '../../lib/router.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { lsGet, lsSet } from '../../lib/storage';
  import { designBus } from '../../lib/events.svelte';
  import { getLinks, listArtifacts } from '../../lib/api/design';
  import type { DesignArtifact, DesignLinksResp } from '../../lib/api/types';
  import StudioBadge from './StudioBadge.svelte';
  import StatusPill from './StatusPill.svelte';
  import { relLabel } from './model';

  interface Props {
    storyId: string;
    storyKey: string;
    storyTitle: string;
    isEpic: boolean;
  }
  let { storyId, storyKey, storyTitle, isEpic }: Props = $props();

  const KEY = 'otto.product.designGraph.open';
  let open = $state(lsGet(KEY) !== '0');
  function toggle(): void {
    open = !open;
    lsSet(KEY, open ? '1' : '0');
  }

  let artifacts = $state<DesignArtifact[]>([]);
  let uses = $state<Record<string, { rel: string; target: DesignArtifact }[]>>({});
  let phase = $state<'loading' | 'ready' | 'unavailable' | 'error'>('loading');
  let seq = 0;

  async function load(): Promise<void> {
    const my = ++seq;
    try {
      const list = await listArtifacts({ story_id: storyId, include_children: isEpic, limit: 60 });
      if (my !== seq) return;
      artifacts = list.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
      phase = 'ready';
      // Outgoing artifact links for the first dozen designs (the graph's third column).
      const results = await Promise.all(
        artifacts.slice(0, 12).map((a) =>
          getLinks(a.id, 'out').then(
            (r): [string, DesignLinksResp] => [a.id, r],
            (): [string, DesignLinksResp] => [a.id, { links: [], artifacts: [] }],
          ),
        ),
      );
      if (my !== seq) return;
      const next: typeof uses = {};
      for (const [id, r] of results) {
        const byId = new Map(r.artifacts.map((x) => [x.id, x]));
        next[id] = r.links
          .filter((l) => l.dst_kind === 'artifact' && byId.has(l.dst_id))
          .map((l) => ({ rel: l.rel, target: byId.get(l.dst_id)! }));
      }
      uses = next;
    } catch (e) {
      if (my !== seq) return;
      const status = (e as { status?: number }).status;
      phase = status === 403 || status === 404 || status === 405 ? 'unavailable' : 'error';
    }
  }

  $effect(() => {
    void storyId;
    void isEpic;
    void designBus.resyncTick;
    untrack(() => {
      if (!auth.can('design', 'view')) {
        phase = 'unavailable';
        return;
      }
      phase = 'loading';
      void load();
    });
  });

  let seen = designBus.seq;
  let t: ReturnType<typeof setTimeout> | null = null;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      if (phase === 'unavailable' || !evs.some((e) => e.type !== 'design_learning_update')) return;
      if (t) clearTimeout(t);
      t = setTimeout(() => void load(), 400);
    });
  });

  const openHall = () => router.go(`design/story/${encodeURIComponent(storyId)}`);
  const openArtifact = (id: string) => router.go(`design/a/${encodeURIComponent(id)}`);
</script>

{#if phase !== 'unavailable'}
  <section class="dgs" aria-label="Design Hall" data-testid="design-graph-strip">
    <header>
      <button class="icon-btn" onclick={toggle} aria-expanded={open} aria-label={open ? 'Collapse Design Hall designs' : 'Expand Design Hall designs'}
        title={open ? 'Collapse' : 'Expand'}>
        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={14} />
      </button>
      <Icon name="designHall" size={14} />
      <span class="h">Design Hall</span>
      <span class="dim">
        {#if phase === 'loading'}Loading designs…{:else if phase === 'error'}Couldn’t load linked designs{:else}{artifacts.length} design{artifacts.length === 1 ? '' : 's'} linked to this {isEpic ? 'epic and its stories' : 'story'}{/if}
      </span>
      {#if phase === 'error'}<button class="btn small ghost" onclick={() => void load()}>Retry</button>{/if}
      <span class="grow"></span>
      <button class="btn small" onclick={openHall} data-testid="design-open-in-hall"><Icon name="external" size={12} /> Open in Design Hall</button>
    </header>
    {#if open && phase === 'ready'}
      {#if artifacts.length === 0}
        <p class="empty dim">No design implements this story yet. Designs made here in the arena appear in Design Hall after its next import; designs made in Design Hall link here when they implement the story.</p>
      {:else}
        <div class="graph">
          <div class="story-node" title={storyTitle}>
            <Icon name="ticket" size={12} />
            <span class="key">{storyKey}</span>
            <span class="st">{storyTitle}</span>
          </div>
          <ul class="rows">
            {#each artifacts.slice(0, 12) as a (a.id)}
              <li>
                <span class="edge" aria-hidden="true"></span>
                <span class="rel dim">implements</span>
                <button class="node" onclick={() => openArtifact(a.id)} title={`Open ${a.title} in Design Hall`}>
                  <StudioBadge studio={a.studio} size={16} />
                  <span class="nt">{a.title}</span>
                  {#if a.head_seq != null}<span class="dim">v{a.head_seq}</span>{/if}
                  <StatusPill status={a.status} />
                </button>
                {#each uses[a.id] ?? [] as u (u.target.id + u.rel)}
                  <span class="arrow" aria-hidden="true"><Icon name="chevronRight" size={12} /></span>
                  <span class="rel dim">{relLabel(u.rel)}</span>
                  <button class="node small" onclick={() => openArtifact(u.target.id)} title={`Open ${u.target.title}`}>
                    <StudioBadge studio={u.target.studio} />
                    <span class="nt">{u.target.title}</span>
                  </button>
                {/each}
              </li>
            {/each}
          </ul>
          {#if artifacts.length > 12}
            <button class="btn small ghost more" onclick={openHall}>{artifacts.length - 12} more in Design Hall</button>
          {/if}
        </div>
      {/if}
    {/if}
  </section>
{/if}

<style>
  .dgs {
    flex: none;
    margin-block-end: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  header {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 36px;
    padding: 4px 8px;
    color: var(--text-dim);
  }
  .h {
    font-weight: 600;
    color: var(--text);
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .grow {
    flex: 1;
  }
  .empty {
    margin: 0;
    padding: 0 12px 12px;
    padding-inline-start: 40px;
  }
  .graph {
    display: flex;
    align-items: flex-start;
    gap: 0;
    padding: 4px 12px 12px;
    max-height: 260px;
    overflow: auto;
  }
  .story-node {
    flex: none;
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 220px;
    padding: 6px 10px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: var(--fs-s);
  }
  .key {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .st {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    border-inline-start: 1px solid var(--border-strong);
    margin-block-start: 14px;
    min-width: 0;
  }
  .rows li {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 32px;
    white-space: nowrap;
  }
  .edge {
    width: 14px;
    border-block-start: 1px solid var(--border-strong);
  }
  .rel {
    font-size: var(--fs-xs);
  }
  .node {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
    max-width: 320px;
  }
  .node:hover {
    border-color: var(--border-strong);
    background: var(--hover);
  }
  .node.small {
    height: 22px;
    font-size: var(--fs-xs);
  }
  .nt {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .arrow {
    display: inline-flex;
    color: var(--text-dim);
  }
  :global([dir='rtl']) .arrow {
    transform: scaleX(-1);
  }
  .more {
    align-self: flex-end;
  }
</style>
