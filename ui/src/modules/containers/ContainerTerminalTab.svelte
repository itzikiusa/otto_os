<script lang="ts">
  // Container Terminal tab: spawns an Otto PTY session running `docker exec -it`
  // via `POST /containers/{id}/exec` and embeds `<LazyTerminal preferDom>`
  // inline. Tears down the PTY session automatically on unmount or container switch.
  import { untrack } from 'svelte';
  import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { api } from '../../lib/api/client';
  import { containersApi } from '../../lib/api/containers';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { NO_WORKSPACE } from '../../lib/labels';
  import { loadErrorText } from '../../lib/loadError';
  import type { SessionStatus } from '../../lib/api/types';
  import { parseCommandArgs } from './containers-util';

  interface Props {
    containerId: string;
    containerName: string;
    running: boolean;
    dockerCliAvailable: boolean;
  }
  let { containerId, containerName, running, dockerCliAvailable }: Props = $props();

  let sessionId = $state<string | null>(null);
  let status = $state<SessionStatus | null>(null);
  let opening = $state(false);
  let error = $state('');
  let customCommand = $state('');
  let openSeq = 0;

  async function openShell(): Promise<void> {
    if (!running || opening) return;
    const wsId = ws.currentId;
    if (!wsId) {
      error = `${NO_WORKSPACE} — the terminal session attaches to the active workspace.`;
      return;
    }
    const seq = ++openSeq;
    opening = true;
    error = '';
    try {
      const parsedCmd = parseCommandArgs(customCommand);
      const session = await containersApi.exec(containerId, {
        workspace_id: wsId,
        command: parsedCmd.length > 0 ? parsedCmd : null,
      });
      if (seq !== openSeq) {
        void api.del(`/sessions/${session.id}`).catch(() => {});
        return;
      }
      sessionId = session.id;
    } catch (e) {
      if (seq === openSeq) error = loadErrorText(e);
    } finally {
      if (seq === openSeq) opening = false;
    }
  }

  async function closeShell(): Promise<void> {
    const id = sessionId;
    sessionId = null;
    status = null;
    if (!id) return;
    try {
      await api.del(`/sessions/${id}`);
    } catch {
      /* best-effort teardown */
    }
  }

  $effect(() => {
    void containerId;
    untrack(() => {
      opening = false;
      error = '';
      customCommand = '';
    });
    return () => {
      openSeq += 1;
      untrack(() => void closeShell());
    };
  });
</script>

<div class="ct-exec">
  {#if !running}
    <div class="ct-exec-note">
      <Icon name="info" size={14} />
      <span>Start <strong>{containerName}</strong> to open an interactive shell inside it.</span>
    </div>
  {:else if !dockerCliAvailable}
    <div class="ct-exec-note">
      <Icon name="warning" size={14} />
      <span>The <code dir="ltr">docker</code> CLI isn’t on PATH, so interactive exec sessions aren’t available yet.</span>
    </div>
  {:else if sessionId}
    <div class="ct-exec-bar">
      <span class="ct-exec-title" dir="ltr">{containerName}</span>
      {#if status}
        <span class="ct-exec-status">{status}</span>
      {/if}
      <span class="ct-exec-spacer"></span>
      <button type="button" class="btn small" onclick={() => void closeShell()}>
        Close shell
      </button>
    </div>
    <div class="ct-exec-term">
      {#key sessionId}
        <LazyTerminal
          {sessionId}
          preferDom
          autoFocus
          restartable
          onrestart={() => {
            void closeShell().then(openShell);
          }}
          onstatus={(s) => (status = s)}
        />
      {/key}
    </div>
  {:else}
    <div class="ct-exec-launch">
      <p class="ct-exec-desc">
        Opens an Otto terminal session running <code dir="ltr">docker exec -it {containerName}</code>
        (defaults to <code dir="ltr">bash</code> when installed, otherwise <code dir="ltr">sh</code>).
      </p>

      <div class="field ct-exec-cmd">
        <label for="ct-exec-cmd-input">Command (optional)</label>
        <input
          id="ct-exec-cmd-input"
          class="input ct-mono"
          dir="ltr"
          bind:value={customCommand}
          placeholder="Default interactive shell (bash / sh)"
          disabled={opening}
        />
      </div>

      <button type="button" class="btn primary" onclick={() => void openShell()} disabled={opening}>
        <Icon name="terminal" size={13} />
        {opening ? 'Opening shell…' : 'Open shell'}
      </button>

      {#if error}
        <p class="ct-exec-err">{error}</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .ct-exec {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-block-size: 0;
  }
  .ct-exec-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    font-size: var(--fs-s);
    flex-shrink: 0;
  }
  .ct-exec-title {
    font-family: var(--font-mono);
    font-weight: 500;
  }
  .ct-exec-status {
    color: var(--text-dim);
  }
  .ct-exec-spacer {
    flex: 1;
  }
  .ct-exec-term {
    flex: 1;
    min-block-size: 260px;
    background: var(--term-bg);
  }
  .ct-exec-launch {
    padding: 18px 16px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 10px;
    max-inline-size: 520px;
  }
  .ct-exec-desc {
    margin: 0;
    font-size: var(--fs-m);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .ct-exec-cmd {
    inline-size: 100%;
    margin-block-end: 2px;
  }
  .ct-mono {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .ct-exec-note {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 18px 16px;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .ct-exec-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
</style>
