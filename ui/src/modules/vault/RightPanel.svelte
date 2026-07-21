<script lang="ts">
  // Right panel: Backlinks (linked mentions) · Outgoing links · Outline ·
  // Properties (frontmatter) · OKF validation card (OKF vaults only).
  import { vault } from './vault.svelte';
  import { slugifyHeading } from './mdRender';

  let open = $state({ backlinks: true, outgoing: true, outline: false, props: false, okf: false });

  const props = $derived.by(() => {
    const fm = vault.note?.meta.frontmatter;
    if (!fm || typeof fm !== 'object' || Array.isArray(fm)) return [] as [string, string][];
    return Object.entries(fm as Record<string, unknown>).map(
      ([k, v]) => [k, typeof v === 'string' ? v : JSON.stringify(v)] as [string, string],
    );
  });

  const OKF_FIELDS = new Set(['type', 'title', 'description', 'resource', 'tags', 'timestamp']);

  function jumpToHeading(text: string): void {
    document
      .querySelector(`.read #h-${CSS.escape(slugifyHeading(text))}`)
      ?.scrollIntoView({ behavior: 'smooth', block: 'start' });
  }
</script>

<aside class="right">
  <section>
    <button class="hdr" onclick={() => (open.backlinks = !open.backlinks)}>
      <span class="tri" class:open={open.backlinks}>▸</span>
      Backlinks
      <span class="badge">{vault.backlinks.length}</span>
    </button>
    {#if open.backlinks}
      {#if vault.backlinks.length === 0}
        <div class="none">No linked mentions</div>
      {:else}
        {#each vault.backlinks as bl (bl.path + bl.kind)}
          <button class="item" onclick={() => void vault.open(bl.path)}>
            <div class="t">{bl.title}</div>
            {#if bl.context}<div class="ctx">{bl.context}</div>{/if}
          </button>
        {/each}
      {/if}
    {/if}
  </section>

  <section>
    <button class="hdr" onclick={() => (open.outgoing = !open.outgoing)}>
      <span class="tri" class:open={open.outgoing}>▸</span>
      Outgoing links
      <span class="badge">{vault.note?.outgoing.length ?? 0}</span>
    </button>
    {#if open.outgoing}
      {#each vault.note?.outgoing ?? [] as l, i (i)}
        <button
          class="item"
          class:unresolved={!l.dst_path}
          disabled={!l.dst_path || !/\.md$/i.test(l.dst_path)}
          onclick={() => l.dst_path && void vault.open(l.dst_path)}
        >
          <div class="t">
            {l.kind === 'embed' ? '⧉ ' : ''}{l.alias ?? l.raw_target}
            {#if !l.dst_path}<span class="ghost">unresolved</span>{/if}
          </div>
        </button>
      {/each}
    {/if}
  </section>

  <section>
    <button class="hdr" onclick={() => (open.outline = !open.outline)}>
      <span class="tri" class:open={open.outline}>▸</span>
      Outline
      <span class="badge">{vault.note?.meta.headings.length ?? 0}</span>
    </button>
    {#if open.outline}
      {#each vault.note?.meta.headings ?? [] as h, i (i)}
        <button
          class="item outline"
          style="padding-inline-start: {10 + (h.level - 1) * 12}px"
          onclick={() => jumpToHeading(h.text)}
        >
          <div class="t">{h.text}</div>
        </button>
      {/each}
    {/if}
  </section>

  <section>
    <button class="hdr" onclick={() => (open.props = !open.props)}>
      <span class="tri" class:open={open.props}>▸</span>
      Properties
      <span class="badge">{props.length}</span>
    </button>
    {#if open.props}
      {#if vault.note?.meta.parse_error}
        <div class="none warn">Frontmatter is not parseable YAML</div>
      {/if}
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
      <button class="hdr" onclick={() => (open.okf = !open.okf)}>
        <span class="tri" class:open={open.okf}>▸</span>
        OKF
        {#if vault.okfReport}
          <span class="badge" class:err={!vault.okfReport.conformant}>
            {vault.okfReport.conformant ? '✓' : vault.okfReport.errors.length}
          </span>
        {/if}
      </button>
      {#if open.okf}
        <div class="okf-actions">
          <button class="mini" disabled={vault.okfBusy} onclick={() => void vault.validateOkf()}>
            Validate
          </button>
          <button class="mini" disabled={vault.okfBusy} onclick={() => void vault.generateIndexes()}>
            Generate indexes
          </button>
        </div>
        {#if vault.okfReport}
          {#if vault.okfReport.conformant}
            <div class="none ok">✓ OKF v0.1 conformant ({vault.okfReport.checked_notes} notes)</div>
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
    font-size: 12px;
    font-weight: 600;
    padding: 6px 4px;
    cursor: pointer;
  }
  .tri {
    transition: transform 0.12s;
    font-size: 10px;
    color: var(--text-dim);
  }
  .tri.open {
    transform: rotate(90deg);
  }
  .badge {
    margin-inline-start: auto;
    font-size: 10px;
    color: var(--text-dim);
    background: var(--hover, rgba(127, 127, 127, 0.15));
    border-radius: 999px;
    padding: 1px 7px;
  }
  .badge.err {
    background: rgba(214, 86, 72, 0.25);
    color: var(--status-exited);
  }
  .item {
    display: block;
    width: 100%;
    text-align: start;
    background: none;
    border: none;
    border-radius: 6px;
    padding: 5px 8px;
    cursor: pointer;
    color: var(--text);
  }
  .item:hover {
    background: var(--hover, rgba(127, 127, 127, 0.12));
  }
  .item:disabled {
    cursor: default;
    opacity: 0.7;
  }
  .item .t {
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item .ctx {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item.unresolved .t {
    opacity: 0.6;
  }
  .ghost {
    font-size: 10px;
    border: 1px dashed var(--text-dim);
    border-radius: 4px;
    padding: 0 4px;
    margin-inline-start: 6px;
    color: var(--text-dim);
  }
  .none {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 4px 8px;
  }
  .none.ok {
    color: var(--status-working);
  }
  .none.warn {
    color: var(--status-warn);
  }
  .props {
    width: 100%;
    font-size: 11.5px;
    border-collapse: collapse;
  }
  .props td {
    padding: 3px 6px;
    vertical-align: top;
    border-top: 1px solid var(--border);
    word-break: break-word;
  }
  .props .k {
    color: var(--text-dim);
    white-space: nowrap;
  }
  .props tr.okf .k {
    color: var(--accent, #7a9cff);
  }
  .okf-actions {
    display: flex;
    gap: 6px;
    padding: 2px 4px 6px;
  }
  .mini {
    font-size: 11px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: 6px;
    padding: 3px 8px;
    cursor: pointer;
  }
  .finding.err .t b {
    color: var(--status-exited);
  }
  .finding.warn .t b {
    color: var(--status-warn);
  }
</style>
