<script lang="ts">
  // After "Apply changes": the old one and the new one side by side, and
  // one choice. Go back puts the old version back on top (nothing is lost
  // either way); both close the notes that asked for the change. A change
  // that came back identical (no new version) is said plainly instead.
  import { getArtifact, listComments, restoreVersion, updateComment } from '../../../lib/api/design';
  import { router } from '../../../lib/router.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import CreateModel from './CreateModel.svelte';
  import { createHref } from './plan';

  interface Props {
    id: string;
    before: number;
  }
  let { id, before }: Props = $props();
  let busy = $state(false);
  /** It came back exactly as it was (the same content is the same version). */
  let same = $state(false);

  $effect(() => {
    const aid = id;
    const b = before;
    void getArtifact(aid)
      .then((d) => (same = (d.artifact.head_seq ?? 0) <= b))
      .catch(() => {});
  });

  async function closeNotes(): Promise<void> {
    const open = await listComments(id, { resolved: false }).catch(() => []);
    await Promise.all(open.filter((c) => c.pin_3d && !c.parent_id).map((c) => updateComment(id, c.id, { resolved: true }).catch(() => undefined)));
  }

  async function choose(keep: boolean): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      if (!keep) await restoreVersion(id, `v${before}`, { message: 'Went back to the one before the change' });
      await closeNotes();
      toasts.success(keep ? 'Kept the new one' : 'Kept the old one');
      router.replace(createHref('a', id));
    } catch {
      toasts.warn('That didn’t work', 'Please try again.');
    } finally {
      busy = false;
    }
  }
</script>

{#if same}
  <h1 class="cr-h1">It came out the same</h1>
  <p class="cr-sub" data-testid="create-same">I made it again, but it came out just like before. Try saying it differently.</p>
  <div class="cr-two">
    <div class="cr-stage"><CreateModel artifactId={id} label="Your one" /></div>
    <div>
      <button type="button" class="cr-primary cmp-wide" onclick={() => router.replace(createHref('a', id, 'change'))} data-testid="create-try-again">Say it differently</button>
      <button type="button" class="cr-quiet" disabled={busy} onclick={() => void closeNotes().then(() => router.replace(createHref('a', id)))}>Keep it like this</button>
    </div>
  </div>
{:else}
<h1 class="cr-h1">Which one do you like?</h1>
<p class="cr-sub">Before on the left, new on the right.</p>
<div class="cmp">
  <figure>
    <div class="cmp-stage"><CreateModel artifactId={id} seq={before} label="Before" /></div>
    <figcaption>Before</figcaption>
  </figure>
  <figure class="cmp-new">
    <div class="cmp-stage"><CreateModel artifactId={id} label="New" /></div>
    <figcaption>New</figcaption>
  </figure>
</div>
<div class="cr-sticky cmp-btns">
  <button type="button" class="cr-secondary" disabled={busy} onclick={() => void choose(false)} data-testid="create-go-back">Keep the old one</button>
  <button type="button" class="cr-secondary" disabled={busy} onclick={() => void choose(true)} data-testid="create-keep-new">Keep the new one</button>
</div>
{/if}

<style>
  .cmp {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--sp-3);
  }
  figure {
    margin: 0;
  }
  figcaption {
    text-align: center;
    font-weight: 600;
    margin-block-start: var(--sp-2);
  }
  .cmp-stage {
    aspect-ratio: 1;
  }
  .cmp-wide {
    inline-size: 100%;
  }
  .cmp-btns {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--sp-3);
  }
  @media (max-height: 500px) and (orientation: landscape) {
    .cmp-stage {
      aspect-ratio: 16 / 9;
    }
  }
</style>
