<script lang="ts">
  // Voice inside Create: who is talking (a made character or someone new),
  // how they sound (plain descriptions, never voice names), what they say —
  // then "Make them talk". Each line comes back as a speech bubble with
  // play, change the words, and say it again. (Writing the lines for you
  // from a scene isn't something Otto can do yet, so there is no button for it.)
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { listArtifacts } from '../../../lib/api/design';
  import * as mapi from '../../../lib/api/media';
  import type { DesignArtifact, TtsClip, TtsProvider, TtsVoice } from '../../../lib/api/types';
  import { loadErrorText } from '../../../lib/loadError';
  import { router } from '../../../lib/router.svelte';
  import { radioKey } from '../../../lib/radioKey';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import AudioLine from './AudioLine.svelte';
  import { pickVoiceProvider, type CreationMeta } from './createStore.svelte';
  import { PLAIN_VOICES, pickVoiceId, type PlainVoice } from './plan';

  interface Props {
    who: string | null;
  }
  let { who }: Props = $props();

  let provider = $state<TtsProvider | null>(null);
  let voices = $state<TtsVoice[]>([]);
  let characters = $state<DesignArtifact[]>([]);
  let speaker = $state<string | null>(null); // artifact id, or null = someone new
  let newName = $state('');
  let sound = $state<PlainVoice>(PLAIN_VOICES[0]);
  let lines = $state('');
  let clips = $state<TtsClip[]>([]);
  let error = $state<string | null>(null);
  let working = $state(false);
  let editing = $state<string | null>(null);
  let editText = $state('');
  const speakerName = $derived(speaker ? (characters.find((c) => c.id === speaker)?.title ?? 'Someone') : newName.trim() || 'Someone');
  const typed = $derived(lines.split('\n').map((l) => l.trim()).filter(Boolean));

  $effect(() => {
    const id = ws.currentId;
    const w = who;
    untrack(() => void load(id, w));
  });

  async function load(wsId: string | null, w: string | null): Promise<void> {
    if (!wsId) return;
    error = null;
    try {
      const [ps, arts] = await Promise.all([mapi.listTtsProviders(), listArtifacts({ workspace_id: wsId, studio: '3d', limit: 100 })]);
      provider = pickVoiceProvider(ps);
      characters = arts.filter((a) => a.format === 'glb' && (a.meta as { create?: CreationMeta })?.create?.kind === 'character').slice(0, 8);
      if (w && !characters.some((c) => c.id === w)) {
        const a = arts.find((x) => x.id === w);
        if (a) characters = [a, ...characters];
      }
      speaker = w ?? characters[0]?.id ?? null;
      voices = provider ? await mapi.listTtsVoices(provider.id).catch(() => []) : [];
      clips = speaker ? await mapi.listTtsClips({ workspace_id: wsId, artifact_id: speaker, limit: 30 }).catch(() => []) : [];
    } catch (e) {
      error = loadErrorText(e);
    }
  }

  async function say(text: string, choice = sound): Promise<TtsClip | null> {
    const wsId = ws.currentId;
    if (!wsId || !provider) return null;
    const voice = pickVoiceId(choice, voices) ?? provider.default_voice ?? 'Samantha';
    return mapi.synthesizeTts({
      workspace_id: wsId,
      provider_id: provider.id,
      voice_id: voice,
      text,
      speed: choice.speed,
      format: 'wav',
      title: text.slice(0, 60),
      character_name: speakerName,
      artifact_id: speaker,
    });
  }

  async function makeThemTalk(): Promise<void> {
    if (!typed.length || working) return;
    working = true;
    try {
      const made: TtsClip[] = [];
      for (const t of typed) {
        const c = await say(t);
        if (c) made.push(c);
      }
      clips = [...made.reverse(), ...clips];
      lines = '';
    } catch {
      toasts.warn('I couldn’t make them talk', 'Please try again in a moment.');
    } finally {
      working = false;
    }
  }

  async function redo(c: TtsClip, text = c.text): Promise<void> {
    working = true;
    try {
      const n = await say(text);
      if (n) clips = clips.map((x) => (x.id === c.id ? n : x));
      editing = null;
    } catch {
      toasts.warn('I couldn’t say it again', 'Please try again.');
    } finally {
      working = false;
    }
  }

</script>

