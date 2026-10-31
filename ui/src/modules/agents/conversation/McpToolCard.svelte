<script lang="ts">
  // Rich inline cards rendered inside ToolStep for session-to-session MCP tools
  // (Peer Session Card) and database MCP tools (Database Query Card).
  import Icon from '../../../lib/components/Icon.svelte';
  import Badge from '../../../lib/components/Badge.svelte';
  import type { BadgeTone } from '../../../lib/status';
  import { plural } from '../../../lib/plural';
  import { ensureHljs, highlightBlock } from '../../../lib/hl';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { database } from '../../../lib/stores/database.svelte';
  import { router } from '../../../lib/router.svelte';
  import type { StepStatus } from './format';
  import type { RichMcpCardData } from './mcpToolInfo';

  interface Props {
    card: RichMcpCardData;
    status: StepStatus;
  }
  let { card, status }: Props = $props();

  let hlReady = $state(false);
  $effect(() => {
    if (card.kind !== 'db' || !card.sql) return;
    void ensureHljs().then(() => (hlReady = true));
  });

  const sqlHtml = $derived(
    card.kind === 'db' && card.sql ? highlightBlock(card.sql, hlReady ? 'sql' : null) : '',
  );

  const peerTitle = $derived.by(() => {
    if (card.kind !== 'peer') return '';
    if (card.targetId) {
      const live = ws.getSession(card.targetId);
      if (live?.title) return live.title;
    }
    return card.targetFallback;
  });

  const connLabel = $derived.by(() => {
    if (card.kind !== 'db') return '';
    if (card.connectionId) {
      const hit = database.connections.find((c) => c.id === card.connectionId);
      if (hit?.name) return hit.name;
      return card.connectionId;
    }
    return 'Workspace DB';
  });

  const badgeProps = $derived.by((): { tone: BadgeTone; label: string; live: boolean } => {
    if (status === 'running') {
      return {
        tone: 'info',
        label: card.kind === 'peer' ? (card.op === 'wait' ? 'Waiting…' : 'Active…') : 'Running…',
        live: true,
      };
    }
    if (status === 'err') return { tone: 'bad', label: 'Failed', live: false };
    if (status === 'none') return { tone: 'neutral', label: 'No result', live: false };
    if (card.kind === 'peer') {
      const label =
        card.op === 'open'
          ? 'Spawned'
          : card.op === 'send' || card.op === 'broadcast'
            ? 'Sent'
            : 'Done';
      return { tone: 'ok', label, live: false };
    }
    return { tone: 'ok', label: 'Completed', live: false };
  });

  function formatDuration(ms: number): string {
    return ms < 1000 ? `${Math.round(ms)} ms` : `${(ms / 1000).toFixed(2)} s`;
  }

  function openPeerThread(targetId: string): void {
    ws.openSession(targetId);
    router.go(
      router.module === 'threads'
        ? `threads/${encodeURIComponent(targetId)}`
        : `agents/${encodeURIComponent(targetId)}`,
    );
  }

  function openInConnections(connId: string | null): void {
    if (connId) void database.openConnection(connId);
    router.go('database');
  }
</script>

{#if card.kind === 'peer'}
  <div class="mcp-card peer-card" data-peer-card={card.op}>
    <div class="mcp-head">
      <span class="mcp-glyph peer" aria-hidden="true"><Icon name="people" size={14} /></span>
      <div class="mcp-titles">
        <span class="mcp-kicker">{card.actionLabel}</span>
        <span class="mcp-primary">{peerTitle}</span>
        {#if card.targetId && card.targetId !== peerTitle}
          <span class="mcp-sub mono" dir="ltr">{card.targetId.slice(0, 12)}</span>
        {/if}
      </div>
      <Badge tone={badgeProps.tone} label={badgeProps.label} live={badgeProps.live} variant="outline" />
      {#if card.targetId}
        <button
          type="button"
          class="btn small"
          onclick={() => openPeerThread(card.targetId!)}
          title="Switch to peer session {peerTitle}"
        >
          Open thread <Icon name="external" size={12} />
        </button>
      {/if}
    </div>
    {#if card.snippet}
      <div class="peer-snippet">{card.snippet}</div>
    {/if}
  </div>
{:else}
  <div class="mcp-card db-card" data-db-card={card.op}>
    <div class="mcp-head">
      <span class="mcp-glyph db" aria-hidden="true"><Icon name="db" size={14} /></span>
      <div class="mcp-titles">
        <span class="mcp-kicker">{card.actionLabel}</span>
        <span class="mcp-primary mono" dir="ltr">{connLabel}</span>
        {#if card.scope}
          <span class="mcp-sub mono" dir="ltr">· {card.scope}</span>
        {/if}
      </div>
      {#if card.rowCount !== null}
        <span class="db-metric mono" dir="ltr">{plural(card.rowCount, 'row')}</span>
      {/if}
      {#if card.durationMs !== null}
        <span class="db-metric mono" dir="ltr">{formatDuration(card.durationMs)}</span>
      {/if}
      <Badge tone={badgeProps.tone} label={badgeProps.label} live={badgeProps.live} variant="outline" />
      <button
        type="button"
        class="btn small"
        onclick={() => openInConnections(card.connectionId)}
        title="Open {connLabel} in Database Explorer"
      >
        Open in Connections <Icon name="external" size={12} />
      </button>
    </div>
    {#if card.sql}
      <pre class="db-sql mono hljs" dir="ltr">{@html sqlHtml}</pre>
    {/if}
  </div>
{/if}

<style>
  .mcp-card {
    margin-block: 2px 6px;
    margin-inline: 28px 8px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .peer-card {
    border-inline-start: 2px solid var(--accent-line-strong);
  }
  .db-card {
    border-inline-start: 2px solid var(--info);
  }
  .mcp-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    min-width: 0;
  }
  .mcp-glyph {
    display: inline-grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-s);
    flex-shrink: 0;
  }
  .mcp-glyph.peer {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .mcp-glyph.db {
    background: var(--info-soft);
    color: var(--info);
  }
  .mcp-titles {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 6px;
    flex: 1 1 180px;
    min-width: 0;
  }
  .mcp-kicker {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .mcp-primary {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
    unicode-bidi: isolate;
  }
  .mcp-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    unicode-bidi: isolate;
  }
  .db-metric {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    padding: 2px 6px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    border: 1px solid var(--border);
    white-space: nowrap;
    unicode-bidi: isolate;
  }
  .peer-snippet {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.45;
    padding-inline-start: 32px;
    unicode-bidi: plaintext;
    text-align: start;
    overflow-wrap: anywhere;
  }
  .db-sql {
    margin: 0;
    padding: 6px 10px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-xs);
    line-height: 1.5;
    max-height: 160px;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    text-align: start;
    unicode-bidi: isolate;
  }
</style>
