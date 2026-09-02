<script lang="ts">
  // THE one button. A single input where the user pastes a Jira key, a
  // GitHub/Confluence URL, or a finding/story/test id — or just describes what
  // they want (free text → a channel run). As they type we debounce-detect the
  // source and show what Otto will run; the prominent "Run with Otto" button
  // launches it. Around the input: source chips (what you can paste), the
  // pipeline rail (how a run executes — and how this differs from Workflows),
  // and the launch parameters (mode, repo, provider/model, auto-open PR).
  import { api } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import { runWithOtto } from '../../lib/stores/runWithOtto.svelte';
  import { runWithOttoApi } from '../../lib/api/runWithOtto';
  import { router } from '../../lib/router.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import RunStageRail from './RunStageRail.svelte';
  import { SOURCE_KINDS, sourceColor, sourceLabel } from './runStatus';
  import type { OttoRun, Repo, RunDetectResp, RunMode } from '../../lib/api/types';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
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
  let autoOpenPr = $state(false);
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
    error = '';
    const q = query.trim();
    if (!q) {
      error = 'Paste a source or describe what you want.';
      return;
    }
    busy = true;
    try {
      const run = await runWithOtto.launch(wsId, {
        source_kind: detected?.source_kind,
        source_ref: detected?.source_ref,
        url: detected?.url,
        seed_text: detected ? undefined : q,
        mode,
        provider: effectiveProvider,
        model: model.trim() || undefined,
        repo_id: repoId || undefined,
        auto_open_pr: autoOpenPr,
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

  function onKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      void launch();
    }
  }
</script>

<div class="launcher">
  {#if error}<div class="err" role="alert">{error}</div>{/if}

  <!-- what you can paste -->
  <div class="sources" role="group" aria-label="Supported sources">
    {#each SOURCE_KINDS as s (s.kind)}
      <button
        type="button"
        class="src-chip"
        style="--src: {s.color}"
        title={s.example}
        onclick={() => useTemplate(s.template)}
      >
        <Icon name={s.icon} size={11} />
        {s.label}
      </button>
    {/each}
    <span class="src-free">…or just describe what you want</span>
  </div>

  <label class="big-input">
    <textarea
      bind:this={inputEl}
      bind:value={query}
      oninput={onInput}
      onkeydown={onKeydown}
      rows="2"
      placeholder="Paste a Jira key, a GitHub/Confluence URL, or a finding/story/test id… or describe what you want"
    ></textarea>
  </label>

  <div class="detect" aria-live="polite">
    {#if detecting}
      <span class="muted">Detecting source…</span>
    {:else if detected}
      <span class="badge src-badge" style="--src: {sourceColor(detected.source_kind)}">{sourceLabel(detected.source_kind)}</span>
      <span class="ref">{detected.source_ref}</span>
      {#if detected.url}<a class="link" href={detected.url} target="_blank" rel="noreferrer">link</a>{/if}
    {:else if query.trim().length >= 3}
      <span class="muted">Free-text → <span class="badge src-badge" style="--src: {sourceColor('channel')}">chat / free text</span> run</span>
    {:else}
      <span class="muted">Otto detects the source as you type.</span>
    {/if}
  </div>

  <div class="controls">
    <div class="seg" role="group" aria-label="Run mode">
      <button
        type="button"
        class="seg-btn"
        class:active={mode === 'single_agent'}
        onclick={() => (mode = 'single_agent')}
        title="One headless agent makes the change on an isolated branch"
      >Single agent</button>
      <button
        type="button"
        class="seg-btn"
        class:active={mode === 'goal_loop'}
        onclick={() => (mode = 'goal_loop')}
        title="A full Plan → Execute → Evaluate loop iterates until the goal is met"
      >Goal loop</button>
    </div>

    <label class="ctl">
      <span>Repo</span>
      <select bind:value={repoId} aria-label="Repository">
        <option value="">Auto — from source, else first repo</option>
        {#each repos as r (r.id)}
          <option value={r.id} title={r.path}>{r.name}</option>
        {/each}
      </select>
    </label>
    <button type="button" class="btn small" disabled={registering} onclick={() => (picking = true)}>
      {registering ? 'Registering…' : 'Browse…'}
    </button>

    <label class="ctl">
      <span>Provider</span>
      <select bind:value={provider} aria-label="Provider">
        {#each providers as p (p)}
          <option value={p}>{p}</option>
        {/each}
      </select>
    </label>

    <!-- Catalog-backed model control; hides itself when the provider has no
         model-flag template. Blank = provider default. -->
    <div class="model-ctl">
      <ModelPicker {provider} value={model} onchange={(m) => (model = m)} />
    </div>

    <label class="chk"><input type="checkbox" bind:checked={autoOpenPr} /> Auto-open PR</label>

    <button class="btn primary run" disabled={busy} onclick={launch}>
      {busy ? 'Launching…' : 'Run with Otto'}
    </button>
  </div>

  <!-- how a run executes — the fixed, evidence-gated pipeline -->
  <div class="how">
    <RunStageRail />
    <p class="how-note">
      Every run travels this exact pipeline — proof pack, AI review, and your approval are always
      on, ending in a PR draft. Want a custom shape instead?
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
    padding: 0.85rem 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    margin-bottom: 1rem;
    background-image: radial-gradient(
      circle at top right,
      color-mix(in srgb, var(--accent) 6%, transparent),
      transparent 55%
    );
  }
  .sources {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    flex-wrap: wrap;
  }
  .src-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    font: inherit;
    font-size: 0.72rem;
    padding: 0.14rem 0.55rem;
    border-radius: 999px;
    cursor: pointer;
    color: var(--src);
    border: 1px solid color-mix(in srgb, var(--src) 35%, var(--border));
    background: color-mix(in srgb, var(--src) 9%, transparent);
  }
  .src-chip:hover {
    border-color: var(--src);
    background: color-mix(in srgb, var(--src) 16%, transparent);
  }
  .src-free { font-size: 0.72rem; color: var(--text-dim); margin-left: 0.2rem; }
  .big-input textarea {
    width: 100%;
    box-sizing: border-box;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 0.6rem 0.7rem;
    font: inherit;
    font-size: 0.95rem;
    resize: vertical;
  }
  .big-input textarea::placeholder { color: var(--text-dim); }
  .big-input textarea:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
    outline-offset: 1px;
  }
  .detect { display: flex; align-items: center; gap: 0.4rem; font-size: 0.82rem; min-height: 1.2rem; flex-wrap: wrap; }
  .ref { color: var(--text); font-variant-numeric: tabular-nums; }
  .link { color: var(--accent); font-size: 0.78rem; }
  .muted { color: var(--text-dim); }
  .controls { display: flex; align-items: center; gap: 0.75rem; flex-wrap: wrap; }
  .seg { display: inline-flex; border: 1px solid var(--border); border-radius: var(--radius-s); overflow: hidden; }
  .seg-btn {
    background: var(--bg); color: var(--text-dim); border: none;
    padding: 0.4rem 0.7rem; font: inherit; font-size: 0.82rem; cursor: pointer;
  }
  .seg-btn.active { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .ctl { display: inline-flex; align-items: center; gap: 0.4rem; font-size: 0.82rem; color: var(--text-dim); }
  .ctl select {
    background: var(--bg); color: var(--text); border: 1px solid var(--border);
    border-radius: var(--radius-s); padding: 0.35rem 0.5rem; font: inherit; font-size: 0.82rem;
    max-width: 15rem;
  }
  .chk { display: flex; align-items: center; gap: 0.4rem; font-size: 0.82rem; color: var(--text); }
  .model-ctl { min-width: 12rem; max-width: 18rem; }
  .run { margin-left: auto; font-size: 0.95rem; padding: 0.5rem 1.1rem; }
  .badge {
    font-size: 0.7rem; padding: 0.05rem 0.45rem; border-radius: 999px;
    border: 1px solid var(--border); color: var(--text-dim); text-transform: capitalize;
  }
  .src-badge {
    color: var(--src);
    border-color: color-mix(in srgb, var(--src) 40%, var(--border));
    background: color-mix(in srgb, var(--src) 10%, transparent);
  }
  .how {
    border-top: 1px solid var(--border);
    padding-top: 0.6rem;
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .how-note { margin: 0; font-size: 0.75rem; color: var(--text-dim); }
  .how-note a { color: var(--accent); }
  .err {
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
    color: var(--status-exited); padding: 0.5rem 0.75rem;
    border-radius: var(--radius-s); font-size: 0.85rem;
  }
</style>
