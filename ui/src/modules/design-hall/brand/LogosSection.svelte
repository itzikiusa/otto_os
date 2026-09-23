<script lang="ts">
  // Brand Kit → Logos: one tile per logo (full / mark / mono). A logo's asset is
  // an image artifact in the graph (`otto://design/<id>` — the kit `embeds`
  // it); uploading a file imports it as a Graphics artifact in the kit's
  // project. A slot with no asset shows a generated placeholder in the kit's
  // colours so the kit is usable before the real files arrive.
  import { untrack } from 'svelte';
  import type { BrandDoc, BrandLogoKind } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { fetchContent } from '../../../lib/api/design';
  import { importDesignFile } from '../create';
  import { openArtifact } from '../nav';
  import { parseOttoUri } from '../model';
  import { LOGO_KINDS, brandPalette } from './tokens';

  interface Props {
    doc: BrandDoc;
    readonly?: boolean;
    workspaceId: string;
    projectId?: string | null;
  }
  let { doc = $bindable(), readonly = false, workspaceId, projectId = null }: Props = $props();

  const KIND_LABEL: Record<BrandLogoKind, string> = { full: 'Full', mark: 'Mark', mono: 'Mono' };
  const pal = $derived(brandPalette(doc));
  const initial = $derived((doc.name || 'B').trim().charAt(0).toUpperCase() || 'B');
  const word = $derived((doc.name || 'Brand').replace(/\s*brand kit\s*$/i, '').trim().slice(0, 18) || 'Brand');

  // Object URLs of uploaded logo images, by artifact id.
  let urls = $state<Record<string, string>>({});
  const wanted = $derived(
    (doc.logos ?? []).map((l) => (l.asset ? parseOttoUri(l.asset)?.artifactId : null)).filter((x): x is string => !!x),
  );
  $effect(() => {
    const ids = wanted;
    untrack(() => {
      for (const id of ids) {
        if (urls[id]) continue;
        urls[id] = '';
        fetchContent(id, { asText: false })
          .then((c) => {
            if (c.blobUrl) urls[id] = c.blobUrl;
          })
          .catch(() => {
            /* a missing / unreadable asset keeps the placeholder */
          });
      }
    });
  });
  $effect(() => () => {
    for (const u of Object.values(urls)) if (u) URL.revokeObjectURL(u);
  });

  let fileInput = $state<HTMLInputElement | null>(null);
  let uploadIndex = $state(-1);
  let uploading = $state(false);

  function pick(i: number): void {
    uploadIndex = i;
    fileInput?.click();
  }

  async function onFile(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    const i = uploadIndex;
    if (!file || i < 0 || !doc.logos[i]) return;
    uploading = true;
    try {
      const id = await importDesignFile(file, { workspaceId, projectId });
      doc.logos[i].asset = `otto://design/${id}`;
      toasts.success('Logo added', `${file.name} is now a design in this workspace. Save the kit to keep it.`);
    } catch (err) {
      toasts.error('Couldn’t add the logo', err instanceof Error ? err.message : String(err));
    } finally {
      uploading = false;
    }
  }

  function add(): void {
    const used = new Set((doc.logos ?? []).map((l) => l.kind));
    const kind = LOGO_KINDS.find((k) => !used.has(k)) ?? 'full';
    doc.logos = [...(doc.logos ?? []), { name: `${KIND_LABEL[kind]} logo`, kind, asset: '' }];
  }

  function remove(i: number): void {
    doc.logos = doc.logos.filter((_, j) => j !== i);
  }
</script>

