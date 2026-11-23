<script lang="ts">
  // Smart mode, the default front door: ONE box — say what you want — and
  // "Make it". The app plans the people, things, voices and sounds (a
  // production) and shows the plan before making anything. A dropped
  // picture goes to the one-thing path (a production takes words only). The
  // quiet chip beside "Use a picture" says who plans it (agent · model) and
  // opens the shared model panel; the pick is remembered per workspace, the
  // way Threads remembers its start pick (agent · model · effort). Above the
  // box, a quiet "In progress" strip lists the creations being made; a new
  // one can always be started and takes its turn. Once there are creations, a
  // quiet "For" picker says which project the scene is for: a new one (the
  // default, so nothing of another project is assumed), or an existing one,
  // whose people, things and place the planner uses again where they fit.
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { ApiError } from '../../../lib/api/client';
  import { untrack } from 'svelte';
  import { createProduction, listCreations } from '../../../lib/api/productions';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { formatEffortLabel, formatModelLabel, supportsEffortSelection, type EffortLevel } from '../../../lib/providerModels';
  import { terminalTakesEffort } from '../../../lib/providerRegistry';
  import { agentProviders, defaultAgentProvider, harnessLabel, orderThreadHarnesses, providerReadiness } from '../../../lib/providers';
  import { router } from '../../../lib/router.svelte';
  import { readStartPick, writeStartPick, type StartPick } from '../../../lib/startPick';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { createStore, readPicture } from './createStore.svelte';
  import { SMART_EXAMPLES, agentPickLabel, createHref, productionBody, projectsOf, type ProjectChoice } from './plan';
  import KitMissing from './KitMissing.svelte';
  import InProgress from './InProgress.svelte';

  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));

  interface Props {
    /** "New scene in this project": the project it starts in. */
    project?: string | null;
  }
  let { project = null }: Props = $props();
  let projects = $state<ProjectChoice[]>([]);
  let projectId = $state(untrack(() => project ?? ''));
  const projectName = $derived(projects.find((x) => x.id === projectId)?.name ?? null);
  $effect(() => {
    void listCreations()
      .then((c) => (projects = projectsOf(c.productions)))
      .catch(() => undefined);
  });

  let idea = $state('');
  let starting = $state(false);
  let pictureFor = $state(false);
  let fileEl = $state<HTMLInputElement | null>(null);
  const ready = $derived(createStore.engine.checked && !!createStore.engine.provider);

  // Who plans it: the last pick in this workspace, else (null) the
  // workspace's default agent with its default model — what the daemon runs
  // when the request names neither.
  let pick = $state<StartPick | null>(null);
  const agentReady = (p: string): boolean => providerReadiness(p).available;
  const agents = $derived(orderThreadHarnesses(agentProviders()));
  const wsDefault = $derived(String(ws.current?.settings?.default_provider ?? ''));
  const agent = $derived(pick?.provider ?? (agents.includes(wsDefault) ? wsDefault : defaultAgentProvider()));
  // Effort the way Threads offers it: only for an agent that takes one, said on the chip when one was picked.
  const takesEffort = $derived(terminalTakesEffort(agent, auth.meta, supportsEffortSelection(agent, auth.meta)));
  const effort = $derived(takesEffort ? ((pick?.effort ?? null) as EffortLevel | null) : null);
  const agentLabel = $derived(agentPickLabel(harnessLabel(agent), pick?.model ?? null, formatModelLabel(agent, pick?.model, auth.meta), effort ? formatEffortLabel(effort) : null));
  /** Other creations are being made: this one takes its turn. */
  const busyNow = $derived(createStore.inProgress().some((p) => p.status === 'running' || p.status === 'planning'));
  let pickScope = '';
  let agentEl = $state<HTMLButtonElement | null>(null);
  let pickerOpen = $state(false);
  const pickStore = (): Storage | null => (typeof localStorage === 'undefined' ? null : localStorage);
  $effect(() => {
    const scope = `create:${ws.currentId ?? ''}`;
    if (scope === pickScope || agents.length === 0) return;
    pickScope = scope;
    pick = readStartPick(pickStore(), scope, agents, agentReady);
  });
  function choose(next: StartPick): void {
    pick = next;
    writeStartPick(pickStore(), pickScope, next);
  }

  async function makeIt(): Promise<void> {
    const text = idea.trim();
    if (!text || starting) return;
    const wsId = ws.currentId;
    if (!wsId) return;
    starting = true;
    try {
      const p = await createProduction(productionBody(wsId, text, pick ? { ...pick, effort } : null, projectId || null));
      createStore.watchProduction(p);
      router.go(createHref('p', p.id));
    } catch (e) {
      if (e instanceof ApiError && e.status === 400) toasts.warn('I couldn’t start that', e.message);
      else if (e instanceof ApiError && (e.status === 404 || e.status === 405)) {
        toasts.warn('Smart making isn’t ready on this computer yet', 'Choose what to make yourself for now.');
        router.go(createHref('kinds'));
      } else toasts.warn('I couldn’t start that', 'Please try again in a moment.');
    } finally {
      starting = false;
    }
  }

  async function onPicture(e: Event): Promise<void> {
    const input = e.currentTarget as HTMLInputElement;
    const f = input.files?.[0];
    input.value = '';
    if (!f) return;
    try {
      createStore.picture = await readPicture(f);
      pictureFor = true;
    } catch (err) {
      toasts.warn('That picture won’t work', err instanceof Error ? err.message : '');
    }
  }
</script>

