<script lang="ts">
  // A Threads conversation on the structured agent runtime: the timeline
  // (your messages, the agent's streaming answer, a quiet work log of
  // reasoning / commands / edits / tools with approvals inline) over one
  // composer whose send turns into Stop while a turn runs; a message sent
  // meanwhile is queued and goes out when the turn ends.
  // Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/MessagesTimeline.tsx + ThreadTimeline.tsx
  // (messages with work-log groups between them, follow the bottom),
  // ComposerPrimaryActions.tsx (send ↔ stop), QueuedRunsControl.tsx (one
  // queued follow-up), ThreadStatusLine.tsx ("Working for 12s"),
  // ThreadErrorBanner.tsx, and apps/web/src/session-logic.ts (`derivePhase`).
  // The composer's model / effort pick applies to the next turn and the ones
  // after it (ComposerModelEffort.tsx → the turn's model selection); pasted or
  // attached images go with the message (composerAttachmentFiles.ts).
  // Slash commands as in composerSlashCommandSearch.ts: the client's own
  // commands act on the view; any other `/command` goes to the agent as typed.
  // The access chip beside the model is T3's runtime mode (runtimeModeConfig.ts:
  // labels, icons, descriptions); it rides the next prompt and sticks. PDFs go
  // as file chips to the providers that take them (Claude).
  import { tick, untrack } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import { ApiError } from '../../../lib/api/client';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { harnessLabel } from '../../../lib/providers';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { formatEffortShort, formatModelLabel, supportsEffortSelection, type EffortLevel } from '../../../lib/providerModels';
  import { auth } from '../../../lib/stores/auth.svelte';
  import type { PromptImage, RuntimeAccessMode } from '../../../lib/api/types';
  import {
    INTERACTIVE_SLASH_COMMANDS,
    isImmediateSlashCommand,
    parseInteractiveSlashInput,
    type ComposerSlashCommand,
    type ParsedSlashIntent,
  } from '../../../lib/slashActions';
  import StageSlashPopup from '../StageSlashPopup.svelte';
  import { threads } from '../threadsStore.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import { toastError } from '../../../lib/toastError';
  import type { ApprovalDecision } from '../../../lib/api/types';
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { approvalOutcome, isRunning, latestTurn, pendingApprovals, timelineRows, type WorkEntry } from './timeline';
  import RuntimeAccessChip from './RuntimeAccessChip.svelte';
  import RuntimeApprovalRow from './RuntimeApprovalRow.svelte';
  import RuntimeAttachments from './RuntimeAttachments.svelte';
  import RuntimeLinkedThreads from './RuntimeLinkedThreads.svelte';
  import RuntimeUiControl from './RuntimeUiControl.svelte';
  import RuntimeWorkGroup from './RuntimeWorkGroup.svelte';
  import RuntimeWorkRow from './RuntimeWorkRow.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';

  interface Props {
    sessionId: string;
    focused: boolean;
    /** Hosted outside Threads (Personal Agents' chat): a linked-thread chip switches the host's view. */
    onswitch?: (id: string) => void;
  }
  let { sessionId, focused, onswitch }: Props = $props();

  $effect(() => {
    const id = sessionId;
    return untrack(() => runtimeViews.open(id));
  });

  const session = $derived(runtimeThreads.get(sessionId));
  const p = $derived(runtimeViews.projections[sessionId] ?? null);
  const loadError = $derived(runtimeViews.loadError[sessionId] ?? null);
  const rows = $derived(p ? timelineRows(p.items) : []);
  const approvals = $derived(new Map((p ? pendingApprovals(p) : []).map((a) => [a.requestId, a] as const)));
  const requests = $derived(new Map((p?.requests ?? []).map((r) => [r.id, r] as const)));
  const turn = $derived(p ? latestTurn(p) : null);
  const closed = $derived(session?.status === 'closed');
  /** The agent couldn't start (a refused handshake answers the prompt 400;
   *  the session goes dormant with `last_error`) — shown inline, T3 Code's
   *  ThreadErrorBanner, until the next prompt starts it. */
  let startError = $state<string | null>(null);
  const sessionError = $derived(
    startError ?? (p?.session && (p.session.status === 'stopped' || p.session.status === 'error') ? p.session.last_error : null),
  );
  const plan = $derived.by(() => {
    if (!p || !turn) return null;
    const pl = p.plans.findLast((x) => x.turn_id === turn.id);
    return pl && pl.steps.some((s) => s.status !== 'completed') ? pl : null;
  });

  let text = $state('');
  let queued = $state<string | null>(null);
  let sending = $state(false);
  /** The accepted prompt's first sequence, until the projection reaches it. */
  let awaitSeq = $state<number | null>(null);
  let stopping = $state(false);
  let scroller = $state<HTMLDivElement | null>(null);
  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let atBottom = true;

  const running = $derived((p ? isRunning(p) : false) || sending || (awaitSeq !== null && (p?.lastSequence ?? 0) < awaitSeq));
  const workingFor = $derived.by(() => {
    if (!running || !turn?.started_at || turn.status !== 'running') return null;
    const s = Math.max(0, Math.floor((now() - Date.parse(turn.started_at)) / 1000));
    return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
  });

  // ── Model / effort for the next turn ──────────────────────────────────
  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));
  let model = $state<string | null>(null);
  let effort = $state<EffortLevel | null>(null);
  let pickerOpen = $state(false);
  let agentEl = $state<HTMLButtonElement | null>(null);
  const provider = $derived(session?.provider ?? '');
  /** "Claude", for "Withdrawn by Claude". */
  const agentName = $derived(harnessLabel(provider).replace(/ Code$/, ''));
  const agentLabel = $derived.by(() => {
    const m = model ?? session?.model ?? null;
    const label = m ? formatModelLabel(provider, m, auth.meta) : harnessLabel(provider);
    return effort ? `${label} · ${formatEffortShort(effort)}` : label;
  });

  // ── Access mode for the next turn: sticky per thread ────────────────
  let pickedMode = $state<RuntimeAccessMode | null>(null);
  const accessMode = $derived(pickedMode ?? session?.access_mode ?? 'approval-required');

  // ── Images and PDFs (paste or attach; sent with the next message) ─────
  const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
  // The drivers that take image input (Claude content blocks, Codex image
  // items); PDFs go to Claude only (document blocks).
  const canImages = $derived(provider === 'claude' || provider === 'codex');
  const canFiles = $derived(provider === 'claude');
  const takes = (f: File): boolean => f.type.startsWith('image/') || (canFiles && f.type === 'application/pdf');
  let images = $state<(PromptImage & { name: string })[]>([]);
  let fileEl = $state<HTMLInputElement | null>(null);
  async function addImages(files: Iterable<File>): Promise<void> {
    for (const f of files) {
      if (!takes(f)) continue;
      if (f.size > MAX_IMAGE_BYTES) {
        toastError('File too large', new Error(`${f.name} is over 10 MB`));
        continue;
      }
      const data = new Uint8Array(await f.arrayBuffer());
      let bin = '';
      for (let i = 0; i < data.length; i += 0x8000) bin += String.fromCharCode(...data.subarray(i, i + 0x8000));
      images = [...images, { name: f.name || 'pasted image', mime: f.type, data_base64: btoa(bin) }];
    }
  }
  function onPaste(e: ClipboardEvent): void {
    const files = [...(e.clipboardData?.files ?? [])].filter(takes);
    if (!canImages || files.length === 0) return;
    e.preventDefault();
    void addImages(files);
  }

  async function send(t: string): Promise<void> {
    sending = true;
    const sentImages = images;
    images = [];
    const pics = sentImages.filter((f) => f.mime.startsWith('image/'));
    const docs = sentImages.filter((f) => !f.mime.startsWith('image/'));
    try {
      awaitSeq = (
        await runtimeThreads.prompt(sessionId, {
          text: t,
          ...(model ? { model } : {}),
          ...(effort ? { effort } : {}),
          ...(pics.length ? { images: pics.map(({ mime, data_base64 }) => ({ mime, data_base64 })) } : {}),
          ...(docs.length ? { files: docs } : {}),
          // Only a person's pick changes the mode; otherwise the session keeps its own.
          ...(pickedMode ? { access_mode: pickedMode } : {}),
        })
      ).sequence;
      startError = null;
    } catch (e) {
      if (e instanceof ApiError && e.status === 400) startError = e.message;
      else toastError('Couldn’t send message', e);
      if (!text) text = t;
      if (images.length === 0) images = sentImages;
    } finally {
      sending = false;
    }
  }

  function submit(): void {
    let t = text.trim();
    if ((!t && images.length === 0) || closed) return;
    const intent = parseInteractiveSlashInput(t);
    if (intent?.kind === 'send-prompt') t = intent.prompt;
    else if (intent && runSlash(intent)) {
      text = '';
      return;
    }
    text = '';
    if (running) queued = t;
    else void send(t);
  }

  // ── Slash commands: the ones that act on this view ─────────────────────
  const SLASH_NAMES = new Set(['diff', 'files', 'terminal', 'new', 'fork', 'review', 'btw']);
  const SLASH = INTERACTIVE_SLASH_COMMANDS.filter((c) => SLASH_NAMES.has(c.name));
  const popId = $props.id();
  let cmdIdx = $state(0);
  let cmdDismissed = $state(false);
  const suggestions = $derived.by(() => {
    const prefix = /^\/([\w:-]*)$/.exec(text)?.[1].toLowerCase();
    return prefix === undefined || cmdDismissed ? [] : SLASH.filter((c) => c.name.includes(prefix));
  });
  $effect(() => {
    void suggestions.length;
    cmdIdx = 0;
  });
  $effect(() => {
    void text;
    cmdDismissed = false;
  });

  function runSlash(intent: ParsedSlashIntent): boolean {
    switch (intent.kind) {
      case 'open-diff':
        threads.setInspectorTab('changes');
        return true;
      case 'open-files':
        threads.setInspectorTab('files');
        return true;
      case 'toggle-terminal':
        threads.toggleTerminal(sessionId);
        return true;
      case 'new-thread':
        void newThread(intent.beside);
        return true;
      case 'side-chat':
        threads.openSideChat(sessionId, intent.question);
        return true;
      default:
        return false;
    }
  }

  async function newThread(beside: boolean): Promise<void> {
    if (!session) return;
    if (!beside) return threads.startNewThread({ workspaceId: session.workspace_id, cwd: session.cwd });
    try {
      const { provider, cwd, model } = session;
      const created = await runtimeThreads.start(session.workspace_id, {
        provider,
        cwd,
        title: `${harnessLabel(provider)} thread`,
        model: model ?? null,
        prompt: '',
      });
      threads.openBeside(created.id);
    } catch (e) {
      toastError('Couldn’t start thread', e);
    }
  }

  function acceptCmd(c: ComposerSlashCommand): void {
    if (isImmediateSlashCommand(c.name)) {
      text = `/${c.name}`;
      submit();
    } else {
      text = `/${c.name} `;
      cmdDismissed = true;
      composerEl?.focus();
    }
  }

  // The queued follow-up goes out once the turn has ended.
  $effect(() => {
    if (!running && queued) {
      const q = queued;
      queued = null;
      void send(q);
    }
  });

  async function stop(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await runtimeThreads.interrupt(sessionId);
    } catch (e) {
      toastError('Couldn’t stop the turn', e);
    } finally {
      stopping = false;
    }
  }

  async function respond(requestId: string, decision: ApprovalDecision): Promise<void> {
    try {
      await runtimeThreads.respond(sessionId, requestId, decision);
    } catch (e) {
      toastError('Couldn’t answer the approval', e);
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.isComposing) return;
    if (suggestions.length > 0) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        cmdIdx = (cmdIdx + (e.key === 'ArrowDown' ? 1 : suggestions.length - 1)) % suggestions.length;
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        cmdDismissed = true;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        e.preventDefault();
        acceptCmd(suggestions[cmdIdx]);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  }

  // Follow the bottom while the reader is there (T3's scroll anchoring).
  function onScroll(): void {
    if (scroller) atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 40;
  }
  $effect(() => {
    void p?.lastSequence;
    if (!atBottom) return;
    void tick().then(() => scroller?.scrollTo({ top: scroller.scrollHeight }));
  });
  // Focus the composer when the pane becomes the focused one (not per event).
  $effect(() => {
    const el = composerEl;
    if (focused && el) untrack(() => el.focus({ preventScroll: true }));
  });
