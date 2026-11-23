<script lang="ts">
  // "Skeleton added · 3 moves": said wherever a made person shows, read from
  // the model file itself (rigRead.ts), so it is only said when it is true.
  import Icon from '../../../lib/components/Icon.svelte';
  import { rigWords } from './rig';
  import { readRig } from './rigRead';

  interface Props {
    artifactId: string;
    versionId?: string | null;
  }
  let { artifactId, versionId = null }: Props = $props();
  let words = $state<string | null>(null);

  $effect(() => {
    const id = artifactId;
    const v = versionId;
    let alive = true;
    void readRig(id, v).then((r) => {
      if (alive) words = rigWords(r);
    });
    return () => {
      alive = false;
    };
  });
</script>

{#if words}<span class="rl" data-testid="create-rig-words"><Icon name="user" size={14} /> {words}</span>{/if}

<style>
  .rl {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
