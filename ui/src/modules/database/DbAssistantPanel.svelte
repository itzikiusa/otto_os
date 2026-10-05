<script lang="ts">
  // The DB Assistant: a file-backed, embedded agent that investigates the active
  // database connection. Its live SHELL (the same Terminal as Agents, reused) sits
  // right here BESIDE the query editor/results — the user types directly to the
  // agent in that terminal (a real two-way session), exactly like Canvas's
  // ConversationPanel. The agent runs read-only against the DB via a seeded `q`
  // tool and writes its proposed SQL, surfaced below with Insert / Run. The
  // session is hidden from the Agents section (meta.source = 'db_assist').
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import AgentByline from '../../lib/components/AgentByline.svelte';
  import LiveWorkingDot from '../../lib/components/LiveWorkingDot.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import type { DbAssistMode } from '../../lib/api/types';
  import { allProviders } from '../../lib/providers';

  let draft = $state('');

  // Surface a hint when a turn runs unusually long — the agent may be wedged
  // and Stop is the way out (it releases the input; the session stays live).
  let busyLong = $state(false);
  $effect(() => {
    if (!database.assistBusy) {
      busyLong = false;
      return;
    }
    const t = setTimeout(() => (busyLong = true), 20_000);
    return () => clearTimeout(t);
  });

  // Mode → the panel's title, empty-state hint, and the Ask placeholder.
  const MODE: Record<DbAssistMode, { title: string; hint: string; placeholder: string }> = {
    nl: {
      title: 'Ask in English',
      hint: 'Describe the query you want in plain English. The agent reads the full schema, can sample real data read-only, and proposes a runnable query below.',
      placeholder: 'e.g. top 10 customers by total order value last month',
    },
    ask: {
      title: 'Ask Otto',
      hint: 'Ask anything about this database — its schema, the data, or how to write a query. The agent inspects the live DB read-only and answers in its shell.',
      placeholder: 'Ask about the schema or the data…',
    },
    investigate: {
      title: 'Examine with Otto',
      hint: 'The agent is seeded with the current query and a sample of its result. Ask it to explain, dig in, or find an issue — it can sample more data read-only.',
      placeholder: 'What should the agent look into?',
    },
  };
  const info = $derived(MODE[database.assistMode]);

  // Provider picker (chosen BEFORE the first turn; the choice locks once a session
  // exists). Same source/defaulting as NewSession.
  const providers = $derived(allProviders());
  const defaultProvider = $derived(
    (typeof ws.current?.settings?.default_provider === 'string' &&
      ws.current.settings.default_provider) ||
      auth.meta?.default_provider ||
      '',
  );
  // Preselect the configured default agent when the panel opens with none chosen.
  $effect(() => {
    if (!database.assistProvider && providers.length > 0) {
      const def = defaultProvider && providers.includes(defaultProvider) ? defaultProvider : null;
      database.setAssistProvider(def ?? (providers.includes('claude') ? 'claude' : providers[0]));
    }
  });

  // Once the session is live the empty state (provider picker + Ask box) gives way
  // to the real, interactive terminal — that IS the conversation surface from here.
  const started = $derived(database.assistSessionId !== null);

  /** Closing discards the live session and its working files — confirm when
   *  there is a conversation or a proposal that would be lost. */
  async function close(): Promise<void> {
    if (database.assistSessionId || database.assistProposedSql.trim()) {
      const ok = await confirmer.ask(
        'Closing ends the agent’s session and deletes its working files, including the proposed query. Copy anything you want to keep first.',
        { title: 'Close the DB assistant?', confirmLabel: 'Close and discard', cancelLabel: 'Keep open', danger: true },
      );
      if (!ok) return;
    }
    await database.closeAssist();
  }

  async function send(): Promise<void> {
    const p = draft.trim();
    if (!p || database.assistBusy) return;
    draft = '';
    await database.startAssist(p);
  }
  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }
</script>

