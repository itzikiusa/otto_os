<!--
  Quick-create from the rail's "+": a name, an avatar and (optionally) a
  starting point. Everything else is set up by chatting with the agent —
  persona, routines, rules — or in Settings (model, folder, delivery).
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
    onclose: () => void;
  }
  let { needsMain, oncreated, onclose }: Props = $props();

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

  /** What it should do, in the user's words — becomes its instructions. */
  // svelte-ignore state_referenced_locally
  let purpose = $state(needsMain ? (templateById('main-agent')?.description ?? '') : '');
  function pick(id: string): void {
    start = start === id ? '' : id;
    const t = templateById(start);
    if (t) {
      if (!name.trim() || TEMPLATES.some((x) => x.name === name)) name = t.name;
      purpose = t.description;
    }
  }

  /** Common jobs that need no template — they only fill the name and purpose. */
  const IDEAS = [
    {
      label: 'Watch my PRs',
      name: 'PR watcher',
      purpose: 'Watch my open pull requests and tell me when one needs me: reviews, failing checks, conflicts.',
      routine: { name: 'Morning PR brief', instruction: 'List my open pull requests that need me (reviews, failing checks, conflicts), most urgent first.' },
    },
    {
      label: 'Morning brief',
      name: 'Morning brief',
      purpose: 'Each workday morning, brief me on what needs me today: my PRs, failing builds and anything urgent.',
      routine: { name: 'Morning brief', instruction: 'Brief me on what needs me today: my PRs, failing builds and anything urgent.' },
    },
  ];
  /** The idea picked — its routine prefills the first routine form. */
  let picked = $state<(typeof IDEAS)[number] | null>(null);
  function idea(i: (typeof IDEAS)[number]): void {
    start = '';
    picked = i;
    if (!name.trim() || [...IDEAS, ...TEMPLATES].some((x) => x.name === name)) name = i.name;
    purpose = i.purpose;
  }

  async function create(): Promise<void> {
    const wsId = ws.currentId;
    if (busy) return;
    if (!wsId) {
      error = 'Select a workspace first — what you typed is kept.';
      return;
    }
    if (!name.trim()) {
      error = 'Give it a name.';
      return;
    }
    busy = true;
    error = '';
    const t = start ? templateById(start) : undefined;
    // A template's full instructions, unless its one-line purpose was rewritten.
    const soul = t && purpose.trim() === t.description ? t.soul_md : purpose.trim();
    try {
      const a = await personalAgents.create(wsId, {
        name: name.trim(),
        avatar: avatar.trim(),
        soul_md: soul,
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
      // An idea's routine waits in "Add routine" (prefilled), not a toast action.
      const draft = !t && picked && purpose.trim() === picked.purpose ? picked.routine : null;
      const drafted = !!draft;
      if (draft) personalAgents.routineDrafts = { ...personalAgents.routineDrafts, [a.id]: draft };
      toasts.success(`Created ${a.name}`, t?.schedule ? 'Chat with it to set up the rest.' : drafted ? 'Its first routine is ready in the panel.' : 'Give it a routine from its panel, or just ask it.');
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
    <div class="field">
      <label for="{uid}-purpose">What it does</label>
      <textarea id="{uid}-purpose" class="input" rows="3" dir="auto" bind:value={purpose} placeholder="e.g. Watch my pull requests and tell me when one needs me"></textarea>
      <div class="qc-starts" role="group" aria-label="Or start from">
        {#if needsMain}<button type="button" class="qc-start" aria-pressed={start === 'main-agent'} onclick={() => pick('main-agent')}>Main agent</button>{/if}
        {#each IDEAS as i (i.label)}<button type="button" class="qc-start" aria-pressed={!start && purpose === i.purpose} title={i.purpose} onclick={() => idea(i)}>{i.label}</button>{/each}
        {#each TEMPLATES as t (t.id)}<button type="button" class="qc-start" aria-pressed={start === t.id} title={t.description} onclick={() => pick(t.id)}>{t.title}</button>{/each}
      </div>
    </div>
    <div class="field">
      <label for="{uid}-tone">Personality</label>
      <select id="{uid}-tone" class="input" value={personality} onchange={(e) => pickPersonality((e.currentTarget as HTMLSelectElement).value)}>
        {#each PRESETS as p (p.id)}<option value={p.id}>{p.label}</option>{/each}
      </select>
      {#if sample}<p class="qc-hint" dir="auto">{sample.description}</p>{/if}
    </div>
    <fieldset class="qc-avatars">
      <legend>Avatar</legend>
      {#each CHARACTERS as c (c.id)}
        <button type="button" class="qc-glyph" class:on={avatar === `char:${c.id}`} aria-label="Avatar: {c.label}" title={c.label} aria-pressed={avatar === `char:${c.id}`} onclick={() => { avatar = `char:${c.id}`; avatarPicked = true; }}>
          <img src={c.url} alt="" width="24" height="24" loading="lazy" decoding="async" />
        </button>
      {/each}
    </fieldset>
    <p class="qc-hint">Add routines from its panel, or just tell it in chat.</p>
    {#if error}<p class="qc-err" use:announceShown={{ text: error, assertive: true }}>{error}</p>{/if}
    <div class="qc-actions">
      <span class="qc-gap"></span>
      <button type="button" class="btn" onclick={onclose}>Cancel</button>
      <button type="submit" class="btn primary" disabled={busy}>{busy ? 'Creating…' : 'Create'}</button>
    </div>
  </form>
</Modal>

<style>
  .qc { display: flex; flex-direction: column; gap: 12px; }
  .qc :global(.field) { margin: 0; }
  .qc-who { display: flex; align-items: flex-end; gap: 12px; }
  .qc-name { flex: 1; margin: 0; }
  .qc-avatars { display: flex; flex-wrap: wrap; gap: 6px; border: 0; padding: 0; margin: 0; align-items: center; }
  .qc-avatars legend { font-size: var(--fs-s); color: var(--text-dim); margin-block-end: 4px; }
  .qc-glyph {
    inline-size: 32px; block-size: 32px; border-radius: var(--radius-m); border: 1px solid var(--border);
    background: var(--surface-2); cursor: pointer; padding: 2px; display: grid; place-items: center;
  }
  .qc-glyph img { display: block; }
  .qc-glyph.on { border-color: var(--accent-line-strong); background: var(--accent-soft); }
  .qc-glyph:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .qc-hint { margin: 4px 0 0; font-size: var(--fs-s); color: var(--text-dim); }
  .qc-starts { display: flex; flex-wrap: wrap; gap: 6px; margin-block-start: 6px; }
  .qc-start {
    border: 1px solid var(--border); border-radius: 999px; background: var(--surface); color: var(--text-dim);
    padding: 2px 10px; font: inherit; font-size: var(--fs-s); cursor: pointer;
  }
  .qc-start:hover { color: var(--text); border-color: var(--border-strong); }
  .qc-start[aria-pressed='true'] { color: var(--accent-text); border-color: var(--accent-line-strong); background: var(--accent-soft); }
  .qc-start:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .qc-err { margin: 0; color: var(--danger); font-size: var(--fs-s); }
  .qc-actions { display: flex; gap: 8px; align-items: center; }
  .qc-gap { flex: 1; }
</style>
