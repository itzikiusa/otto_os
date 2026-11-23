<script lang="ts">
  // Sound & music inside Create: where are we, what should we hear, make it.
  // The made sounds play right here with plain names, and a background sound
  // can repeat. When the sound maker isn't set up, one calm state says so.
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import * as mapi from '../../../lib/api/media';
  import type { AudioJob, TtsClip } from '../../../lib/api/types';
  import { pollWhileVisible } from '../../../lib/poll';
  import { router } from '../../../lib/router.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import AudioLine from './AudioLine.svelte';
  import { createStore } from './createStore.svelte';
  import { createHref, timeText } from './plan';

  const PLACES = ['A windy cave', 'A busy market', 'A quiet forest at night'];
  const SOUNDS = ['Footsteps on stones', 'Swords clashing', 'Birds singing'];

  let what = $state<'sound' | 'music'>('sound');
  let place = $state('');
  let hear = $state('');
  let repeat = $state(true);
  let job = $state<AudioJob | null>(null);
  let clips = $state<TtsClip[]>([]);
  let starting = $state(false);
  let repeating = $state<Record<string, boolean>>({});
  const ready = $derived(createStore.soundReady);
  const can = $derived(!!hear.trim() && !starting && !(job && (job.status === 'queued' || job.status === 'running')));

  $effect(() => {
    const id = ws.currentId;
    untrack(() => {
      void createStore.loadSoundEngine();
      if (id) void loadClips(id);
    });
  });

  async function loadClips(wsId: string): Promise<void> {
    const [s, m] = await Promise.all([
      mapi.listTtsClips({ workspace_id: wsId, kind: 'sound', limit: 30 }).catch(() => []),
      mapi.listTtsClips({ workspace_id: wsId, kind: 'music', limit: 30 }).catch(() => []),
    ]);
    clips = [...s, ...m].sort((a, b) => b.created_at.localeCompare(a.created_at));
  }

  async function make(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || !can) return;
    starting = true;
    const words = place.trim() ? `${hear.trim()}, in ${place.trim().replace(/^an? /i, 'a ')}` : hear.trim();
    try {
      job = await mapi.startAudioJob({
        workspace_id: wsId,
        kind: what,
        prompt: words.slice(0, 500),
        seconds: what === 'music' ? 60 : repeat ? 20 : 8,
        loop: repeat,
        title: hear.trim().slice(0, 60),
      });
      const poller = pollWhileVisible(async () => {
        if (!job) return;
        job = await mapi.getAudioJob(job.id);
        if (job.status !== 'queued' && job.status !== 'running') {
          poller.stop();
          if (job.status === 'succeeded') {
            hear = '';
            await loadClips(wsId);
          }
        }
      }, { ms: 2000, immediate: false });
    } catch {
      toasts.warn('I couldn’t start that', 'Please try again in a moment.');
    } finally {
      starting = false;
    }
  }
</script>

{#if ready === false}
  <div data-testid="create-sound-waiting">
    <EmptyState
      variant="page"
      icon="volume"
      title="Sounds aren’t ready yet"
      body="You’ll be able to make sounds and music here soon."
      actionLabel="Make a voice instead"
      actionKind="secondary"
      onaction={() => router.go(createHref('voice'))}
    />
  </div>
{:else if ready === null}
  <div class="sc-calm"><span class="spinner" role="img" aria-label="Loading"></span></div>
{:else}
  <h1 class="cr-h1">Sounds and music</h1>
  <div class="cr-chips" role="group" aria-label="Sound or music">
    <button type="button" class="cr-chip" aria-pressed={what === 'sound'} onclick={() => (what = 'sound')}><Icon name="volume" size={16} /> A sound</button>
    <button type="button" class="cr-chip" aria-pressed={what === 'music'} onclick={() => (what = 'music')}><Icon name="play" size={16} /> Music</button>
  </div>
  <div class="cr-card sc-form">
    <label class="sc-label" for="sc-place">Where are we?</label>
    <input id="sc-place" class="sc-input" dir="auto" maxlength="120" placeholder="A windy cave" bind:value={place} />
    <div class="cr-chips">{#each PLACES as x (x)}<button type="button" class="cr-chip" onclick={() => (place = x)}>{x}</button>{/each}</div>
    <label class="sc-label" for="sc-hear">What should we hear?</label>
    <input id="sc-hear" class="sc-input" dir="auto" maxlength="300" placeholder={what === 'music' ? 'Slow, happy music with a flute' : 'Footsteps on stones'} bind:value={hear} data-testid="create-sound-words" />
    {#if what === 'sound'}
      <div class="cr-chips">{#each SOUNDS as x (x)}<button type="button" class="cr-chip" onclick={() => (hear = x)}>{x}</button>{/each}</div>
    {/if}
    <button type="button" class="cr-chip sc-repeat" aria-pressed={repeat} onclick={() => (repeat = !repeat)}>
      <Icon name={repeat ? 'check' : 'refresh'} size={16} /> Play it over and over
    </button>
    <p class="sc-time"><Icon name="clock" size={16} /> This takes {timeText(what === 'music' ? 55 : 90).toLowerCase()}.</p>
  </div>
  {#if job && (job.status === 'queued' || job.status === 'running')}
    <p class="sc-now" data-testid="create-sound-making">Making it… {job.progress_pct}%</p>
  {:else if job?.status === 'failed'}
    <p class="cr-note">That one didn’t work. Try different words.</p>
  {/if}
  <div class="cr-sticky">
    <button type="button" class="cr-primary" disabled={!can} onclick={() => void make()} data-testid="create-make">{starting ? 'Starting…' : 'Make it'}</button>
  </div>
  {#if clips.length}
    <h2 class="cr-h2">Made so far</h2>
    <div class="cr-card">
      {#each clips as c (c.id)}
        <div class="sc-clip" data-testid="create-sound-clip">
          <AudioLine clipId={c.id} title={c.title || c.text} detail={c.kind === 'music' ? 'Music' : 'Sound'} loop={repeating[c.id] ?? false} />
          <button type="button" class="cr-chip" aria-pressed={repeating[c.id] ?? false} onclick={() => (repeating = { ...repeating, [c.id]: !(repeating[c.id] ?? false) })}>Repeat</button>
        </div>
      {/each}
    </div>
  {/if}
{/if}

<style>
  .sc-calm {
    display: grid;
    justify-items: center;
    padding-block: var(--sp-7);
  }
  .sc-form {
    margin-block-start: var(--sp-3);
  }
  .sc-label {
    display: block;
    font-weight: 600;
    font-size: var(--fs-l);
    margin-block: var(--sp-3) var(--sp-2);
  }
  .sc-label:first-child {
    margin-block-start: 0;
  }
  .sc-input {
    inline-size: 100%;
    min-block-size: 48px;
    padding: var(--sp-2) var(--sp-3);
    border: 1.5px solid var(--control-border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-l);
  }
  .sc-repeat {
    margin-block-start: var(--sp-3);
  }
  .sc-time {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    color: var(--text-dim);
    font-size: var(--fs-m);
    margin: var(--sp-3) 0 0;
  }
  .sc-now {
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .sc-clip {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
    flex-wrap: wrap;
  }
</style>
