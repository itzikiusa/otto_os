<script lang="ts">
  // Shared "Send to agent" dialog — B2a.
  //
  // Workflow:
  //   1. On mount (or when open becomes true) → POST /preview → show the
  //      operator the redacted packet + a "Redacted: N secrets, M PII" badge.
  //   2. If the operator clicks Send → POST /send → closes the dialog with a
  //      toast confirmation.
  //
  // Props:
  //   workspaceId  — workspace that owns both the source data and the target session.
  //   sessionId    — the agent session to inject into (null = let user pick).
  //   kind         — 'api' | 'db' | 'broker' (label + icon only; payload shape is free).
  //   payload      — the raw JSON value to redact and inject.
  //   onclose      — called when the dialog should be dismissed (cancel or after send).
  import { untrack } from 'svelte';
  import { api } from '../api/client';
  import { ws } from '../stores/workspace.svelte';
  import { toasts } from '../toast.svelte';
  import Modal from './Modal.svelte';
  import type {
    ContextPacketKind,
    ContextPacketPreviewResp,
    RedactionHit,
    Session,
  } from '../api/types';

  interface Props {
    workspaceId: string;
    sessionId: string | null;
    kind: ContextPacketKind;
    payload: unknown;
    onclose: () => void;
  }
  let { workspaceId, sessionId, kind, payload, onclose }: Props = $props();

  // ── State ─────────────────────────────────────────────────────────────────
  // The session the operator wants to target. Defaults to the prop, falls back
  // to the currently-focused workspace session, or lets the user pick.
  let pickedSessionId = $state<string | null>(untrack(() => sessionId));

  // Preview result (null = not yet loaded, 'loading' = in-flight).
  type PreviewState = 'loading' | ContextPacketPreviewResp | null;
  let preview = $state<PreviewState>(null);
  let previewErr = $state<string | null>(null);

  let sending = $state(false);

  // Derive the running agent sessions so the operator can pick one.
  const agentSessions = $derived(
    ws.agentSessions.filter((s: Session) => s.status !== 'exited'),
  );

  // ── Preview logic ─────────────────────────────────────────────────────────
  async function loadPreview(sid: string) {
    preview = 'loading';
    previewErr = null;
    try {
      const resp = await api.post<ContextPacketPreviewResp>(
        `/workspaces/${workspaceId}/agents/${sid}/context-packet/preview`,
        { kind, payload },
      );
      preview = resp;
    } catch (e) {
      previewErr = e instanceof Error ? e.message : String(e);
      preview = null;
    }
  }

  // Trigger a preview whenever the target session resolves.
  $effect(() => {
    const sid = pickedSessionId;
    if (sid) void loadPreview(sid);
    else { preview = null; previewErr = null; }
  });

  // ── Send ──────────────────────────────────────────────────────────────────
  async function sendPacket() {
    if (!pickedSessionId) return;
    sending = true;
    try {
      const resp = await api.post<{ ok: boolean; size_bytes: number; redactions: RedactionHit[] }>(
        `/workspaces/${workspaceId}/agents/${pickedSessionId}/context-packet/send`,
        { kind, payload },
      );
      const total = resp.redactions.reduce((s, h) => s + h.count, 0);
      const label = kindLabel(kind);
      toasts.success(
        total > 0
          ? `${label} sent — ${total} secret${total === 1 ? '' : 's'} redacted`
          : `${label} sent to agent`,
      );
      onclose();
    } catch (e) {
      toasts.error(e instanceof Error ? e.message : String(e));
    } finally {
      sending = false;
    }
  }

  // ── Helpers ───────────────────────────────────────────────────────────────
  function kindLabel(k: ContextPacketKind): string {
    return k === 'api' ? 'API response' : k === 'db' ? 'DB result' : 'Broker message';
  }

  function redactionSummary(hits: RedactionHit[]): string {
    if (hits.length === 0) return 'No secrets detected';
    const parts = hits.map((h) => `${h.count} ${h.kind}`);
    return `Redacted: ${parts.join(', ')}`;
  }

  function previewJson(value: unknown): string {
    try { return JSON.stringify(value, null, 2); }
    catch { return String(value); }
  }
