<script lang="ts">
  // ⌘T sheet: workspace (current / none), provider (from /meta.providers),
  // title, cwd.
  import Modal from '../../lib/components/Modal.svelte';
  import { toastError } from '../../lib/toastError';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import AccountPicker from '../../lib/components/AccountPicker.svelte';
  import NetworkProfilePicker from '../connections/NetworkProfilePicker.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ContextPreview from './ContextPreview.svelte';
  import { router } from '../../lib/router.svelte';
  import { ws, SCRATCH_WORKSPACE_ID } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { allProviders, providerReadiness } from '../../lib/providers';

  /** Per-provider ceiling on one batch — a typo in the stepper shouldn't be able
   *  to fork 200 agent processes at once. */
  const MAX_PER_PROVIDER = 20;

  interface Props {
    onclose: () => void;
    /** Pre-select "No workspace" (the palette / sidebar "New session (no
     *  workspace)" entry points). */
    initialScratch?: boolean;
  }
  let { onclose, initialScratch = false }: Props = $props();

  // Workspace-less ("scratch") session: lives in the daemon's hidden scratch
  // workspace instead of the current one, so it needs no workspace at all.
  // Forced on when the user has no workspace to put the session in. The prop
  // is an initial value only — the segmented control owns it from then on.
  // svelte-ignore state_referenced_locally
  let scratch = $state(initialScratch);
  const scratchMode = $derived(scratch || ws.current === null);
  /** Default cwd of a workspace-less session: the daemon user's home. */
  const scratchHome = $derived(ws.scratch?.root_path ?? '~');
  /** Whether a typed cwd is the home folder itself (trust + sandbox then cover
   *  everything under it — see the notice under the field). */
  const isHome = (p: string): boolean => {
    const t = p.trim().replace(/\/+$/, '');
    return t === '~' || (ws.scratch !== null && t === ws.scratch.root_path);
  };
  /** Switch between the current workspace and no workspace, carrying the cwd
   *  along when it still sits on the previous mode's default. */
  function setScratch(on: boolean): void {
    const prev = on ? (ws.current?.root_path ?? '') : scratchHome;
    const next = on ? scratchHome : (ws.current?.root_path ?? '');
    if (cwd.trim() === '' || cwd === prev) cwd = next;
    scratch = on;
  }

  const providers = $derived(allProviders());
  // Effective default agent: this workspace's override, else the global default
  // (a workspace-less session has no workspace setting to consult).
  const wsDefault = $derived(
    !scratchMode && typeof ws.current?.settings?.default_provider === 'string'
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
  let accountIds = $state<Record<string, string>>({});
  let networkProfileId = $state('');
  const networkWorkspace = $derived(scratchMode ? SCRATCH_WORKSPACE_ID : ws.current!.id);
  $effect(() => { void networkWorkspace; networkProfileId = ''; });
  let title = $state('');
  /** Optional opening message (A6) — delivered by the daemon once the CLI is ready. */
  let prompt = $state('');
  let cwd = $state('');
  let browser = $state(false);
  let busy = $state(false);
  interface PendingSpawn {
    provider: string;
    title: string;
    cwd: string;
    run: () => Promise<{id: string}>;
  }
  // Only failures survive an attempt; callbacks capture the submitted request.
  let pendingSpawns = $state<PendingSpawn[]>([]);
  let batchFailures = $state<string[]>([]);
  let batchSuccesses = $state<string[]>([]);
  // Daemon-side folder picker for the working directory (and the extra-dirs
  // field): pointing a session at a folder outside the workspace should not
  // require creating a workspace for it, or typing an absolute path by hand.
  let browsing: 'cwd' | 'extra' | null = $state(null);

  // ── Per-device memory of the last sheet ──────────────────────────────────
  // The sheet used to forget everything between opens: the Browser tools
  // toggle and (when no default agent is configured) which agent was picked.
  // Accounts are deliberately NOT remembered — a deleted account's id would
  // be sent silently while the picker showed "Default CLI account". Settings stay authoritative — a
  // configured default provider always wins over the remembered one.
  const PREFS_KEY = 'otto_new_session_prefs';
  type Prefs = { provider?: string; browser?: boolean };
  function loadPrefs(): Prefs {
    try {
      const raw = localStorage.getItem(PREFS_KEY);
      const v: unknown = raw ? JSON.parse(raw) : null;
      return v && typeof v === 'object' ? (v as Prefs) : {};
    } catch {
      return {};
    }
  }
  function savePrefs(p: Prefs): void {
    try {
      localStorage.setItem(PREFS_KEY, JSON.stringify(p));
    } catch {
      /* private window / blocked storage: nothing is remembered */
    }
  }
  const prefs = loadPrefs();
  browser = prefs.browser === true;

  const countOf = (p: string): number => counts[p] ?? 0;
  const total = $derived(Object.values(counts).reduce((a, b) => a + b, 0));
  /** Distinct providers in the batch, in the grid's own order. */
  const chosen = $derived(providers.filter((p) => countOf(p) > 0));
  /** "3 claude, 2 codex, 1 shell" — the batch, spelled out. */
  const batchSummary = $derived(chosen.map((p) => `${counts[p]} ${p}`).join(', '));

  /** Set the count for one provider, keeping `provider` on something selected. */
  function bump(p: string, delta: number): void {
    if (delta > 0 && !providerReadiness(p).available) return;
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
  // this workspace's sessions, with the workspace root first — or, for a
  // workspace-less session, the home folder then the scratch sessions' cwds.
  // Offered as a datalist on the cwd field so the common case is one keystroke.
  const recentDirs = $derived.by((): string[] => {
    const seen: string[] = [];
    const push = (d: string | null | undefined): void => {
      if (d && !seen.includes(d)) seen.push(d);
    };
    const inScope = (s: { workspace_id: string }): boolean =>
      (s.workspace_id === SCRATCH_WORKSPACE_ID) === scratchMode;
    push(scratchMode ? scratchHome : ws.current?.root_path);
    for (const s of [...ws.sessions].reverse()) if (inScope(s)) push(s.cwd);
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
    if (t && (t.closest('input, textarea, select, [contenteditable="true"]') || gridEl?.contains(t))) return;
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
      const available = providers.filter((p) => providerReadiness(p).available);
      const def = defaultProvider && available.includes(defaultProvider) ? defaultProvider : null;
      const last = prefs.provider && available.includes(prefs.provider) ? prefs.provider : null;
      const initial = def ?? last ?? (available.includes('claude') ? 'claude' : available[0]);
      if (initial) selectProvider(initial);
    }
    if (cwd === '') {
      if (scratchMode) cwd = scratchHome;
      else if (ws.current) cwd = ws.current.root_path;
    }
  });

  async function create(): Promise<void> {
    if (busy) return;
    if (pendingSpawns.length) { await retryFailed(); return; }
    if (total === 0) return;
    const unavailable = chosen.find((p) => !providerReadiness(p).available);
    if (unavailable) { toasts.error(`Couldn’t start ${unavailable}`, providerReadiness(unavailable).message); return; }
    const dirs = [...extraDirs];
    const pending = dirDraft.trim();
    if (pending !== '' && !dirs.includes(pending)) dirs.push(pending);
    const base = title.trim(), firstMessage = prompt.trim();
    let dir = cwd.trim();
    const home = ws.scratch?.root_path;
    if (home && (dir === '~' || dir.startsWith('~/'))) dir = home + dir.slice(1);
    const options = {scratch: scratchMode};
    const workspaceId = ws.currentId;
    const sessionModel = supportsModel && model.trim() !== '' ? model.trim() : null;
    const spawns = chosen.flatMap((p) => Array.from({length: counts[p]}, () => p));
    pendingSpawns = spawns.map((p, i) => {
      const meta: Record<string, unknown> = {};
      if (accountIds[p]) meta.account_id = accountIds[p];
      if (networkProfileId) meta.network_profile_id = networkProfileId;
      if (browser && (p === 'claude' || p === 'codex')) meta.browser = true;
      if (dirs.length > 0) meta.extra_dirs = [...dirs];
      const sessionTitle = base === '' ? null : spawns.length > 1 ? `${base} ${i + 1}` : base;
      const request = {provider: p, title: sessionTitle, cwd: dir === '' ? null : dir, model: sessionModel};
      return {provider: p, title: sessionTitle ?? p, cwd: dir, run: async () => {
        // A workspace change must not redirect a retained failed request.
        if (!options.scratch && ws.currentId !== workspaceId) throw new Error('Return to the original workspace to retry this session.');
        if (!providerReadiness(p).available) throw new Error(providerReadiness(p).message);
        return firstMessage !== '' && p !== 'shell'
          ? ws.openSessionWithPrompt({...request, prompt: firstMessage, meta}, options)
          : ws.createSessionQuiet({...request, kind: 'agent', meta: Object.keys(meta).length ? meta : null}, options);
      }};
    });
    batchSuccesses = [];
    await retryFailed();
  }

  async function retryFailed(): Promise<void> {
    if (busy || pendingSpawns.length === 0) return;
    busy = true;
    const submitted = pendingSpawns;
    const failed: PendingSpawn[] = [], failures: string[] = [], created: string[] = [];
    try {
      for (const spawn of submitted) {
        try {
          const session = await spawn.run();
          created.push(session.id);
        } catch (error) {
          failed.push(spawn);
          failures.push(`${spawn.title} (${spawn.provider}): ${error instanceof Error ? error.message : String(error)}`);
        }
      }
      // Commit outcome before navigation; successful members can never enter a retry.
      pendingSpawns = failed;
      batchFailures = failures;
      batchSuccesses = [...batchSuccesses, ...created];
      if (created.length > 0) savePrefs({provider, browser});
      if (batchSuccesses.length > 1) ws.setViewMode('tiled');
      for (const id of created.slice(0, -1)) ws.openSession(id);
      if (created.length > 0) ws.navigateToSession(created[created.length - 1]);
      if (failed.length > 0) {
        toasts.error(batchSuccesses.length ? 'Some sessions did not start' : 'Could not create session', failures.join('\n'));
        return;
      }
      onclose();
    } catch (error) {
      toastError('Couldn’t create session', error);
    } finally {
      busy = false;
    }
  }

  /** Esc / backdrop / ✕: an opening message or title typed here must not
   *  vanish on a stray key — ask first when something was entered. */
  async function requestClose(): Promise<void> {
    const edited = prompt.trim() !== '' || title.trim() !== '' || extraDirs.length > 0;
    if (edited) {
      const ok = await confirmer.ask('The title, opening message and folders you entered will be lost.', {
        title: 'Discard this new session?',
        confirmLabel: 'Discard',
        cancelLabel: 'Keep editing',
        danger: true,
      });
      if (!ok) return;
    }
    onclose();
  }

</script>

<svelte:window onkeydown={onGlobalKeydown} />

<Modal title="New session" onclose={requestClose} dismissable={!busy}>
  <!-- Workspace: the current one, or none (a workspace-less session in the
       daemon's hidden scratch workspace). With no workspace at all only "No
       workspace" exists, pre-selected. -->
  <div class="field">
    <div id="ns-ws-label" class="provider-label">Workspace</div>
    <div class="seg" role="radiogroup" aria-labelledby="ns-ws-label">
      {#if ws.current}
        <button
          type="button"
          class="seg-btn"
          class:active={!scratchMode}
          role="radio"
          aria-checked={!scratchMode}
          title={ws.current.root_path}
          onclick={() => setScratch(false)}
        >
          {ws.current.name}
        </button>
      {/if}
      <button
        type="button"
        class="seg-btn"
        class:active={scratchMode}
        role="radio"
        aria-checked={scratchMode}
        title="Not tied to any workspace — starts in your home folder by default"
        onclick={() => setScratch(true)}
      >
        No workspace
      </button>
    </div>
    {#if scratchMode}
      <span class="hint">
        Listed under “No workspace” in the sidebar; archive, resume and hand over like any other session.
      </span>
    {/if}
  </div>

  <div class="field">
    <div id="ns-provider-label" class="provider-label">
      Provider <span class="dim">· use − / + on a card to start several at once</span>
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
            disabled={!providerReadiness(p).available}
            title={providerReadiness(p).message}
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
              title={`One less ${p} session`}
              onclick={() => bump(p, -1)}
            ><Icon name="minus" size={12} /></button>
            <span class="count" class:zero={countOf(p) === 0} aria-live="polite">
              {countOf(p)}
            </span>
            <button
              type="button"
              class="cbtn"
              disabled={countOf(p) >= MAX_PER_PROVIDER || !providerReadiness(p).available}
              aria-label={`One more ${p} session`}
              title={`One more ${p} session`}
              onclick={() => bump(p, 1)}
            ><Icon name="plus" size={12} /></button>
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
    {#each chosen.filter((p) => p === 'claude' || p === 'codex') as accountProvider (accountProvider)}
      <AccountPicker provider={accountProvider} value={accountIds[accountProvider] || ''}
        workspaceId={scratchMode ? SCRATCH_WORKSPACE_ID : ws.current!.id}
        onchange={(id) => (accountIds[accountProvider] = id)} />
    {/each}
  {#if supportsModel}
    <ModelPicker {provider} value={model} onchange={(m) => (model = m)} />
  {/if}

  {#if auth.can('connections', 'view')}
    <NetworkProfilePicker workspaceId={networkWorkspace} value={networkProfileId} onchange={(id) => networkProfileId = id} editable={auth.can('connections', 'edit') && (scratchMode || ws.myRole !== 'viewer')} disabled={busy} />
  {/if}


  <div class="field">
    <label for="ns-title">Title <span class="dim">(optional)</span></label>
    <input dir="auto" id="ns-title" class="input" bind:value={title} placeholder="Auto-named from your theme (Settings → Session Names)" />
    {#if total > 1 && title.trim() !== ''}
      <span class="hint">Numbered per session — “{title.trim()} 1” … “{title.trim()} {total}”.</span>
    {/if}
  </div>

  {#if chosen.some((p) => p !== 'shell')}
    <div class="field">
      <label for="ns-prompt">First message <span class="dim">(optional)</span></label>
      <textarea dir="auto"
        id="ns-prompt"
        class="input prompt-input"
        rows="3"
        bind:value={prompt}
        placeholder="What should the agent start on? Sent once it is ready."
      ></textarea>
      <span class="hint">
        {total > 1 ? 'Sent to every agent in this batch. ' : ''}⌘↩ {total > 1 ? 'creates the sessions' : 'creates the session'}.
      </span>
    </div>
  {/if}

  <div class="field">
    <label for="ns-cwd">Working folder</label>
    <div class="dir-add">
      <input dir="ltr"
        id="ns-cwd"
        class="input mono"
        bind:value={cwd}
        spellcheck="false"
        list="ns-recent-dirs"
        placeholder="/absolute/path/to/folder"
      />
      <button type="button" class="btn" title="Browse for a working folder" onclick={() => (browsing = 'cwd')}>Browse…</button>
    </div>
    <datalist id="ns-recent-dirs">
      {#each recentDirs as d (d)}<option value={d}></option>{/each}
    </datalist>
    <span class="hint">
      Any folder on this machine — it does not have to be inside a workspace.
      {scratchMode ? 'Defaults to your home folder.' : 'Defaults to the workspace root.'}
    </span>
    {#if isHome(cwd)}
      <!-- Trust and the sandbox follow the session cwd (not the workspace):
           a home-rooted session is trusted for, and may write under, all of ~. -->
      <span class="hint home-notice">
        Home folder: the agent is trusted for, and may write anywhere under, ~
      </span>
    {/if}
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
              aria-label="Remove {dir}"
              onclick={() => removeDir(dir)}
            ><Icon name="x" size={12} /></button>
          </li>
        {/each}
      </ul>
    {/if}
    <div class="dir-add">
      <input dir="ltr"
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

  <!-- The context preview is workspace-scoped (skills/soul/context of a
       workspace), so a workspace-less session has nothing to preview. -->
  {#if supportsContext && ws.currentId && !scratchMode}
    <div class="field">
      <button
        type="button"
        class="preview-toggle"
        onclick={() => (showPreview = !showPreview)}
        aria-expanded={showPreview}
      >
        <span class="chevron" class:open={showPreview}><Icon name="chevronRight" noflip size={12} /></span>
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

  {#if batchFailures.length}
    <div role="status">
      <p>{batchSuccesses.length} started; {pendingSpawns.length} still need to start. Retry uses the submitted settings.</p>
      {#each batchFailures as failure, i}
        <p>{failure}<br /><span class="hint">{pendingSpawns[i]?.cwd}</span></p>
      {/each}
    </div>
  {/if}

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={busy || (pendingSpawns.length === 0 && total === 0)} onclick={create}>
      {busy ? 'Starting…' : pendingSpawns.length ? `Retry ${pendingSpawns.length} failed` : total > 1 ? `Start ${total} sessions` : 'Start session'}
    </button>
  {/snippet}
</Modal>

{#if browsing}
  <FolderPicker
    title={browsing === 'cwd' ? 'Choose working folder' : 'Choose an additional folder'}
    start={(browsing === 'cwd' ? cwd : dirDraft) ||
      (scratchMode ? scratchHome : ws.current?.root_path) ||
      '~'}
    onpick={(path: string) => {
      if (browsing === 'cwd') cwd = path;
      else dirDraft = path;
      browsing = null;
    }}
    onclose={() => (browsing = null)}
  />
{/if}

<style>
  .prompt-input {
    resize: vertical;
    min-height: 64px;
    font: inherit;
    font-size: var(--fs-s);
    line-height: 1.4;
  }
  .provider-label {
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
    margin-bottom: 4px;
  }
  /* Segmented control: current workspace vs no workspace (same look as the
     handover sheet's target switch). */
  .seg {
    display: flex;
    gap: 4px;
    padding: 2px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
  }
  .seg-btn {
    flex: 1;
    min-width: 0;
    padding: 6px 10px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .seg-btn.active {
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--glass-shadow);
  }
  .seg + .hint {
    display: block;
    margin-top: 6px;
  }
  .home-notice {
    display: block;
    margin-top: 4px;
    color: var(--text-dim);
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
    transition: border-color var(--dur-fast) ease-out, background var(--dur-fast) ease-out;
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
    background: var(--accent-soft);
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
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1;
    cursor: pointer;
  }
  .cbtn:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--accent);
  }
  .cbtn:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  /* Touch: the ± targets have to be tappable on a phone, where the card is the
     same size but fingers are not cursors. */
  @media (pointer: coarse) {
    .cbtn {
      width: 30px;
      height: 30px;
      font-size: var(--fs-l);
    }
  }
  .count {
    min-width: 18px;
    text-align: center;
    font-size: var(--fs-s);
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
    font-size: var(--fs-m);
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .default-badge {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .provider-desc {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .toggle-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
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
    font-size: var(--fs-m);
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
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .dir-path {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .dir-remove {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    padding: 2px 4px;
    border-radius: var(--radius-s);
    line-height: 1;
  }
  .dir-remove:hover {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
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
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
    text-align: start;
  }
  .preview-toggle .hint {
    font-weight: 400;
  }
  .preview-toggle .chevron {
    display: inline-flex;
    color: var(--text-dim);
    transition: transform var(--dur-fast) ease-out;
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
