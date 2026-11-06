<!--
  Quick-create from the rail's "+": a name, an avatar and (optionally) a
  starting point. Everything else is set up by chatting with the agent —
  persona, routines, rules. "More options…" opens the full sheet.
-->
<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import type { PersonalAgent } from '../../lib/api/types';
  import AgentAvatar from './AgentAvatar.svelte';
  import { agentTemplates, templateById } from './templates';
  import { avatarForPersonality, CHARACTERS } from './characters';
  import { personalityPresets } from './personalities';
  import { personalAgentsApi } from '../../lib/api/personalAgents';

  interface Props {
    /** No main agent yet: offer (and pre-pick) the main-agent start. */
    needsMain: boolean;
    oncreated: (a: PersonalAgent) => void;
    onmore: () => void;
    onclose: () => void;
  }
  let { needsMain, oncreated, onmore, onclose }: Props = $props();

  const TEMPLATES = agentTemplates().filter((t) => !t.primary);
  const PRESETS = personalityPresets();
  // svelte-ignore state_referenced_locally
  let start = $state<string>(needsMain ? 'main-agent' : '');
  // svelte-ignore state_referenced_locally
  let name = $state(needsMain ? 'Otto' : '');
  // svelte-ignore state_referenced_locally
  let personality = $state(needsMain ? 'chief' : 'warm');
  // svelte-ignore state_referenced_locally
  let avatar = $state(avatarForPersonality(needsMain ? 'chief' : 'warm'));
  /** The avatar follows the personality until the user picks one. */
  let avatarPicked = $state(false);
  function pickPersonality(id: string): void {
    personality = id;
    if (!avatarPicked) avatar = avatarForPersonality(id);
  }
  const sample = $derived(PRESETS.find((p) => p.id === personality));
  let busy = $state(false);
  let error = $state('');
  const uid = $props.id();

  function pick(id: string): void {
    start = id;
    const t = templateById(id);
    if (t && !name.trim()) name = t.name;
    if (id === 'main-agent' && !name.trim()) name = 'Otto';
  }

  async function create(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || busy) return;
    if (!name.trim()) {
      error = 'Give it a name.';
      return;
    }
    busy = true;
    error = '';
    const t = start ? templateById(start) : undefined;
    try {
      const a = await personalAgents.create(wsId, {
        name: name.trim(),
        avatar: avatar.trim(),
        soul_md: t?.soul_md ?? '',
        enabled: true,
        ...(t?.primary ? { primary: true } : {}),
      });
      if (t?.schedule) {
        await personalAgents.createSchedule(a.id, {
          schedule: t.schedule,
          timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
          directive: t.directive,
          enabled: true,
          idempotency_key: crypto.randomUUID(),
        });
      }
      await personalAgentsApi.setPersonality(a.id, { preset: personality }).catch(() => {});
      await personalAgents.loadAutonomy(a.id);
      toasts.success(`Created ${a.name}`, 'Chat with it to set up the rest.');
      oncreated(a);
    } catch (e) {
      error = `Couldn’t create the agent. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="New agent" width={440} {onclose}>
  <form class="qc" onsubmit={(e) => { e.preventDefault(); void create(); }} data-testid="pa-quick-create">
    <div class="qc-who">
      <AgentAvatar {avatar} name={name || '?'} size={48} />
      <div class="field qc-name">
        <label for="{uid}-name">Name</label>
        <input id="{uid}-name" class="input" bind:value={name} maxlength="80" placeholder="e.g. Bitbucket helper" dir="auto" />
      </div>
    </div>
    <fieldset class="qc-avatars">
      <legend>Personality</legend>
      {#each PRESETS as p (p.id)}
        <button type="button" class="qc-pchip" class:on={personality === p.id} aria-pressed={personality === p.id} title={p.description} onclick={() => pickPersonality(p.id)}>{p.label}</button>
      {/each}
      {#if sample}<p class="qc-hint qc-sample" dir="auto">{sample.description} How {name.trim() || 'it'} would answer: “{sample.sample}”</p>{/if}
    </fieldset>
    <fieldset class="qc-avatars">
      <legend>Avatar</legend>
      {#each CHARACTERS as c (c.id)}
        <button type="button" class="qc-glyph" class:on={avatar === `char:${c.id}`} aria-label="Avatar: {c.label}" title={c.label} aria-pressed={avatar === `char:${c.id}`} onclick={() => { avatar = `char:${c.id}`; avatarPicked = true; }}>
          <img src={c.url} alt="" width="28" height="28" loading="lazy" decoding="async" />
        </button>
      {/each}
    </fieldset>
    <div class="field">
      <label for="{uid}-start">Start from</label>
      <select id="{uid}-start" class="input" value={start} onchange={(e) => pick((e.currentTarget as HTMLSelectElement).value)}>
        <option value="">Blank — set it up by chatting</option>
        {#if needsMain}<option value="main-agent">Main agent — routes your work to the others</option>{/if}
        {#each TEMPLATES as t (t.id)}<option value={t.id}>{t.title}</option>{/each}
      </select>
      {#if start}<p class="qc-hint">{templateById(start)?.description}</p>{/if}
    </div>
    {#if error}<p class="qc-err" use:announceShown={{ text: error, assertive: true }}>{error}</p>{/if}
    <div class="qc-actions">
      <button type="button" class="btn ghost" onclick={onmore}>More options…</button>
      <span class="qc-gap"></span>
      <button type="button" class="btn" onclick={onclose}>Cancel</button>
      <button type="submit" class="btn primary" disabled={busy}>{busy ? 'Creating…' : 'Create'}</button>
    </div>
  </form>
</Modal>

<style>
  .qc { display: flex; flex-direction: column; gap: 12px; }
  .qc-who { display: flex; align-items: flex-end; gap: 12px; }
  .qc-name { flex: 1; margin: 0; }
  .qc-avatars { display: flex; flex-wrap: wrap; gap: 6px; border: 0; padding: 0; margin: 0; align-items: center; }
  .qc-avatars legend { font-size: var(--fs-s); color: var(--text-dim); margin-block-end: 4px; }
  .qc-glyph {
    inline-size: 36px; block-size: 36px; border-radius: var(--radius-m); border: 1px solid var(--border);
    background: var(--surface-2); cursor: pointer; padding: 2px; display: grid; place-items: center;
  }
  .qc-glyph img { display: block; }
  .qc-pchip { border: 1px solid var(--border); border-radius: 999px; background: var(--surface); color: var(--text); padding: 2px 10px; font: inherit; font-size: var(--fs-s); cursor: pointer; }
  .qc-pchip.on { border-color: var(--accent-line-strong); background: var(--accent-soft); }
  .qc-pchip:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .qc-sample { flex-basis: 100%; }
  .qc-glyph.on { border-color: var(--accent-line-strong); background: var(--accent-soft); }
  .qc-glyph:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .qc-hint { margin: 4px 0 0; font-size: var(--fs-s); color: var(--text-dim); }
  .qc-err { margin: 0; color: var(--danger); font-size: var(--fs-s); }
  .qc-actions { display: flex; gap: 8px; align-items: center; }
  .qc-gap { flex: 1; }
</style>
