<script lang="ts">
  // The DB Assistant: a file-backed, embedded agent that investigates the active
  // database connection. Its live SHELL (the same Terminal as Agents, reused) sits
  // right here BESIDE the query editor/results — the user types directly to the
  // agent in that terminal (a real two-way session), exactly like Canvas's
  // ConversationPanel. The agent runs read-only against the DB via a seeded `q`
  // tool and writes its proposed SQL, surfaced below with Insert / Run. The
  // session is hidden from the Agents section (meta.source = 'db_assist').
  import Icon from '../../lib/components/Icon.svelte';
  import Terminal from '../../lib/components/Terminal.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import type { DbAssistMode } from '../../lib/api/types';

  let draft = $state('');

  // Mode → the panel's title, empty-state hint, and the Ask placeholder.
  const MODE: Record<DbAssistMode, { title: string; hint: string; placeholder: string }> = {
    nl: {
      title: 'Ask in English',
      hint: 'Describe the query you want in plain English. The agent reads the full schema, can sample real data read-only, and proposes a runnable query below.',
      placeholder: 'e.g. top 10 customers by total order value last month',
    },
    ask: {
      title: 'Ask AI',
      hint: 'Ask anything about this database — its schema, the data, or how to write a query. The agent inspects the live DB read-only and answers in its shell.',
      placeholder: 'Ask about the schema or the data…',
    },
    investigate: {
      title: 'Examine with AI',
      hint: 'The agent is seeded with the current query and a sample of its result. Ask it to explain, dig in, or find an issue — it can sample more data read-only.',
      placeholder: 'What should the agent look into?',
    },
  };
  const info = $derived(MODE[database.assistMode]);

  // Provider picker (chosen BEFORE the first turn; the choice locks once a session
  // exists). Same source/defaulting as NewSession.
  const providers = $derived(auth.meta?.providers ?? ['claude', 'codex', 'shell']);
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
    {#if database.assistBusy}<span class="da-working">working…</span>{/if}
    <button
      class="da-act"
      onclick={() => void database.summarizeAssist()}
      disabled={database.assistBusy || !database.assistId}
      title="Write a summary of this investigation and download it as Markdown"
    >
      <Icon name="arrowDown" size={13} /> Summarize
    </button>
    <button
      class="da-close"
      onclick={() => void database.closeAssist()}
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
        <Terminal sessionId={database.assistSessionId} readOnly={false} forceDark={true} />
      {/key}
    {:else}
      <div class="da-empty">
        <p class="lead">{info.title}</p>
        <p class="hint">{info.hint}</p>
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
          <textarea
            bind:value={draft}
            onkeydown={onKey}
            placeholder={info.placeholder}
            rows="3"
            disabled={database.assistBusy}
          ></textarea>
          <button class="da-send" onclick={send} disabled={database.assistBusy || !draft.trim()}>
            {#if database.assistBusy}Starting…{:else}<Icon name="arrowUp" size={15} /> Ask{/if}
          </button>
        </div>
        <p class="sub">The agent's live shell appears here once it starts — you then
          keep the conversation going by typing directly in it.</p>
      </div>
    {/if}
  </div>

  <!-- The agent's proposed SQL (start response + live db_assist_updated). -->
  {#if database.assistProposedSql.trim()}
    <div class="da-sql">
      <div class="da-sql-head">
        <span class="da-sql-label"><Icon name="db" size={12} /> Proposed query</span>
        <span class="grow"></span>
        <button
          class="da-sql-btn"
          onclick={() => database.insertAssistSql()}
          title="Put this query into the active editor tab"
        >
          <Icon name="send" size={11} /> Insert into editor
        </button>
        <button
          class="da-sql-btn primary"
          onclick={() => void database.runAssistSql()}
          title="Insert this query into the editor and run it"
        >
          <Icon name="play" size={11} /> Run
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
    gap: 7px;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
  }
  .da-provider {
    border: 1px solid var(--border);
    background: var(--bg);
    color: var(--text);
    border-radius: 6px;
    font-size: 11px;
    padding: 2px 5px;
    cursor: pointer;
    text-transform: capitalize;
  }
  .da-working {
    font-size: 11px;
    color: var(--accent);
    font-weight: 600;
  }
  .da-act {
    margin-inline-start: auto;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    border-radius: var(--radius-s);
    font-size: 11.5px;
    padding: 3px 8px;
    cursor: pointer;
  }
  .da-act:hover:not(:disabled) {
    border-color: var(--accent);
  }
  .da-act:disabled {
    opacity: 0.5;
    cursor: default;
  }
  .da-close {
    display: inline-flex;
    border: none;
    background: none;
    color: var(--text-dim, #888);
    cursor: pointer;
    padding: 4px;
    border-radius: 6px;
  }
  .da-close:hover {
    background: color-mix(in srgb, var(--text) 8%, transparent);
  }
  .da-shell {
    flex: 1 1 auto;
    min-height: 0;
    display: flex;
    position: relative;
    background: #131318;
  }
  .da-shell > :global(*) {
    flex: 1 1 auto;
    min-height: 0;
  }
  .da-empty {
    margin: auto;
    text-align: center;
    color: var(--text-dim, #aaa);
    padding: 20px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    max-width: 360px;
  }
  .da-empty .lead {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: #eee;
  }
  .da-empty .hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
  }
  .da-empty .sub {
    margin: 0;
    font-size: 11px;
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
    border: 1px solid color-mix(in srgb, #fff 18%, transparent);
    background: color-mix(in srgb, #fff 6%, transparent);
    color: #ddd;
    border-radius: 999px;
    font-size: 11.5px;
    padding: 3px 11px;
    cursor: pointer;
    text-transform: capitalize;
  }
  .da-prov:hover {
    border-color: var(--accent);
    color: #fff;
  }
  .da-prov.on {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: #fff;
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
    border: 1px solid color-mix(in srgb, #fff 18%, transparent);
    border-radius: 10px;
    background: color-mix(in srgb, #fff 5%, transparent);
    color: #eee;
    font: inherit;
    font-size: 13px;
    padding: 8px 10px;
    outline: none;
  }
  .da-ask textarea:focus {
    border-color: var(--accent);
  }
  .da-send {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    align-self: center;
    border: none;
    border-radius: 999px;
    background: var(--accent);
    color: #fff;
    font-size: 12.5px;
    font-weight: 600;
    padding: 6px 16px;
    cursor: pointer;
  }
  .da-send:disabled {
    opacity: 0.45;
    cursor: default;
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
    font-size: 11.5px;
    font-weight: 600;
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .da-sql-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    border-radius: var(--radius-s);
    font-size: 11.5px;
    padding: 3px 9px;
    cursor: pointer;
  }
  .da-sql-btn:hover {
    border-color: var(--accent);
  }
  .da-sql-btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .da-sql-text {
    margin: 0;
    padding: 8px 10px;
    overflow: auto;
    font-size: 12px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--text);
    border-top: 1px solid var(--border);
  }
  .da-sql-note {
    flex: none;
    padding: 6px 10px;
    font-size: 11.5px;
    line-height: 1.45;
    color: var(--text-dim);
    border-top: 1px solid var(--border);
  }
</style>
