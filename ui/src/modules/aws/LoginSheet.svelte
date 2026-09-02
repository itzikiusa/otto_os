<script lang="ts">
  // "Sign in" sheet: hosts the `aws sso login --profile …` PTY session the
  // daemon spawned (`POST /aws/accounts/{id}/login`) in a live <Terminal>, and
  // polls `/test` every 3 s until credentials work — then refreshes the account's
  // permission probe and closes itself.
  import { onDestroy } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { awsApi } from '../../lib/api/aws';
  import { aws } from '../../lib/stores/aws.svelte';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    accountId: string;
    sessionId: string;
    onclose: () => void;
  }
  let { accountId, sessionId, onclose }: Props = $props();

  const account = $derived(aws.account(accountId));
  let lastMsg = $state('Waiting for the browser sign-in to complete…');
  let done = $state(false);
  let timer: ReturnType<typeof setTimeout> | null = null;
  let closed = false;

  async function poll(): Promise<void> {
    if (closed) return;
    try {
      const r = await awsApi.test(accountId);
      if (r.ok) {
        done = true;
        lastMsg = `Signed in${r.identity ? ` as ${r.identity.arn}` : ''}.`;
        toasts.success('AWS sign-in complete', account?.name ?? accountId);
        void aws.loadPermissions(accountId, true);
        timer = setTimeout(() => {
          if (!closed) onclose();
        }, 900);
        return;
      }
      lastMsg = r.login_required ? 'Still waiting for sign-in…' : r.message;
    } catch (e) {
      lastMsg = e instanceof Error ? e.message : String(e);
    }
    timer = setTimeout(() => void poll(), 3000);
  }

  // First probe after a short grace period (the PTY needs a moment to print
  // the device-code URL).
  timer = setTimeout(() => void poll(), 3000);

  onDestroy(() => {
    closed = true;
    if (timer) clearTimeout(timer);
  });
</script>

<Modal title={`Sign in — ${account?.name ?? 'AWS'}`} width={760} {onclose}>
  <p class="hint">
    Follow the prompt below (it opens your browser for AWS SSO). This window closes by itself
    once the credentials work.
  </p>
  <div class="term">
    {#key sessionId}
      <Terminal {sessionId} preferDom autoFocus showToolbar={false} />
    {/key}
  </div>
  <p class="status" class:ok={done} aria-live="polite">{lastMsg}</p>
  {#snippet footer()}
    <button class="ghost" onclick={onclose}>Close</button>
  {/snippet}
</Modal>

<style>
  .hint {
    margin: 0 0 8px;
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .term {
    height: min(360px, 50vh);
    min-height: 200px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    background: var(--term-bg);
  }
  .status {
    margin: 8px 0 0;
    font-size: 12px;
    color: var(--text-dim);
  }
  .status.ok {
    color: var(--status-working);
  }
  .ghost {
    padding: 6px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
</style>
