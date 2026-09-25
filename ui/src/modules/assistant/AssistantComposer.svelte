<script lang="ts">
  // The thread composer: multi-line (Enter sends, Shift+Enter breaks the
  // line), a leading `@claude` / `@codex` routes one turn (the daemon strips
  // it), the model chip pins the thread, attachments land in the assistant's
  // inbox, and the mic is honestly disabled until voice ships.
  import Icon from '../../lib/components/Icon.svelte';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import { assistantApi } from '../../lib/api/assistant';
  import { assistant, describeError } from '../../lib/stores/assistant.svelte';
  import { formatBytes } from '../../lib/metric-format';
  import { parseRouteHint, providerLabel, providerName } from './model';
  import PinModelSheet from './PinModelSheet.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import type { AssistantAttachment, AssistantThread } from '../../lib/api/types';

  interface Props {
    thread: AssistantThread;
    /** The backing session is still answering — the daemon refuses a new turn (409). */
    busy?: boolean;
  }
  let { thread, busy = false }: Props = $props();

  let text = $state('');
  let sending = $state(false);
  let error = $state('');
  let pinning = $state(false);
  let files = $state<AssistantAttachment[]>([]);
  let uploading = $state(0);
  let ta = $state<HTMLTextAreaElement | null>(null);
  let fileInput = $state<HTMLInputElement | null>(null);

  $effect(() => guardUnsaved(() => !!text.trim() || files.length > 0 || sending || uploading > 0, { what: 'this message' }));

  const hint = $derived(parseRouteHint(text));
  const hasText = $derived(hint.text.trim() !== '' || files.length > 0);
  const canSend = $derived(!busy && !sending && uploading === 0 && hasText);
  const sendTitle = $derived(busy ? 'Otto is still answering — send when it’s done' : 'Send (Enter)');

  // Grow with the text up to ~8 lines, then scroll inside.
  $effect(() => {
    void text;
    const el = ta;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  });

  async function send(): Promise<void> {
    if (!canSend) return;
    // The raw text goes as typed: the daemon routes on the leading mention and strips it.
    const body = text.trim();
    const attachments = files;
    sending = true;
    error = '';
    text = '';
    files = [];
    try {
      await assistant.send(thread.id, body, attachments);
    } catch (e) {
      // Nothing is lost: the draft comes back with the reason.
      text = text.trim() ? `${body}\n\n${text}` : body;
      files = [...attachments, ...files.filter((file) => !attachments.some((sent) => sent.id === file.id))];
      error = `Couldn’t send. ${describeError(e)}`;
    } finally {
      sending = false;
      ta?.focus();
    }
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      void send();
    }
  }

  async function onFiles(list: FileList | null): Promise<void> {
    if (!list) return;
    error = '';
    for (const f of Array.from(list)) {
      uploading += 1;
      try {
        const content_base64 = await toBase64(f);
        const att = await assistantApi.attach(thread.id, { name: f.name, content_base64, mime: f.type || undefined });
        files = [...files, att];
      } catch (e) {
        error = `Couldn’t attach ${f.name}. ${describeError(e)}`;
      } finally {
        uploading -= 1;
      }
    }
    if (fileInput) fileInput.value = '';
  }

  function toBase64(blob: Blob): Promise<string> {
    return new Promise((resolve, reject) => {
      const r = new FileReader();
      r.onerror = () => reject(r.error ?? new Error('read failed'));
      r.onload = () => {
        const s = String(r.result ?? '');
        resolve(s.slice(s.indexOf(',') + 1));
      };
      r.readAsDataURL(blob);
    });
  }

  const chipLabel = $derived(thread.route_pinned ? providerLabel(thread.provider, thread.model) : 'Auto');
</script>

