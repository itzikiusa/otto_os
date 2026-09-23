<script module lang="ts">
  // Thumbnail cache shared by every card on screen: object URLs (thumbnails,
  // images) and small text sources (live previews), keyed by the version they
  // came from so a new head re-fetches. Bounded — the oldest entries are
  // evicted (and their object URLs revoked) past the cap.
  const CAP = 120;
  const urls = new Map<string, Promise<string | null>>();
  const texts = new Map<string, Promise<string | null>>();

  function remember<T>(m: Map<string, Promise<T>>, key: string, make: () => Promise<T>, onEvict?: (v: T) => void): Promise<T> {
    const hit = m.get(key);
    if (hit) {
      m.delete(key);
      m.set(key, hit); // refresh recency
      return hit;
    }
    const p = make();
    m.set(key, p);
    while (m.size > CAP) {
      const [oldKey, oldVal] = m.entries().next().value as [string, Promise<T>];
      m.delete(oldKey);
      if (onEvict) void oldVal.then(onEvict);
    }
    return p;
  }

  const revoke = (u: string | null) => {
    if (u) URL.revokeObjectURL(u);
  };
</script>

<script lang="ts">
  // The picture on a design card: the stored PNG thumbnail when there is one;
  // otherwise, with `live`, a small sandboxed render of the actual design
  // (HTML/SVG/Mermaid/D2 — scripts off, opaque origin) or the image itself;
  // otherwise a quiet placeholder with the studio badge. Never blocks a card.
  import type { DesignArtifact } from '../../lib/api/types';
  import { fetchContent, getArtifact, thumbnailUrl } from '../../lib/api/design';
  import { renderMermaid } from '../canvas/mermaid';
  import { renderD2 } from '../canvas/d2';
  import StudioBadge from './StudioBadge.svelte';
  import { formatLabel, isImageFormat, renderKind } from './model';

  interface Props {
    artifact: DesignArtifact;
    /** Render the design itself when there is no stored thumbnail. */
    live?: boolean;
    /** A small cell (mosaics): the placeholder shows the badge only. */
    compact?: boolean;
    /** Render this version instead of the head (a variant tray card). The
     *  stored thumbnail belongs to the head, so it is skipped. */
    versionId?: string | null;
  }
  let { artifact, live = false, compact = false, versionId = null }: Props = $props();

  type Pic =
    | { kind: 'img'; src: string }
    | { kind: 'doc'; html: string }
    | { kind: 'none' };
  let pic = $state<Pic>({ kind: 'none' });
  let boxW = $state(0);

  const version = $derived(versionId ?? artifact.head_version_id ?? 'none');
  const rk = $derived(renderKind(artifact.format));

  async function sourceOf(a: DesignArtifact, v: string | null): Promise<string | null> {
    return remember(texts, `${a.id}:${v ?? a.head_version_id}`, async () => {
      try {
        const d = await getArtifact(a.id, v ? { content: true, version: v } : { content: true });
        return d.content_truncated ? null : d.content;
      } catch {
        return null;
      }
    });
  }

  function svgDoc(svg: string): string {
    return (
      '<!doctype html><html><head><meta charset="utf-8"><style>html,body{margin:0;height:100%;background:#fff}' +
      'body{display:flex;align-items:center;justify-content:center}svg{max-width:92%;max-height:92%;height:auto}</style>' +
      `</head><body>${svg}</body></html>`
    );
  }

  let gen = 0;
  $effect(() => {
    const a = artifact;
    const v = versionId;
    void version;
    const myGen = ++gen;
    void (async () => {
      let next: Pic = { kind: 'none' };
      if (a.thumb_blob && !v) {
        const u = await remember(urls, `t:${a.id}:${a.thumb_blob}`, () => thumbnailUrl(a.id).catch(() => null), revoke);
        if (u) next = { kind: 'img', src: u };
      } else if (live && isImageFormat(a.format)) {
        const u = await remember(
          urls,
          `c:${a.id}:${v ?? a.head_version_id}`,
          () => fetchContent(a.id, v ? { asText: false, version: v } : { asText: false }).then((c) => c.blobUrl, () => null),
          revoke,
        );
        if (u) next = { kind: 'img', src: u };
      } else if (live && (rk === 'html' || rk === 'svg' || rk === 'mermaid' || rk === 'd2')) {
        const src = await sourceOf(a, v);
        if (src && src.trim()) {
          if (rk === 'html') next = { kind: 'doc', html: src };
          else if (rk === 'svg') next = { kind: 'doc', html: svgDoc(src) };
          else {
            const r = rk === 'mermaid' ? await renderMermaid(`dh-thumb-${a.id}-${v ?? 'h'}-${myGen}`, src) : await renderD2(a.id, src);
            if (r.svg) next = { kind: 'doc', html: svgDoc(r.svg) };
          }
        }
      }
      if (myGen === gen) pic = next;
    })();
  });

  // HTML previews render at a 1280px desktop width, scaled to the card.
  const scale = $derived(boxW > 0 ? boxW / 1280 : 0.2);
</script>

<div class="thumb" bind:clientWidth={boxW} aria-hidden="true">
  {#if pic.kind === 'img'}
    <img src={pic.src} alt="" loading="lazy" />
  {:else if pic.kind === 'doc'}
    <iframe
      title=""
      tabindex="-1"
      sandbox=""
      srcdoc={pic.html}
      style:transform={`scale(${scale})`}
      style:height={`${Math.ceil(100 / scale)}%`}
    ></iframe>
  {:else}
    <div class="placeholder">
      <StudioBadge studio={artifact.studio} size={compact ? 20 : 28} />
      {#if !compact}<span class="fmt">{formatLabel(artifact.format)}</span>{/if}
    </div>
  {/if}
</div>

<style>
  .thumb {
    position: relative;
    width: 100%;
    height: 100%;
    overflow: hidden;
    background: var(--surface-2);
  }
  img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  iframe {
    position: absolute;
    inset-block-start: 0;
    inset-inline-start: 0;
    width: 1280px;
    border: 0;
    transform-origin: 0 0;
    pointer-events: none;
    background: white;
  }
  :global([dir='rtl']) iframe {
    transform-origin: 100% 0;
  }
  .placeholder {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    background-image: radial-gradient(color-mix(in srgb, var(--text) 10%, transparent) 1px, transparent 1px);
    background-size: 12px 12px;
  }
  .fmt {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
