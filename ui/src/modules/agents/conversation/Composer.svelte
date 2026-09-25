<script lang="ts">
  // Chat composer (sessionId mode, editor role, session not exited): ⏎ submits
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
  import { toasts } from '../../../lib/toast.svelte';
  import { activity } from '../../../lib/stores/activity.svelte';
  import type { SessionStatus, SlashCommand } from '../../../lib/api/types';
  import { fetchSlashCommands, submitPrompt, uploadInboxImage } from './api';

  import { transcript } from '../../../lib/stores/transcript.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { events } from '../../../lib/events.svelte';
  import { sessionState } from '../../../lib/status';
  import StatusDot from '../../../lib/components/StatusDot.svelte';
  import Icon from '../../../lib/components/Icon.svelte';

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
  }
  let { sessionId, status, onresume, cwd = '', branch = null, model = null, termStatus = '', termInput = '' }: Props = $props();

  // The keyed parent creates one instance per session. Capture ownership once
  // so an upload or submit that finishes after navigation still updates A.
  const ownerId = untrack(() => sessionId);
  // Read the shared draft directly: an older pending send can finish after
  // this same session is reopened, so component-local copies would go stale.
  const text = $derived(transcript.draft(ownerId));
  const shortCwd = $derived(cwd.replace(/^\/Users\/[^/]+/, '~').replace(/^\/home\/[^/]+/, '~'));
  const sending = $derived(transcript.sending(ownerId));
  let uploading = $state(0);
  let ta = $state<HTMLTextAreaElement | null>(null);

  interface Attachment {
    path: string;
    name: string;
    /** Object URL of the local file — preview only, never sent. */
    url: string;
  }
  const attachments = $derived(transcript.attachments(ownerId));

  // The shared session state (lib/status.ts): the same words the sidebar, tab
  // and pane header use — a resumable session is "Suspended", not "ended".
  const st = $derived(
    sessionState(
      ws.sessions.find((s) => s.id === sessionId),
      status,
      ws.needsYou[sessionId] === true,
      { stale: events.state !== 'connected' },
    ),
  );
  const exited = $derived(st.inactive);
  const pendingNudges = $derived(activity.tasks(sessionId).filter((t) => t.nudge_pending).length);
  const statusLabel = $derived(st.key === 'working' ? 'Working…' : st.label);

  // Three lines by default (rows=3 + line-height), growing with the text up
  // to ~40% of the window; the textarea itself never scrolls sideways (wrap +
  // overflow-x hidden), so nothing overlays the placeholder.
  function autosize(): void {
    if (!ta) return;
    ta.style.height = 'auto';
    const cap = Math.max(160, Math.floor(window.innerHeight * 0.4));
    ta.style.height = `${Math.min(cap, Math.max(ta.scrollHeight, 0))}px`;
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
    if (!body || !transcript.tryBeginSend(ownerId)) return;
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
      toasts.error('Send failed', e instanceof Error ? e.message : String(e));
    } finally {
      transcript.finishSend(ownerId);
      ta?.focus();
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

  async function addImages(files: File[]): Promise<void> {
    const imgs = files.filter((f) => f.type.startsWith('image/'));
    if (!imgs.length) return;
    uploading += imgs.length;
    for (const f of imgs) {
      try {
        const name = f.name && f.name !== 'image.png' ? f.name : `paste-${Date.now()}.${(f.type.split('/')[1] ?? 'png').replace('jpeg', 'jpg')}`;
        const path = await uploadInboxImage(ownerId, f, name);
        transcript.setAttachments(ownerId, [...transcript.attachments(ownerId), { path, name, url: URL.createObjectURL(f) }]);
      } catch (e) {
        toasts.error('Image upload failed', e instanceof Error ? e.message : String(e));
      } finally {
        uploading -= 1;
      }
    }
    ta?.focus();
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

  const canSend = $derived(!sending && (text.trim().length > 0 || attachments.length > 0));
</script>

<div class="composer" data-status={status} ondragover={(e) => e.preventDefault()} ondrop={onDrop} role="group" aria-label="Message composer">
  {#if exited}
    <div class="exited">
      <span class="dim">{st.key === 'suspended' ? `${st.hint}.` : 'This session has ended.'}</span>
      <button class="btn small primary" onclick={onresume}>Resume</button>
    </div>
  {:else}
    <div class="box-wrap">
      {#if cmdOpen}
        <div class="cmd-pop" bind:this={listEl} role="listbox" aria-label="Slash commands" data-slash-pop>
          {#each suggestions as c, i (c.name)}
            <!-- svelte-ignore a11y_click_events_have_key_events a11y_interactive_supports_focus -->
            <div
              class="cmd-row"
              class:active={i === cmdIdx}
              role="option"
              tabindex="-1"
              aria-selected={i === cmdIdx}
              onmousedown={(e) => e.preventDefault()}
              onclick={() => acceptCmd(c)}
              onmouseenter={() => (cmdIdx = i)}
            >
              <span class="cmd-name mono">/{c.name}</span>
              <span class="cmd-desc dim">{c.description}</span>
              <span class="cmd-src dim">{c.source === 'builtin' ? '' : c.source}</span>
            </div>
          {/each}
          <div class="cmd-hint dim">↑↓ choose · Tab/⏎ complete · Esc dismiss</div>
        </div>
      {/if}
      <div class="box">
        <textarea
          bind:this={ta}
          bind:value={() => text, (value) => transcript.setDraft(ownerId, value)}
          rows="3"
          placeholder="Message the agent — ⏎ to send, ⇧⏎ for a newline, / for commands, paste an image to attach"
          spellcheck="false"
          {...{ autocorrect: 'off' }}
          autocapitalize="off"
          autocomplete="off"
          dir="auto"
          oninput={autosize}
          onkeydown={onKeydown}
          onpaste={onPaste}
          aria-autocomplete="list"
        ></textarea>
        <button class="send icon-btn" onclick={() => void send()} disabled={!canSend} title="Send (⏎)" aria-label="Send"><Icon name="send" size={14} /></button>
      </div>
      {#if attachments.length}
        <div class="thumbs" data-attachments={attachments.length}>
          {#each attachments as a (a.path)}
            <div class="thumb" title={a.path}>
              <img src={a.url} alt={a.name} />
              <button class="thumb-x" onclick={() => removeAttachment(a)} title="Remove" aria-label="Remove {a.name}"><Icon name="x" size={12} /></button>
            </div>
          {/each}
        </div>
      {/if}
    </div>
    {#if termInput}
      <div class="term-input" data-term-input title="Typed in the terminal but not sent. Sending from here appends your text after it — the CLI submits both as one message.">
        <span class="dim">In terminal:</span> <span class="mono">{termInput}</span>
      </div>
    {/if}
    <div class="status" data-status-line>
      <StatusDot state={st} />
      <span>{statusLabel}</span>
      {#if uploading}<span class="dim">· uploading {uploading} image{uploading > 1 ? 's' : ''}…</span>{/if}
      {#if pendingNudges}
        <span class="dim" title="Board tasks waiting for the agent to go idle">· {pendingNudges} board task{pendingNudges > 1 ? 's' : ''} pending</span>
      {/if}
      {#if shortCwd}<span class="sep sep-cwd">·</span><span class="mono cwd" title={cwd}>{shortCwd}</span>{/if}
      {#if branch}<span class="sep sep-branch">·</span><span class="mono branch" title="Git branch">⎇ {branch}</span>{/if}
      {#if model}<span class="sep sep-model">·</span><span class="mono model" title="Model">{model}</span>{/if}
      {#if termStatus}<span class="sep">·</span><span class="term-status" title="The agent's own status line">{termStatus}</span>{/if}
      <span class="grow"></span>
      <span class="dim hint">Slash commands pass straight to the CLI</span>
    </div>
  {/if}
</div>

<style>
  .composer {
    border-top: 1px solid var(--border);
    background: var(--surface);
    padding: 8px 12px 6px;
    flex-shrink: 0;
    /* Shed the status line's secondary spans by the COMPOSER's width — it sits
       in a split pane as readily as a full-window chat. Same numbers as the
       `@container` blocks below. */
    container-type: inline-size;
  }
  .box-wrap {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .box {
    display: flex;
    align-items: flex-end;
    gap: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    padding: 8px 6px 8px 12px;
    overflow: hidden;
    min-width: 0;
  }
  .box:focus-within {
    border-color: color-mix(in srgb, var(--accent) 60%, var(--border));
  }
  textarea {
    flex: 1;
    min-width: 0;
    resize: none;
    border: 0;
    outline: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.5;
    min-height: 62px;
    max-height: 40vh;
    padding: 2px 0;
    overflow-x: hidden;
    overflow-y: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    scrollbar-gutter: stable;
  }
  textarea::placeholder {
    color: var(--text-dim);
  }
  .send {
    flex-shrink: 0;
    color: var(--accent-text);
  }
  .send:disabled {
    opacity: 0.4;
    cursor: default;
  }
  :global([dir='rtl']) .send {
    transform: scaleX(-1);
  }
  /* Completion popup: anchored above the box, clamped to the pane, scrolls. */
  .cmd-pop {
    position: absolute;
    bottom: calc(100% + 6px);
    inset-inline-start: 0;
    width: min(100%, 560px);
    max-width: 100%;
    max-height: min(320px, 50vh);
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.18);
    padding: 4px;
    z-index: 5;
  }
  .cmd-row {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: baseline;
    gap: 10px;
    padding: 5px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .cmd-row.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .cmd-name {
    color: var(--accent-text);
    white-space: nowrap;
  }
  .cmd-desc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    font-size: var(--fs-xs);
  }
  .cmd-src {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .cmd-hint {
    font-size: var(--fs-xs);
    padding: 4px 8px 2px;
    border-top: 1px solid var(--border);
    margin-top: 2px;
  }
  .thumbs {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .thumb {
    position: relative;
    width: 84px;
    height: 84px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--surface-2);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .thumb-x {
    position: absolute;
    top: 3px;
    inset-inline-end: 3px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    border: 0;
    background: rgba(0, 0, 0, 0.6);
    color: #fff; /* on the fixed dark scrim over the image, in every theme */
    display: grid;
    place-items: center;
    padding: 0;
    cursor: pointer;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 5px 2px 0;
    min-width: 0;
    flex-wrap: wrap;
    row-gap: 2px;
  }
  .sep {
    opacity: 0.6;
  }
  .cwd,
  .branch {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
  }
  .branch {
    color: var(--accent-text);
  }
  .term-status {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    flex: 0 1 auto;
    max-width: 48%;
  }
  .term-input {
    margin-top: 6px;
    font-size: var(--fs-xs);
    padding: 4px 10px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--status-warn) 12%, var(--surface));
    border: 1px dashed color-mix(in srgb, var(--status-warn) 50%, var(--border));
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hint {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .exited {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 6px 0;
    font-size: var(--fs-s);
  }
  /* ≤420px: the slash-command hint is the first thing to go (it is a one-off
     tip, not state). Replaces the old window media query — a wide window with a
     narrow pane used to keep it and push the status line into a second row. */
  @container (max-width: 420px) {
    .hint {
      display: none;
    }
  }
  /* ≤320px: cwd, branch and model go too — all three are visible in the pane
     header or the chat header, so nothing becomes unreachable. */
  @container (max-width: 320px) {
    .cwd,
    .branch,
    .model,
    .sep-cwd,
    .sep-branch,
    .sep-model {
      display: none;
    }
  }
</style>
