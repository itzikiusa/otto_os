///<reference types="svelte" />
;
  import { SvelteMap } from 'svelte/reactivity';

  /** Image uploads still in flight, per session. Module-level on purpose: the
   *  composer is keyed by session and remounts when you switch tabs, but an
   *  upload started in A keeps running and lands in A's draft — so A's Send
   *  must stay blocked after a remount until it finishes. */
  const pendingUploads = new SvelteMap<string, number>();
  function addPending(owner: string, n: number): void {
    const next = (pendingUploads.get(owner) ?? 0) + n;
    if (next > 0) pendingUploads.set(owner, next);
    else pendingUploads.delete(owner);
  }
;;// Chat composer (sessionId mode, editor role, session not exited). One box
// that grows with the text (2 lines at rest, up to ~40% of the window) with
// its tools on a row inside it: attach image · the key hints · Stop (while
// the agent works: one Esc into its terminal, what "esc to interrupt" does)
// · Send. While the agent is busy a send is still accepted — Claude Code /
// Codex queue typed input and deliver it when the turn ends (the chat shows
// it as a "Queued" chip until then), so the box says so instead of blocking.
// ⏎ submits
// exactly the typed text as ONE prompt into the agent's PTY, ⇧⏎ inserts a
// newline, `/` passes through untouched (slash commands are the CLI's) — with
// a completion popup listing the provider's built-ins plus the user's own
// commands/skills (`GET …/slash-commands`). Pasted / dropped images upload
// to the session inbox and show as thumbnails under the box; on send each
// becomes an explicit `[Image: <path>]` line after the text. The status line
// shows the session status and any board tasks still waiting to be nudged in.
// OS text prediction / autocorrect is off here: the bubble it pops over the
// box is noise when you are typing paths, flags and slash commands.


import { untrack } from 'svelte';
import { toastError } from '../../../lib/toastError';
import { announce } from '../../../lib/announce.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { activity } from '../../../lib/stores/activity.svelte';
import type { SessionStatus, SlashCommand } from '../../../lib/api/types';
import { fetchSlashCommands, interruptAgent, submitPrompt, uploadInboxImage } from './api';
import { tick } from 'svelte';

