<script lang="ts">
  // ⌘T sheet: provider (from /meta.providers), title, cwd.
  import Modal from '../../lib/components/Modal.svelte';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import ContextPreview from './ContextPreview.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { allProviders } from '../../lib/providers';

  /** Per-provider ceiling on one batch — a typo in the stepper shouldn't be able
   *  to fork 200 agent processes at once. */
  const MAX_PER_PROVIDER = 20;

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  const providers = $derived(allProviders());
  // Effective default agent: this workspace's override, else the global default.
  const wsDefault = $derived(
    typeof ws.current?.settings?.default_provider === 'string'
      ? (ws.current.settings.default_provider as string)
      : '',
  );
  const defaultProvider = $derived(wsDefault || (auth.meta?.default_provider ?? ''));
  // The PRIMARY provider: what the radiogroup / arrow keys select, and what the
  // model picker, context preview and browser toggle apply to. Always one of
  // the providers with a non-zero count (see `bump`).
  let provider = $state('');
  // How many sessions to start per provider. The card click keeps its old
  // exclusive-select behaviour (one claude); the ± stepper is what turns this
  // into a batch — "2 codex and 3 claude and 1 shell" in one go, which until
  // now was only reachable by typing it at the command palette or repeating
  // this sheet once per session.
  let counts = $state<Record<string, number>>({});
  // Model pinned for THIS session only ('' = provider default). Reset on
  // provider switch — model ids are provider-specific.
  let model = $state('');
  let title = $state('');
  let cwd = $state('');
  let browser = $state(false);
  let busy = $state(false);
  // Daemon-side folder picker for the working directory (and the extra-dirs
  // field): pointing a session at a folder outside the workspace should not
  // require creating a workspace for it, or typing an absolute path by hand.
  let browsing: 'cwd' | 'extra' | null = $state(null);

  const countOf = (p: string): number => counts[p] ?? 0;
  const total = $derived(Object.values(counts).reduce((a, b) => a + b, 0));
  /** Distinct providers in the batch, in the grid's own order. */
  const chosen = $derived(providers.filter((p) => countOf(p) > 0));
  /** "3 claude, 2 codex, 1 shell" — the batch, spelled out. */
  const batchSummary = $derived(chosen.map((p) => `${counts[p]} ${p}`).join(', '));

  /** Set the count for one provider, keeping `provider` on something selected. */
  function bump(p: string, delta: number): void {
    const n = Math.min(MAX_PER_PROVIDER, Math.max(0, countOf(p) + delta));
    const next = { ...counts };
    if (n === 0) delete next[p];
    else next[p] = n;
    counts = next;
    if (n > 0) {
      // Adding to a provider makes it primary only when the current primary
      // isn't in the batch at all — bumping codex shouldn't silently retarget
      // the model picker away from the claude the user just configured.
      if (countOf(provider) === 0) selectPrimary(p);
    } else if (p === provider) {
      // The primary was zeroed out: fall back to whatever is still selected.
      const fallback = providers.find((q) => (next[q] ?? 0) > 0);
      if (fallback) selectPrimary(fallback);
    }
  }

  /** Switch the primary provider without touching the batch counts. */
  function selectPrimary(p: string): void {
    if (p !== provider) model = '';
    provider = p;
  }

  // Recently used working directories, newest first: every distinct cwd across
  // this workspace's sessions, with the workspace root first. Offered as a
  // datalist on the cwd field so the common case is one keystroke.
  const recentDirs = $derived.by((): string[] => {
    const seen: string[] = [];
    const push = (d: string | null | undefined): void => {
      if (d && !seen.includes(d)) seen.push(d);
    };
    push(ws.current?.root_path);
    for (const s of [...ws.sessions].reverse()) push(s.cwd);
    return seen.slice(0, 12);
  });

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

  // Provider picker keyboard nav: the cards are an ARIA radiogroup with a roving
  // tabindex, so Tab moves to the next FIELD (Title) while Left/Right (and
  // Up/Down) move the selection between providers — like a native radio group.
  let cardEls = $state<HTMLButtonElement[]>([]);
  let gridEl = $state<HTMLDivElement | null>(null);

  // On open, pull keyboard focus into the selected card. Without this, focus
  // stays wherever it was before ⌘T — usually the xterm textarea, so arrows
  // (and everything else) kept going to the terminal instead of this sheet.
  let didAutofocus = false;
  $effect(() => {
    if (didAutofocus || provider === '') return;
    didAutofocus = true;
    queueMicrotask(() => cardEls[providers.indexOf(provider)]?.focus());
  });

  // Sheet-level keys: ⌘/Ctrl+Enter starts the session from anywhere in the
  // modal; arrows switch provider unless they belong to a text field (caret
  // movement) or to the grid itself (its radiogroup handler already ran).
  function onGlobalKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      if (!busy && total > 0) void create();
      return;
    }
    const t = e.target as HTMLElement | null;
    if (t && (t.closest('input, textarea, [contenteditable="true"]') || gridEl?.contains(t))) return;
    onProviderKeydown(e);
  }

  /** Click / arrow selection: exclusive, exactly as before the batch stepper
   *  existed — picking codex means "one codex", not "codex on top of claude".
   *  Adding to a batch is the explicit ± affordance. */
  function selectProvider(p: string, focus = false): void {
    selectPrimary(p);
    counts = { [p]: 1 };
    if (focus) {
      const idx = providers.indexOf(p);
      queueMicrotask(() => cardEls[idx]?.focus());
    }
  }

  function onProviderKeydown(e: KeyboardEvent): void {
    const idx = providers.indexOf(provider);
    if (idx < 0 || providers.length === 0) return;
    let next = idx;
    switch (e.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        next = (idx + 1) % providers.length;
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        next = (idx - 1 + providers.length) % providers.length;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = providers.length - 1;
        break;
      case '+':
      case '=':
        e.preventDefault();
        bump(providers[idx], 1);
        return;
      case '-':
      case '_':
        e.preventDefault();
        bump(providers[idx], -1);
        return;
      default:
        return; // let other keys (Tab, Enter, Space) behave normally
    }
    e.preventDefault();
    selectProvider(providers[next], true);
  }

  // Browser tools wire an MCP server into the workspace .mcp.json; only
  // claude/codex load MCP servers, so the toggle is hidden for plain shells.
  const supportsBrowser = $derived(provider === 'claude' || provider === 'codex');
  /** A pinned model is provider-specific, so it only applies to a batch that
   *  targets ONE provider (any number of sessions of it). */
  const supportsModel = $derived(chosen.length <= 1);

  // Context preview: only claude/codex materialize an Otto context; plain shells
  // and custom providers get nothing, so there's nothing to preview for them.
  const supportsContext = $derived(provider === 'claude' || provider === 'codex');
  let showPreview = $state(false);

  $effect(() => {
    if (provider === '' && providers.length > 0) {
      // Preselect the configured default agent when it's still available;
      // when none is set, prefer claude (the historical default, matching the
      // channel bridge), then fall back to the first available provider.
      const def = defaultProvider && providers.includes(defaultProvider) ? defaultProvider : null;
      selectProvider(def ?? (providers.includes('claude') ? 'claude' : providers[0]));
    }
    if (cwd === '' && ws.current) cwd = ws.current.root_path;
  });

  async function create(): Promise<void> {
    if (busy || total === 0) return;
    busy = true;
    try {
      // Fold a pending draft (typed but not yet "Add"-ed) into the list.
      const dirs = [...extraDirs];
      const pending = dirDraft.trim();
      if (pending !== '' && !dirs.includes(pending)) dirs.push(pending);
      const base = title.trim();
      const dir = cwd.trim();

      // Flatten the batch into one spawn per session, provider by provider in
      // grid order, so "3 claude, 2 codex" starts in a predictable order.
      const spawns = chosen.flatMap((p) => Array.from({ length: counts[p] }, () => p));
      const created: string[] = [];
      const failures: string[] = [];
      for (const [i, p] of spawns.entries()) {
        const meta: Record<string, unknown> = {};
        if (browser && (p === 'claude' || p === 'codex')) meta.browser = true;
        if (dirs.length > 0) meta.extra_dirs = dirs;
        try {
          // Quiet creates throughout: routing to each session as it appears
          // would yank the user through the whole batch. We open them below.
          const s = await ws.createSessionQuiet({
            kind: 'agent',
            provider: p,
            // A typed title is a BASE name for a batch — numbered so the
            // sessions stay tellable apart; alone it is used verbatim.
            title: base === '' ? null : spawns.length > 1 ? `${base} ${i + 1}` : base,
            cwd: dir === '' ? null : dir,
            meta: Object.keys(meta).length > 0 ? meta : null,
            // The pinned model only applies to a single-provider batch.
            model: supportsModel && model.trim() !== '' ? model.trim() : null,
          });
          created.push(s.id);
        } catch (e) {
          failures.push(`${p}: ${e instanceof Error ? e.message : String(e)}`);
        }
      }

      // Foreground everything that started (tiled when there's more than one,
      // matching how the command palette lands a multi-agent spawn), routing to
      // the last so Back returns through them.
      if (created.length > 1) ws.setViewMode('tiled');
      for (const id of created.slice(0, -1)) ws.openSession(id);
      if (created.length > 0) ws.navigateToSession(created[created.length - 1]);

      if (failures.length > 0) {
        toasts.error(
          created.length > 0 ? 'Some sessions did not start' : 'Could not create session',
          failures.join('\n'),
        );
        if (created.length === 0) return; // keep the sheet open to retry
      }
      onclose();
    } catch (e) {
      toasts.error('Could not create session', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<svelte:window onkeydown={onGlobalKeydown} />

<Modal title="New Session" {onclose}>
  <div class="field">
    <div id="ns-provider-label" class="provider-label">
      Provider <span class="dim">(← → to switch, ± for more than one)</span>
    </div>
    <div
      bind:this={gridEl}
      class="provider-grid"
      role="radiogroup"
      tabindex="-1"
      aria-labelledby="ns-provider-label"
      onkeydown={onProviderKeydown}
    >
      {#each providers as p, i (p)}
        <div class="provider-card" class:selected={countOf(p) > 0} class:primary={provider === p}>
          <button
            bind:this={cardEls[i]}
            class="card-main"
            role="radio"
            aria-checked={provider === p}
            tabindex={provider === p ? 0 : -1}
            onclick={() => selectProvider(p)}
          >
            <span class="provider-name">
              {p}
              {#if p === defaultProvider}<span class="default-badge">default</span>{/if}
            </span>
            <span class="provider-desc">
              {p === 'claude' ? 'Claude Code CLI' : p === 'codex' ? 'Codex CLI' : p === 'shell' ? 'Plain shell' : 'Custom provider'}
            </span>
          </button>
          <div class="count-ctl">
            <button
              type="button"
              class="cbtn"
              disabled={countOf(p) === 0}
              aria-label={`One less ${p} session`}
              onclick={() => bump(p, -1)}
            >−</button>
            <span class="count" class:zero={countOf(p) === 0} aria-live="polite">
              {countOf(p)}
            </span>
            <button
              type="button"
              class="cbtn"
              disabled={countOf(p) >= MAX_PER_PROVIDER}
              aria-label={`One more ${p} session`}
              onclick={() => bump(p, 1)}
            >+</button>
          </div>
        </div>
      {/each}
    </div>
    {#if total > 1}
      <span class="hint batch">Starting {total} sessions — {batchSummary}</span>
    {/if}
  </div>

  <!-- Hidden entirely when the provider's spec has no model-flag template, and
       for a mixed batch (model ids are provider-specific). -->
  {#if supportsModel}
    <ModelPicker {provider} value={model} onchange={(m) => (model = m)} />
  {/if}

  <div class="field">
    <label for="ns-title">Title <span class="dim">(optional)</span></label>
    <input id="ns-title" class="input" bind:value={title} placeholder="Auto-named from your theme (Settings → Session Names)" />
    {#if total > 1 && title.trim() !== ''}
      <span class="hint">Numbered per session — “{title.trim()} 1” … “{title.trim()} {total}”.</span>
    {/if}
  </div>

  <div class="field">
    <label for="ns-cwd">Working directory</label>
    <div class="dir-add">
      <input
        id="ns-cwd"
        class="input mono"
        bind:value={cwd}
        spellcheck="false"
        list="ns-recent-dirs"
        placeholder="/absolute/path/to/folder"
      />
      <button type="button" class="btn" onclick={() => (browsing = 'cwd')}>Browse…</button>
    </div>
    <datalist id="ns-recent-dirs">
      {#each recentDirs as d (d)}<option value={d}></option>{/each}
    </datalist>
    <span class="hint">
      Any folder on this machine — it does not have to be inside a workspace.
      Defaults to the workspace root.
    </span>
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
      <button type="button" class="btn" onclick={() => (browsing = 'extra')}>Browse…</button>
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

  {#if supportsContext && ws.currentId}
    <div class="field">
      <button
        type="button"
        class="preview-toggle"
        onclick={() => (showPreview = !showPreview)}
        aria-expanded={showPreview}
      >
        <span class="chevron" class:open={showPreview}>▸</span>
        Preview context
        <span class="hint">— exactly what Otto would inject before spawning</span>
      </button>
      {#if showPreview}
        <div class="preview-box">
          <ContextPreview
            wsId={ws.currentId}
            {provider}
            overrides={{ cwd: cwd.trim() === '' ? undefined : cwd.trim() }}
          />
        </div>
      {/if}
    </div>
  {/if}

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={busy || total === 0} onclick={create}>
      {busy ? 'Starting…' : total > 1 ? `Start ${total} Sessions` : 'Start Session'}
    </button>
  {/snippet}
</Modal>

{#if browsing}
  <FolderPicker
    title={browsing === 'cwd' ? 'Choose working directory' : 'Choose an additional directory'}
    start={(browsing === 'cwd' ? cwd : dirDraft) || ws.current?.root_path || '~'}
    onpick={(path: string) => {
      if (browsing === 'cwd') cwd = path;
      else dirDraft = path;
      browsing = null;
    }}
    onclose={() => (browsing = null)}
  />
{/if}

<style>
  .provider-label {
    font-size: 11.5px;
    font-weight: 500;
    color: var(--text-dim);
    margin-bottom: 4px;
  }
  .provider-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: 8px;
  }
  .provider-grid:focus {
    outline: none;
  }
  .provider-card {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    transition: border-color 130ms ease-out, background 130ms ease-out;
  }
  /* The card body is the (exclusive) provider choice; the stepper below it is
     the batch count, so they are separate controls inside one card. */
  .card-main {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    padding: 10px 12px 6px;
    background: none;
    border: none;
    border-radius: var(--radius-m) var(--radius-m) 0 0;
    color: inherit;
    font: inherit;
    cursor: pointer;
    text-align: start;
  }
  .card-main:hover {
    background: color-mix(in srgb, var(--surface-2) 70%, var(--surface));
  }
  .provider-card.selected {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  /* The one the model picker / context preview / browser toggle apply to. */
  .provider-card.primary {
    box-shadow: 0 0 0 1px var(--accent) inset;
  }
  .count-ctl {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 2px;
    padding: 0 8px 6px;
  }
  .cbtn {
    width: 20px;
    height: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    background: var(--surface);
    color: var(--text-dim);
    font: inherit;
    font-size: 13px;
    line-height: 1;
    cursor: pointer;
  }
  .cbtn:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--accent);
  }
  .cbtn:disabled {
    opacity: 0.35;
    cursor: default;
  }
  /* Touch: the ± targets have to be tappable on a phone, where the card is the
     same size but fingers are not cursors. */
  @media (pointer: coarse) {
    .cbtn {
      width: 30px;
      height: 30px;
      font-size: 15px;
    }
  }
  .count {
    min-width: 18px;
    text-align: center;
    font-size: 12px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .count.zero {
    color: var(--text-dim);
    font-weight: 400;
  }
  .hint.batch {
    display: block;
    margin-top: 6px;
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

  .preview-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    background: none;
    border: none;
    padding: 2px 0;
    cursor: pointer;
    font: inherit;
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
    text-align: start;
  }
  .preview-toggle .hint {
    font-weight: 400;
  }
  .preview-toggle .chevron {
    font-size: 9px;
    color: var(--text-dim);
    transition: transform 120ms ease-out;
  }
  .preview-toggle .chevron.open {
    transform: rotate(90deg);
  }
  .preview-box {
    margin-top: 8px;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    max-height: 360px;
    overflow: auto;
  }
</style>
