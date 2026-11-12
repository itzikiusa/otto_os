<script lang="ts">
  // The Connections hub's SSH / custom-CLI terminal pane.
  //
  // ONE Terminal for every SSH tab, with `keepAlive` — the Agents pattern
  // (SessionView): switching tabs hands the new session id to the same
  // Terminal, which PARKS the old session's xterm + socket (termPark.ts:
  // LRU- and byte-bounded, TTL'd) and adopts the new one's. Leaving for a DB
  // tab, Structure or another module parks it too. It used to sit in a
  // `{#key sessionId}` without keepAlive, so every switch disposed it and
  // replayed 4,000 lines on return — losing the selection (what ⌘C copies),
  // the scroll position and the find state.
  //
  // When the session has ended, SshExitPanel says why and offers Reconnect
  // (the ONE restart control: the Terminal's own "Restart session" overlay
  // button is not enabled here) and "Diagnose with an agent…".
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { PRIMARY_SCROLLBACK } from '../../lib/components/termFlow';
  import type { Connection, SessionStatus, TestConnectionResp } from '../../lib/api/types';
  import type { SshTab } from '../../lib/stores/database/model';
  import SshExitPanel from './SshExitPanel.svelte';
  import DiagnoseButton from './DiagnoseButton.svelte';
  import { probeOf } from './diagnose-prompt';
  import { sshTabs } from './ssh-tabs.svelte';

  interface Props {
    tab: SshTab;
    conn: Connection | null;
    onreconnect: () => void;
    onedit?: (c: Connection) => void;
  }
  let { tab, conn, onreconnect, onedit }: Props = $props();

  const TerminalLazy = lazyComponent(() => import('../../lib/components/Terminal.svelte'));

  const sessionId = $derived(tab.sessionId);
  const exited = $derived(sshTabs.liveness(sessionId) === 'exited');
  // Bound to the session it reports for, so a frame can never be filed
  // under the tab the pane switched to.
  const onstatus = $derived((s: SessionStatus) => sshTabs.report(sessionId, s));
</script>

<div class="ssh-pane">
  <div class="term-pane">
    <LazyMount
      lazy={TerminalLazy}
      what="the terminal"
      props={{ sessionId, scrollback: PRIMARY_SCROLLBACK, keepAlive: true, onrestart: onreconnect, onstatus }}
    />
  </div>
  {#if exited}
    <SshExitPanel {conn} name={tab.name} kind={tab.kind} {onreconnect} {onedit} extra={conn ? diagnose : undefined} />
  {/if}
</div>

{#snippet diagnose({ result }: { result: TestConnectionResp | null })}
  {#if conn && conn.kind === 'ssh' && (!result || !result.ok)}
    <DiagnoseButton {conn} probe={result ? probeOf(result) : null} />
  {/if}
{/snippet}

<style>
  .ssh-pane {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  /* The terminal fills the workbench body below the tab strip. */
  .term-pane {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
    overflow: hidden;
  }
  .term-pane :global(> *) {
    flex: 1;
    min-height: 0;
    min-width: 0;
  }
</style>