import { transcript } from '../../../lib/stores/transcript.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { events } from '../../../lib/events.svelte';
import { sessionState } from '../../../lib/status';
import StatusDot from '../../../lib/components/StatusDot.svelte';
import Icon from '../../../lib/components/Icon.svelte';
;

  interface Props {
    sessionId: string;
    status: SessionStatus;
    /** Exited / reconnectable → Resume instead of a textarea. */
    onresume: () => void;
    /** Status row: where the agent runs, on which branch, which model, and
     *  whatever its own status line shows (context %, limits, mode …). */
    cwd?: string;
    branch?: string | null;
    model?: string | null;
    termStatus?: string;
    /** Unsent text sitting in the terminal's input box — a chat send is
     *  appended to it by the CLI, so it is shown as the message's prefix. */
    termInput?: string;
    /** "Claude" / "Codex" — the placeholder and hints name the agent. */
    agentName?: string;
  };;

  interface Attachment {
    path: string;
    name: string;
    /** Object URL of the local file — preview only, never sent. */
    url: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  
  
  
  
  
  

  let { sessionId, status, onresume, cwd = '', branch = null, model = null, termStatus = '', termInput = '', agentName = 'the agent' }: Props = $props();

  // The keyed parent creates one instance per session. Capture ownership once
  // so an upload or submit that finishes after navigation still updates A.
  const ownerId = untrack(() => sessionId);
  // Read the shared draft directly: an older pending send can finish after
  // this same session is reopened, so component-local copies would go stale.
  const text = $derived(transcript.draft(ownerId));
  const shortCwd = $derived(cwd.replace(/^\/Users\/[^/]+/, '~').replace(/^\/home\/[^/]+/, '~'));
  const sending = $derived(transcript.sending(ownerId));
  const uploading = $derived(pendingUploads.get(ownerId) ?? 0);
  // a11y-6-01: the upload edge is announced; the visible "Uploading…" is not a live region.
  const isUploading = $derived(uploading > 0);
  $effect(() => { if (isUploading) announce('Uploading images…'); });
  let ta = $state<HTMLTextAreaElement | null>(null);

  // A pending send/upload must not take the caret back from search, another
  // editor or a dialog opened in the meantime. The body case covers a composer
  // button losing focus when it becomes disabled during its request.
  function restoreComposerFocus(): void {
    const focused = document.activeElement;
    if (ta?.isConnected && (focused === document.body || composerEl?.contains(focused))) {
      ta.focus();
    }
  }

  const attachments = $derived(transcript.attachments(ownerId));

  // The shared session state (lib/status.ts): the same words the sidebar, tab
  // and pane header use — a resumable session is "Suspended", not "ended".
  const st = $derived(
    sessionState(
      ws.getSession(sessionId),
      status,
      ws.needsYou[sessionId] === true,
      { stale: events.state !== 'connected' },
    ),
  );
  const exited = $derived(st.inactive);
  const pendingNudges = $derived(activity.tasks(sessionId).filter((t) => t.nudge_pending).length);
  const statusLabel = $derived(st.key === 'working' ? 'Working…' : st.label);

  // Two lines at rest (CSS min-height; one in a narrow tile), growing with the
  // text up to ~40% of the window; the textarea itself never scrolls sideways (wrap +
  // overflow-x hidden), so nothing overlays the placeholder.
  // The cap is the smaller of 40% of the window and half of the PANE the composer
  // sits in (a short tile or split pane), so the box never crowds out the chat;
  // past it the textarea scrolls inside.
  // Half the pane is not enough on its own: in a short tile the composer's own
  // rows (attachments, toolbar) and the pane's fixed rows (header, status) eat
  // the rest, so the cap also reserves MIN_CHAT px for the conversation itself.
  let paneH = $state(0);
  /** Conversation viewport kept visible however tall the draft gets (~5 lines). */
  const MIN_CHAT = 96;
  /** `paneBudget()`, cached. Measuring walks every pane child (computed style
   *  + offsetHeight), which on a keystroke forces a full layout of the chat —
   *  so it's refreshed only when the pane resizes or the composer's own rows
   *  change (attachments), never per keystroke. Plain `let`: nothing renders it. */
  let budget = Infinity;
  function measureBudget(): void {
    budget = paneBudget();
  }
  function paneBudget(): number {
    const pane = composerEl?.parentElement;
    if (!pane || !composerEl || !ta) return Infinity;
    let fixed = 0;
    for (const el of Array.from(pane.children) as HTMLElement[]) {
      if (el === composerEl || parseFloat(getComputedStyle(el).flexGrow) > 0) continue;
      fixed += el.offsetHeight;
    }
    const chrome = composerEl.offsetHeight - ta.offsetHeight;
    return pane.clientHeight - fixed - chrome - MIN_CHAT;
  }
  /** One forced layout: reset → read scrollHeight → set. */
  function autosize(): void {
    if (!ta) return;
    ta.style.height = 'auto';
    const byWindow = Math.floor(window.innerHeight * 0.4);
    const byPane = paneH > 0 ? Math.floor(paneH * 0.5) : byWindow;
    // One line is the floor when the pane is too short for the budget.
    const cap = Math.max(24, Math.min(byWindow, byPane, Math.floor(budget)));
    ta.style.height = `${Math.min(cap, Math.max(ta.scrollHeight, 0))}px`;
  }
  let composerEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const pane = composerEl?.parentElement;
    if (!pane || typeof ResizeObserver === 'undefined') return;
    const measure = (): void => {
      const h = pane.clientHeight;
      if (Math.abs(h - paneH) > 1) {
        paneH = h;
        measureBudget();
        autosize();
      }
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(pane);
    return () => ro.disconnect();
  });
  // The composer's own optional rows (the attachment strip, the "In terminal"
  // line) eat into the pane budget: re-measure when they appear or go.
  $effect(() => {
    void attachments.length;
    void !!termInput;
    void tick().then(() => {
      measureBudget();
      autosize();
    });
  });
  // The ONE per-keystroke resize: typing updates the bound draft, which re-runs
  // this (there is no `oninput` resize too); a draft restored on mount, a
  // reopened session or an accepted /command lands here the same way.
  $effect(() => {
    void text;
    void tick().then(autosize);
  });

  // ---- busy: queue + interrupt -------------------------------------------------
  const busy = $derived(status === 'working');
  let stopping = $state(false);
  async function interrupt(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await interruptAgent(ownerId);
      toasts.info('Interrupt sent', `${agentName} stops after its current step`);
    } catch (e) {
      toastError('Couldn’t interrupt the agent', e);
    } finally {
      stopping = false;
      restoreComposerFocus();
    }
  }
  let fileEl = $state<HTMLInputElement | null>(null);
  function pickImages(e: Event): void {
    const input = e.currentTarget as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    input.value = '';
    void addImages(files);
  }

  // ---- slash-command completion ------------------------------------------------
  let cmds = $state<SlashCommand[] | null>(null);
  let cmdsFor = '';
  let cmdIdx = $state(0);
  let cmdDismissed = $state(false);
  let listEl = $state<HTMLDivElement | null>(null);
  /** `/pre` on a single line → the prefix being completed, else null. */
  const cmdPrefix = $derived.by(() => {
    const m = /^\/([\w:-]*)$/.exec(text);
    return m ? m[1].toLowerCase() : null;
  });
  const suggestions = $derived.by(() => {
    if (cmdPrefix == null || cmdDismissed || !cmds) return [] as SlashCommand[];
    const starts = cmds.filter((c) => c.name.toLowerCase().startsWith(cmdPrefix));
    const within = cmds.filter((c) => !c.name.toLowerCase().startsWith(cmdPrefix) && c.name.toLowerCase().includes(cmdPrefix));
    return [...starts, ...within].slice(0, 12);
  });
  const cmdOpen = $derived(suggestions.length > 0);
  // The slash popup is a listbox driven from the textarea (aria-activedescendant):
  // focus never leaves the composer, ↑/↓ move the active option.
  const popId = $props.id();
  $effect(() => {
    // Load once per session, the first time a `/` is typed at the start.
    if (cmdPrefix == null || cmdsFor === sessionId) return;
    cmdsFor = sessionId;
    void fetchSlashCommands(sessionId)
      .then((list) => (cmds = list))
      .catch(() => (cmds = []));
  });
  $effect(() => {
    void suggestions.length;
    cmdIdx = 0;
  });
  $effect(() => {
    // Typing again after Esc re-opens the list.
    void text;
    cmdDismissed = false;
  });
  $effect(() => {
    const el = listEl?.children[cmdIdx] as HTMLElement | undefined;
    el?.scrollIntoView({ block: 'nearest' });
  });
  // The popup opens ABOVE the box: cap its height to the room between the box and
  // the top of the pane (or the window), so it never runs off the top edge.
  let wrapEl = $state<HTMLDivElement | null>(null);
  let popMax = $state(320);
  function measurePopMax(): void {
    if (!cmdOpen || !wrapEl) return;
    const paneTop = Math.max(0, composerEl?.parentElement?.getBoundingClientRect().top ?? 0);
    const room = wrapEl.getBoundingClientRect().top - paneTop - 12;
    // Honour the real room even when it is tiny (a short split + attachments):
    // a taller floor would push the list past the pane's clip. It scrolls.
    popMax = Math.floor(Math.max(0, Math.min(320, window.innerHeight * 0.5, room)));
  }
  $effect(() => {
    void suggestions.length;
    void attachments.length;
    void paneH;
    if (!cmdOpen || !wrapEl) return;
    measurePopMax();
    window.addEventListener('resize', measurePopMax);
    return () => window.removeEventListener('resize', measurePopMax);
  });
  function acceptCmd(c: SlashCommand): void {
    transcript.setDraft(ownerId, `/${c.name} `);
    cmdDismissed = true;
    queueMicrotask(() => {
      autosize();
      ta?.focus();
      ta?.setSelectionRange(text.length, text.length);
    });
  }

  async function send(): Promise<void> {
    const typed = text.replace(/\s+$/, '');
    const imgs = attachments.map((a) => `[Image: ${a.path}]`);
    const body = [typed, ...imgs].filter(Boolean).join('\n');
    // An upload still in flight would land in the NEXT draft: wait for it, send by hand.
    if (!body || uploading > 0 || !transcript.tryBeginSend(ownerId)) return;
    try {
      const submittedImages = [...attachments];
      await submitPrompt(ownerId, body);
      // Do not erase new text or files added while this send was in flight.
      if (transcript.draft(ownerId).replace(/\s+$/, '') === typed) {
        transcript.setDraft(ownerId, '');
      }
      for (const a of submittedImages) URL.revokeObjectURL(a.url);
      transcript.setAttachments(ownerId, transcript.attachments(ownerId).filter((a) => !submittedImages.includes(a)));
      queueMicrotask(autosize);
    } catch (e) {
      toastError('Couldn’t send the message', e);
    } finally {
      transcript.finishSend(ownerId);
      restoreComposerFocus();
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.isComposing) return;
    if (cmdOpen) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        cmdIdx = (cmdIdx + 1) % suggestions.length;
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        cmdIdx = (cmdIdx - 1 + suggestions.length) % suggestions.length;
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        cmdDismissed = true;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        const pick = suggestions[cmdIdx];
        // ⏎ on an exact, fully-typed name sends it; anything else completes.
        if (e.key === 'Enter' && pick && `/${pick.name}`.toLowerCase() === text.trim().toLowerCase()) {
          e.preventDefault();
          void send();
          return;
        }
        e.preventDefault();
        if (pick) acceptCmd(pick);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }
  const sendTitle = $derived(
    uploading > 0
      ? 'Uploading images… send is available when they finish'
      : busy
        ? `Send (⏎) — ${agentName} is busy, so it is queued and delivered when the current turn ends`
        : 'Send (⏎)',
  );

  async function addImages(files: File[]): Promise<void> {
    const imgs = files.filter((f) => f.type.startsWith('image/'));
    if (!imgs.length) return;
    addPending(ownerId, imgs.length);
    for (const f of imgs) {
      try {
        const name = f.name && f.name !== 'image.png' ? f.name : `paste-${Date.now()}.${(f.type.split('/')[1] ?? 'png').replace('jpeg', 'jpg')}`;
        const path = await uploadInboxImage(ownerId, f, name);
        transcript.setAttachments(ownerId, [...transcript.attachments(ownerId), { path, name, url: URL.createObjectURL(f) }]);
      } catch (e) {
        toastError('Couldn’t upload the image', e);
      } finally {
        addPending(ownerId, -1);
      }
    }
    restoreComposerFocus();
  }

  function removeAttachment(a: Attachment): void {
    URL.revokeObjectURL(a.url);
    transcript.setAttachments(ownerId, attachments.filter((x) => x !== a));
  }

  function onPaste(e: ClipboardEvent): void {
    const files = Array.from(e.clipboardData?.files ?? []).filter((f) => f.type.startsWith('image/'));
    if (!files.length) return;
    e.preventDefault();
    void addImages(files);
  }

  function onDrop(e: DragEvent): void {
    const files = Array.from(e.dataTransfer?.files ?? []);
    if (!files.some((f) => f.type.startsWith('image/'))) return;
    e.preventDefault();
    void addImages(files);
  }

  const canSend = $derived(!sending && uploading === 0 && (text.trim().length > 0 || attachments.length > 0));