<div class="composer">
  {#if hint.provider}
    <p class="route" data-testid="route-hint">
      <ProviderIcon provider={hint.provider} size={12} /> This message goes to <strong>{providerName(hint.provider)}</strong>. The rest of the thread keeps its model.
    </p>
  {/if}
  {#if files.length || uploading}
    <ul class="files" aria-label="Attachments">
      {#each files as f, i (f.id)}
        <li class="file">
          <Icon name="file" size={12} /><span class="fname" title={f.path}>{f.name}</span><span class="dim">{formatBytes(f.size)}</span>
          <button class="icon-btn x" onclick={() => (files = files.filter((_, j) => j !== i))} aria-label={`Remove ${f.name}`} title={`Remove ${f.name}`}><Icon name="x" size={12} /></button>
        </li>
      {/each}
      {#if uploading}<li class="file dim">Attaching {uploading} {uploading === 1 ? 'file' : 'files'}…</li>{/if}
    </ul>
  {/if}
  <div class="box">
    <button class="icon-btn" onclick={() => fileInput?.click()} aria-label="Attach files" title="Attach files">
      <Icon name="plus" size={14} />
    </button>
    <input bind:this={fileInput} type="file" multiple hidden onchange={(e) => void onFiles(e.currentTarget.files)} />
    <textarea
      bind:this={ta}
      bind:value={text}
      rows="1"
      class="ta"
      placeholder={viewport.isPhone ? 'Reply to Otto…' : 'Reply to Otto…  (@codex to route one turn)'}
      aria-label="Message Otto"
      onkeydown={onKey}
    ></textarea>
    <button class="chip-btn" onclick={() => (pinning = true)} title={thread.route_pinned ? 'Pinned for this thread — change or unpin' : 'Routing rules pick the model — pin one for this thread'} aria-label={`Model: ${chipLabel}. Change`} data-testid="model-chip">
      {#if thread.route_pinned}<ProviderIcon provider={thread.provider} size={12} />{:else}<Icon name="split" size={12} />{/if}
      <span>{chipLabel}</span>
      <Icon name="chevronDown" size={12} />
    </button>
    <button class="icon-btn mic" disabled aria-label="Dictate (voice arrives in a later phase)" title="Voice arrives in a later phase">
      <Icon name="mic" size={14} />
    </button>
    <button class="icon-btn send" onclick={() => void send()} disabled={!canSend} aria-label="Send" title={sendTitle}>
      <Icon name="arrowUp" size={14} />
    </button>
  </div>
  {#if error}<p class="err" role="alert">{error}</p>{/if}
</div>
{#if pinning}
  <PinModelSheet {thread} onclose={() => (pinning = false)} />
{/if}

<style>
  .composer {
    padding: 10px 20px 14px;
    border-top: 1px solid var(--border);
    background: var(--bg);
  }
  .box {
    display: flex;
    align-items: flex-end;
    gap: 6px;
    max-width: 820px;
    padding: 6px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-l);
    background: var(--surface);
  }
  .box:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .box > .icon-btn {
    width: 28px;
    height: 28px;
  }
  .ta {
    flex: 1;
    min-width: 0;
    min-height: 28px;
    max-height: 200px;
    resize: none;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.45;
    padding: 5px 4px;
    outline: none; /* the ring is drawn on .box:focus-within */
  }
  .ta::placeholder {
    color: var(--text-dim);
  }
  .chip-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 28px;
    padding: 0 10px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    white-space: nowrap;
    cursor: pointer;
    flex-shrink: 0;
  }
  .chip-btn:hover:not(:disabled) {
    background: var(--hover);
  }
  .chip-btn :global(svg:last-child) {
    color: var(--text-dim);
  }
  /* The send control is the composer's one filled action (accent-solid fill,
     accent-contrast glyph); dimmed until there is something to send. */
  .box > .send {
    border-radius: 999px;
    background: var(--accent-solid);
    color: var(--accent-contrast);
  }
  .box > .send:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent-solid) 88%, var(--text));
    color: var(--accent-contrast);
  }
  .box > .send:disabled {
    background: var(--surface-3);
    color: var(--text-dim);
  }
  .route,
  .err {
    max-width: 820px;
    margin: 0 0 6px;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .route strong {
    color: var(--text);
  }
  .err {
    margin: 6px 0 0;
    color: var(--danger);
  }
  .files {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    max-width: 820px;
  }
  .file {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 26px;
    padding: 3px 4px 3px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    font-size: var(--fs-s);
    max-width: 100%;
  }
  .fname {
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .file > .dim, .file > .x { flex-shrink: 0; }
  .file > .dim { white-space: nowrap; }
  .dim {
    color: var(--text-dim);
  }
  .x {
    width: 20px;
    height: 20px;
  }
  @media (max-width: 640px) {
    .composer {
      padding: 8px 10px 10px;
    }
    .chip-btn span {
      max-width: 72px;
      overflow: hidden;
      text-overflow: ellipsis;
    }
    .box > .icon-btn,
    .chip-btn,
    .send {
      min-width: 36px;
      height: 36px;
    }
  }
</style>