</script>

{#snippet entryRow({ item, approval }: WorkEntry)}
  {@const pending = item.type === 'approval_request' ? approvals.get(item.request_id) : undefined}
  {#if pending}
    <RuntimeApprovalRow approval={pending} onrespond={respond} />
  {:else}
    {@const ask = approval ?? (item.type === 'approval_request' ? item : undefined)}
    {@const gate = approval ? approvals.get(approval.request_id) : undefined}
    <RuntimeWorkRow
      {item}
      output={p?.output[item.id]}
      pending={gate}
      note={ask && !gate ? approvalOutcome(ask, requests.get(ask.request_id), agentName) : null}
      onrespond={respond}
    />
  {/if}
{/snippet}

<section class="rt" aria-label="Conversation" data-testid="runtime-thread-{sessionId}">
  <RuntimeLinkedThreads {sessionId} {onswitch} />
  <RuntimeUiControl {sessionId} />
  <div class="rt-scroll" bind:this={scroller} onscroll={onScroll}>
    <div class="rt-col">
      <LoadState what="conversation" loading={!p && !loadError} error={p ? null : loadError} empty={!p} onretry={() => runtimeViews.retry(sessionId)}>
        {#if rows.length === 0}
          <EmptyState
            icon="threads"
            title="Start the conversation"
            body="Replies, tool calls and approvals stream here as the agent works."
          />
        {/if}
        {#each rows as row (row.kind === 'message' ? row.item.id : row.id)}
          {#if row.kind === 'message' && row.item.type === 'user_message'}
            <div class="rt-user">
              {#if row.item.attachments?.length}<RuntimeAttachments {sessionId} attachments={row.item.attachments} />{/if}
              {#if row.item.text}<div class="rt-bubble" dir="auto">{row.item.text}</div>{/if}
            </div>
          {:else if row.kind === 'message' && row.item.type === 'assistant_message'}
            <div class="rt-assistant" class:streaming={row.item.streaming} dir="auto">
              <Markdown md={row.item.text} />
            </div>
          {:else if row.kind === 'work'}
            <div class="rt-work">
              {#each row.parts as part (part.kind === 'group' ? part.id : part.entry.item.id)}
                {#if part.kind === 'item'}
                  {@render entryRow(part.entry)}
                {:else}
                  <RuntimeWorkGroup id={part.id} entries={part.entries} open={part.entries.some((e) => e.approval && approvals.has(e.approval.request_id))}>
                    {#each part.entries as e (e.item.id)}{@render entryRow(e)}{/each}
                  </RuntimeWorkGroup>
                {/if}
              {/each}
            </div>
          {/if}
        {/each}
        {#if workingFor}
          <p class="rt-status"><span class="spinner" aria-hidden="true"></span>Working for {workingFor}</p>
        {:else if turn && !running && turn.status === 'interrupted'}
          <p class="rt-status">Stopped</p>
        {/if}
      </LoadState>
    </div>
  </div>

  <div class="rt-dock">
    {#if sessionError}
      <p class="rt-banner" dir="auto" use:announceShown={{ text: sessionError, assertive: true }}><Icon name="warning" size={13} />{sessionError}</p>
    {/if}
    {#if plan}
      <ol class="rt-plan" aria-label="Plan">
        {#each plan.steps as s (s.id)}
          <li class="step-{s.status}" dir="auto"><Icon name={s.status === 'completed' ? 'check' : 'dot'} size={12} />{s.text}</li>
        {/each}
      </ol>
    {/if}
    {#if queued}
      <div class="rt-queued" data-testid="runtime-queued">
        <span class="ellipsis" dir="auto">Queued: {queued}</span>
        <button type="button" class="btn small" onclick={() => ((text = queued ?? ''), (queued = null))}>Edit</button>
      </div>
    {/if}
    <div class="rt-composer">
      {#if suggestions.length > 0}
        <StageSlashPopup id={popId} items={suggestions} active={cmdIdx} onaccept={acceptCmd} onhover={(i) => (cmdIdx = i)} />
      {/if}
      <textarea
        bind:this={composerEl}
        bind:value={text}
        dir="auto"
        rows={2}
        disabled={closed}
        placeholder={closed
          ? 'This thread is closed'
          : approvals.size > 0
            ? 'Waiting for your approval above'
            : running
              ? 'Queue a follow-up for when this turn ends'
              : 'Ask anything. / commands'}
        aria-label="Message"
        aria-autocomplete="list"
        aria-controls={suggestions.length > 0 ? `${popId}-list` : undefined}
        aria-activedescendant={suggestions.length > 0 ? `${popId}-opt-${cmdIdx}` : undefined}
        data-testid="runtime-composer"
        onkeydown={onKeydown}
        onpaste={onPaste}
      ></textarea>
      {#if images.length > 0}
        <div class="rt-images" data-testid="runtime-images">
          {#each images as img, i (i)}
            <span class="rt-image">
              <Icon name={img.mime.startsWith('image/') ? 'image' : 'file'} size={12} /><span class="ellipsis" dir="auto">{img.name}</span>
              <button type="button" class="icon-btn" aria-label="Remove {img.name}" title="Remove" onclick={() => (images = images.filter((_, j) => j !== i))}>
                <Icon name="x" size={12} />
              </button>
            </span>
          {/each}
        </div>
      {/if}
      <div class="rt-foot">
        {#if canImages}
        <button
          type="button"
          class="icon-btn"
          aria-label={canFiles ? 'Attach an image or PDF' : 'Attach an image'}
          title={canFiles ? 'Attach an image or PDF (or paste one)' : 'Attach an image (or paste one)'}
          data-testid="runtime-attach"
          disabled={closed}
          onclick={() => fileEl?.click()}
        >
          <Icon name="image" size={14} />
        </button>
        <input
          bind:this={fileEl}
          type="file"
          accept={canFiles ? 'image/*,application/pdf' : 'image/*'}
          multiple
          hidden
          onchange={(e) => {
            void addImages(e.currentTarget.files ?? []);
            e.currentTarget.value = '';
          }}
        />
        {/if}
        <span class="rt-grow">
          <button
            bind:this={agentEl}
            type="button"
            class="rt-agent"
            aria-haspopup="dialog"
            aria-expanded={pickerOpen}
            aria-label="Model for the next turn: {agentLabel}"
            title="Model and effort for the next turn"
            data-testid="runtime-model"
            disabled={closed || !session}
            onpointerenter={() => ModelPicker.prefetch()}
            onclick={() => (pickerOpen = true)}
          >
            {#if session && hasProviderIcon(session.provider)}<ProviderIcon provider={session.provider} size={13} />{/if}
            <span class="ellipsis">{agentLabel}</span>
          </button>
          <RuntimeAccessChip
            mode={accessMode}
            agent={harnessLabel(provider)}
            cwd={session?.cwd ?? 'this folder'}
            disabled={closed || !session}
            onpick={(m) => (pickedMode = m)}
          />
        </span>
        {#if running && !text.trim()}
          <button
            type="button"
            class="rt-send stop"
            aria-label="Stop the turn"
            title="Stop"
            data-testid="runtime-stop"
            disabled={stopping}
            onclick={() => void stop()}
          >
            <Icon name="stop" size={14} />
          </button>
        {:else}
          <button
            type="button"
            class="rt-send"
            aria-label={running ? 'Queue message' : 'Send message'}
            title={running ? 'Queue (↵)' : 'Send (↵)'}
            data-testid="runtime-send"
            disabled={closed || (!text.trim() && images.length === 0)}
            onclick={submit}
          >
            <Icon name="arrowUp" size={16} />
          </button>
        {/if}
      </div>
    </div>
  </div>
</section>

{#if pickerOpen && agentEl && ModelPicker.component}
  <ModelPicker.component
    anchor={agentEl}
    providers={[provider]}
    {provider}
    lockProvider
    model={model ?? session?.model ?? null}
    {effort}
    mode="full"
    onmodel={(m) => (model = m)}
    oneffort={supportsEffortSelection(provider) ? (e) => (effort = e) : undefined}
    onclose={() => (pickerOpen = false)}
  />
{/if}

<style>
  .rt {
    display: flex;
    flex-direction: column;
    min-block-size: 0;
    block-size: 100%;
    background: var(--bg);
  }
  .rt-scroll {
    flex: 1;
    min-block-size: 0;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .rt-col,
  .rt-dock {
    inline-size: 100%;
    max-inline-size: 760px;
    margin-inline: auto;
    padding-inline: 16px;
  }
  /* T3's MessagesTimeline rhythm: a turn opens with clear air above the
     user bubble; the answer and its work log sit close beneath it. */
  .rt-col {
    display: flex;
    flex-direction: column;
    padding-block: 16px 24px;
  }
  .rt-user {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 6px;
    padding-block-end: 12px;
  }
  .rt-user:not(:first-child) {
    margin-block-start: 20px;
  }
  .rt-assistant {
    padding-block-end: 12px;
  }
  .rt-assistant :global(:is(code, a.ref-chip)) {
    unicode-bidi: isolate;
  }
  .rt-bubble {
    max-inline-size: min(80%, 78ch);
    padding: 8px 14px;
    border: 1px solid color-mix(in srgb, var(--accent) 34%, var(--border));
    border-radius: var(--radius-l);
    background: color-mix(in srgb, var(--accent) 16%, var(--surface));
    color: var(--text);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    unicode-bidi: plaintext;
  }
  .rt-assistant {
    min-width: 0;
    color: var(--text);
  }
  .rt-assistant.streaming :global(.md > :last-child)::after {
    content: '▍';
    margin-inline-start: 2px;
    color: var(--text-dim);
  }
  .rt-work {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding-block-end: 10px;
  }
  .rt-status {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .rt-status .spinner {
    --spinner-size: 11px;
  }
  .rt-dock {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-block: 0 12px;
  }
  .rt-banner {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    padding: 6px 10px;
    border-radius: var(--radius-m);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
  .rt-plan {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-s);
    list-style: none;
  }
  .rt-plan li {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-wrap: anywhere;
  }
  .rt-plan .step-running {
    color: var(--text);
  }
  .rt-plan .step-completed {
    color: var(--success);
  }
  .rt-queued {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding-block: 4px;
    padding-inline: 10px 6px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rt-queued .ellipsis {
    flex: 1;
    min-width: 0;
  }
  .rt-composer {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
  }
  .rt-composer:focus-within {
    border-color: var(--border-strong);
  }
  .rt-composer textarea {
    inline-size: 100%;
    min-block-size: 44px;
    max-block-size: 220px;
    resize: none;
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.55;
    unicode-bidi: plaintext;
    text-align: start;
    outline: none;
    field-sizing: content;
  }
  .rt-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .rt-grow {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .rt-images {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .rt-image {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: 220px;
    padding-inline: 8px 2px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .rt-agent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: 100%;
    min-width: 0;
    block-size: 28px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    font: inherit;
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rt-agent:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .rt-send {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    inline-size: 30px;
    block-size: 30px;
    border: 0;
    border-radius: 50%;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    cursor: pointer;
  }
  .rt-send.stop {
    background: var(--text);
    color: var(--bg);
  }
  .rt-send:disabled {
    background: var(--surface-3);
    color: var(--text-dim);
    cursor: default;
  }
</style>
