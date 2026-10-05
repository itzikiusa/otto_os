<script lang="ts">
  // Right panel: Backlinks (linked mentions) · Outgoing links · Outline ·
  // Properties (frontmatter) · OKF validation card (OKF vaults only).
  import { vault } from './vault.svelte';
  import PropertiesEditor from './PropertiesEditor.svelte';
  import KnowledgeMetadata from './KnowledgeMetadata.svelte';
  import { slugifyHeading } from './mdRender';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { plural } from '../../lib/plural';
  import Badge from '../../lib/components/Badge.svelte';

  let open = $state({ backlinks: true, outgoing: true, outline: false, props: false, okf: false });

  // V7: a hub note can have thousands of backlinks / outgoing links; mount a
  // page per list and grow on demand (reset when another note opens).
  const LIST_PAGE = 100;
  let shownOutgoing = $state(LIST_PAGE);
  $effect(() => {
    void vault.notePath;
    shownOutgoing = LIST_PAGE;
  });

  async function showMoreBacklinks(): Promise<void> {
    await vault.loadBacklinkContexts(vault.visibleBacklinks);
  }

  const props = $derived.by(() => {
    const fm = vault.note?.meta.frontmatter;
    if (!fm || typeof fm !== 'object' || Array.isArray(fm)) return [] as [string, string][];
    return Object.entries(fm as Record<string, unknown>).map(
      ([k, v]) => [k, typeof v === 'string' ? v : JSON.stringify(v)] as [string, string],
    );
  });

  const OKF_FIELDS = new Set(['type', 'title', 'description', 'resource', 'tags', 'timestamp', 'generated', 'verified', 'sources', 'usage_window', 'status', 'stale_after', 'computation', 'executor', 'attester']);

  function jumpToHeading(text: string): void {
    document
      .querySelector(`.read #h-${CSS.escape(slugifyHeading(text))}`)
      ?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
</script>

<aside class="right">
  {#if vault.current?.okf && vault.note && !vault.note.meta.reserved}
    <section><h3 class="hdr">Knowledge provenance</h3><KnowledgeMetadata frontmatter={vault.note.meta.frontmatter} /></section>
  {/if}
  <section>
    <button class="hdr" aria-expanded={open.backlinks} onclick={() => (open.backlinks = !open.backlinks)}>
      <span class="tri" class:open={open.backlinks}><Icon name="chevronRight" noflip size={12} /></span>
      Backlinks
      <span class="hdr-count"><Badge label={String(vault.backlinks.length)} /></span>
    </button>
    {#if open.backlinks}
      <LoadState
        what="backlinks"
        variant="compact"
        loading={vault.backlinksLoading}
        error={vault.backlinksError || null}
        empty={vault.backlinks.length === 0}
        onretry={() => void vault.reloadBacklinks()}
      >
        {#snippet emptyView()}<div class="none">No linked mentions</div>{/snippet}
        {#each vault.backlinks.slice(0, vault.visibleBacklinks) as bl (bl.path + bl.kind)}
          <button class="item" title={bl.path} onclick={() => void vault.open(bl.path)}>
            <div class="t">{bl.title}</div>
            {#if bl.context}<div class="ctx">{bl.context}</div>{/if}
          </button>
        {/each}
        {#if vault.backlinks.length > vault.visibleBacklinks}
          <button class="more" disabled={vault.loadingBacklinkContexts} onclick={() => void showMoreBacklinks()}>Show more ({vault.backlinks.length - vault.visibleBacklinks} hidden)</button>
        {/if}
      </LoadState>
    {/if}
  </section>

  <section>
    <button class="hdr" aria-expanded={open.outgoing} onclick={() => (open.outgoing = !open.outgoing)}>
      <span class="tri" class:open={open.outgoing}><Icon name="chevronRight" noflip size={12} /></span>
      Outgoing links
      <span class="hdr-count"><Badge label={String(vault.note?.outgoing.length ?? 0)} /></span>
    </button>
    {#if open.outgoing}
      {#each (vault.note?.outgoing ?? []).slice(0, shownOutgoing) as l, i (i)}
        <button
          class="item"
          class:unresolved={!l.dst_path}
          title={l.dst_path ?? `${l.raw_target} (unresolved)`}
          disabled={!l.dst_path || !/\.md$/i.test(l.dst_path)}
          onclick={() => l.dst_path && void vault.open(l.dst_path)}
        >
          <div class="t">
            {#if l.kind === 'embed'}<span class="embed-ic" title="Embedded"><Icon name="image" size={11} /></span>{/if}{l.alias ?? l.raw_target}
            {#if !l.dst_path}<span class="ghost">unresolved</span>{/if}
          </div>
        </button>
      {/each}
      {#if (vault.note?.outgoing.length ?? 0) > shownOutgoing}
        <button class="more" onclick={() => (shownOutgoing += LIST_PAGE * 5)}>Show more ({(vault.note?.outgoing.length ?? 0) - shownOutgoing} hidden)</button>
      {/if}
    {/if}
  </section>

  <section>
    <button class="hdr" aria-expanded={open.outline} onclick={() => (open.outline = !open.outline)}>
      <span class="tri" class:open={open.outline}><Icon name="chevronRight" noflip size={12} /></span>
      Outline
      <span class="hdr-count"><Badge label={String(vault.note?.meta.headings.length ?? 0)} /></span>
    </button>
    {#if open.outline}
      {#each vault.note?.meta.headings ?? [] as h, i (i)}
        <button
          class="item outline"
          style="padding-inline-start: {10 + (h.level - 1) * 12}px"
          title={h.text}
          onclick={() => jumpToHeading(h.text)}
        >
          <div class="t">{h.text}</div>
        </button>
      {/each}
    {/if}
  </section>

  <section>
    <button class="hdr" aria-expanded={open.props} onclick={() => (open.props = !open.props)}>
      <span class="tri" class:open={open.props}><Icon name="chevronRight" noflip size={12} /></span>
      Properties
      <span class="hdr-count"><Badge label={String(props.length)} /></span>
    </button>
    {#if open.props}
      {#if vault.note?.meta.content_index_status === 'size_limited'}
        <div class="none warn">Content indexing skipped: this note exceeds 4 MiB. Search by file name; the original file is unchanged.</div>
      {/if}
      {#if vault.note?.meta.parse_error}
        <div class="none warn">Frontmatter is not parseable YAML</div>
      {/if}
      {#key vault.notePath}<PropertiesEditor />{/key}
      <table class="props">
        <tbody>
          {#each props as [k, v] (k)}
            <tr class:okf={vault.current?.okf && OKF_FIELDS.has(k)}>
              <td class="k">{k}</td>
              <td class="v">{v}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </section>

  {#if vault.current?.okf}
    <section>
      <button class="hdr" aria-expanded={open.okf} onclick={() => (open.okf = !open.okf)}>
        <span class="tri" class:open={open.okf}><Icon name="chevronRight" noflip size={12} /></span>
        OKF
        {#if vault.okfReport}
          <span class="hdr-count">
            {#if vault.okfReport.conformant}
              <Badge tone="ok" title="Conformant"><Icon name="check" size={11} /></Badge>
            {:else}
              <Badge tone="bad" label={String(vault.okfReport.errors.length)} title={plural(vault.okfReport.errors.length, 'error')} />
            {/if}
          </span>
        {/if}
      </button>
      {#if open.okf}
        <div class="okf-actions">
          <button class="btn small" disabled={vault.okfBusy} onclick={() => void vault.validateOkf()}>
            {vault.okfBusy ? 'Working…' : 'Validate'}
          </button>
          <button class="btn small" disabled={vault.okfBusy} onclick={() => void vault.generateIndexes()} title="Write the index.md files OKF expects in each folder">
            Generate indexes
          </button>
        </div>
        {#if vault.okfReport}
          {#if vault.okfReport.conformant}
            <div class="none ok"><Icon name="check" size={11} /> OKF conformant ({vault.okfReport.checked_notes} notes)</div>
          {/if}
          {#each vault.okfReport.errors as f, i (i)}
            <button class="item finding err" onclick={() => f.path.endsWith('.md') && void vault.open(f.path)}>
              <div class="t"><b>{f.rule}</b> {f.path}</div>
              <div class="ctx">{f.message}</div>
            </button>
          {/each}
          {#each vault.okfReport.warnings.slice(0, 50) as f, i (i)}
            <button class="item finding warn" onclick={() => f.path.endsWith('.md') && void vault.open(f.path)}>
              <div class="t"><b>{f.rule}</b> {f.path}</div>
              <div class="ctx">{f.message}</div>
            </button>
          {/each}
          {#if vault.okfReport.warnings.length > 50}
            <div class="none">+{vault.okfReport.warnings.length - 50} more warnings</div>
          {/if}
        {/if}
      {/if}
    </section>
  {/if}
</aside>

<style>
  .right {
    height: 100%;
    overflow-y: auto;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  section {
    border-bottom: 1px solid var(--border);
    padding-bottom: 6px;
  }
  .hdr {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    background: none;
    border: none;
    color: var(--text);
    font-size: var(--fs-s);
    font-weight: 600;
    padding: 6px 4px;
    cursor: pointer;
  }
  .tri {
    display: inline-flex;
    transition: transform var(--dur-fast);
    color: var(--text-dim);
  }
  .tri.open {
    transform: rotate(90deg);
  }
  .hdr-count {
    margin-inline-start: auto;
    display: inline-flex;
  }
  .item {
    display: block;
    width: 100%;
    text-align: start;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    padding: 4px 8px;
    cursor: pointer;
    color: var(--text);
  }
  .item:hover {
    background: var(--hover);
  }
  .item:disabled {
    cursor: default;
  }
  .item .t {
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item .ctx {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item.unresolved .t {
    color: var(--text-dim);
  }
  .ghost {
    font-size: var(--fs-xs);
    border: 1px dashed var(--text-dim);
    border-radius: var(--radius-s);
    padding: 0 4px;
    margin-inline-start: 6px;
    color: var(--text-dim);
  }
  .none {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 4px 8px;
  }
  .none.ok {
    color: var(--success);
  }
  .none.warn {
    color: var(--warning);
  }
  .props {
    width: 100%;
    font-size: var(--fs-xs);
    border-collapse: collapse;
  }
  .props td {
    padding: 2px 6px;
    vertical-align: top;
    border-top: 1px solid var(--border);
    word-break: break-word;
  }
  .props .k {
    color: var(--text-dim);
    white-space: nowrap;
  }
  .props tr.okf .k {
    color: var(--accent-text);
  }
  .okf-actions {
    display: flex;
    gap: 6px;
    padding: 2px 4px 6px;
  }
  .embed-ic {
    display: inline-flex;
    vertical-align: -1px;
    margin-inline-end: 4px;
    color: var(--text-dim);
  }
  .none.ok {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .finding.err .t b {
    color: var(--danger);
  }
  .finding.warn .t b {
    color: var(--warning);
  }
  .more {
    display: block;
    margin: 4px 10px 8px;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .more:hover {
    background: var(--hover);
    color: var(--text);
  }
</style>
