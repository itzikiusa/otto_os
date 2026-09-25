<script lang="ts">
  // Agent UI control: "Claude · ‹session› is driving Connections" — the one
  // strip that says an agent is operating THIS document (the side pane beside
  // its session, usually). Like a browser's "controlled by automated software"
  // infobar: it sits directly under the page's toolbar (the first PageHeader
  // hosts it; the shell shows it above pages without one), stays while a
  // command runs and for a minute after the last one, then goes by itself.
  //
  //   [mark] Claude · Refactor billing  is driving Connections  ● Run query…   Recent  ■ Stop  ✕
  //
  // Stop turns UI control off for that session (the daemon cancels anything
  // pending; the agent's next call asks the user again). Recent lists this
  // document's last 20 agent actions. Nothing here pulses under reduced motion.
  import Icon from './Icon.svelte';
  import ProviderIcon from './ProviderIcon.svelte';
  import { uiControl, agentName } from '../stores/uiControl.svelte';
  import { now, rel } from '../stores/now.svelte';
  import { moduleLabel } from '../sidebar';
  import { paneKey, routeOf } from '../sidePane';
  import { router } from '../router.svelte';
  import { providerName } from '../uiCommands/frames';
  import { ctxMenu, type MenuItem } from '../contextmenu.svelte';

  const d = $derived(uiControl.driving);
  const show = $derived(uiControl.visible(now()));
  const last = $derived(uiControl.recent.find((r) => d && r.agent.session_id === d.agent.session_id) ?? null);
  /** The module being driven; a shell command (state/open) names the page. */
  const module = $derived.by(() => {
    const m = d?.module && d.module !== 'shell' ? d.module : paneKey(routeOf(router.parts.join('/')));
    return moduleLabel(m);
  });
  const who = $derived(d ? providerName(d.agent.provider) : '');
  const title = $derived(d?.agent.title.trim() ?? '');
  let stopping = $state(false);

  async function stop(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await uiControl.stop();
    } finally {
      stopping = false;
    }
  }

  function openRecent(e: MouseEvent | KeyboardEvent): void {
    const many = new Set(uiControl.recent.map((r) => r.agent.session_id)).size > 1;
    const items: MenuItem[] = uiControl.recent.map((r) => ({
      label: r.note && r.ok === null ? `${r.label} — ${r.note}` : r.label,
      icon: r.ok === null ? 'clock' : r.ok ? 'check' : 'x',
      hint: many ? `${providerName(r.agent.provider)} · ${rel(r.at)}` : rel(r.at),
      title: r.error ? `${r.label} failed: ${r.error}` : `${agentName(r.agent)} · ${new Date(r.at).toLocaleTimeString()}`,
    }));
    if (items.length === 0) items.push({ label: 'No actions yet', disabled: true });
    ctxMenu.show(e, items);
  }
</script>

{#if show && d}
  <div
    class="adb"
    class:waiting={d.awaitingHuman}
    role="region"
    aria-label="Agent control"
    data-testid="agent-driving-bar"
    data-session={d.agent.session_id}
  >
    <span class="adb-mark" aria-hidden="true"><ProviderIcon provider={d.agent.provider || 'agent'} size={14} /></span>
    <span class="adb-who" title={agentName(d.agent)}>
      <strong>{who}</strong>{#if title}<span class="adb-title">{` · ${title}`}</span>{/if}
    </span>
    <span class="adb-verb">is driving {module}</span>
    <span class="adb-now" role="status" aria-live="polite">
      {#if d.awaitingHuman}
        <span class="adb-dot warn" aria-hidden="true"></span>Waiting for you to confirm
      {:else if d.running > 0}
        <span class="adb-dot live" aria-hidden="true"></span>{d.current ?? 'Working'}…
      {:else if last}
        <span class="adb-last">Last: {last.label}{last.ok === false ? ' (failed)' : ''} · {rel(last.at)}</span>
      {/if}
    </span>
    <span class="adb-grow"></span>
    <button
      class="btn small ghost adb-btn"
      onclick={openRecent}
      aria-haspopup="menu"
      aria-label="Recent agent actions"
      title="This window's recent agent actions"
      data-testid="agent-driving-recent"
    ><Icon name="clock" size={12} /><span class="adb-btn-label">Recent</span></button>
    <button
      class="btn small adb-btn"
      onclick={stop}
      disabled={stopping}
      title="Stop — turn off UI control for “{title || who}”. It asks you again before driving Otto."
      aria-label="Stop {who} driving Otto"
      data-testid="agent-driving-stop"
    ><Icon name="stop" size={12} /><span class="adb-btn-label">Stop</span></button>
    {#if d.running === 0}
      <button
        class="icon-btn adb-hide"
        onclick={() => uiControl.dismiss()}
        title="Hide until the agent acts again"
        aria-label="Hide until the agent acts again"
      ><Icon name="x" size={12} /></button>
    {/if}
  </div>
{/if}

<style>
  .adb {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    min-height: 30px;
    padding-block: 3px;
    padding-inline: 20px 12px;
    border-top: 1px solid var(--separator);
    /* Agent identity is a neutral 2 px start rule (patterns §2), the live
       tint the Running tone; amber only while it waits on you (§1). */
    box-shadow: inset 2px 0 0 var(--border-strong);
    background: var(--info-soft);
    color: var(--text);
    font-size: var(--fs-s);
    cursor: default;
    user-select: none;
    -webkit-user-select: none;
  }
  :global([dir='rtl']) .adb {
    box-shadow: inset -2px 0 0 var(--border-strong);
  }
  .adb.waiting {
    background: var(--warning-soft);
  }
  .adb-mark {
    display: inline-flex;
    flex-shrink: 0;
  }
  .adb-who {
    min-width: 0;
    max-width: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex-shrink: 1;
  }
  .adb-title {
    color: var(--text);
  }
  .adb-verb {
    color: var(--text-dim);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .adb-now {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim);
    flex-shrink: 1;
  }
  .adb.waiting .adb-now {
    color: var(--warning);
    font-weight: 500;
  }
  .adb-last {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .adb-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .adb-dot.live {
    background: var(--info);
    animation: adb-pulse 1.4s ease-in-out infinite;
  }
  .adb-dot.warn {
    background: var(--status-warn);
    animation: adb-pulse 1.4s ease-in-out infinite;
  }
  @keyframes adb-pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .adb-dot.live,
    .adb-dot.warn {
      animation: none;
    }
  }
  .adb-grow {
    flex: 1 1 0;
    min-width: 0;
  }
  .adb-btn {
    flex-shrink: 0;
    gap: 4px;
  }
  .adb-hide {
    flex-shrink: 0;
  }
  /* Narrow panes keep the controls and "is driving …": the last-action text
     ellipsizes first, then (phone widths) the button labels go. In the side
     pane the iframe IS the viewport, so these track the pane. */
  @media (max-width: 520px) {
    .adb-last {
      display: none;
    }
  }
  @media (max-width: 640px) {
    .adb-btn-label {
      display: none;
    }
    .adb {
      padding-inline: 12px 8px;
    }
  }
</style>
