<!--
  An agent's personality on its General settings: one tone preset (a select,
  with its description and a sample answer) plus the owner's own words.
  Tone only — never rules, permissions or approvals.
-->
<script lang="ts">
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { PersonalAgentAutonomy } from '../../lib/api/types';
  import { personalityOf, personalityPresets } from './personalities';

  interface Props {
    agentId: string;
    name: string;
    autonomy: PersonalAgentAutonomy | null;
    /** May change the preset. */
    editable: boolean;
    /** May write its own words (owner-only). */
    ownerEditable: boolean;
    onsaved: (a: PersonalAgentAutonomy) => void;
  }
  let { agentId, name, autonomy, editable, ownerEditable, onsaved }: Props = $props();
  const uid = $props.id();

  const PRESETS = personalityPresets();
  const persona = $derived(personalityOf(autonomy?.personality?.preset, !!autonomy?.primary));
  let noteDraft = $state<string | null>(null);
  /** The calm save cue (the rules section's pattern). */
  let saveState = $state<'' | 'saving' | 'saved'>('');
  let savedTimer: ReturnType<typeof setTimeout> | undefined;

  async function save(body: { preset?: string; note?: string }): Promise<void> {
    saveState = 'saving';
    clearTimeout(savedTimer);
    try {
      await personalAgentsApi.setPersonality(agentId, body);
      onsaved(await personalAgentsApi.autonomy(agentId));
      void personalAgents.loadAutonomy(agentId);
      noteDraft = null;
      saveState = 'saved';
      savedTimer = setTimeout(() => (saveState = ''), 2000);
    } catch (e) {
      saveState = '';
      toasts.error('Couldn’t change the personality', loadErrorText(e));
    }
  }
</script>

<section class="pa-panel pers" data-testid="pa-personality">
  <div class="pers-head">
    <h2>Personality</h2>
    <span class="dim" aria-live="polite">{saveState === 'saving' ? 'Saving…' : saveState === 'saved' ? 'Saved' : ''}</span>
  </div>
  <div class="field">
    <label for="{uid}-tone">Tone</label>
    <select id="{uid}-tone" class="input" value={persona?.id ?? ''} disabled={!editable} onchange={(e) => void save({ preset: (e.currentTarget as HTMLSelectElement).value })}>
      {#each PRESETS as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
    </select>
  </div>
  {#if persona}
    <p class="dim">{persona.description}</p>
    <p class="sample" dir="auto"><span class="dim">How {name} would answer:</span> “{persona.sample}”</p>
  {/if}
  {#if ownerEditable}
    {#if noteDraft === null}
      <button type="button" class="pers-link" onclick={() => (noteDraft = autonomy?.personality?.note ?? '')}>{autonomy?.personality?.note ? `Your words: “${autonomy.personality.note}”` : 'Add your own words'}</button>
    {:else}
      <div class="pers-row">
        <input class="input" dir="auto" maxlength="400" aria-label="How it should come across, in your words" placeholder="e.g. a bit more blunt, no emoji" bind:value={noteDraft} />
        <button type="button" class="btn small" onclick={() => void save({ note: noteDraft ?? '' })}>Save</button>
      </div>
    {/if}
  {/if}
  <p class="dim">Tone only — it never changes rules or approvals. You can also say “be more blunt” in chat.</p>
</section>

<style>
  .pa-panel { border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 12px 14px; color: var(--text); min-width: 0; }
  .pers { display: flex; flex-direction: column; gap: 8px; }
  .pers-head { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; }
  .pers h2 { margin: 0; font-size: var(--fs-l); font-weight: 600; }
  .pers :global(.field) { margin-bottom: 0; }
  .dim { margin: 0; color: var(--text-dim); font-size: var(--fs-s); }
  .sample { margin: 0; font-size: var(--fs-m); }
  .pers-row { display: flex; gap: 6px; align-items: center; }
  .pers-row > :global(.input) { flex: 1; min-width: 0; }
  .pers-link { align-self: flex-start; border: 0; background: none; padding: 0; font: inherit; font-size: var(--fs-s); color: var(--accent-text); cursor: pointer; text-align: start; }
  .pers-link:hover { text-decoration: underline; }
</style>
