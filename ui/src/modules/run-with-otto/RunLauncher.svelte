<script lang="ts">
  // THE one button. A single input where the user pastes a Jira key, a
  // GitHub/Confluence URL, or a finding/story/test id — or just describes what
  // they want (free text → a channel run). As they type we debounce-detect the
  // source and show what Otto will run; the prominent "Run with Otto" button
  // launches it.
  import { runWithOtto } from '../../lib/stores/runWithOtto.svelte';
  import { runWithOttoApi } from '../../lib/api/runWithOtto';
  import type { OttoRun, RunDetectResp, RunMode } from '../../lib/api/types';

  interface Props {
    wsId: string;
    onLaunched: (run: OttoRun) => void;
  }
  let { wsId, onLaunched }: Props = $props();

  let query = $state('');
  let mode = $state<RunMode>('single_agent');
  let provider = $state('');
  let autoOpenPr = $state(false);

  let detected = $state<RunDetectResp['detected'] | null>(null);
  let detecting = $state(false);
  let busy = $state(false);
  let error = $state('');

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
        provider: provider.trim() || undefined,
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

  <label class="big-input">
    <textarea
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
      <span class="badge src-{detected.source_kind}">{detected.source_kind}</span>
      <span class="ref">{detected.source_ref}</span>
      {#if detected.url}<a class="link" href={detected.url} target="_blank" rel="noreferrer">link</a>{/if}
    {:else if query.trim().length >= 3}
      <span class="muted">Free-text → <span class="badge src-channel">channel</span> run</span>
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
      >Single agent</button>
      <button
        type="button"
        class="seg-btn"
        class:active={mode === 'goal_loop'}
        onclick={() => (mode = 'goal_loop')}
      >Goal loop</button>
    </div>

    <label class="prov">
      <span>Provider</span>
      <input bind:value={provider} placeholder="default" />
    </label>

    <label class="chk"><input type="checkbox" bind:checked={autoOpenPr} /> Auto-open PR</label>

    <button class="btn primary run" disabled={busy} onclick={launch}>
      {busy ? 'Launching…' : 'Run with Otto'}
    </button>
  </div>
</div>

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
  }
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
  .prov { display: flex; align-items: center; gap: 0.4rem; font-size: 0.82rem; color: var(--text-dim); }
  .prov input {
    background: var(--bg); color: var(--text); border: 1px solid var(--border);
    border-radius: var(--radius-s); padding: 0.35rem 0.5rem; font: inherit; width: 9ch;
  }
  .chk { display: flex; align-items: center; gap: 0.4rem; font-size: 0.82rem; color: var(--text); }
  .run { margin-left: auto; font-size: 0.95rem; padding: 0.5rem 1.1rem; }
  .badge {
    font-size: 0.7rem; padding: 0.05rem 0.45rem; border-radius: 999px;
    border: 1px solid var(--border); color: var(--text-dim); text-transform: capitalize;
  }
  .err {
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
    color: var(--status-exited); padding: 0.5rem 0.75rem;
    border-radius: var(--radius-s); font-size: 0.85rem;
  }
</style>