;
async () => {



 { const $$_div0 = svelteHTML.createElement("div", {            "class":`composer`,...__sveltets_2_empty({"data-status":status}),"ondragover":(e) => e.preventDefault(),"ondrop":onDrop,"role":`group`,"aria-label":`Message composer`,});composerEl = $$_div0;
  if(exited){
     { svelteHTML.createElement("div", { "class":`exited`,});
       { svelteHTML.createElement("span", { "class":`dim`,});st.key === 'suspended' ? `${st.hint}.` : 'This session has ended.'; }
       { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":onresume,});  }
     }
  }else{
     { const $$_div1 = svelteHTML.createElement("div", {  "class":`box-wrap`,});wrapEl = $$_div1;
      if(cmdOpen){
         { const $$_div2 = svelteHTML.createElement("div", {           "class":`cmd-pop`,"id":`${popId}-list`,"style":`--pop-max:${popMax}px`,"role":`listbox`,"aria-label":`Slash commands`,...__sveltets_2_empty({"data-slash-pop":true}),});listEl = $$_div2;
              for(let c of __sveltets_2_ensureArray(suggestions)){let i = 1;c.name;
            
             { svelteHTML.createElement("div", {                "id":`${popId}-opt-${i}`,"class":`cmd-row`,"role":`option`,"tabindex":-1,"aria-selected":i === cmdIdx,"onmousedown":(e) => { e.preventDefault(); acceptCmd(c); },"onmouseenter":() => (cmdIdx = i),});i === cmdIdx;
               { svelteHTML.createElement("span", { "class":`cmd-name mono`,}); c.name; }
               { svelteHTML.createElement("span", { "class":`cmd-desc dim`,});c.description; }
               { svelteHTML.createElement("span", { "class":`cmd-src dim`,});c.source === 'builtin' ? '' : c.source; }
             }
          }
           { svelteHTML.createElement("div", { "class":`cmd-hint dim`,});        }
         }
      }
       { svelteHTML.createElement("div", {  "class":`box`,});busy;
         { const $$_textarea3 = svelteHTML.createElement("textarea", {                              "bind:value":__sveltets_2_get_set_binding(() => text,(value) => transcript.setDraft(ownerId, value)),"rows":1,"placeholder":busy ? `Queue a message for ${agentName}…` : `Message ${agentName} — / for commands`,"aria-label":`Message ${agentName}`,"spellcheck":`false`,...{ autocorrect: 'off' },"autocapitalize":`off`,"autocomplete":`off`,"dir":`auto`,"onkeydown":onKeydown,"onpaste":onPaste,"aria-autocomplete":`list`,"aria-controls":cmdOpen ? `${popId}-list` : undefined,"aria-activedescendant":cmdOpen ? `${popId}-opt-${cmdIdx}` : undefined,});ta = $$_textarea3; }
         { svelteHTML.createElement("div", { "class":`tools`,});
           { const $$_input4 = svelteHTML.createElement("input", {        "type":`file`,"accept":`image/*`,"multiple":true,"hidden":true,"onchange":pickImages,});fileEl = $$_input4;}
           { svelteHTML.createElement("button", {       "class":`icon-btn tool`,"onclick":() => fileEl?.click(),"aria-label":`Attach images`,"title":`Attach images (or paste / drop them)`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":14,}});} }
           { svelteHTML.createElement("span", {   "class":`keys`,"aria-hidden":`true`,});busy ? 'Busy — ⏎ queues' : '⏎ send';     }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
          if(busy){
             { svelteHTML.createElement("button", {       "class":`btn small stop`,"onclick":() => void interrupt(),"disabled":stopping,"title":`Interrupt ${agentName} — sends Esc to its terminal`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}  }
          }
          if(uploading > 0){ { svelteHTML.createElement("span", { "class":`uploading`,});  }}
           { svelteHTML.createElement("button", {         "class":`send`,"onclick":() => void send(),"disabled":!canSend,"title":sendTitle,"aria-label":uploading > 0 ? 'Send (uploading images)' : 'Send',}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":14,}});} }
         }
       }
      if(attachments.length){
         { svelteHTML.createElement("div", {   "class":`thumbs`,...__sveltets_2_empty({"data-attachments":attachments.length}),});
             for(let a of __sveltets_2_ensureArray(attachments)){a.path;
             { svelteHTML.createElement("div", {   "class":`thumb`,"title":a.path,});
               { svelteHTML.createElement("img", {    "src":a.url,"alt":a.name,});}
               { svelteHTML.createElement("button", {       "class":`thumb-x`,"onclick":() => removeAttachment(a),"title":`Remove`,"aria-label":`Remove ${a.name}`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
             }
          }
         }
      }
     }
    if(termInput){
       { svelteHTML.createElement("div", {   "class":`term-input`,...__sveltets_2_empty({"data-term-input":true}),"title":`Typed in the terminal but not sent. Sending from here appends your text after it — the CLI submits both as one message.`,});
         { svelteHTML.createElement("span", { "class":`dim`,});  }  { svelteHTML.createElement("span", { "class":`mono`,});termInput; }
       }
    }
     { svelteHTML.createElement("div", {  "class":`status`,...__sveltets_2_empty({"data-status-line":true}),});
       { const $$_toDsutatS2C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS2C({ target: __sveltets_2_any(), props: {  "state":st,}});}
       { svelteHTML.createElement("span", {});statusLabel; }
      if(uploading){ { svelteHTML.createElement("span", { "class":`dim`,});  uploading; uploading > 1 ? 's' : '';  }}
      if(pendingNudges){
         { svelteHTML.createElement("span", {   "class":`dim`,"title":`Board tasks waiting for the agent to go idle`,}); pendingNudges;  pendingNudges > 1 ? 's' : '';  }
      }
      if(shortCwd){ { svelteHTML.createElement("span", { "class":`sep sep-cwd`,});  } { svelteHTML.createElement("span", {   "class":`mono cwd`,"title":cwd,});shortCwd; }}
      if(branch){ { svelteHTML.createElement("span", { "class":`sep sep-branch`,});  } { svelteHTML.createElement("span", {   "class":`mono branch`,"title":`Git branch`,}); branch; }}
      if(model){ { svelteHTML.createElement("span", { "class":`sep sep-model`,});  } { svelteHTML.createElement("span", {   "class":`mono model`,"title":`Model`,});model; }}
      if(termStatus){ { svelteHTML.createElement("span", { "class":`sep`,});  } { svelteHTML.createElement("span", {   "class":`term-status`,"title":`The agent’s own status line`,});termStatus; }}
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Composer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Composer__SvelteComponent_ = ReturnType<typeof Composer__SvelteComponent_>;
/*Ωignore_endΩ*/export default Composer__SvelteComponent_;