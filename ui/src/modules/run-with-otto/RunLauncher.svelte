<script lang="ts">
  // THE one button. A single input where the user pastes a Jira key, a
  // GitHub/Confluence URL, or a finding/story/test id — or just describes what
  // they want (free text → a channel run). As they type we debounce-detect the
  // source and show what Otto will run; the prominent "Run with Otto" button
  // launches it. Around the input: source chips (what you can paste), the
  // pipeline rail (how a run executes — and how this differs from Workflows),
  // and the launch parameters (mode, repo, provider/model). There is no
  // "auto-open PR" toggle: the daemon stores `auto_open_pr` but no engine path
  // reads it, and opening the PR is the one outward action that must stay an
  // explicit, confirmed click in RunDetail (patterns.md §5).
  import { api } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { runWithOtto } from '../../lib/stores/runWithOtto.svelte';
  import { runWithOttoApi } from '../../lib/api/runWithOtto';
  import { router } from '../../lib/router.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import RunStageRail from './RunStageRail.svelte';
  import { SOURCE_KINDS, sourceLabel } from './runStatus';
  import type { OttoRun, Repo, RunDetectResp, RunMode } from '../../lib/api/types';
  import { agentProviders, defaultAgentProvider, providerSupportsModel } from '../../lib/providers';
  import { registry } from '../../lib/commands.svelte';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';

  interface Props {
    wsId: string;
    onLaunched: (run: OttoRun) => void;
  }
  let { wsId, onLaunched }: Props = $props();

  let query = $state('');
  let mode = $state<RunMode>('single_agent');
  let provider = $state(defaultAgentProvider());
  let model = $state('');
  let repoId = $state('');
  let inputEl: HTMLTextAreaElement | undefined = $state();

  let detected = $state<RunDetectResp['detected'] | null>(null);
  let detecting = $state(false);
  let busy = $state(false);
  let error = $state('');

  // Registered repos → the Repo select. Reloaded when the workspace changes.
  let repos = $state<Repo[]>([]);
  let picking = $state(false);
  let registering = $state(false);
  $effect(() => {
    const id = wsId;
    void (async () => {
      try {
        repos = await api.get<Repo[]>(`/workspaces/${id}/repos`);
      } catch {
        repos = [];
      }
    })();
  });

  // Providers from the live registry (built-ins + custom, e.g. grok); `shell`
  // can't take an agent prompt. Both single-agent and goal-loop modes honor the
  // chosen provider (single-agent runs claude on the fast PTY, any other
  // provider as a real session — see run_engine::execute_single_agent).
  const providers = $derived(agentProviders());
  const effectiveProvider = $derived(provider);

  // --- debounced source detection -----------------------------------------
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let detectAbort: AbortController | null = null;

  function onInput(): void {
    detected = null;
    error = '';
    if (debounceTimer) clearTimeout(debounceTimer);
    detectAbort?.abort();
    const q = query.trim();
    if (q.length < 3) {
      detecting = false;
      return;
    }
    detecting = true;
    debounceTimer = setTimeout(() => void runDetect(q), 250);
  }

  async function runDetect(q: string): Promise<void> {
    detectAbort = new AbortController();
    try {
      const resp = await runWithOttoApi.detect(wsId, q, detectAbort.signal);
      // Ignore a stale response (the input moved on).
      if (q !== query.trim()) return;
      detected = resp.detected ?? null;
    } catch {
      detected = null;
    } finally {
      if (q === query.trim()) detecting = false;
    }
  }

  /** A source chip inserts its paste template and focuses the input. */
  function useTemplate(template: string): void {
    query = template;
    detected = null;
    inputEl?.focus();
    onInput();
  }

  /** Browse… → any folder inside a git repo registers (or finds) that repo and
   *  selects it — the daemon resolves the git toplevel via repos/detect. */
  async function onPickFolder(path: string): Promise<void> {
    picking = false;
    registering = true;
    error = '';
    try {
      const repo = await api.post<Repo>(`/workspaces/${wsId}/repos/detect`, { path });
      if (!repos.some((r) => r.id === repo.id)) repos = [...repos, repo];
      repoId = repo.id;
    } catch (e) {
      error = e instanceof Error ? e.message : 'Could not register that folder as a repo';
    } finally {
      registering = false;
    }
  }

  async function launch(): Promise<void> {
    if (busy) return;
    error = '';
    const q = query.trim();
    if (!q) {
      error = 'Paste a source or describe what you want.';
      return;
    }
    busy = true;
    try {
      // ⌘↩ right after a paste lands while detection is still debouncing/in
      // flight — launching now would send a Jira key or URL as FREE TEXT (the
      // daemon only parses url/source_ref, never seed_text). Finish detecting.
      if (detecting) {
        if (debounceTimer) clearTimeout(debounceTimer);
        detectAbort?.abort();
        await runDetect(q);
      }
      const run = await runWithOtto.launch(wsId, {
        source_kind: detected?.source_kind,
        source_ref: detected?.source_ref,
        url: detected?.url,
        seed_text: detected ? undefined : q,
        mode,
        provider: effectiveProvider,
        model: model.trim() || undefined,
        repo_id: repoId || undefined,
      });
      query = '';
      detected = null;
      onLaunched(run);
    } catch (e) {
      error = e instanceof Error ? e.message : 'Launch failed';
    } finally {
      busy = false;
    }
  }

  // ⌘K: the page's main verb — jump to the launcher input.
  $effect(() =>
    registry.register('run-with-otto', [
      {
        id: 'rwo.new',
        title: 'New run with Otto…',
        group: 'Run with Otto',
        keywords: 'launch jira github issue pr finding story test run',
        run: () => inputEl?.focus(),
      },
    ]),
  );

  function onKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      void launch();
    }
  }