<section id="brand-logos" class="bsec" aria-labelledby="brand-logos-h">
  <div class="sec-h">
    <h2 id="brand-logos-h">Logos</h2>
    <span class="meta">SVG or PNG files, stored as designs and embedded by the kit</span>
  </div>

  <div class="logos" data-testid="brand-logos">
    {#each doc.logos ?? [] as logo, i (i)}
      {@const aid = logo.asset ? parseOttoUri(logo.asset)?.artifactId : null}
      <figure class="ltile" class:mono={logo.kind === 'mono'}>
        <div class="lt-in" style:background={logo.kind === 'mono' ? undefined : pal.surface}>
          {#if aid && urls[aid]}
            <img src={urls[aid]} alt={logo.name} />
          {:else}
            <svg viewBox={logo.kind === 'mark' ? '0 0 40 40' : '0 0 200 40'} class:mark={logo.kind === 'mark'} role="img" aria-label="{logo.name} (placeholder)">
              <rect x="0" y="0" width="40" height="40" rx="11" fill={logo.kind === 'mono' ? 'currentColor' : pal.primary} />
              <text x="20" y="27" text-anchor="middle" font-size="20" font-weight="800" fill={logo.kind === 'mono' ? 'var(--surface)' : pal.accent}>{initial}</text>
              {#if logo.kind !== 'mark'}
                <text x="52" y="28" font-size="22" font-weight="700" fill={logo.kind === 'mono' ? 'currentColor' : pal.ink}>{word}</text>
              {/if}
            </svg>
          {/if}
        </div>
        <figcaption>
          <input class="input name" value={logo.name} oninput={(e) => (logo.name = e.currentTarget.value)} disabled={readonly} aria-label="Logo name" />
          <select class="input kind" value={logo.kind} onchange={(e) => (logo.kind = e.currentTarget.value as BrandLogoKind)} disabled={readonly} aria-label="Logo variant">
            {#each LOGO_KINDS as k (k)}<option value={k}>{KIND_LABEL[k]}</option>{/each}
          </select>
          <div class="acts">
            {#if !aid}<span class="dim">Placeholder</span>{/if}
            <span class="grow"></span>
            {#if aid}
              <button class="icon-btn" aria-label="Open the {logo.name} design" title="Open design" onclick={() => openArtifact(aid)}><Icon name="external" size={14} /></button>
            {/if}
            {#if !readonly}
              <button class="icon-btn" aria-label="{aid ? 'Replace' : 'Upload'} {logo.name}" title={aid ? 'Replace file' : 'Upload file'} onclick={() => pick(i)} disabled={uploading}><Icon name="image" size={14} /></button>
              <button class="icon-btn" aria-label="Remove {logo.name}" title="Remove" onclick={() => remove(i)}><Icon name="trash" size={14} /></button>
            {/if}
          </div>
        </figcaption>
      </figure>
    {/each}
  </div>
  {#if !readonly && (doc.logos?.length ?? 0) < 16}
    <button class="btn small addbtn" onclick={add}><Icon name="plus" size={12} /> Add logo</button>
  {/if}
  <input bind:this={fileInput} class="file" type="file" accept=".svg,.png,.jpg,.jpeg,.webp" onchange={onFile} tabindex="-1" aria-hidden="true" />
</section>

<style>
  .bsec {
    display: flex;
    flex-direction: column;
    gap: 12px;
    scroll-margin-top: 12px;
  }
  .sec-h {
    display: flex;
    align-items: baseline;
    gap: 10px;
    flex-wrap: wrap;
  }
  h2 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .meta,
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .logos {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 12px;
  }
  .ltile {
    margin: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    min-width: 0;
  }
  .lt-in {
    height: 120px;
    display: grid;
    place-items: center;
    padding: 16px;
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
    color: var(--text);
  }
  .lt-in svg {
    width: 100%;
    max-width: 190px;
    height: 40px;
  }
  .lt-in svg.mark {
    width: 56px;
    height: 56px;
  }
  .lt-in img {
    max-width: 100%;
    max-height: 88px;
    object-fit: contain;
  }
  figcaption {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 84px;
    gap: 6px;
    padding: 10px 12px 10px;
  }
  .name,
  .kind {
    height: 26px;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .acts {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .grow {
    flex: 1;
  }
  .addbtn {
    align-self: flex-start;
  }
  .file {
    display: none;
  }
</style>
