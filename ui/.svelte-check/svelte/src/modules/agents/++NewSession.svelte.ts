///<reference types="svelte" />
;// ⌘T sheet: workspace (current / none), provider (from /meta.providers),
// title, cwd.

import Modal from '../../lib/components/Modal.svelte';
import { toastError } from '../../lib/toastError';
import ModelPicker from '../../lib/components/ModelPicker.svelte';
import AccountPicker from '../../lib/components/AccountPicker.svelte';
import NetworkProfilePicker from '../connections/NetworkProfilePicker.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import Icon from '../../lib/components/Icon.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import ContextPreview from './ContextPreview.svelte';
import McpProfilePicker from './McpProfilePicker.svelte';
import { router } from '../../lib/router.svelte';
import { api } from '../../lib/api/client';
import { checkFolder } from '../../lib/folderCheck';
import { radioKey } from '../../lib/radioKey';
import { ws, SCRATCH_WORKSPACE_ID } from '../../lib/stores/workspace.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { allProviders, providerReadiness } from '../../lib/providers';
;

  interface Props {
    onclose: () => void;
    /** Pre-select "No workspace" (the palette / sidebar "New session (no
     *  workspace)" entry points). */
    initialScratch?: boolean;
  };;
  interface PendingSpawn {
    provider: string;
    title: string;
    cwd: string;
    run: () => Promise<{id: string}>;
  };;
  type Prefs = { provider?: string; browser?: boolean };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  /** Per-provider ceiling on one batch — a typo in the stepper shouldn't be able
   *  to fork 200 agent processes at once. */
  const MAX_PER_PROVIDER = 20;

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
  /** Inline reason a typed working folder was refused (missing / a file). */
  let cwdError = $state('');
  let checkingCwd = false;
  let browser = $state(false);
  let mcpProfileIds = $state<string[]>([]); // [] = the default MCP profile
  let busy = $state(false);

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
  /** Spoken only once the user steps a count (± on a card), so opening the
   *  sheet or picking a provider doesn't announce "1 claude session". */
  let countsTouched = $state(false);
  const countAnnounce = $derived(
    countsTouched
      ? chosen.map((p) => `${counts[p]} ${p} ${counts[p] === 1 ? 'session' : 'sessions'}`).join(', ') || 'No sessions selected'
      : '',
  );

  /** Set the count for one provider, keeping `provider` on something selected. */
  function bump(p: string, delta: number): void {
    if (delta > 0 && !providerReadiness(p).available) return;
    countsTouched = true;
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
    if (path !== '' && !extraDirs.includes(path)) extraDirs = [...extraDirs, path];
    dirDraft = '';
  }

  function removeDir(dir: string): void {
    extraDirs = extraDirs.filter((d) => d !== dir);
  }

  function onDirKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') { e.preventDefault(); addDir(); }
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
    providerKeys(e, provider);
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

  /** The radiogroup's own keys: only on a focused provider CARD (a radio). Keys
   *  on a card's ± buttons bubble here too, and arrows there used to re-select
   *  (wiping the batch) while +/- bumped the primary instead of that card. */
  function onProviderKeydown(e: KeyboardEvent): void {
    const card = (e.target as HTMLElement | null)?.closest('[role="radio"]');
    if (!card) return;
    const p = providers[cardEls.indexOf(card as HTMLButtonElement)];
    if (p) providerKeys(e, p);
  }

  /** Arrow / Home / End move the exclusive choice from `from`; +/- step the
   *  batch count of `from` (the focused card, or the primary sheet-wide). */
  function providerKeys(e: KeyboardEvent, from: string): void {
    const avail = providers.filter((p) => providerReadiness(p).available);
    const idx = providers.indexOf(from);
    if (idx < 0 || avail.length === 0) return;
    const availIdx = Math.max(0, avail.indexOf(from));
    let nextProvider = avail[availIdx];
    switch (e.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        nextProvider = avail[(availIdx + 1) % avail.length];
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        nextProvider = avail[(availIdx - 1 + avail.length) % avail.length];
        break;
      case 'Home':
        nextProvider = avail[0];
        break;
      case 'End':
        nextProvider = avail[avail.length - 1];
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
    selectProvider(nextProvider, true);
  }

  // Browser tools attach the `otto-browser` MCP server through each CLI's own
  // config (claude: .mcp.json, agy: .agents/mcp_config.json, codex: per-spawn
  // `-c` overrides). Other providers don't load it, so the toggle is hidden.
  const BROWSER_PROVIDERS = ['claude', 'codex', 'agy'];
  const supportsBrowser = $derived(BROWSER_PROVIDERS.includes(provider));
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
    // A typo must not start an agent in a fresh, empty stray folder (the
    // daemon creates a missing cwd): check it exists first, like onboarding.
    if (dir !== '') {
      if (checkingCwd) return;
      checkingCwd = true;
      cwdError = '';
      try {
        const check = await checkFolder(dir, (url) => api.get(url));
        if (!check.ok) {
          cwdError = check.message;
          return;
        }
      } finally {
        checkingCwd = false;
      }
    }
    const options = {scratch: scratchMode};
    const workspaceId = ws.currentId;
    const sessionModel = supportsModel && model.trim() !== '' ? model.trim() : null;
    const spawns = chosen.flatMap((p) => Array.from({length: counts[p]}, () => p));
    pendingSpawns = spawns.map((p, i) => {
      const meta: Record<string, unknown> = {};
      if (accountIds[p]) meta.account_id = accountIds[p];
      if (networkProfileId) meta.network_profile_id = networkProfileId;
      if (browser && BROWSER_PROVIDERS.includes(p)) meta.browser = true;
      if (dirs.length > 0) meta.extra_dirs = [...dirs];
      if (mcpProfileIds.length && p !== 'shell') meta.mcp_profiles = [...mcpProfileIds];
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
        toasts.error(batchSuccesses.length ? 'Some sessions didn’t start' : 'Couldn’t create session', failures.join('\n'));
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

;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onkeydown":onGlobalKeydown,});}

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {       children:() => { return __sveltets_2_any(0); },"title":`New session`,"height":800,"onclose":requestClose,"dismissable":!busy,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":requestClose,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || (pendingSpawns.length === 0 && total === 0),"onclick":create,});
      busy ? 'Starting…' : pendingSpawns.length ? `Retry ${pendingSpawns.length} failed` : total > 1 ? `Start ${total} sessions` : 'Start session';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("div", {   "id":`ns-ws-label`,"class":`provider-label`,});  }
     { svelteHTML.createElement("div", {     "class":`seg`,"role":`radiogroup`,"aria-labelledby":`ns-ws-label`,});
      if(ws.current){
         { svelteHTML.createElement("button", {                  "type":`button`,"class":`seg-btn`,"role":`radio`,"aria-checked":!scratchMode,"tabindex":scratchMode ? -1 : 0,"onkeydown":radioKey,"title":ws.current.root_path,"onclick":() => setScratch(false),});!scratchMode;
          ws.current.name;
         }
      }
       { svelteHTML.createElement("button", {                  "type":`button`,"class":`seg-btn`,"role":`radio`,"aria-checked":scratchMode,"tabindex":scratchMode || !ws.current ? 0 : -1,"onkeydown":radioKey,"title":`Not tied to any workspace — starts in your home folder by default`,"onclick":() => setScratch(true),});scratchMode;
         
       }
     }
    if(scratchMode){
       { svelteHTML.createElement("span", { "class":`hint`,});
                       
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("div", {   "id":`ns-provider-label`,"class":`provider-label`,});
        { svelteHTML.createElement("span", { "class":`dim`,});             }
     }
     { const $$_div2 = svelteHTML.createElement("div", {            "class":`provider-grid`,"role":`radiogroup`,"tabindex":-1,"aria-labelledby":`ns-provider-label`,"onkeydown":onProviderKeydown,});gridEl = $$_div2;
          for(let p of __sveltets_2_ensureArray(providers)){let i = 1;p;
         { svelteHTML.createElement("div", {     "class":`provider-card`,"role":`none`,});countOf(p) > 0;provider === p;
           { const $$_button4 = svelteHTML.createElement("button", {                "class":`card-main`,"role":`radio`,"aria-checked":provider === p,"disabled":!providerReadiness(p).available,"title":providerReadiness(p).message,"tabindex":provider === p ? 0 : -1,"onclick":() => selectProvider(p),});cardEls[i] = $$_button4;
             { svelteHTML.createElement("span", { "class":`provider-name`,});
               { const $$_nocIredivorP6C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP6C({ target: __sveltets_2_any(), props: {    "provider":p,"size":14,}});}
              p;
              if(p === defaultProvider){ { svelteHTML.createElement("span", { "class":`default-badge`,});  }}
             }
             { svelteHTML.createElement("span", { "class":`provider-desc`,});
              p === 'claude' ? 'Claude Code CLI' : p === 'codex' ? 'Codex CLI' : p === 'shell' ? 'Plain shell' : 'Custom provider';
             }
           }
           { svelteHTML.createElement("div", {       "class":`count-ctl`,"role":`group`,"id":`ns-steppers-${i}`,"aria-label":`${p} sessions`,});
             { svelteHTML.createElement("button", {             "type":`button`,"class":`cbtn`,"disabled":countOf(p) === 0,"aria-label":`One less ${p} session`,"title":`One less ${p} session`,"onclick":() => bump(p, -1),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`minus`,"size":12,}});} }
             { svelteHTML.createElement("span", {    "class":`count`,"aria-hidden":`true`,});countOf(p) === 0;countOf(p); }
             { svelteHTML.createElement("button", {             "type":`button`,"class":`cbtn`,"disabled":countOf(p) >= MAX_PER_PROVIDER || !providerReadiness(p).available,"aria-label":`One more ${p} session`,"title":`One more ${p} session`,"onclick":() => bump(p, 1),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} }
           }
         }
      }
     }
    
     { svelteHTML.createElement("div", {     "role":`group`,"aria-label":`Sessions per provider`,"aria-owns":providers.map((_, i) => `ns-steppers-${i}`).join(' '),}); }
    if(total > 1){
       { svelteHTML.createElement("span", { "class":`hint batch`,}); total;   batchSummary; }
    }
    
     { svelteHTML.createElement("span", {   "class":`sr-only`,"role":`status`,});countAnnounce; }
   }

  
       for(let accountProvider of __sveltets_2_ensureArray(chosen.filter((p) => p === 'claude' || p === 'codex'))){accountProvider;
       { const $$_rekciPtnuoccA1C = __sveltets_2_ensureComponent(AccountPicker); new $$_rekciPtnuoccA1C({ target: __sveltets_2_any(), props: {        "provider":accountProvider,"value":accountIds[accountProvider] || '',"workspaceId":scratchMode ? SCRATCH_WORKSPACE_ID : ws.current!.id,"onchange":(id) => (accountIds[accountProvider] = id),}});}
    }
  if(supportsModel){
     { const $$_rekciPledoM1C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM1C({ target: __sveltets_2_any(), props: {     provider,"value":model,"onchange":(m) => (model = m),}});}
  }

  if(auth.can('connections', 'view')){
     { const $$_rekciPeliforPkrowteN1C = __sveltets_2_ensureComponent(NetworkProfilePicker); new $$_rekciPeliforPkrowteN1C({ target: __sveltets_2_any(), props: {          "workspaceId":networkWorkspace,"value":networkProfileId,"onchange":(id) => networkProfileId = id,"editable":auth.can('connections', 'edit') && (scratchMode || ws.myRole !== 'viewer'),"disabled":busy,}});}
  }


   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`ns-title`,});  { svelteHTML.createElement("span", { "class":`dim`,});  } }
     { svelteHTML.createElement("input", {          "dir":`auto`,"id":`ns-title`,"class":`input`,"bind:value":title,"placeholder":`Auto-named from your theme (Settings → Session Names)`,});/*Ωignore_startΩ*/() => title = __sveltets_2_any(null);/*Ωignore_endΩ*/}
    if(total > 1 && title.trim() !== ''){
       { svelteHTML.createElement("span", { "class":`hint`,});    title.trim();   title.trim(); total;  }
    }
   }

  if(chosen.some((p) => p !== 'shell')){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`ns-prompt`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
       { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`ns-prompt`,"class":`input prompt-input`,"rows":3,"bind:value":prompt,"placeholder":`What should the agent start on? Sent once it is ready.`,});/*Ωignore_startΩ*/() => prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("span", { "class":`hint`,});
        total > 1 ? 'Sent to every agent in this batch. ' : ''; total > 1 ? 'creates the sessions' : 'creates the session';
       }
     }
  }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`ns-cwd`,});  }
     { svelteHTML.createElement("div", { "class":`dir-add`,});
       { svelteHTML.createElement("input", {                    "dir":`ltr`,"id":`ns-cwd`,"class":`input mono`,"bind:value":cwd,"oninput":() => (cwdError = ''),"aria-invalid":cwdError !== '',"aria-describedby":`ns-cwd-err`,"spellcheck":`false`,"list":`ns-recent-dirs`,"placeholder":`/absolute/path/to/folder`,});/*Ωignore_startΩ*/() => cwd = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"title":`Browse for a working folder`,"onclick":() => (browsing = 'cwd'),});  }
     }
     { svelteHTML.createElement("datalist", { "id":`ns-recent-dirs`,});
         for(let d of __sveltets_2_ensureArray(recentDirs)){d; { svelteHTML.createElement("option", { "value":d,}); }}
     }
     { svelteHTML.createElement("span", { "class":`hint`,});
                    
      scratchMode ? 'Defaults to your home folder.' : 'Defaults to the workspace root.';
     }
     { svelteHTML.createElement("span", {     "id":`ns-cwd-err`,"class":`hint cwd-err`,"role":`status`,});cwdError; }
    if(isHome(cwd)){
      
       { svelteHTML.createElement("span", { "class":`hint home-notice`,});
                    
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`ns-extra-dir`,});   { svelteHTML.createElement("span", { "class":`dim`,});  } }
    if(extraDirs.length > 0){
       { svelteHTML.createElement("ul", { "class":`dir-list`,});
           for(let dir of __sveltets_2_ensureArray(extraDirs)){dir;
           { svelteHTML.createElement("li", { "class":`dir-row`,});
             { svelteHTML.createElement("span", {   "class":`dir-path mono`,"title":dir,});dir; }
             { svelteHTML.createElement("button", {           "type":`button`,"class":`dir-remove`,"title":`Remove directory`,"aria-label":`Remove ${dir}`,"onclick":() => removeDir(dir),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
           }
        }
       }
    }
     { svelteHTML.createElement("div", { "class":`dir-add`,});
       { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`ns-extra-dir`,"class":`input mono`,"bind:value":dirDraft,"spellcheck":`false`,"placeholder":`/absolute/path/to/repo`,"onkeydown":onDirKeydown,});/*Ωignore_startΩ*/() => dirDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {     "type":`button`,"class":`btn`,"onclick":() => (browsing = 'extra'),});  }
       { svelteHTML.createElement("button", {       "type":`button`,"class":`btn`,"disabled":dirDraft.trim() === '',"onclick":addDir,});  }
     }
     { svelteHTML.createElement("span", { "class":`hint`,});         { svelteHTML.createElement("code", {});  }  }
   }

  if(supportsBrowser){
     { svelteHTML.createElement("label", { "class":`toggle-row`,});
       { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":browser,});/*Ωignore_startΩ*/() => browser = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", { "class":`toggle-text`,});
         { svelteHTML.createElement("span", { "class":`toggle-title`,});  }
         { svelteHTML.createElement("span", { "class":`hint`,});            }
       }
     }
  }

  if(chosen.some((p) => p !== 'shell') && auth.can('mcp', 'view')){ { const $$_rekciPeliforPpcM1C = __sveltets_2_ensureComponent(McpProfilePicker); new $$_rekciPeliforPpcM1C({ target: __sveltets_2_any(), props: {    "value":mcpProfileIds,"onchange":(ids) => (mcpProfileIds = ids),}});}}

  
  if(supportsContext && ws.currentId && !scratchMode){
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("button", {         "type":`button`,"class":`preview-toggle`,"onclick":() => (showPreview = !showPreview),"aria-expanded":showPreview,});
         { svelteHTML.createElement("span", {  "class":`chevron`,});showPreview; { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"noflip":true,"size":12,}});} }
         
         { svelteHTML.createElement("span", { "class":`hint`,});        }
       }
      if(showPreview){
         { svelteHTML.createElement("div", { "class":`preview-box`,});
           { const $$_weiverPtxetnoC3C = __sveltets_2_ensureComponent(ContextPreview); new $$_weiverPtxetnoC3C({ target: __sveltets_2_any(), props: {      "wsId":ws.currentId,provider,"overrides":{ cwd: cwd.trim() === '' ? undefined : cwd.trim() },}});}
         }
      }
     }
  }

   { svelteHTML.createElement("div", { "role":`status`,});
    if(batchFailures.length){
       { svelteHTML.createElement("p", {});batchSuccesses.length;  pendingSpawns.length;          }
         for(let failure of __sveltets_2_ensureArray(batchFailures)){let i = 1;
         { svelteHTML.createElement("p", {});failure; { svelteHTML.createElement("br", {});} { svelteHTML.createElement("span", { "class":`hint`,});pendingSpawns[i]?.cwd; } }
      }
    }
   }

  
 Modal}

if(browsing){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":browsing === 'cwd' ? 'Choose working folder' : 'Choose an additional folder',"start":(browsing === 'cwd' ? cwd : dirDraft) ||
      (scratchMode ? scratchHome : ws.current?.root_path) ||
      '~',"onpick":(path: string) => {
      if (browsing === 'cwd') cwd = path;
      else dirDraft = path;
      browsing = null;
    },"onclose":() => (browsing = null),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NewSession__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NewSession__SvelteComponent_ = ReturnType<typeof NewSession__SvelteComponent_>;
/*Ωignore_endΩ*/export default NewSession__SvelteComponent_;