</script>

<div class="launcher">
  <!-- what you can paste -->
  <div class="sources" role="group" aria-label="Insert a source template">
    {#each SOURCE_KINDS as s (s.kind)}
      <button
        type="button"
        class="src-chip"
        title="Insert a {s.label} reference — e.g. {s.example}"
        onclick={() => useTemplate(s.template)}
      >
        <Icon name={s.icon} size={12} />
        {s.label}
      </button>
    {/each}
    <span class="src-free">…or just describe what you want</span>
  </div>

  <textarea
    class="big-input"
    bind:this={inputEl}
    bind:value={query}
    oninput={onInput}
    onkeydown={onKeydown}
    rows="2"
    aria-label="Source or description"
    aria-describedby="rwo-detect"
    placeholder="Paste a Jira key, a GitHub/Confluence URL, or a finding/story/test id… or describe what you want"
  ></textarea>

  <!-- what Otto will run + THE button, right under the input it acts on -->
  <div class="submit-row">
    <div class="detect" id="rwo-detect" aria-live="polite">
      {#if detecting}
        <span class="muted">Detecting source…</span>
      {:else if detected}
        <span class="muted">Runs from</span>
        <span class="chip">{sourceLabel(detected.source_kind)}</span>
        <span class="ref mono" title={detected.source_ref}>{detected.source_ref}</span>
        {#if detected.url}
          <a class="link" href={detected.url} target="_blank" rel="noreferrer" title={detected.url}>Open source <Icon name="external" size={12} /></a>
        {/if}
      {:else if query.trim().length >= 3}
        <span class="muted">No known source found — runs as</span>
        <span class="chip">{sourceLabel('channel')}</span>
      {:else}
        <span class="muted">Otto detects the source as you type.</span>
      {/if}
    </div>
    <span class="kbd-hint" aria-hidden="true">⌘↩</span>
    <button class="btn primary run" disabled={busy} onclick={launch} title="Launch the run (⌘↩)">
      <Icon name="play" size={12} />
      {busy ? 'Launching…' : 'Run with Otto'}
    </button>
  </div>
  {#if error}<div class="err" role="alert">{error}</div>{/if}

  <div class="options" role="group" aria-label="Run options">
    <div class="segmented" role="group" aria-label="Run mode">
      <button
        type="button"
        class:active={mode === 'single_agent'}
        aria-pressed={mode === 'single_agent'}
        onclick={() => (mode = 'single_agent')}
        title="One headless agent makes the change on an isolated branch"
      >Single agent</button>
      <button
        type="button"
        class:active={mode === 'goal_loop'}
        aria-pressed={mode === 'goal_loop'}
        onclick={() => (mode = 'goal_loop')}
        title="A full Plan → Execute → Evaluate loop iterates until the goal is met"
      >Goal loop</button>
    </div>

    <div class="ctl">
      <label for="rwo-repo">Repo</label>
      <select id="rwo-repo" class="input" bind:value={repoId} aria-label="Repository"
        title={repos.find((r) => r.id === repoId)?.path ?? 'Auto: the repo named by the source, else the first registered repo'}>
        <option value="">Auto (from source)</option>
        {#each repos as r (r.id)}
          <option value={r.id} title={r.path}>{r.name}</option>
        {/each}
      </select>
      <button type="button" class="btn" disabled={registering} onclick={() => (picking = true)}
        title="Pick any folder inside a git repository to add it">
        {registering ? 'Adding…' : 'Browse…'}
      </button>
    </div>

    <div class="ctl">
      <label for="rwo-provider">Provider</label>
      <!-- A model id belongs to one provider: switching clears it, else a
           hidden stale model (e.g. claude's "opus") rides along to codex. -->
      <select id="rwo-provider" class="input" bind:value={provider} aria-label="Provider" onchange={() => (model = '')}>
        {#each providers as p (p)}
          <option value={p}>{p}</option>
        {/each}
      </select>
    </div>

    <!-- Catalog-backed model control; hides itself when the provider has no
         model-flag template. Blank = provider default. Compact: the label is
         announced (sr-only) and the visible "Model" matches its siblings. -->
    {#if providerSupportsModel(provider)}
      <div class="ctl model-ctl" title="Pins the model for this run only. Empty uses the provider’s default.">
        <span class="ctl-lbl" aria-hidden="true">Model</span>
        <ModelPicker {provider} value={model} onchange={(m) => (model = m)} id="rwo-model" compact />
      </div>
    {/if}
  </div>

  <!-- how a run executes — the fixed, evidence-gated pipeline -->
  <div class="how">
    <RunStageRail />
    <p class="how-note">
      Every run gets a proof pack, an AI review and your approval, and ends in a PR draft you open
      yourself. Need a custom shape?
      <a
        href="#/workflows"
        onclick={(e) => {
          e.preventDefault();
          router.go('workflows');
        }}>Build a Workflow</a
      >.
    </p>
  </div>
</div>

{#if picking}
  <FolderPicker
    title="Choose a repository"
    gitOnly
    onpick={(p) => void onPickFolder(p)}
    onclose={() => (picking = false)}
  />
{/if}

<style>
  .launcher {
    border: 1px solid var(--border);
    background: var(--surface);
    border-radius: var(--radius-l);
    padding: 12px 16px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-block-end: 16px;
  }
  .sources {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .src-chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font: inherit;
    font-size: var(--fs-xs);
    height: 22px;
    padding: 0 9px;
    border-radius: 999px;
    cursor: pointer;
    color: var(--text-dim);
    border: 1px solid var(--border);
    background: var(--surface-2);
  }
  .src-chip:hover {
    color: var(--text);
    background: var(--hover);
  }
  .src-free { font-size: var(--fs-xs); color: var(--text-dim); margin-inline-start: 2px; }
  .big-input {
    width: 100%;
    box-sizing: border-box;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px 10px;
    font: inherit;
    font-size: var(--fs-l);
    line-height: 1.45;
    resize: vertical;
    min-height: 64px;
  }
  .big-input::placeholder { color: var(--text-dim); }
  .big-input:focus-visible {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  /* detect line on the start, the one primary on the end — the button sits
     directly under the input it launches. */
  .submit-row { display: flex; align-items: center; gap: 8px; min-height: 28px; }
  .detect {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    overflow: hidden;
  }
  .ref {
    color: var(--text);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mono { font-family: var(--font-mono); font-size: var(--fs-s); }
  .link {
    color: var(--accent-text);
    display: inline-flex;
    align-items: center;
    gap: 3px;
    white-space: nowrap;
    flex: none;
  }
  .muted { color: var(--text-dim); white-space: nowrap; }
  /* Same keycap look as the palette / bar hints (the mono face drew ⌘ at
     half size). */
  .kbd-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-family: var(--font-ui);
    line-height: 1;
    padding: 3px 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    flex: none;
  }
  .run { flex: none; }
  /* One control height (27px, the shared .input) across the options row. */
  .options { display: flex; align-items: center; gap: 8px 16px; flex-wrap: wrap; }
  .options .segmented > button { height: 21px; }
  .ctl { display: inline-flex; align-items: center; gap: 6px; min-width: 0; }
  .ctl > label,
  .ctl-lbl { font-size: var(--fs-s); color: var(--text-dim); white-space: nowrap; }
  .ctl select { max-width: 14rem; font-size: var(--fs-s); }
  .model-ctl { min-width: 12rem; max-width: 18rem; }
  /* ModelPicker is a `.field` (the global 12px bottom margin) — flatten it
     into the row and match the sibling controls' height. */
  .model-ctl :global(.field) { margin: 0; flex: 1; min-width: 0; }
  .model-ctl :global(.select),
  .model-ctl :global(.input) { height: 27px; padding-block: 0; }
  .how {
    border-top: 1px solid var(--border);
    padding-top: 10px;
    margin-top: 4px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .how-note { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); line-height: 1.5; }
  .how-note a { color: var(--accent-text); }
  .err {
    background: var(--danger-soft);
    color: var(--danger);
    padding: 6px 10px;
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
  @media (max-width: 640px) {
    .submit-row { flex-wrap: wrap; }
    .detect { flex-basis: 100%; }
    .kbd-hint { display: none; }
    .run { margin-inline-start: auto; }
  }
</style>
