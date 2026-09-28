<script lang="ts">
  import {onMount} from 'svelte';
  import {pollWhileVisible} from '../../lib/poll';
  import type {RecapDetail, RecapDraft, RecapEventData} from '../../lib/api/room-recap-types';
  import {recapRequest, recapBlob} from './recap-client';
  import {confirmer} from '../../lib/confirm.svelte';
  import {copyTextOrThrow} from '../../lib/clipboard';
  import RecapImage from './RecapImage.svelte';
  let {recapId}: {recapId: string} = $props();
  let detail = $state<RecapDetail | null>(null), error = $state(''), actionError = $state(''), loading = $state(true), busy = $state(false), copied = $state(false);
  let tab = $state<'transcript' | 'activity' | 'summary'>('transcript'), after = $state(0), previous = $state<number[]>([]);
  let live = true, request = 0;
  onMount(() => { const poller = pollWhileVisible(signal => busy ? undefined : load(!detail, signal), {ms: 4000}); return () => { live = false; request++; poller.stop(); }; });
  async function load(showLoading = true, signal?: AbortSignal) { if (showLoading) loading = true; const seq = ++request; try { const value = await recapRequest<RecapDetail>(`/room-recaps/${encodeURIComponent(recapId)}?after=${after}&limit=100`, undefined, signal); if (live && seq === request) { detail = value; error = ''; } return true; } catch (e) { if (live && seq === request) error = e instanceof Error ? e.message : 'Could not load this recap.'; return false; } finally { if (seq === request) loading = false; } }
  async function generate() { if (!await confirmer.ask('Send captured text and representative shared-screen images to Codex using your ChatGPT subscription to create a draft. This uses your subscription allowance. Not every saved image is included; the draft reports coverage. The local archive remains available if generation fails.', {title: 'Generate recap summary?', confirmLabel: 'Generate summary'})) return; busy = true; actionError = ''; try { await recapRequest(`/room-recaps/${recapId}/summary`, {}); tab = 'summary'; await load(false); } catch (e) { actionError = e instanceof Error ? e.message : 'Summary generation could not start.'; } finally { busy = false; } }
  async function cancel() { try { await recapRequest(`/room-recaps/${recapId}/summary`, undefined, undefined, 'DELETE'); await load(false); } catch { actionError = 'Could not cancel generation. Try again.'; } }
  async function exportArchive() { try { const blob = await recapBlob(`/room-recaps/${recapId}/export`); const url = URL.createObjectURL(blob); const link = document.createElement('a'); link.href = url; link.download = `room-recap-${recapId}.json`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 30000); } catch { actionError = 'Could not export the recap. Try again.'; } }
  function draftText(draft: RecapDraft) { return ['Codex draft', draft.overview, 'Decisions', ...draft.decisions, 'Actions', ...draft.actions, 'Open questions', ...draft.open_questions, 'Coverage', ...draft.coverage].join('\n\n'); }
  async function copyDraft() { if (!detail?.draft) return; try { await copyTextOrThrow(draftText(detail.draft)); copied = true; } catch { actionError = 'Could not copy the draft. Select the text and copy it manually.'; } }
  function terminalText(data: string): string { try { return new TextDecoder().decode(Uint8Array.from(atob(data), c => c.charCodeAt(0))).replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '').replace(/\x1b\][^\x07]*(?:\x07|\x1b\\)/g, ''); } catch { return 'Terminal text could not be decoded.'; } }
  function activity(payload: RecapEventData): string {
    switch (payload.type) {
      case 'capture': return `Capture ${payload.state.replaceAll('_', ' ')}${payload.reason ? `: ${payload.reason}` : ''}`;
      case 'participants': return `Participants: ${payload.members.map(m => m.name).join(', ')}`;
      case 'presentation': return `${payload.operation}: ${payload.presentation.title}`;
      case 'annotation': return `${payload.annotation.tool} annotation on source ${payload.annotation.source_id}`;
      case 'annotations_cleared': return 'Shared-screen annotations cleared';
      case 'annotation_removed': return 'Shared-screen annotation removed';
      case 'audio_pending': return `Transcribing ${payload.member_name}’s audio…`;
      default: return '';
    }
  }
