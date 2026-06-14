<script lang="ts">
  // ⌘T sheet: provider (from /meta.providers), title, cwd.
  import Modal from '../../lib/components/Modal.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  const providers = $derived(auth.meta?.providers ?? ['claude', 'codex', 'shell']);
  // Effective default agent: this workspace's override, else the global default.
  const wsDefault = $derived(
    typeof ws.current?.settings?.default_provider === 'string'
      ? (ws.current.settings.default_provider as string)
      : '',
  );
  const defaultProvider = $derived(wsDefault || (auth.meta?.default_provider ?? ''));
  let provider = $state('');
  let title = $state('');
  let cwd = $state('');
  let browser = $state(false);
  let busy = $state(false);

  // Extra directories the agent is allowed to access beyond cwd. The backend
  // turns each entry in meta.extra_dirs into a `--add-dir <path>` arg for the CLI.
  let extraDirs = $state<string[]>([]);
  let dirDraft = $state('');

  function addDir(): void {
    const path = dirDraft.trim();
    if (path === '' || extraDirs.includes(path)) {
      dirDraft = '';
      return;
    }
    extraDirs = [...extraDirs, path];
    dirDraft = '';
  }

  function removeDir(dir: string): void {
    extraDirs = extraDirs.filter((d) => d !== dir);
  }

  function onDirKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      addDir();
    }
  }

  // Browser tools wire an MCP server into the workspace .mcp.json; only
  // claude/codex load MCP servers, so the toggle is hidden for plain shells.
  const supportsBrowser = $derived(provider === 'claude' || provider === 'codex');

  $effect(() => {
    if (provider === '' && providers.length > 0) {
      // Preselect the configured default agent when it's still available;
      // when none is set, prefer claude (the historical default, matching the
      // channel bridge), then fall back to the first available provider.
      const def = defaultProvider && providers.includes(defaultProvider) ? defaultProvider : null;
      provider = def ?? (providers.includes('claude') ? 'claude' : providers[0]);
    }
    if (cwd === '' && ws.current) cwd = ws.current.root_path;
  });

  async function create(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      // Fold a pending draft (typed but not yet "Add"-ed) into the list.
      const dirs = [...extraDirs];
      const pending = dirDraft.trim();
      if (pending !== '' && !dirs.includes(pending)) dirs.push(pending);

      const meta: Record<string, unknown> = {};
      if (browser && supportsBrowser) meta.browser = true;
      if (dirs.length > 0) meta.extra_dirs = dirs;

      await ws.createSession({
        kind: 'agent',
        provider,
        title: title.trim() === '' ? null : title.trim(),
        cwd: cwd.trim() === '' ? null : cwd.trim(),
        meta: Object.keys(meta).length > 0 ? meta : null,
      });
      onclose();
      router.go('agents');
    } catch (e) {
      toasts.error('Could not create session', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="New Session" {onclose}>
  <div class="field">
    <label for="ns-provider">Provider</label>
    <div class="provider-grid" id="ns-provider">
      {#each providers as p (p)}
        <button class="provider-card" class:selected={provider === p} onclick={() => (provider = p)}>
          <span class="provider-name">
            {p}
            {#if p === defaultProvider}<span class="default-badge">default</span>{/if}
          </span>
          <span class="provider-desc">
            {p === 'claude' ? 'Claude Code CLI' : p === 'codex' ? 'Codex CLI' : p === 'shell' ? 'Plain shell' : 'Custom provider'}
          </span>
        </button>
      {/each}
    </div>
  </div>

  <div class="field">
    <label for="ns-title">Title <span class="dim">(optional)</span></label>
    <input id="ns-title" class="input" bind:value={title} placeholder="{provider} #{ws.sessions.length + 1}" />
  </div>

  <div class="field">
    <label for="ns-cwd">Working directory</label>
    <input id="ns-cwd" class="input mono" bind:value={cwd} spellcheck="false" />
    <span class="hint">Defaults to the workspace root</span>
  </div>

  <div class="field">
    <label for="ns-extra-dir">Additional directories <span class="dim">(optional)</span></label>
    {#if extraDirs.length > 0}
      <ul class="dir-list">
        {#each extraDirs as dir (dir)}
          <li class="dir-row">
            <span class="dir-path mono" title={dir}>{dir}</span>
            <button
              type="button"
              class="dir-remove"
              title="Remove directory"
              onclick={() => removeDir(dir)}
            >✕</button>
          </li>
        {/each}
      </ul>
    {/if}
    <div class="dir-add">
      <input
        id="ns-extra-dir"
        class="input mono"
        bind:value={dirDraft}
        spellcheck="false"
        placeholder="/absolute/path/to/repo"
        onkeydown={onDirKeydown}
      />
      <button type="button" class="btn" disabled={dirDraft.trim() === ''} onclick={addDir}>Add</button>
    </div>
    <span class="hint">Extra repos the agent may access (passed as <code>--add-dir</code>).</span>
  </div>

  {#if supportsBrowser}
    <label class="toggle-row">
      <input type="checkbox" bind:checked={browser} />
      <span class="toggle-text">
        <span class="toggle-title">Browser tools</span>
        <span class="hint">Give the agent a real browser via MCP (navigate, click, read pages).</span>
      </span>
    </label>
  {/if}

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={busy || provider === ''} onclick={create}>
      {busy ? 'Starting…' : 'Start Session'}
    </button>
  {/snippet}
</Modal>

<style>
  .provider-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: 8px;
  }
  .provider-card {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    cursor: pointer;
    text-align: left;
    transition: border-color 130ms ease-out, background 130ms ease-out;
  }
  .provider-card:hover {
    background: color-mix(in srgb, var(--surface-2) 70%, var(--surface));
  }
  .provider-card.selected {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .provider-name {
    font-size: 13px;
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .default-badge {
    font-size: 9.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 6px;
    border-radius: 99px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .provider-desc {
    font-size: 11px;
    color: var(--text-dim);
  }
  .toggle-row {
    display: flex;
    align-items: flex-start;
    gap: 9px;
    padding: 4px 0;
    cursor: pointer;
  }
  .toggle-row input {
    margin-top: 2px;
  }
  .toggle-text {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .toggle-title {
    font-size: 13px;
    font-weight: 600;
  }
  .dir-list {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .dir-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 5px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .dir-path {
    flex: 1;
    min-width: 0;
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .dir-remove {
    flex-shrink: 0;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 10px;
    padding: 2px 4px;
    border-radius: 3px;
    line-height: 1;
  }
  .dir-remove:hover {
    color: var(--danger, #e5534b);
    background: color-mix(in srgb, var(--danger, #e5534b) 12%, transparent);
  }
  .dir-add {
    display: flex;
    gap: 6px;
  }
  .dir-add .input {
    flex: 1;
    min-width: 0;
  }
</style>
