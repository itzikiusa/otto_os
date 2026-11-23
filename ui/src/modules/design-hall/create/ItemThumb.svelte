<script lang="ts">
  // A piece's picture: the production's own picture of it when there is one
  // (its concept picture as soon as it is drawn, later a picture of the made
  // model; fetched again when it changes), else the made thing's saved
  // thumbnail, else a plain icon (never a drawn stand-in that could pass for
  // a result).
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import { getArtifact, thumbnailUrl } from '../../../lib/api/design';
  import { productionPreviewUrl } from '../../../lib/api/productions';

  interface Props {
    artifactId?: string | null;
    icon: IconName;
    /** Pixels, or `fill` to take the parent's box. */
    size?: number | 'fill';
    /** The production's picture of one piece (`GET …/items/{item}/preview`) as `production/item/rev` (a plain string, so a poll that brings the same picture fetches nothing). */
    preview?: string | null;
  }
  let { artifactId = null, icon, size = 56, preview = null }: Props = $props();
  const px = $derived(size === 'fill' ? '100%' : `${size}px`);
  let src = $state<string | null>(null);

  function previewUrl(key: string): Promise<string> {
    const [production, item, rev] = key.split('/');
    return productionPreviewUrl(production, item, Number(rev));
  }
  async function thumbUrl(id: string): Promise<string | null> {
    const d = await getArtifact(id);
    return d.artifact.thumb_blob ? thumbnailUrl(id, d.artifact.thumb_blob) : null;
  }

  $effect(() => {
    const id = artifactId;
    const pv = preview;
    let alive = true;
    let made: string | null = null;
    void (async () => {
      const u = (pv ? await previewUrl(pv).catch(() => null) : null) ?? (id ? await thumbUrl(id).catch(() => null) : null);
      made = u;
      // Keep the last picture until the new one is there (no flash back to the icon).
      if (alive && u) src = u;
      else if (u) URL.revokeObjectURL(u);
    })();
    return () => {
      alive = false;
      const old = made;
      // Let the next picture take over before this one goes.
      if (old) setTimeout(() => URL.revokeObjectURL(old), 2000);
    };
  });
</script>

<span class="cr-tile it" class:fill={size === 'fill'} class:shown={!!src} style:inline-size={px} style:block-size={px} data-testid="create-thumb">
  {#if src}<img {src} alt="" />{:else}<Icon name={icon} size={24} />{/if}
</span>

<style>
  .it.shown {
    border-color: var(--success);
  }
  .it.fill {
    inline-size: 100%;
    block-size: 100%;
    border: 0;
  }
</style>