<section class="db-assist">
  <header class="da-head">
    <span class="da-title"><Icon name="zap" size={15} /> {info.title}</span>
    {#if !started && providers.length > 1}
      <select
        class="da-provider"
        value={database.assistProvider}
        onchange={(e) => database.setAssistProvider((e.currentTarget as HTMLSelectElement).value)}
        title="Which agent investigates this database"
      >
        {#each providers as p (p)}
          <option value={p}>{p}</option>
        {/each}
      </select>
    {/if}
    {#if database.assistBusy}
      <span class="da-working"><LiveWorkingDot label="Working…" /></span>
      <!-- Stop of a query/turn: neutral and immediate (patterns §7) — the
           session stays live, nothing is discarded. -->
      <button
        class="btn small da-stop"
        onclick={() => database.stopAssist()}
        title="Stop waiting on this turn — releases the input (a started session stays live)"
      >
        <Icon name="stop" size={12} /> Stop
      </button>
    {/if}
    <button
      class="btn small da-summarize"
      onclick={() => void database.summarizeAssist()}
      disabled={database.assistBusy || !database.assistId}
      title="Write a summary of this investigation and download it as Markdown"
    >
      <Icon name="download" size={13} /> Summarize
    </button>
    <button
      class="icon-btn"
      onclick={() => void close()}
      aria-label="Close DB assistant"
      title="Close — discards the session and working files"
    >
      <Icon name="x" size={15} />
    </button>
  </header>

  <!-- The live, fully-interactive shell of the chosen agent (the SAME Terminal as
       Agents). readOnly is FALSE — the user types directly to the agent here. -->
  <div class="da-shell">
    {#if database.assistSessionId}
      {#key database.assistSessionId}
        <Terminal sessionId={database.assistSessionId} readOnly={false} forceDark preferDom />
      {/key}
    {:else}
      <div class="da-empty">
        <!-- Nothing running yet: the shared empty state names the mode; the
             composer under it starts the agent. -->
        <EmptyState icon="db" title={info.title} body={info.hint} />
        {#if providers.length > 1}
          <div class="da-providers" role="radiogroup" aria-label="Agent">
            {#each providers as p (p)}
              <button
                class="da-prov"
                class:on={database.assistProvider === p}
                role="radio"
                aria-checked={database.assistProvider === p}
                onclick={() => database.setAssistProvider(p)}
              >{p}</button>
            {/each}
          </div>
        {/if}
        <div class="da-ask">
          <textarea dir="auto"
            bind:value={draft}
            onkeydown={onKey}
            placeholder={info.placeholder}
            rows="3"
            disabled={database.assistBusy}
          ></textarea>
          <button class="btn primary da-send" onclick={send} disabled={database.assistBusy || !draft.trim()}>
            {#if database.assistBusy}Starting…{:else}<Icon name="arrowUp" size={15} /> Ask{/if}
          </button>
        </div>
        {#if busyLong}
          <p class="sub warn">This is taking longer than usual — the agent may be stuck.
            You can Stop (top right) and ask again.</p>
        {/if}
        <p class="sub">The agent’s live shell appears here once it starts — you then
          keep the conversation going by typing directly in it.</p>
      </div>
    {/if}
  </div>

  <!-- The agent's proposed SQL (start response + live db_assist_updated). -->
  {#if database.assistProposedSql.trim()}
    <div class="da-sql">
      <div class="da-sql-head">
        <span class="da-sql-label"><Icon name="db" size={12} /> Proposed query</span>
        <AgentByline provider={database.assistProvider} label="Draft" />
        <span class="grow"></span>
        <button
          class="btn small ghost"
          onclick={() => {
            database.assistProposedSql = '';
            database.assistNote = '';
          }}
          title="Drop this proposed query (nothing is changed in your editor)"
        >
          Discard
        </button>
        <button
          class="btn small"
          onclick={() => void database.insertAssistSql()}
          title="Put this query into the active editor tab"
        >
          <Icon name="arrowDown" size={12} /> Insert into editor
        </button>
        <button
          class="btn small primary"
          onclick={() => void database.runAssistSql()}
          title="Insert this query into the editor and run it read-only — a write asks you first"
        >
          <Icon name="play" size={12} /> Run
        </button>
      </div>
      <pre class="da-sql-text mono">{database.assistProposedSql}</pre>
      {#if database.assistNote.trim()}
        <div class="da-sql-note">{database.assistNote}</div>
      {/if}
    </div>
  {/if}
</section>

<style>
  .db-assist {
    width: 100%;
    height: 100%;
    min-height: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    color: var(--text);
    overflow: hidden;
  }
  .da-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .da-title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-m);
    font-weight: 600;
    white-space: nowrap;
  }
  .da-provider {
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    padding: 2px 4px;
    cursor: pointer;
    text-transform: capitalize;
  }
  .da-working {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--accent-text);
    font-weight: 600;
  }
  /* Summarize is pushed to the far end; Stop sits beside "working…". */
  .da-summarize {
    margin-inline-start: auto;
  }
  .da-empty .sub.warn {
    color: var(--warning);
    opacity: 1;
  }
  .da-shell {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    position: relative;
    background: var(--term-bg);
  }
  .da-shell > :global(*) {
    flex: 1 1 auto;
    min-height: 0;
  }
  .da-empty {
    margin: auto;
    text-align: center;
    /* Sits on --term-bg, which tracks the app scheme — the standard tokens
       already read correctly on it (the live Terminal paints its own dark). */
    color: var(--text-dim);
    padding: 20px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    max-width: 360px;
  }
  .da-empty .sub {
    margin: 0;
    font-size: var(--fs-xs);
    line-height: 1.45;
    opacity: 0.8;
  }
  .da-providers {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
  }
  .da-prov {
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    border-radius: 999px;
    font-size: var(--fs-s);
    padding: 2px 10px;
    cursor: pointer;
    text-transform: capitalize;
  }
  .da-prov:hover {
    border-color: var(--accent);
    color: var(--text);
  }
  .da-prov.on {
    border-color: var(--accent);
    background: var(--accent-soft-strong);
    color: var(--text);
  }
  .da-ask {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .da-ask textarea {
    width: 100%;
    box-sizing: border-box;
    resize: none;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    padding: 8px 10px;
    outline: none;
  }
  .da-ask textarea:focus {
    border-color: var(--accent-text); box-shadow: 0 0 0 3px var(--accent-soft-strong)
  }
  .da-send {
    align-self: center;
  }
  /* Proposed-SQL block (read-only) with Insert / Run. */
  .da-sql {
    flex: none;
    border-top: 1px solid var(--border);
    background: var(--surface-2);
    max-height: 38%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .da-sql-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    flex: none;
  }
  .da-sql-label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .da-sql-text {
    margin: 0;
    padding: 8px 10px;
    overflow: auto;
    font-size: var(--fs-s);
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text);
    border-top: 1px solid var(--border);
  }
  .da-sql-note {
    flex: none;
    padding: 6px 10px;
    font-size: var(--fs-s);
    line-height: 1.45;
    color: var(--text-dim);
    border-top: 1px solid var(--border);
  }
</style>
