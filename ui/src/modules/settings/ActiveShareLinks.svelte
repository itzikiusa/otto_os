<script lang="ts">
  // Settings → Sharing → Active links: every live share link the caller has
  // minted, across ALL sessions (`GET /auth/shares`), with Revoke per row and
  // a caller-wide Revoke all (`POST /auth/shares/revoke-all`). The per-session
  // ShareModal only shows one session's links — without this page a forgotten
  // link on another session could only be found by opening each session.
  import { tick } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { MyShare } from '../../lib/api/types';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  import { rel } from '../../lib/stores/now.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';

  let shares = $state<MyShare[]>([]);
  let loading = $state(true);
  let error = $state('');
  let revoking = $state<Set<string>>(new Set());
  let revokingAll = $state(false);
  let tableEl = $state<HTMLElement>();
  let headingEl = $state<HTMLElement>();

  /** After row `i` is gone, keep focus in the table: the same index's Revoke
   *  (the next row slid up), else the previous one, else the section heading —
   *  the confirm returned focus to a button that no longer exists. */
  async function settleFocus(i: number): Promise<void> {
    await tick();
    const btns = tableEl?.querySelectorAll<HTMLButtonElement>('td.act button');
    (btns?.[i] ?? btns?.[i - 1] ?? headingEl)?.focus();
  }

  async function load(): Promise<void> {
    loading = true;
    error = '';
    try {
      shares = await api.get<MyShare[]>('/auth/shares');
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => { void load(); });

  function name(s: MyShare): string {
    return s.session_title?.trim() || (s.session_title === null ? 'Deleted session' : 'Untitled session');
  }

  async function revoke(s: MyShare): Promise<void> {
    const ok = await confirmer.ask(
      `Revoke the ${s.role} link to “${name(s)}”? Guests attached through it are disconnected and it can’t be used again.`,
      { title: 'Revoke share link', danger: true, confirmLabel: 'Revoke link' },
    );
    if (!ok) return;
    revoking = new Set([...revoking, s.id]);
    try {
      await api.del(`/auth/shares/${encodeURIComponent(s.id)}`);
      const at = shares.findIndex((x) => x.id === s.id);
      shares = shares.filter((x) => x.id !== s.id);
      toasts.success('Share link revoked');
      void settleFocus(at);
    } catch (e) {
      toastError('Couldn’t revoke the link', e);
    } finally {
      const next = new Set(revoking);
      next.delete(s.id);
      revoking = next;
    }
  }

  async function revokeAll(): Promise<void> {
    const n = shares.length;
    if (!n || revokingAll) return;
    const ok = await confirmer.ask(
      `Revoke all ${n} of your share links, on every session? Every guest is disconnected and none of the links can be used again.`,
      { title: 'Revoke all share links', danger: true, confirmLabel: `Revoke ${n} ${n === 1 ? 'link' : 'links'}` },
    );
    if (!ok) return;
    revokingAll = true;
    try {
      await api.post('/auth/shares/revoke-all', {});
      shares = [];
      toasts.success('All share links revoked');
      void settleFocus(0);
    } catch (e) {
      toastError('Couldn’t revoke the links', e);
      void load();
    } finally {
      revokingAll = false;
    }
  }
</script>

<div class="links-head">
  <div class="section-title" tabindex="-1" bind:this={headingEl}>Active links</div>
  {#if shares.length > 0}
    <button class="btn small danger" disabled={revokingAll} aria-busy={revokingAll} onclick={() => void revokeAll()}>
      {revokingAll ? 'Revoking…' : 'Revoke all'}
    </button>
  {/if}
</div>
<LoadState what="your share links" {loading} {error} empty={shares.length === 0} onretry={() => void load()} rows={2}>
  {#snippet emptyView()}
    <div class="card"><EmptyState icon="link" title="No active share links" body="Links you create from a session’s Share dialog show up here until they expire or you revoke them." /></div>
  {/snippet}
  <div class="card table-wrap">
    <table class="links" data-testid="active-share-links" bind:this={tableEl}>
      <thead>
        <tr><th scope="col">Session</th><th scope="col">Access</th><th scope="col">Label</th><th scope="col">Expires</th><th scope="col"><span class="sr-only">Actions</span></th></tr>
      </thead>
      <tbody>
        {#each shares as s (s.id)}
          <tr>
            <td class="sess">
              {#if s.session_title !== null}
                <button type="button" class="link-btn" title="Open this session" onclick={() => ws.navigateToSession(s.session_id)}>{name(s)}</button>
              {:else}
                <span class="dim">{name(s)}</span>
              {/if}
              <span class="mono dim prefix" title="Token prefix">{s.token_prefix}…</span>
            </td>
            <td data-label="Access">{s.role === 'editor' ? 'Can type' : 'View only'}</td>
            <td class="dim" data-label="Label">{s.label ?? '—'}</td>
            <td
              data-label="Expires"
              title={s.dormant
                ? 'Window lapsed — the link holder can still request a new code until it is revoked'
                : new Date(s.expires_at).toLocaleString()}
            >{s.dormant ? 'Dormant (revivable)' : rel(s.expires_at)}</td>
            <td class="act">
              <button
                class="btn small"
                disabled={revoking.has(s.id) || revokingAll}
                aria-busy={revoking.has(s.id)}
                onclick={() => void revoke(s)}
              >
                {revoking.has(s.id) ? 'Revoking…' : 'Revoke'}
              </button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</LoadState>

<style>
  .links-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .table-wrap { padding: 0; overflow-x: auto; }
  .links { width: 100%; border-collapse: collapse; font-size: var(--fs-s); }
  .links th {
    text-align: start;
    font-weight: 600;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 8px 10px;
    border-block-end: 1px solid var(--border);
  }
  .links td { padding: 8px 10px; border-block-end: 1px solid var(--border); vertical-align: middle; }
  .links tr:last-child td { border-block-end: 0; }
  .sess { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .prefix { font-size: var(--fs-xs); }
  .act { text-align: end; white-space: nowrap; }
  .link-btn {
    background: none;
    border: 0;
    padding: 0;
    font: inherit;
    text-align: start;
    color: var(--accent-text);
    cursor: pointer;
  }
  .link-btn:hover { text-decoration: underline; }
  .link-btn:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; border-radius: var(--radius-s); }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  @media (max-width: 640px) {
    .links, .links tbody { display: block; }
    .links thead { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    .links tbody tr { display: grid; grid-template-columns: minmax(0, 1fr) auto; padding: 10px; gap: 8px; border-block-end: 1px solid var(--border); }
    .links td { padding: 0; border: 0; overflow-wrap: anywhere; }
    .links .sess, .links td[data-label] { grid-column: 1 / -1; }
    .links td[data-label] { display: grid; grid-template-columns: 64px minmax(0, 1fr); gap: 8px; }
    .links td[data-label]::before { content: attr(data-label); color: var(--text-dim); }
    .links .act { grid-column: 2; }
    .link-btn { min-height: var(--hit-min); }
  }
</style>
