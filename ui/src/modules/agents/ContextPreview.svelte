<script lang="ts">
  // Dry-run preview of what a session spawn would materialize for a workspace
  // + provider — the exact skill files, soul, generated AGENTS.md / CLAUDE.md
  // content and runtime hooks — WITHOUT spawning a session or touching disk.
  //
  // Crucially it labels each artifact advisory vs enforced:
  //   • advisory — instruction files / skills: guidance the model MAY ignore.
  //   • enforced — hooks / settings the runtime imposes regardless.
  // See docs/contracts/api.md (POST /workspaces/{id}/context/preview).
  import { contextApi } from '../../lib/api/context';
  import type {
    ContextPreviewProvider,
    ContextPreviewReq,
  } from '../../lib/api/types';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    /** Workspace to preview. */
    wsId: string;
    /** Provider to preview (claude / codex). Others materialize nothing. */
    provider: string;
    /** Optional overrides so a not-yet-saved selection can be previewed. */
    overrides?: Omit<ContextPreviewReq, 'provider'>;
  }
  let { wsId, provider, overrides = {} }: Props = $props();

  let result: ContextPreviewProvider | null = $state(null);
  let loading = $state(false);
  let loadedKey = $state(''); // wsId|provider for the result currently shown
  // Which artifact's full content is expanded (path), if any.
  let openFile: string | null = $state(null);

  async function run(): Promise<void> {
    if (!wsId || loading) return;
    loading = true;
    try {
      const resp = await contextApi.preview(wsId, { provider, ...overrides });
      result = resp.providers.find((p) => p.provider === provider) ?? resp.providers[0] ?? null;
      loadedKey = `${wsId}|${provider}`;
      openFile = null;
    } catch (e) {
      toasts.error('Preview failed', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  // Invalidate a stale result when the workspace/provider changes.
  $effect(() => {
    if (loadedKey && loadedKey !== `${wsId}|${provider}`) result = null;
  });

  function fmtBytes(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  }

  // Last path segment, for a compact file label (the absolute path is in title).
  function leaf(path: string): string {
    const parts = path.split('/');
    return parts[parts.length - 1] || path;
  }

  const kindLabels: Record<string, string> = {
    instructions: 'Instructions',
    skill: 'Skill',
    skill_asset: 'Skill asset',
    hooks: 'Hooks',
    manifest: 'Manifest',
  };

  function toggleFile(path: string): void {
    openFile = openFile === path ? null : path;
  }
</script>

<div class="preview">
  <div class="head">
    <button class="btn" disabled={loading} onclick={run}>
      {loading ? 'Previewing…' : result ? 'Refresh preview' : 'Preview context'}
    </button>
    <span class="hint">
      Exactly what a <code>{provider}</code> spawn would write here — no session is started.
    </span>
  </div>

  {#if result}
    {#if result.skipped}
      <div class="skipped dim">
        <code>{provider}</code> materializes no context (plain shells and custom providers
        get nothing injected).
      </div>
    {:else}
      <!-- Advisory vs enforced legend -->
      <div class="legend">
        <span class="badge advisory">advisory</span>
        <span class="legend-text">instructions &amp; skills — guidance the model may ignore</span>
        <span class="badge enforced">enforced</span>
        <span class="legend-text">hooks &amp; settings — imposed by the runtime</span>
      </div>

      <!-- Selected skills + soul -->
      <div class="summary">
        <div class="summary-row">
          <span class="summary-lbl">Soul</span>
          {#if result.soul}
            <span class="chip mono">{result.soul}</span>
          {:else}
            <span class="dim">none</span>
          {/if}
          <span class="badge advisory" title="The model reads the soul but may ignore it">advisory</span>
        </div>
        <div class="summary-row">
          <span class="summary-lbl">Skills</span>
          {#if result.skills.length === 0}
            <span class="dim">none</span>
          {:else}
            <span class="chips">
              {#each result.skills as s (s.name)}
                <span class="chip mono" title={s.description}>{s.name}<span class="ver">v{s.version}</span></span>
              {/each}
            </span>
          {/if}
          <span class="badge advisory">advisory</span>
        </div>
      </div>

      <!-- Files the spawn would write -->
      <div class="files-head">
        <span class="files-title">{result.files.length} file{result.files.length === 1 ? '' : 's'}</span>
      </div>
      <ul class="files">
        {#each result.files as f (f.path)}
          <li class="file" class:enforced={f.enforcement === 'enforced'}>
            <button
              class="file-head"
              type="button"
              onclick={() => toggleFile(f.path)}
              aria-expanded={openFile === f.path}
            >
              <span class="file-kind kind-{f.kind}">{kindLabels[f.kind] ?? f.kind}</span>
              <span class="file-name mono" title={f.path}>{leaf(f.path)}</span>
              <span class="badge {f.enforcement}">{f.enforcement}</span>
              <span class="file-size dim">{fmtBytes(f.size)}</span>
              <span class="chevron" class:open={openFile === f.path}>▸</span>
            </button>
            {#if openFile === f.path}
              <pre class="file-body mono">{f.first_lines}{f.truncated ? '\n…' : ''}</pre>
            {/if}
          </li>
        {/each}
      </ul>

      <!-- Generated instruction file (full) -->
      {#if result.instructions_file_name}
        <details class="generated">
          <summary>
            Generated {result.instructions_file_name}
            <span class="badge advisory">advisory</span>
          </summary>
          <pre class="mono">{result.generated_instructions}</pre>
        </details>
      {/if}

      <!-- Generated hooks / settings (full) -->
      {#if result.generated_hooks}
        <details class="generated">
          <summary>
            Generated hooks / settings
            <span class="badge enforced">enforced</span>
          </summary>
          <pre class="mono">{result.generated_hooks}</pre>
        </details>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .preview {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .hint {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .skipped {
    font-size: 12px;
    padding: 8px 10px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
  }

  /* Advisory / enforced badge + legend */
  .legend {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    font-size: 11px;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .legend-text {
    color: var(--text-dim);
    margin-inline-end: 8px;
  }
  .badge {
    font-size: 9.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 6px;
    border-radius: 99px;
    flex-shrink: 0;
  }
  .badge.advisory {
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
    border: 1px solid color-mix(in srgb, var(--text-dim) 30%, transparent);
  }
  .badge.enforced {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  }

  .summary {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .summary-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 12px;
  }
  .summary-lbl {
    font-weight: 600;
    min-width: 48px;
  }
  .chips {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .chip {
    font-size: 11px;
    padding: 1px 7px;
    border-radius: 99px;
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .chip .ver {
    margin-inline-start: 4px;
    color: var(--text-dim);
    font-size: 9.5px;
  }

  .files-head {
    margin-top: 2px;
  }
  .files-title {
    font-size: 12px;
    font-weight: 600;
  }
  .files {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .file {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    overflow: hidden;
  }
  .file.enforced {
    border-color: color-mix(in srgb, var(--accent) 35%, var(--border));
  }
  .file-head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 9px;
    background: none;
    border: none;
    cursor: pointer;
    text-align: start;
    font: inherit;
  }
  .file-head:hover {
    background: color-mix(in srgb, var(--surface) 50%, var(--surface-2));
  }
  .file-kind {
    font-size: 10px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
    flex-shrink: 0;
    min-width: 78px;
  }
  .file-name {
    font-size: 11.5px;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .file-size {
    font-size: 10.5px;
    flex-shrink: 0;
  }
  .chevron {
    font-size: 9px;
    color: var(--text-dim);
    transition: transform 120ms ease-out;
    flex-shrink: 0;
  }
  .chevron.open {
    transform: rotate(90deg);
  }
  .file-body {
    margin: 0;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
    background: var(--surface);
    font-size: 11px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 240px;
    overflow: auto;
  }

  .generated summary {
    cursor: pointer;
    font-size: 12px;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 0;
  }
  .generated pre {
    margin: 6px 0 0;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: 11px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 320px;
    overflow: auto;
  }
</style>
