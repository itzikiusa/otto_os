<script lang="ts">
  // Structured reading view for typed OKF notes: metadata header (type,
  // owners, status, tags, resource), key-field cards (endpoints, environments,
  // dependencies, data stores… from frontmatter or the matching body
  // section), the kind's required-content checklist, outgoing links +
  // backlinks with hover previews, and LIVE CONTEXT from the rest of Otto.
  // Sits above the unchanged markdown body; plain notes never mount it.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import { sentenceCase } from '../../lib/labels';
  import type { VaultNote } from '../../lib/api/types';
  import { vault } from './vault.svelte';
  import LiveContext from './LiveContext.svelte';
  import { linkPreview } from './previewStore.svelte';
  import type { StructuredModel } from './structuredNote';

  let { model, note }: { model: StructuredModel; note: VaultNote } = $props();

  const outgoing = $derived.by(() => {
    const seen = new Set<string>();
    return note.outgoing.filter((o) => {
      const key = o.dst_path ?? `?${o.raw_target}`;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    });
  });
  const missing = $derived(model.sections.filter((s) => !s.present).length);
  let showSections = $state(false);

  const showPreview = (e: Event, path: string | null | undefined): void => linkPreview.show(e.currentTarget as Element, path);
  const hidePreview = (): void => linkPreview.hide();

  function open(path: string | null | undefined): void {
    hidePreview();
    if (path && /\.md$/i.test(path)) void vault.open(path);
  }
</script>