</script>

<Modal title="Send {kindLabel(kind)} to agent" width={560} {onclose}>
  {#snippet children()}
    <!-- Session picker (shown when sessionId prop is null OR agent list non-empty) -->
    {#if agentSessions.length > 0}
      <div class="field">
        <label for="cp-session">Target agent session</label>
        <select
          id="cp-session"
          bind:value={pickedSessionId}
          disabled={sending}
        >
          <option value={null}>— pick a session —</option>
          {#each agentSessions as s (s.id)}
            <option value={s.id}>{s.title || s.id.slice(0, 8)} ({s.status})</option>
          {/each}
        </select>
      </div>
    {:else}
      <p class="no-sessions">No running agent sessions in this workspace.</p>
    {/if}

    <!-- Preview pane -->
    {#if pickedSessionId}
      {#if preview === 'loading'}
        <div class="preview-loading">Previewing redactions…</div>
      {:else if previewErr}
        <div class="preview-err">Preview failed: {previewErr}</div>
      {:else if preview}
        <!-- Redaction badge -->
        {@const total = (preview as ContextPacketPreviewResp).redactions.reduce((s, h) => s + h.count, 0)}
        <div class="badge {total > 0 ? 'badge-warn' : 'badge-ok'}">
          {redactionSummary((preview as ContextPacketPreviewResp).redactions)}
        </div>
        <!-- Redacted payload preview (read-only) -->
        <div class="preview-wrap">
          <pre class="preview-code">{previewJson((preview as ContextPacketPreviewResp).redacted)}</pre>
        </div>
        <div class="size-hint">{(preview as ContextPacketPreviewResp).size_bytes} bytes will be injected</div>
      {/if}
    {/if}
  {/snippet}

  {#snippet footer()}
    <button class="btn-ghost" onclick={onclose} disabled={sending}>Cancel</button>
    <button
      class="btn-primary"
      onclick={sendPacket}
      disabled={sending || !pickedSessionId || preview === 'loading' || preview === null}
    >
      {sending ? 'Sending…' : 'Send to agent'}
    </button>
  {/snippet}
</Modal>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 12px;
  }
  label {
    font-size: 12px;
    font-weight: 500;
    color: var(--fg-muted);
  }
  select {
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface);
    color: var(--fg);
    font-size: 13px;
  }
  .no-sessions {
    font-size: 13px;
    color: var(--fg-muted);
    margin-bottom: 12px;
  }
  .preview-loading,
  .preview-err {
    font-size: 13px;
    color: var(--fg-muted);
    margin-bottom: 8px;
  }
  .preview-err { color: var(--red); }
  .badge {
    display: inline-block;
    font-size: 11px;
    font-weight: 600;
    padding: 3px 8px;
    border-radius: 999px;
    margin-bottom: 8px;
  }
  .badge-ok  { background: var(--green-bg, #d1fae5); color: var(--green, #065f46); }
  .badge-warn { background: var(--yellow-bg, #fef3c7); color: var(--yellow, #92400e); }
  .preview-wrap {
    max-height: 260px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-raised, var(--surface));
  }
  .preview-code {
    margin: 0;
    padding: 10px 12px;
    font-size: 11.5px;
    font-family: var(--font-mono, monospace);
    white-space: pre-wrap;
    word-break: break-all;
  }
  .size-hint {
    font-size: 11px;
    color: var(--fg-muted);
    margin-top: 6px;
    text-align: right;
  }
  .btn-ghost {
    padding: 6px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: transparent;
    color: var(--fg);
    cursor: pointer;
    font-size: 13px;
  }
  .btn-primary {
    padding: 6px 14px;
    border: none;
    border-radius: var(--radius);
    background: var(--accent);
    color: var(--accent-fg, #fff);
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
  }
  .btn-primary:disabled,
  .btn-ghost:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
