<script lang="ts">
  // Remote CRUD for one repo (`GET|POST /repos/{id}/remotes`). URLs arrive with
  // any `user:password@` userinfo already stripped by the daemon, so nothing
  // credentialed is ever rendered here. Push/checkout still use `origin`
  // explicitly — renaming or removing it does not re-point them.
  import type { RemoteInfo, RemoteOpReq } from '../../lib/api/types';
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';

  interface Props {
    repoId: string;
    onclose: () => void;
  }
  let { repoId, onclose }: Props = $props();

  let remotes = $state<RemoteInfo[]>([]);
  let loading = $state(true);
  let busy = $state('');
  let error = $state<string | null>(null);

  $effect(() => {
    const id = repoId;
    loading = true;
    api
      .get<RemoteInfo[]>(`/repos/${id}/remotes`)
      .then((rows) => {
        remotes = rows;
        error = null;
      })
      .catch((e: unknown) => {
        error = e instanceof Error ? e.message : String(e);
      })
      .finally(() => {
        loading = false;
      });
  });

  async function send(req: RemoteOpReq, label: string): Promise<void> {
    busy = req.name;
    error = null;
    try {
      remotes = await api.post<RemoteInfo[]>(`/repos/${repoId}/remotes`, req);
      toasts.success(label, req.name);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      error = msg;
      toasts.error(`${label} failed`, msg);
    } finally {
      busy = '';
    }
  }

  async function addRemote(): Promise<void> {
    const name = await confirmer.promptText('Name for the new remote', {
      title: 'Add remote',
      confirmLabel: 'Next',
      placeholder: 'upstream',
    });
    if (!name) return;
    const url = await confirmer.promptText(`Fetch/push URL for \`${name}\``, {
      title: 'Add remote',
      confirmLabel: 'Add',
      placeholder: 'https://github.com/owner/repo.git',
    });
    if (!url) return;
    await send({ op: 'add', name, url }, 'Remote added');
  }

  async function editUrl(r: RemoteInfo): Promise<void> {
    const url = await confirmer.promptText(`New URL for \`${r.name}\``, {
      title: 'Edit remote URL',
      confirmLabel: 'Save',
      initial: r.fetch_url,
    });
    if (!url || url === r.fetch_url) return;
    await send({ op: 'set_url', name: r.name, url }, 'Remote updated');
  }

  async function removeRemote(r: RemoteInfo): Promise<void> {
    const ok = await confirmer.ask(`Remove remote \`${r.name}\`? Local branches are kept.`, {
      title: 'Remove remote',
      confirmLabel: 'Remove',
    });
    if (!ok) return;
    await send({ op: 'remove', name: r.name }, 'Remote removed');
  }
</script>

<Modal title="Remotes" width={520} {onclose}>
  <div class="rp">
    {#if loading}
      <Skeleton rows={2} height={38} />
    {:else if remotes.length === 0}
      <p class="rp-empty">This repository has no remotes. Add one to fetch, pull or push.</p>
    {:else}
      <ul class="rp-list">
        {#each remotes as r (r.name)}
          <li class="rp-row">
            <div class="rp-id">
              <span class="rp-name">{r.name}</span>
              <span class="mono rp-url" title={r.fetch_url}>{r.fetch_url}</span>
              {#if r.push_url && r.push_url !== r.fetch_url}
                <span class="mono rp-url" title={r.push_url}>push: {r.push_url}</span>
              {/if}
            </div>
            <button class="btn ghost small" disabled={busy === r.name} onclick={() => editUrl(r)}>
              Edit URL
            </button>
            <button
              class="btn ghost small danger"
              disabled={busy === r.name}
              onclick={() => removeRemote(r)}
            >
              Remove
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    {#if error}
      <p class="rp-err"><Icon name="x" size={12} /> {error}</p>
    {/if}
  </div>

  {#snippet footer()}
    <button class="btn ghost" onclick={onclose}>Close</button>
    <button class="btn primary" onclick={addRemote} disabled={!!busy}>Add remote</button>
  {/snippet}
</Modal>

<style>
  .rp {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .rp-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .rp-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .rp-id {
    display: flex;
    flex-direction: column;
    gap: 2px;
    flex: 1;
    min-width: 0;
  }
  .rp-name {
    font-size: 12.5px;
    font-weight: 600;
  }
  .rp-url {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rp-empty {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
  }
  .rp-err {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: 12px;
    color: var(--status-exited);
  }
</style>