<div class="structured" data-testid="vault-structured">
  <div class="meta-head">
    <span class="kind">{model.label}</span>
    {#if model.status}<Badge label={sentenceCase(model.status)} />{/if}
    {#if model.operation}
      <code class="op"><span class="method m-{model.operation.method.toLowerCase()}">{model.operation.method}</span> {model.operation.path}</code>
    {/if}
  </div>
  <dl class="facts">
    {#if model.owners.length}<div><dt>Owners</dt><dd>{model.owners.join(', ')}</dd></div>{/if}
    {#if model.resource && !model.operation}<div><dt>Resource</dt><dd class="mono" title={model.resource}>{model.resource}</dd></div>{/if}
    {#if model.tags.length}
      <div><dt>Tags</dt><dd class="tags">{#each model.tags as t (t)}<button class="chip tag" onclick={() => vault.searchTag(t)} title="Search #{t}">#{t}</button>{/each}</dd></div>
    {/if}
  </dl>

  {#if model.cards.length}
    <div class="cards">
      {#each model.cards as c (c.key)}
        <div class="sn-card">
          <h4>{c.title} <span class="n">{c.items.length}</span></h4>
          <ul>
            {#each c.items as it, i (i)}
              <li>
                {#if it.path}
                  <button class="lnk" onclick={() => open(it.path)} onmouseenter={(e) => showPreview(e, it.path)} onmouseleave={hidePreview} onfocus={(e) => showPreview(e, it.path)} onblur={hidePreview}>{it.value}</button>
                {:else}
                  <span>{it.value}</span>
                {/if}
              </li>
            {/each}
          </ul>
        </div>
      {/each}
    </div>
  {/if}

  <div class="checklist">
    <button class="toggle" aria-expanded={showSections} onclick={() => (showSections = !showSections)}>
      <Icon name={missing ? 'warning' : 'check'} size={12} />
      {missing ? `${missing} of ${model.sections.length} expected sections missing` : `All ${model.sections.length} expected sections present`}
    </button>
    {#if showSections}
      <ul class="secs">
        {#each model.sections as s (s.label)}<li class:missing={!s.present}><Icon name={s.present ? 'check' : 'x'} size={12} />{s.label}</li>{/each}
      </ul>
    {/if}
  </div>

  <div class="links">
    <div class="col" data-testid="vault-structured-outgoing">
      <h4>Links to <span class="n">{outgoing.length}</span></h4>
      {#if outgoing.length === 0}<p class="dim">No outgoing links</p>{/if}
      <div class="chips">
        {#each outgoing.slice(0, 40) as o (o.dst_path ?? o.raw_target)}
          <button
            class="chip lnkchip"
            class:unresolved={!o.dst_path}
            disabled={!o.dst_path || !/\.md$/i.test(o.dst_path)}
            title={o.dst_path ?? `${o.raw_target} (unresolved)`}
            onclick={() => open(o.dst_path)}
            onmouseenter={(e) => showPreview(e, o.dst_path)}
            onmouseleave={hidePreview}
            onfocus={(e) => showPreview(e, o.dst_path)}
            onblur={hidePreview}
          >{o.alias ?? o.raw_target.replace(/\.md$/i, '').split('/').pop()}</button>
        {/each}
        {#if outgoing.length > 40}<span class="dim">+{outgoing.length - 40} more in the side panel</span>{/if}
      </div>
    </div>
    <div class="col" data-testid="vault-structured-backlinks">
      <h4>Linked from <span class="n">{vault.backlinks.length}</span></h4>
      <!-- A failed lookup is never shown as "No backlinks yet": the first load
           fails inline with Retry, a failed refresh keeps the last good links
           under a slim stale bar (LoadState). -->
      <LoadState
        what="backlinks"
        variant="compact"
        loading={vault.backlinksLoading}
        error={vault.backlinksError || null}
        empty={vault.backlinks.length === 0}
        onretry={() => void vault.reloadBacklinks()}
      >
        {#snippet emptyView()}<p class="dim">No backlinks yet</p>{/snippet}
      <div class="chips">
        {#each vault.backlinks.slice(0, 40) as b (b.path + b.kind)}
          <button
            class="chip lnkchip"
            title={b.path}
            onclick={() => open(b.path)}
            onmouseenter={(e) => showPreview(e, b.path)}
            onmouseleave={hidePreview}
            onfocus={(e) => showPreview(e, b.path)}
            onblur={hidePreview}
          >{b.title}</button>
        {/each}
        {#if vault.backlinks.length > 40}<span class="dim">+{vault.backlinks.length - 40} more in the side panel</span>{/if}
      </div>
      </LoadState>
    </div>
  </div>

  <LiveContext hints={model.hints} wsId={vault.wsId} />
</div>

<style>
  .structured {
    margin: 0 0 18px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    font-size: var(--fs-s);
  }
  .meta-head { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
  .kind {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--accent-text);
    background: var(--accent-soft);
    padding: 2px 8px;
    border-radius: 999px;
  }
  .op { font-family: var(--font-mono); font-size: var(--fs-s); overflow-wrap: anywhere; }
  .method { font-weight: 600; color: var(--info); }
  .m-post { color: var(--success); } .m-delete { color: var(--danger); } .m-put, .m-patch { color: var(--warning); }
  .facts { display: grid; gap: 4px; margin: 10px 0 0; }
  .facts div { display: flex; gap: 10px; min-width: 0; }
  dt { color: var(--text-dim); min-width: 72px; font-size: var(--fs-xs); padding-block-start: 2px; }
  dd { margin: 0; min-width: 0; overflow-wrap: anywhere; }
  .mono { font-family: var(--font-mono); font-size: var(--fs-xs); }
  .tags { display: flex; flex-wrap: wrap; gap: 4px; }
  .tag { cursor: pointer; color: var(--accent-text); }
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 8px; margin-block-start: 12px; }
  .sn-card { border: 1px solid var(--border); border-radius: var(--radius-m); padding: 8px 10px; min-width: 0; background: var(--bg); }
  h4 { margin: 0 0 6px; font-size: var(--fs-xs); text-transform: uppercase; letter-spacing: .06em; color: var(--text-dim); }
  .n { font-weight: 400; margin-inline-start: 4px; }
  ul { margin: 0; padding-inline-start: 16px; }
  li { margin: 2px 0; overflow-wrap: anywhere; }
  .lnk { background: none; border: 0; padding: 0; color: var(--accent-text); cursor: pointer; font: inherit; text-align: start; }
  .lnk:hover { text-decoration: underline; }
  .checklist { margin-block-start: 10px; }
  .toggle { display: inline-flex; align-items: center; gap: 6px; background: none; border: 0; padding: 2px 0; color: var(--text-dim); cursor: pointer; font-size: var(--fs-xs); }
  .secs { list-style: none; padding: 4px 0 0; display: flex; flex-wrap: wrap; gap: 4px 12px; font-size: var(--fs-xs); }
  .secs li { display: inline-flex; align-items: center; gap: 4px; color: var(--success); }
  .secs li.missing { color: var(--warning); }
  .links { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 12px; margin-block-start: 12px; }
  .col { min-width: 0; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .lnkchip { cursor: pointer; color: var(--text); max-width: 100%; overflow: hidden; text-overflow: ellipsis; }
  .lnkchip:hover:not(:disabled) { background: var(--hover); }
  .lnkchip.unresolved { color: var(--text-dim); border-style: dashed; cursor: default; }
  .dim { color: var(--text-dim); font-size: var(--fs-xs); margin: 0; }
  button:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
</style>
