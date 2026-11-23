<script lang="ts">
  // Voice Studio's synthesis fields: three fixed pairs (Provider|Voice,
  // Character|Clip title, Speed|Format), one column when a pair would cut its
  // text, so no field is orphaned and labels never step.
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import type { TtsFormat, TtsProvider, TtsVoice } from '../../../lib/api/types';
  import { groupVoicesByLocale, localeLabel } from './ttsPresets';

  interface Props {
    providers: TtsProvider[];
    voices: TtsVoice[];
    loadingVoices: boolean;
    voicesError: string | null;
    onretryvoices: () => void;
    supportedFormats: TtsFormat[];
    providerId: string;
    voiceId: string;
    characterName: string;
    title: string;
    speed: number;
    format: TtsFormat;
  }
  let {
    providers,
    voices,
    loadingVoices,
    voicesError,
    onretryvoices,
    supportedFormats,
    providerId = $bindable(),
    voiceId = $bindable(),
    characterName = $bindable(),
    title = $bindable(),
    speed = $bindable(),
    format = $bindable(),
  }: Props = $props();

  const voiceGroups = $derived(groupVoicesByLocale(voices));
</script>

<div class="tts-controls">
  <div class="tts-field">
    <label class="k" for="tts-provider">Provider</label>
    <span class="tts-sel">
      <select id="tts-provider" class="input" bind:value={providerId} data-testid="tts-provider-select">
        {#each providers as p (p.id)}
          <option value={p.id} disabled={!p.enabled || !p.ready}>
            {p.name}{p.ready ? '' : ' (Needs setup)'}
          </option>
        {/each}
      </select>
      <Icon name="chevronDown" size={14} />
    </span>
  </div>

  <div class="tts-field">
    <label class="k" for="tts-voice">Voice{loadingVoices ? ' (loading…)' : voices.length ? ` (${voices.length})` : ''}</label>
    {#if voices.length > 0}
      <span class="tts-sel">
        <select id="tts-voice" class="input" bind:value={voiceId} data-testid="tts-voice-select">
          {#each voiceGroups as g (g.locale)}
            {#if g.locale}
              <optgroup label={localeLabel(g.locale)}>
                {#each g.voices as v (v.id)}<option value={v.id}>{v.name}</option>{/each}
              </optgroup>
            {:else}
              {#each g.voices as v (v.id)}<option value={v.id}>{v.name}</option>{/each}
            {/if}
          {/each}
        </select>
        <Icon name="chevronDown" size={14} />
      </span>
    {:else}
      <input id="tts-voice" dir="ltr" class="input" bind:value={voiceId} placeholder="Voice id, e.g. Samantha" />
    {/if}
    {#if voicesError}
      <div data-testid="tts-voices-error">
        <LoadState what="voices" variant="compact" error={voicesError} empty onretry={onretryvoices} />
      </div>
    {/if}
  </div>
</div>

<div class="tts-controls">
  <div class="tts-field">
    <label class="k" for="tts-char">Character</label>
    <input id="tts-char" dir="auto" class="input" bind:value={characterName} placeholder="Narrator, Hero…" />
  </div>
  <div class="tts-field">
    <label class="k" for="tts-title">Clip title</label>
    <input id="tts-title" dir="auto" class="input" bind:value={title} placeholder="vo_intro_01" />
  </div>
</div>

<div class="tts-controls">
  <div class="tts-field">
    <label class="k" for="tts-speed">Speed ({speed.toFixed(2)}×)</label>
    <input id="tts-speed" class="tts-range" type="range" min="0.5" max="2.0" step="0.05" bind:value={speed} />
  </div>
  <div class="tts-field">
    <label class="k" for="tts-fmt">Format</label>
    <span class="tts-sel">
      <select id="tts-fmt" class="input" bind:value={format}>
        {#each supportedFormats as f (f)}
          <option value={f}>{f.toUpperCase()}</option>
        {/each}
      </select>
      <Icon name="chevronDown" size={14} />
    </span>
  </div>
</div>

<style>
  .k {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .tts-controls {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px 12px;
    align-items: start;
  }
  @container tts-main (max-width: 479px) {
    .tts-controls {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .tts-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  /* Native selects ignore min-height on iOS (27 px next to 39 px inputs), so
     they drop the platform chrome and take the input height + our chevron. */
  .tts-sel {
    position: relative;
    display: block;
  }
  .tts-sel select {
    appearance: none;
    inline-size: 100%;
    padding-inline-end: 32px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tts-sel > :global(svg) {
    position: absolute;
    inset-inline-end: 10px;
    inset-block-start: 50%;
    translate: 0 -50%;
    color: var(--text-dim);
    pointer-events: none;
  }
  .tts-range {
    appearance: none;
    inline-size: 100%;
    block-size: 27px;
    margin: 0;
    background: transparent;
    cursor: pointer;
  }
  .tts-range::-webkit-slider-runnable-track {
    block-size: 4px;
    border-radius: 999px;
    background: var(--control-border);
  }
  .tts-range::-moz-range-track {
    block-size: 4px;
    border-radius: 999px;
    background: var(--control-border);
  }
  .tts-range::-webkit-slider-thumb {
    appearance: none;
    inline-size: 20px;
    block-size: 20px;
    margin-block-start: -8px;
    border: 2px solid var(--surface);
    border-radius: 50%;
    background: var(--accent-solid);
    box-shadow: var(--shadow-xs);
  }
  .tts-range::-moz-range-thumb {
    inline-size: 14px;
    block-size: 14px;
    border: 2px solid var(--surface);
    border-radius: 50%;
    background: var(--accent-solid);
  }
  .tts-range:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  @media (pointer: coarse) {
    .tts-range {
      block-size: 40px;
    }
    .tts-range::-webkit-slider-thumb {
      inline-size: 28px;
      block-size: 28px;
      margin-block-start: -12px;
    }
    .tts-range::-moz-range-thumb {
      inline-size: 22px;
      block-size: 22px;
    }
  }
</style>