{#if error}
  <EmptyState variant="page" tone="error" icon="warning" title="I couldn’t open voices" body={error} actionLabel="Try again" actionIcon="refresh" onaction={() => void load(ws.currentId, who)} />
{:else}
  <h1 class="cr-h1">Who is talking?</h1>
  <div class="cr-chips" role="group" aria-label="Who is talking">
    {#each characters as c (c.id)}
      <button type="button" class="cr-chip" aria-pressed={speaker === c.id} onclick={() => (speaker = c.id)}><Icon name="user" size={16} /> {c.title}</button>
    {/each}
    <button type="button" class="cr-chip" aria-pressed={speaker === null} onclick={() => (speaker = null)}><Icon name="plus" size={16} /> Someone new</button>
  </div>
  {#if speaker === null}
    <label class="vc-label" for="vc-name">Who is it?</label>
    <input id="vc-name" class="vc-input" dir="auto" maxlength="60" placeholder="An old wizard" bind:value={newName} />
  {/if}

  <h2 class="cr-h2">How do they sound?</h2>
  <div class="vc-voices" role="radiogroup" aria-label="How they sound">
    {#each PLAIN_VOICES as v (v.id)}
      <button type="button" class="vc-voice" role="radio" aria-checked={sound.id === v.id} tabindex={sound.id === v.id ? 0 : -1} onkeydown={radioKey} onclick={() => (sound = v)}>
        <span class="vc-ic" aria-hidden="true"><Icon name="mic" size={16} /></span>
        <span><b>{v.label}</b><small>{v.hint}</small></span>
      </button>
    {/each}
  </div>

  <h2 class="cr-h2">What do they say?</h2>
  <div class="cr-card">
    {#each clips as c (c.id)}
      <div class="vc-bubble" data-testid="create-bubble">
        <span class="vc-face" aria-hidden="true"><Icon name="user" size={16} /></span>
        <div class="vc-text">
          {#if editing === c.id}
            <input class="vc-input" dir="auto" maxlength="600" bind:value={editText} aria-label="New words" />
            <div class="vc-tools">
              <button type="button" class="btn" onclick={() => (editing = null)}>Cancel</button>
              <button type="button" class="btn" disabled={working || !editText.trim()} onclick={() => void redo(c, editText.trim())}>Say this</button>
            </div>
          {:else}
            <AudioLine clipId={c.id} title={c.character_name || speakerName} detail={c.text} />
            <div class="vc-tools">
              <button type="button" class="vc-tool" onclick={() => ((editing = c.id), (editText = c.text))}><Icon name="edit" size={16} /> Change the words</button>
              <button type="button" class="vc-tool" disabled={working} onclick={() => void redo(c)}><Icon name="refresh" size={16} /> Say it again</button>
            </div>
          {/if}
        </div>
      </div>
    {/each}
    <label class="vc-label" for="vc-lines">{clips.length ? 'Add more lines' : 'Type what they say — one line per row'}</label>
    <textarea id="vc-lines" class="cr-field vc-lines" dir="auto" maxlength="4000" placeholder="Halt! Who goes there?" bind:value={lines} data-testid="create-lines"></textarea>
  </div>
  <div class="cr-sticky">
    <button type="button" class="cr-primary" disabled={!typed.length || working || !provider} onclick={() => void makeThemTalk()} data-testid="create-talk">
      <Icon name="volume" size={16} /> {working ? 'Making them talk…' : 'Make them talk'}
    </button>
  </div>
{/if}

<style>
  .vc-label {
    display: block;
    font-weight: 600;
    margin-block: var(--sp-3) var(--sp-2);
  }
  .vc-input {
    inline-size: 100%;
    min-block-size: 48px;
    padding: var(--sp-2) var(--sp-3);
    border: 1.5px solid var(--control-border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
  }
  .vc-voices {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--sp-3);
  }
  .vc-voice {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-block-size: 64px;
    padding: var(--sp-3);
    border: 1.5px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .vc-voice[aria-checked='true'] {
    border-color: var(--accent-line-strong);
    background: var(--accent-soft);
  }
  .vc-voice:focus-visible {
    outline: 3px solid var(--accent-text);
    outline-offset: 2px;
  }
  .vc-voice small {
    display: block;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .vc-ic,
  .vc-face {
    flex: none;
    display: grid;
    place-items: center;
    inline-size: 44px;
    block-size: 44px;
    border-radius: 50%;
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .vc-bubble {
    display: flex;
    gap: var(--sp-3);
    align-items: flex-start;
    margin-block-end: var(--sp-3);
  }
  .vc-text {
    flex: 1;
    min-inline-size: 0;
    padding: var(--sp-1) var(--sp-3) var(--sp-2);
    border-radius: var(--radius-s) var(--radius-l) var(--radius-l) var(--radius-l);
    background: var(--surface-2);
  }
  .vc-tools {
    display: flex;
    gap: var(--sp-2);
    justify-content: flex-end;
  }
  .vc-tool {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding-inline: var(--sp-3);
    min-block-size: var(--cr-hit);
    border: 1.5px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
  }
  .vc-tools :global(.btn) {
    min-block-size: var(--cr-hit);
  }
  .vc-lines {
    min-block-size: 96px;
  }
  @media (max-width: 640px) {
    .vc-voices {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
