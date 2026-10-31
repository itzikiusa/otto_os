///<reference types="svelte" />
;// ⌘K palette. Two modes: fuzzy command list (registry) and plain-English
// orchestrator (⇥ toggles). English mode posts /orchestrate, shows the plan
// for confirmation, then /orchestrate/execute.
// Commands mode also fans out to GET /workspaces/{id}/search when the query
// is ≥ 2 chars and a workspace is active; cross-module hits appear as a
// second "Results" section below commands.

import { dialogFocus } from '../lib/dialogFocus';
import { NO_WORKSPACE } from '../lib/labels';
import { toastError } from '../lib/toastError';
import { loadErrorText } from '../lib/loadError';
import { api, isAbortError } from '../lib/api/client';
import type { Action, SearchHit } from '../lib/api/types';
import { tick, untrack } from 'svelte';
import { registry, type Command } from '../lib/commands.svelte';
import { loadFrecency, rankCommands, recordUsage } from '../lib/commandSearch';
import {
    describeAction,
    executePlan as runPlan,
    runEnglish,
    type EnglishOutcome,
  } from '../lib/orchestrate';
import { plural } from '../lib/plural';
import { ui } from '../lib/stores/ui.svelte';
import { ws } from '../lib/stores/workspace.svelte';
import { storeContext } from '../lib/ask';
import { toasts } from '../lib/toast.svelte';
import { confirmer } from '../lib/confirm.svelte';
import { applyClose } from '../lib/orchestrate';
import { closeConfirm } from './closeConfirm';
import Icon, { type IconName } from '../lib/components/Icon.svelte';
import { lsGet, lsSet } from '../lib/storage';
import { COPY_FAILED_BODY, copyText } from '../lib/clipboard';
import { router } from '../lib/router.svelte';
import { hitActions, hitContext, hitRoute, type HitAction } from '../lib/floatingBar';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const HIT_ACTION_LABEL: Record<HitAction, string> = {
    open: 'Open',
    'copy-context': 'Copy context',
    'send-to-agent': 'Send to agent',
  };

  /** The keys that run a hit's actions from the keyboard (shown on the selected hit). */
  const HIT_ACTION_KEY: Record<HitAction, string> = { open: '⏎', 'copy-context': '⌥⏎', 'send-to-agent': '⇧⏎' };

  let mode: 'commands' | 'english' = $state('commands');
  let query = $state('');
  let englishText = $state('');
  let optimize = $state(lsGet('otto_orch_optimize') === '1');
  let aiFallback = $state(lsGet('otto_orch_fallback') !== '0');
  let selected = $state(0);
  let busy = $state(false);
  let plan: Action[] | null = $state(null);
  let optimizedText: string | null = $state(null);
  let inputEl: HTMLInputElement | null = $state(null);
  /** The element focused when the palette opened (restored on close). */
  let returnFocus: HTMLElement | null = null;
  let textareaEl: HTMLTextAreaElement | null = $state(null);
  /** The proposed plan's heading: focused when a plan arrives, so it is read. */
  let planTitleEl: HTMLElement | null = $state(null);
  let downOnBackdrop = false;

  // ---- cross-module search ----
  // Debounced fan-out to GET /workspaces/{id}/search when the query is
  // non-trivial and a workspace is active. Results render as a second section.
  let searchHits: SearchHit[] = $state([]);
  let searchBusy = $state(false);
  let searchAbort: AbortController | null = null;

  const SEARCH_DEBOUNCE_MS = 250;
  let searchTimer: ReturnType<typeof setTimeout> | null = null;
  let searchError = $state<string | null>(null);

  /** Kick off a debounced cross-module search. Cancels any in-flight request. */
  function scheduleSearch(q: string): void {
    if (searchTimer !== null) clearTimeout(searchTimer);
    // Cancel in-flight request immediately so we don't show stale hits.
    searchAbort?.abort();
    searchAbort = null;
    searchError = null;
    searchHits = [];
    if (q.trim().length < 2 || !ws.currentId) {
      searchHits = [];
      searchBusy = false;
      return;
    }
    searchBusy = true;
    const workspaceId = ws.currentId;
    searchTimer = setTimeout(() => {
      void doSearch(q.trim(), workspaceId);
    }, SEARCH_DEBOUNCE_MS);
  }

  async function doSearch(q: string, wsId: string): Promise<void> {
    const ctrl = new AbortController();
    searchAbort = ctrl;
    searchBusy = true;
    searchError = null;
    try {
      const hits = await api.get<SearchHit[]>(
        `/workspaces/${wsId}/search?q=${encodeURIComponent(q)}`,
        ctrl.signal,
      );
      if (searchAbort === ctrl && !ctrl.signal.aborted) searchHits = hits ?? [];
    } catch (e) {
      if (searchAbort === ctrl && !ctrl.signal.aborted && !isAbortError(e)) {
        searchHits = [];
        searchError = loadErrorText(e);
      }
    } finally {
      if (searchAbort === ctrl) {
        searchAbort = null;
        searchBusy = false;
      }
    }
  }

  /** Icon name for a search hit kind. */
  function hitIcon(kind: string): IconName {
    switch (kind) {
      case 'story': return 'ticket';
      case 'workflow': return 'merge';
      case 'api_request': return 'zap';
      case 'swarm_task': return 'check';
      case 'swarm_project': return 'layers';
      case 'memory': return 'db';
      case 'repo': return 'branch';
      case 'broker_cluster': return 'box';
      default: return 'file';
    }
  }

  /** Run a search hit's action. `open` navigates the way the floating bar
   *  does (`hitRoute`); copy-context copies the hit's description; send-to-
   *  agent hands it to the plain-English box as a draft. */
  async function hitAction(hit: SearchHit, action: HitAction): Promise<void> {
    if (action === 'send-to-agent') {
      englishText = `${hitContext(hit)}\n\n`;
      mode = 'english';
      plan = null;
      optimizedText = null;
      return;
    }
    close();
    if (action === 'open') {
      router.go(hitRoute(hit));
    } else if (await copyText(hitContext(hit))) {
      toasts.success('Copied', hit.title);
    } else {
      toasts.error('Couldn’t copy', COPY_FAILED_BODY);
    }
  }

  // Frecency-boosted fuzzy ranking, shared with the floating bar
  // (lib/commandSearch.ts).
  const filtered: { cmd: Command; score: number }[] = $derived(
    rankCommands(registry.all, query, loadFrecency()),
  );

  // Fire a debounced cross-module search whenever the query changes in commands
  // mode. Using $effect ensures this tracks `query` and `mode` reactively.
  $effect(() => {
    if (mode === 'commands') {
      scheduleSearch(query);
    } else {
      // Clear stale results when switching to English mode.
      searchHits = [];
      searchBusy = false;
      if (searchTimer !== null) { clearTimeout(searchTimer); searchTimer = null; }
      searchAbort?.abort();
      searchAbort = null;
    }
  });

  $effect(() => {
    // Reset on open + focus, honoring the requested mode + prefill. Track ONLY
    // `ui.paletteOpen`: the body reads `mode` after writing it, so without the
    // untrack a Commands↔English toggle (or askOtto) re-ran this effect, which
    // flipped `mode` back and wiped `query` / `plan`.
    const open = ui.paletteOpen;
    untrack(() => {
      if (open) {
        registry.want();
        // Hand focus back to whatever had it (a terminal, a list row) on close.
        returnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
        mode = ui.paletteMode;
        query = '';
        selected = 0;
        plan = null;
        optimizedText = null;
        searchHits = [];
        searchBusy = false;
        if (mode === 'english') englishText = ui.palettePrefill;
        ui.palettePrefill = '';
        queueMicrotask(() => {
          if (mode === 'commands') inputEl?.focus();
          else {
            textareaEl?.focus();
            // place caret at the end (after any prefill like "broadcast ")
            const len = englishText.length;
            textareaEl?.setSelectionRange(len, len);
          }
        });
      } else {
        const back = returnFocus;
        returnFocus = null;
        // Only if nothing else took focus meanwhile (a command that opened a
        // sheet or moved to another page owns focus now).
        queueMicrotask(() => {
          if (back?.isConnected && (document.activeElement === document.body || document.activeElement === null)) back.focus();
        });
        // Palette closed — cancel any pending search so we don't waste a round-trip.
        if (searchTimer !== null) { clearTimeout(searchTimer); searchTimer = null; }
        searchAbort?.abort();
        searchAbort = null;
      }
    });
  });

  // Free text in commands mode is always offered as an orchestrator ask, so
  // "open 2 claude sessions" works without knowing about the English mode.
  const askRow = $derived(query.trim() !== '');
  // One keyboard row model: commands, then the ask row, then search hits —
  // ↑/↓ walk all three, so a cross-module hit is reachable without a mouse.
  const hitBase = $derived(filtered.length + (askRow ? 1 : 0));
  const totalRows = $derived(hitBase + searchHits.length);
  // The combobox's live line (always mounted, OUTSIDE the listbox, which may
  // only own options and groups): empty, searching, failed, or the hit count.
  const statusText = $derived(searchError ? `Search failed: ${searchError}` : searchBusy ? 'Searching…'
    : filtered.length === 0 && !askRow ? 'No matching commands' : searchHits.length > 0 ? plural(searchHits.length, 'result') : '');

  $effect(() => {
    void totalRows;
    if (selected >= totalRows) selected = Math.max(0, totalRows - 1);
  });
  // A new query puts the selection back on the best match (row 0): without
  // this, arrowing to row 3 and typing more made Enter run whatever landed on
  // row 3 of the NEW list. Hits arriving later only append (see above).
  $effect(() => {
    void query;
    untrack(() => (selected = 0));
  });

  // Keep the keyboard selection visible in the scrolling list.
  let listEl: HTMLDivElement | null = $state(null);
  $effect(() => {
    const id = `pal-opt-${selected}`;
    listEl?.querySelector(`#${id}`)?.scrollIntoView({ block: 'nearest' });
  });

  /** The default action for a hit on Enter: open it when it can be opened. */
  function hitDefault(hit: SearchHit, e?: KeyboardEvent): void {
    // ⏎ opens; ⌥⏎ copies the hit as context; ⇧⏎ sends it to an agent (the pills
    // under a hit are the mouse shortcut for the same actions).
    const acts = hitActions(hit);
    const want: HitAction | null = e?.altKey ? 'copy-context' : e?.shiftKey ? 'send-to-agent' : null;
    const a = want && acts.includes(want) ? want : acts[0];
    if (a) void hitAction(hit, a);
  }

  function askOtto(): void {
    englishText = query;
    mode = 'english';
    plan = null;
    optimizedText = null;
    void submitEnglish();
    queueMicrotask(() => textareaEl?.focus());
  }

  function close(): void {
    ui.paletteOpen = false;
  }

  /** Drop the proposed plan and put focus back in the request box (the plan
   *  block — and the focused Cancel inside it — is unmounted). */
  function cancelPlan(): void {
    plan = null;
    queueMicrotask(() => textareaEl?.focus());
  }

  function toggleMode(): void {
    mode = mode === 'commands' ? 'english' : 'commands';
    plan = null;
    optimizedText = null;
    queueMicrotask(() => (mode === 'commands' ? inputEl?.focus() : textareaEl?.focus()));
  }

  function setOptimize(v: boolean): void {
    optimize = v;
    lsSet('otto_orch_optimize', v ? '1' : '0');
  }
  function setFallback(v: boolean): void {
    aiFallback = v;
    lsSet('otto_orch_fallback', v ? '1' : '0');
  }

  async function run(cmd: Command): Promise<void> {
    // Record this invocation before closing so loadFrecency() in the next open
    // already sees the updated entry.
    recordUsage(cmd.id);
    close();
    try {
      await cmd.run();
    } catch (e) {
      toastError('Couldn’t run the command', e);
    }
  }

  function onCommandsKey(e: KeyboardEvent): void {
    // An IME composition's Enter/arrows belong to the IME (CJK input), not
    // to the list — confirming a composition must not run the top command.
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === 'Tab') {
      e.preventDefault();
      toggleMode();
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      selected = Math.min(selected + 1, totalRows - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      selected = Math.max(selected - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      const item = filtered[selected];
      if (item) void run(item.cmd);
      else if (askRow && selected === filtered.length) askOtto();
      else if (searchHits[selected - hitBase]) hitDefault(searchHits[selected - hitBase], e);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  }

  function onEnglishKey(e: KeyboardEvent): void {
    if (e.isComposing || e.keyCode === 229) return; // IME composition (see onCommandsKey)
    if (e.key === 'Tab' && englishText === '') {
      e.preventDefault();
      toggleMode();
    } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void submitEnglish();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      if (plan) cancelPlan();
      else close();
    }
  }

  async function submitEnglish(): Promise<void> {
    if (englishText.trim() === '' || busy) return;
    if (!ws.currentId) {
      toasts.error(NO_WORKSPACE, 'Pick one in the navigator, then try again.');
      return;
    }
    busy = true;
    plan = null;
    // Remember which sessions exist now, so we can OPEN whatever gets spawned.
    const before = new Set(ws.sessions.map((s) => s.id));
    try {
      // The shared engine (lib/orchestrate.ts): close → addressed send →
      // deterministic plan (runs at once) → AI planner (confirm below).
      // A permanent delete comes back as `confirm-close` and asks first — the
      // same guard the floating bar has (⌘I lands here).
      const out = await runEnglish(englishText, {
        ...storeContext(ws.currentId),
        optimize,
        aiFallback,
        confirmDestructive: true,
      });
      await applyOutcome(out, before);
    } catch (e) {
      toastError('Couldn’t build the plan', e);
    } finally {
      busy = false;
    }
  }

  /** Render an engine outcome: toast + close, or show the plan to confirm. */
  async function applyOutcome(out: EnglishOutcome, before: Set<string>): Promise<void> {
    const done = (): void => {
      close();
      englishText = '';
    };
    switch (out.kind) {
      case 'empty':
        return;
      case 'confirm-close': {
        const { message, ...opts } = closeConfirm(out);
        if (!(await confirmer.ask(message, opts)) || !ws.currentId) return;
        const count = await applyClose(storeContext(ws.currentId), out.ids, out.permanent);
        toasts.success(
          out.permanent ? 'Sessions deleted' : 'Sessions closed',
          `${plural(count, 'session')} ${out.permanent ? 'removed' : 'archived'}`,
        );
        return done();
      }
      case 'closed':
        toasts.success(
          out.permanent ? 'Sessions deleted' : 'Sessions closed',
          `${plural(out.count, 'session')} ${out.permanent ? 'removed' : 'archived'}`,
        );
        return done();
      case 'nothing-to-close':
        toasts.warn('Nothing to close', 'No matching open session.');
        return done();
      case 'no-session':
        toasts.warn('No matching session', 'Couldn’t find that session to send to.');
        return done();
      case 'not-delivered':
        toasts.warn('Not delivered', 'No running session accepted the message.');
        return done();
      case 'sent':
        toasts.success(
          `${out.broadcast ? 'Broadcast to' : 'Sent to'} ${plural(out.count, 'session')}`,
          out.message,
        );
        return done();
      case 'unparsed':
        toasts.warn(
          "Couldn’t parse that",
          'Try e.g. "open 2 claude sessions", or enable AI fallback for free-form requests.',
        );
        return;
      case 'plan':
        plan = out.plan;
        optimizedText = out.optimizedText;
        // Move to the plan's heading so it is read ("Proposed plan…, N steps").
        await tick();
        planTitleEl?.focus();
        return;
      case 'executed': {
        if (out.fail === 0) toasts.success('Plan executed', `${plural(out.ok, 'action')} completed`);
        else toasts.warn('Plan partially executed', `${out.ok} ok, ${out.fail} failed`);
        await ws.refreshSessions();
        // Foreground the freshly-spawned sessions so they're immediately workable
        // (not left running in the background). Multiple → tile them all.
        const created = ws.sessions.filter((s) => !before.has(s.id) && !s.archived);
        if (created.length > 1) ws.setViewMode('tiled');
        // Open all sessions in the store; navigate the route to the last one so
        // Back/Forward can return to it. Store-only openSession for all but last.
        for (const s of created.slice(0, -1)) ws.openSession(s.id);
        if (created.length > 0) ws.navigateToSession(created[created.length - 1].id);
        plan = null;
        return done();
      }
    }
  }

  async function executePlan(): Promise<void> {
    if (!ws.currentId || !plan || busy) return;
    busy = true;
    const before = new Set(ws.sessions.map((s) => s.id));
    try {
      await applyOutcome(await runPlan(ws.currentId, plan), before);
    } catch (e) {
      toastError('Couldn’t run the plan', e);
    } finally {
      busy = false;
    }
  }
;
async () => {

if(ui.paletteOpen){
   { svelteHTML.createElement("div", {         "class":`pal-backdrop`,"role":`presentation`,"onpointerdown":(e) => {
      downOnBackdrop = e.target === e.currentTarget;
    },"onclick":(e) => {
      if (downOnBackdrop && e.target === e.currentTarget) close();
      downOnBackdrop = false;
    },});
     {const $$action_0 = __sveltets_2_ensureAction(dialogFocus(svelteHTML.mapElementTag('div'),(() => { if (plan) cancelPlan(); else close(); })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {         "class":`palette glass-raised`,"role":`dialog`,"aria-modal":`true`,"aria-label":`Command palette`,});
       { svelteHTML.createElement("div", { "class":`pal-mode-row`,});
         { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Palette mode`,});
           { svelteHTML.createElement("button", {     "aria-pressed":mode === 'commands',"onclick":() => mode !== 'commands' && toggleMode(),});mode === 'commands';
            
           }
           { svelteHTML.createElement("button", {     "aria-pressed":mode === 'english',"onclick":() => mode !== 'english' && toggleMode(),});mode === 'english';
             
           }
         }
         { svelteHTML.createElement("span", { "class":`pal-hint`,});   }
       }

      if(mode === 'commands'){
         { svelteHTML.createElement("div", { "class":`pal-input-row`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":14,}});}
           { const $$_input3 = svelteHTML.createElement("input", {                         "dir":`ltr`,"bind:value":query,"placeholder":`Type a command…`,"onkeydown":onCommandsKey,"spellcheck":`false`,"role":`combobox`,"aria-label":`Search commands`,"aria-expanded":`true`,"aria-controls":`pal-listbox`,"aria-autocomplete":`list`,"aria-activedescendant":totalRows > 0 ? `pal-opt-${selected}` : undefined,"aria-describedby":`pal-status`,});inputEl = $$_input3;/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
         { svelteHTML.createElement("div", {      "class":`pal-status`,"id":`pal-status`,"role":`status`,});!!searchError;if(searchError){statusText;  { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => scheduleSearch(query),});  }} else if (searchHits.length > 0 && !searchBusy){ { svelteHTML.createElement("span", { "class":`sr-only`,});statusText; }}else{statusText;} }
         { const $$_div2 = svelteHTML.createElement("div", {        "class":`pal-list`,"role":`listbox`,"id":`pal-listbox`,"aria-label":`Commands`,});listEl = $$_div2;
              for(let item of __sveltets_2_ensureArray(filtered)){let i = 1;item.cmd.id;
             { svelteHTML.createElement("button", {                "id":`pal-opt-${i}`,"tabindex":-1,"class":`pal-item`,"role":`option`,"aria-selected":i === selected,"onmouseenter":() => (selected = i),"onclick":() => run(item.cmd),});i === selected;
               { svelteHTML.createElement("span", {   "class":`pal-item-title`,"title":item.cmd.title.length > 48 ? item.cmd.title : undefined,});item.cmd.title; }
              if(item.cmd.detail){ { svelteHTML.createElement("span", { "class":`pal-detail`,});item.cmd.detail; }}
               { svelteHTML.createElement("span", { "class":`grow`,}); }
              if(item.cmd.group){ { svelteHTML.createElement("span", { "class":`pal-group`,});item.cmd.group; }}
              if(item.cmd.shortcut){ { svelteHTML.createElement("kbd", {});item.cmd.shortcut; }}
             }
          }
          if(askRow){
             { svelteHTML.createElement("button", {                "id":`pal-opt-${filtered.length}`,"tabindex":-1,"class":`pal-item pal-ask`,"role":`option`,"aria-selected":selected === filtered.length,"onmouseenter":() => (selected = filtered.length),"onclick":askOtto,});selected === filtered.length;
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":13,}});}
               { svelteHTML.createElement("span", {   "class":`pal-item-title`,"title":query,});  query;  }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("span", { "class":`pal-group`,});  }
             }
          }

          if(searchHits.length > 0){
           { svelteHTML.createElement("div", {   "role":`group`,"aria-label":`Results`,});
           { svelteHTML.createElement("div", {   "class":`pal-section-header`,"aria-hidden":`true`,});  }
              for(let hit of __sveltets_2_ensureArray(searchHits)){let h = 1;hit.kind + ':' + hit.id;
             { svelteHTML.createElement("div", {              "class":`pal-hit`,"id":`pal-opt-${hitBase + h}`,"role":`option`,"tabindex":-1,"aria-selected":selected === hitBase + h,"onmouseenter":() => (selected = hitBase + h),});selected === hitBase + h;
               { svelteHTML.createElement("div", { "class":`pal-hit-main`,});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":hitIcon(hit.kind),"size":12,}});}
                 { svelteHTML.createElement("span", {   "class":`pal-hit-title`,"title":hit.title,});hit.title; }
                if(hit.subtitle){
                   { svelteHTML.createElement("span", {   "class":`pal-hit-sub`,"title":hit.subtitle,});hit.subtitle; }
                }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("span", { "class":`pal-group pal-hit-kind`,});hit.kind.replace('_', ' '); }
               }
              
               { svelteHTML.createElement("div", {   "class":`pal-hit-actions`,"aria-hidden":`true`,});
                   for(let action of __sveltets_2_ensureArray(hitActions(hit))){action;
                   { svelteHTML.createElement("button", {         "type":`button`,"class":`pal-hit-btn`,"tabindex":-1,"onmousedown":(e) => e.preventDefault(),"onclick":() => void hitAction(hit, action),});HIT_ACTION_LABEL[action];selected === hitBase + h ? ` ${HIT_ACTION_KEY[action]}` : ''; }
                }
               }
             }
          }
           }
          }
         }
      }else{
         { svelteHTML.createElement("div", { "class":`pal-english`,});
           { const $$_textarea3 = svelteHTML.createElement("textarea", {               "dir":`auto`,"aria-label":`Describe what you want Otto to do`,"bind:value":englishText,"placeholder":`Describe what you want… e.g. 'spawn 3 claude agents and tell them to fix the failing tests'`,"onkeydown":onEnglishKey,"rows":3,"spellcheck":`false`,});textareaEl = $$_textarea3;/*Ωignore_startΩ*/() => englishText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           { svelteHTML.createElement("div", { "class":`pal-english-row`,});
             { svelteHTML.createElement("button", {          "class":`pill-toggle`,"aria-pressed":optimize,"title":`Rewrite your request into a clearer prompt before planning (remembered on this Mac)`,"onclick":() => setOptimize(!optimize),});optimize;
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}  
             }
             { svelteHTML.createElement("button", {          "class":`pill-toggle`,"aria-pressed":aiFallback,"title":`When the request isn’t a known command, ask the AI planner for a plan (you still confirm it)`,"onclick":() => setFallback(!aiFallback),});aiFallback;
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}   
             }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || englishText.trim() === '',"onclick":submitEnglish,});
              busy && !plan ? 'Planning…' : 'Plan it';
              if(!(busy && !plan)){ { svelteHTML.createElement("kbd", { "class":`pal-kbd-in`,});  }}
             }
           }

          if(optimizedText){
             { svelteHTML.createElement("div", { "class":`pal-optimized`,});
               { svelteHTML.createElement("span", { "class":`dim`,});  }
              optimizedText;
             }
          }

          if(plan){
             { svelteHTML.createElement("div", { "class":`pal-plan`,});
               { const $$_h34 = svelteHTML.createElement("h3", {    "class":`pal-plan-title`,"tabindex":-1,});planTitleEl = $$_h34;
                      { svelteHTML.createElement("span", { "class":`sr-only`,}); plural(plan.length, 'step'); }
               }
              if(plan.length > 0){
                 { svelteHTML.createElement("ol", { "class":`pal-plan-steps`,});
                      for(let a of __sveltets_2_ensureArray(plan)){let i = 1;i;
                     { svelteHTML.createElement("li", { "class":`pal-plan-item`,});describeAction(a); }
                  }
                 }
              }else{
                 { svelteHTML.createElement("div", { "class":`pal-empty`,});          }
              }
               { svelteHTML.createElement("div", { "class":`pal-plan-actions`,});
                 { svelteHTML.createElement("button", {   "class":`btn`,"onclick":cancelPlan,});  }
                 { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || plan.length === 0,"onclick":executePlan,});
                  busy ? 'Executing…' : 'Execute plan';
                 }
               }
             }
          }
         }
      }
     }}
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Palette__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Palette__SvelteComponent_ = ReturnType<typeof Palette__SvelteComponent_>;
/*Ωignore_endΩ*/export default Palette__SvelteComponent_;