</script>
<div class="recap-panel">
  {#if loading && !detail}<p role="status">Loading recap…</p>{/if}
  {#if error}<p class="error" role="alert">{error} <button class="btn small" onclick={() => load()}>Retry</button></p>{/if}
  {#if actionError}<p class="error" role="alert">{actionError}</p>{/if}
  {#if detail}
    <div class="recap-heading"><div><strong>{detail.metadata.session_title}</strong><p>{detail.metadata.status.replaceAll('_', ' ')} · {(detail.metadata.bytes_used / 1024 / 1024).toFixed(1)} MiB of {(detail.metadata.quota_bytes / 1024 / 1024).toFixed(0)} MiB</p></div><button class="btn small" onclick={exportArchive}>Export full archive</button></div>
    {#if !detail.metadata.speech_available}<p class="coverage">Speech is unavailable: {detail.metadata.speech_error ?? 'The archive contains available room activity only.'}</p>{/if}
    <nav aria-label="Recap sections">{#each ['transcript', 'activity', 'summary'] as value}<button class="btn" aria-pressed={tab === value} onclick={() => tab = value as typeof tab}>{value === 'transcript' ? 'Transcript' : value === 'activity' ? 'Activity & screens' : 'Summary'}</button>{/each}</nav>
    {#if tab === 'summary'}
      <div class="summary-actions">{#if ['queued', 'running'].includes(detail.metadata.summary_status)}<p role="status">Codex is preparing a draft…</p><button class="btn" onclick={cancel}>Cancel generation</button>{:else}<button class="btn" disabled={busy} onclick={generate}>{detail.draft ? 'Regenerate summary…' : 'Generate summary…'}</button>{/if}</div>
      {#if detail.metadata.summary_error}<p class="error" role="alert">{detail.metadata.summary_error}</p>{/if}
      {#if detail.draft}<article class="draft"><header><strong>Codex draft</strong><button class="btn small" onclick={copyDraft}>{copied ? 'Copied' : 'Copy draft'}</button></header><p>{detail.draft.overview}</p>
        {#each [{title: 'Decisions', items: detail.draft.decisions}, {title: 'Actions', items: detail.draft.actions}, {title: 'Open questions', items: detail.draft.open_questions}, {title: 'Coverage', items: detail.draft.coverage}] as group}<h3>{group.title}</h3>{#if group.items.length}<ul>{#each group.items as item}<li>{item}</li>{/each}</ul>{:else}<p>None recorded.</p>{/if}{/each}
        <p class="hint">Source events: {detail.draft.source_event_ids.join(', ') || 'Not supplied'}</p>
        {#if detail.metadata.summary_through_seq !== null && detail.metadata.last_seq > detail.metadata.summary_through_seq}<p class="coverage">New activity arrived after this draft. Generate again to include it.</p>{/if}
      </article>{:else}<p>No summary yet. Your transcript and activity remain available independently.</p>{/if}
    {:else}
      <p class="hint">Archive events {detail.events[0]?.seq ?? 0}–{detail.events.at(-1)?.seq ?? 0}. Speech and room activity are paginated; export includes the full archive.</p>
      <div class="events">{#each detail.events as event (event.seq)}{@const payload = event.payload}
        {#if tab === 'activity' || ['speech', 'chat', 'terminal', 'gap', 'capture', 'audio_pending'].includes(payload.type)}<article class:gap={payload.type === 'gap'}>
          <header><span>#{event.seq} · {new Date(event.created_at).toLocaleTimeString()}</span><span>{payload.type.replaceAll('_', ' ')}</span></header>
          {#if payload.type === 'speech'}<strong>{payload.member_name}</strong>{#each payload.segments as segment}<p>{segment.text}</p>{/each}
          {:else if payload.type === 'chat'}<strong>{payload.message.name}</strong><p>{payload.message.text}</p>
          {:else if payload.type === 'terminal'}<pre>{terminalText(payload.data_base64)}</pre>
          {:else if payload.type === 'screen'}<strong>{payload.title}</strong><p>{payload.text || 'No readable text detected; inspect the saved sample.'}</p><RecapImage {recapId} imageId={payload.image_id} title={payload.title} />
          {:else if payload.type === 'gap'}<p><strong>Coverage gap:</strong> {payload.reason}</p>
          {:else}<p>{activity(payload)}</p>{/if}
        </article>{/if}
      {/each}</div>
      {#if !detail.events.length}<p>No accepted activity yet.</p>{/if}
      <div class="pagination"><button class="btn" disabled={!previous.length || loading} onclick={() => { after = previous.at(-1) ?? 0; previous = previous.slice(0, -1); void load(); }}>Earlier events</button><button class="btn" disabled={detail.next_cursor === null || loading} onclick={() => { previous = [...previous, after]; after = detail!.next_cursor!; void load(); }}>Next events</button></div>
    {/if}
  {/if}
</div>
<style>
  .recap-panel { min-width: 0; } .recap-heading, header, nav, .pagination, .summary-actions { display: flex; align-items: center; flex-wrap: wrap; gap: 8px; } .recap-heading, header { justify-content: space-between; } p { white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.5; } nav { margin-block: 12px; }
  .hint, header, .recap-heading p { font-size: var(--fs-s); color: var(--text-dim); } .error { color: var(--danger); } .coverage, .gap { background: var(--warning-soft); padding: 12px; }
  article { border-bottom: 1px solid var(--border); padding-block: 12px; } article p { margin-block: 8px; } pre { white-space: pre-wrap; overflow-wrap: anywhere; overflow: auto; max-height: 240px; padding: 12px; background: var(--surface-2); font-size: var(--fs-s); } h3 { font-size: var(--fs-m); } li { margin-block: 8px; line-height: 1.5; } .pagination { margin-block: 16px; }
</style>
