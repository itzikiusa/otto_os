<script lang="ts" module>
  // One shared player for every Create audio line: starting one line stops
  // the one before, unless both were started together (ambience + music).
  let current: HTMLAudioElement | null = null;
  const together = new Set<HTMLAudioElement>();
  export function stopAll(): void {
    current?.pause();
    for (const a of together) a.pause();
    together.clear();
    current = null;
  }
</script>

<script lang="ts">
  // A big round play button with a plain label: who says it, or what it is.
  import { onDestroy } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { fetchTtsClipAudioUrl } from '../../../lib/api/media';
  import { toasts } from '../../../lib/toast.svelte';

  interface Props {
    clipId: string;
    title: string;
    detail?: string;
    /** Keeps playing beside others (place sounds, music). */
    loop?: boolean;
    /** Bumped by a parent to start this line together with others. */
    playTick?: number;
  }
  let { clipId, title, detail, loop = false, playTick = 0 }: Props = $props();

  let audio: HTMLAudioElement | null = null;
  let url: string | null = null;
  let playing = $state(false);
  let loading = $state(false);

  async function ensure(): Promise<HTMLAudioElement> {
    if (audio && url) return audio;
    url = await fetchTtsClipAudioUrl(clipId);
    audio = new Audio(url);
    audio.loop = loop;
    audio.onplay = () => (playing = true);
    audio.onpause = audio.onended = () => (playing = false);
    return audio;
  }

  async function toggle(withOthers = false): Promise<void> {
    if (playing && !withOthers) {
      audio?.pause();
      return;
    }
    loading = true;
    try {
      const a = await ensure();
      if (withOthers) together.add(a);
      else {
        if (current && current !== a) current.pause();
        current = a;
      }
      a.currentTime = 0;
      await a.play();
    } catch {
      toasts.warn('I couldn’t play that', 'Please try again.');
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    if (playTick > 0) void toggle(true);
  });
  $effect(() => {
    if (audio) audio.loop = loop;
  });

  onDestroy(() => {
    audio?.pause();
    if (audio) together.delete(audio);
    if (url) URL.revokeObjectURL(url);
  });
</script>

<div class="al" data-testid="create-audio">
  <button
    type="button"
    class="al-btn"
    aria-label={playing ? `Stop ${title}` : `Play ${title}`}
    title={playing ? 'Stop' : 'Play'}
    disabled={loading}
    onclick={() => void toggle()}
  >
    <Icon name={playing ? 'stop' : 'play'} size={16} />
  </button>
  <div class="al-text">
    <b>{title}</b>
    {#if detail}<span>{detail}</span>{/if}
  </div>
</div>

<style>
  .al {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    padding-block: var(--sp-2);
  }
  .al-btn {
    flex: none;
    display: grid;
    place-items: center;
    inline-size: 48px;
    block-size: 48px;
    border: 0;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent-text);
    cursor: pointer;
  }
  .al-btn:focus-visible {
    outline: 3px solid var(--accent-text);
    outline-offset: 2px;
  }
  .al-text {
    min-inline-size: 0;
    display: grid;
  }
  .al-text span {
    color: var(--text-dim);
    font-size: var(--fs-m);
    overflow-wrap: anywhere;
  }
</style>