{#if createStore.engine.checked && !ready && !createStore.engine.error}
  <KitMissing />
{:else if pictureFor && createStore.picture}
  <h1 class="cr-h1">What is in your picture?</h1>
  <div class="sh-pic cr-card">
    <img src={createStore.picture.url} alt="What you chose" />
    <div class="sh-pick">
      <button type="button" class="cr-secondary" onclick={() => router.go(createHref('new', 'character'))}><Icon name="user" size={16} /> A character</button>
      <button type="button" class="cr-secondary" onclick={() => router.go(createHref('new', 'object'))}><Icon name="box" size={16} /> A thing</button>
    </div>
  </div>
  <button type="button" class="cr-quiet" onclick={() => ((pictureFor = false), (createStore.picture = null))}>Use words instead</button>
{:else}
  <h1 class="cr-h1">What do you want to make?</h1>
  <p class="cr-sub" data-testid="create-project-sub">{projectName ? `The people, things and place of “${projectName}” are used again where they fit.` : 'Tell me in your own words.'}</p>
  <div class="cr-card">
    {#if projects.length}
      <label class="sh-project">
        <span>For</span>
        <select bind:value={projectId} data-testid="create-project">
          <option value="">A new project</option>
          {#each projects as pr (pr.id)}<option value={pr.id}>{pr.name}</option>{/each}
        </select>
      </label>
    {/if}
    <label class="sh-sr" for="create-idea">Describe what you want</label>
    <textarea id="create-idea" class="cr-field" dir="auto" maxlength="4000" placeholder="Describe what you want…" bind:value={idea} data-testid="create-idea"></textarea>
    <div class="cr-chips" aria-label="Ideas to try">
      {#each SMART_EXAMPLES as ex (ex.label)}
        <button type="button" class="cr-chip" onclick={() => (idea = ex.text)}>{ex.label}</button>
      {/each}
    </div>
    <div class="sh-foot">
      <button type="button" class="sh-link" onclick={() => fileEl?.click()}><Icon name="image" size={16} /> Use a picture</button>
      <button
        bind:this={agentEl}
        type="button"
        class="sh-link sh-agent"
        aria-haspopup="dialog"
        aria-expanded={pickerOpen}
        aria-label="Who plans it: {agentLabel}. Change"
        title="Who plans it: {agentLabel}"
        data-testid="create-agent"
        onpointerenter={() => ModelPicker.prefetch()}
        onclick={() => ((pickerOpen = true), ModelPicker.prefetch())}
      >
        {#if hasProviderIcon(agent)}<ProviderIcon provider={agent} size={16} />{:else}<Icon name="terminal" size={16} />{/if}
        <span class="sh-agent-label">{agentLabel}</span>
      </button>
    </div>
  </div>
  <div class="cr-sticky">
    <button type="button" class="cr-primary" disabled={!idea.trim() || starting} onclick={() => void makeIt()} data-testid="create-make">
      {starting ? 'Starting…' : 'Make it'}
    </button>
    {#if busyNow}<p class="sh-turn" data-testid="create-takes-turn">Something else is being made. You can start this one now: it takes its turn.</p>{/if}
  </div>
  <!-- Below the box: on a phone, creations in progress never push "Make it" off the screen. -->
  <InProgress />
  <button type="button" class="cr-quiet" onclick={() => router.go(createHref('kinds'))} data-testid="create-choose">Choose what to make myself</button>
  <button type="button" class="cr-quiet sh-more" onclick={() => router.go('design')}>More options</button>
{/if}

{#if pickerOpen && agentEl && ModelPicker.component}
  <ModelPicker.component
    anchor={agentEl}
    providers={agents}
    provider={agent}
    model={pick?.model ?? null}
    {effort}
    mode="full"
    onprovider={(p) => choose({ provider: p, model: null, effort: null })}
    onmodel={(m) => choose({ provider: pick?.provider ?? agent, model: m, effort: pick?.effort ?? null })}
    oneffort={takesEffort ? (e) => choose({ provider: pick?.provider ?? agent, model: pick?.model ?? null, effort: e }) : undefined}
    onclose={() => (pickerOpen = false)}
  />
{/if}

<input bind:this={fileEl} class="sh-file" type="file" accept="image/png,image/jpeg,image/webp" onchange={onPicture} tabindex="-1" aria-hidden="true" />


<style>
  .sh-sr {
    position: absolute;
    inline-size: 1px;
    block-size: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .sh-project {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin-block-end: var(--sp-2);
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .sh-project select {
    min-inline-size: 0;
    max-inline-size: 100%;
    min-block-size: var(--cr-hit);
    padding-inline: var(--sp-2);
    border: 1px solid var(--control-border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
  }
  .sh-foot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
    margin-block-start: var(--sp-3);
  }
  .sh-link {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    min-block-size: var(--cr-hit);
    padding-inline: var(--sp-2);
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-m);
    cursor: pointer;
  }
  .sh-link:first-child {
    flex: none;
  }
  .sh-agent {
    min-inline-size: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .sh-agent-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sh-more {
    font-size: var(--fs-s);
  }
  .sh-turn {
    margin: var(--sp-2) 0 0;
    color: var(--text-dim);
    font-size: var(--fs-m);
    text-align: center;
  }
  .sh-file {
    display: none;
  }
  .sh-pic {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--sp-4);
    align-items: center;
  }
  .sh-pic img {
    inline-size: 100%;
    max-block-size: 320px;
    object-fit: contain;
    border-radius: var(--radius-m);
  }
  .sh-pick {
    display: grid;
    gap: var(--sp-3);
  }
  @media (max-width: 640px) {
    .sh-pic {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
