<script lang="ts">
  // SQL / Redis / Mongo editor. Wraps the shared CodeEditor with a server-backed
  // completion source (debounced /db/completion). Cmd/Ctrl+Enter runs; toolbar
  // has Run / Save / Explain-with-agent. Results render in the ResultsGrid below.
  import { tick } from 'svelte';
  import type { Completion, CompletionContext, CompletionResult } from '@codemirror/autocomplete';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import ResultsGrid from './ResultsGrid.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { database, ROW_LIMIT_ALL, type QueryTab } from '../../lib/stores/database.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { dbNlToSql, ApiError } from '../../lib/api/client';
  import type { DbCompletionKind, NlToSqlOutcome } from '../../lib/api/types';
  import {
    statementAtCursor,
    extractVars,
    substituteVars,
    renderVar,
    defaultVarSpec,
    type SplitMode,
    type VarSpec,
  } from './sql-util';

  const tab = $derived(database.tab);

  // Default row-cap options (applied when a statement has no explicit LIMIT).
  const ROW_LIMIT_OPTS: { label: string; value: number }[] = [
    { label: '100', value: 100 },
    { label: '500', value: 500 },
    { label: '1,000', value: 1000 },
    { label: '5,000', value: 5000 },
    { label: '10,000', value: 10000 },
    { label: '50,000', value: 50000 },
    { label: 'All', value: ROW_LIMIT_ALL },
  ];

  // Editor language: a small Redis highlighter for redis; SQL for everything else
  // (mysql, clickhouse, and Mongo's SQL subset).
  const lang = $derived(database.queryLanguage === 'redis' ? 'redis' : 'sql');
  // Re-key the editor on tab id + engine so it rebuilds cleanly per query tab.
  const editorPath = $derived(`query-${tab.id}.${lang}`);
  // Statement separator: redis is one command per line; others use `;`.
  const splitMode = $derived<SplitMode>(database.queryLanguage === 'redis' ? 'line' : 'sql');
  // Live selection + cursor (from CodeEditor) → run only the selected/current
  // statement instead of the whole buffer.
  let editorSel = $state<{ text: string; cursor: number }>({ text: '', cursor: 0 });
  // Variables the current tab's statement references (:name / {name}).
  const queryVars = $derived(extractVars(tab.statement, splitMode));
  let varsBarEl = $state<HTMLElement | null>(null);

  // Reset the tracked selection/cursor when switching query tabs, so a stale
  // selection from another tab can never run against the newly-active one.
  $effect(() => {
    void tab.id;
    editorSel = { text: '', cursor: 0 };
  });

  // ── Tab labels ────────────────────────────────────────────────────────────
  // Derive a short, human label from a tab's SQL: prefer an explicit user name,
  // else the table after FROM/INTO/UPDATE, else a leading keyword snippet,
  // falling back to "Query N".
  function tabLabel(t: QueryTab, index: number): string {
    if (t.name && t.name !== 'Query') return t.name;
    const sql = t.statement.trim();
    if (!sql) return `Query ${index + 1}`;
    const from = sql.match(/\b(?:from|into|update|join)\s+`?([\w.$]+)`?/i);
    if (from) {
      const obj = from[1].split('.').pop() ?? from[1];
      const verb = sql.match(/^\s*(\w+)/)?.[1]?.toUpperCase() ?? '';
      return verb && verb !== 'SELECT' ? `${verb} ${obj}` : obj;
    }
    const verb = sql.match(/^\s*(\w+)/)?.[1];
    if (verb) return verb.length > 18 ? `${verb.slice(0, 18)}…` : verb;
    return `Query ${index + 1}`;
  }

  // Inline rename (double-click a chip).
  let renaming = $state<number | null>(null);
  let renameText = $state('');
  function startRename(i: number, t: QueryTab): void {
    renaming = i;
    renameText = t.name && t.name !== 'Query' ? t.name : tabLabel(t, i);
  }
  function commitRename(t: QueryTab): void {
    const v = renameText.trim();
    if (v) t.name = v;
    renaming = null;
  }

  // Map server completion kinds → CodeMirror completion "type" (drives the icon).
  function cmType(kind: DbCompletionKind): string {
    switch (kind) {
      case 'keyword':
        return 'keyword';
      case 'function':
        return 'function';
      case 'table':
      case 'view':
      case 'collection':
        return 'class';
      case 'column':
      case 'field':
        return 'property';
      case 'database':
        return 'namespace';
      case 'command':
        return 'method';
      case 'operator':
        return 'operator';
      default:
        return 'variable';
    }
  }

  // Word boundary the completion replaces (identifiers, incl. dotted prefixes).
  const TOKEN_RE = /[\w$.]*$/;

  // Server-driven completion source. Debounced via a shared in-flight promise so
  // fast typing collapses to the latest prefix. Failures degrade to no results.
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  function completionSource(ctx: CompletionContext): Promise<CompletionResult | null> {
    const before = ctx.matchBefore(TOKEN_RE);
    const word = before?.text ?? '';
    // Only auto-open when there's a token or the user explicitly triggered.
    if (!ctx.explicit && word.length === 0) return Promise.resolve(null);

    const from = before ? before.from : ctx.pos;
    const prefix = ctx.state.sliceDoc(0, ctx.pos);

    return new Promise((resolve) => {
      if (debounceTimer) clearTimeout(debounceTimer);
      debounceTimer = setTimeout(async () => {
        const items = await database.complete(prefix);
        if (items.length === 0) {
          resolve(null);
          return;
        }
        const options: Completion[] = items.map((it) => ({
          label: it.label,
          type: cmType(it.kind),
          detail: it.detail ?? undefined,
          apply: it.insert_text ?? undefined,
        }));
        resolve({ from, options, validFor: TOKEN_RE });
      }, 120);
    });
  }

  function run(): void {
    const whole = tab.statement;
    // Run the selection if there is one, else the statement under the cursor.
    const base = editorSel.text.trim()
      ? editorSel.text
      : statementAtCursor(whole, editorSel.cursor, splitMode);
    if (!base.trim()) {
      void database.runQuery();
      return;
    }
    // Query-level variables: every :name / {name} in the chosen statement needs
    // a value before it can run.
    const names = extractVars(base, splitMode);
    const missing = names.filter((n) => !(tab.vars[n]?.value ?? '').trim());
    if (missing.length > 0) {
      toasts.error(
        'Missing variable value',
        `Set a value for ${missing.map((n) => ':' + n).join(', ')}`,
      );
      void tick().then(() => {
        const inputs = varsBarEl ? Array.from(varsBarEl.querySelectorAll('input')) : [];
        (inputs.find((i) => !i.value.trim()) ?? inputs[0])?.focus();
      });
      return;
    }
    // Render each variable per its type/escape (string → quoted+escaped, number →
    // raw, raw → verbatim), then substitute.
    const rendered = Object.fromEntries(
      names.map((n) => [n, renderVar(tab.vars[n] ?? defaultVarSpec(), splitMode)]),
    );
    const finalSql = names.length > 0 ? substituteVars(base, rendered, splitMode) : base;
    void database.runQuery(finalSql, undefined, { transient: true });
  }

  // Draggable split between the editor and the results (persisted px height).
  let editorH = $state(loadEditorH());
  let resizing = $state(false);
  // Height to restore when collapsing the double-click "expand" toggle.
  let prevEditorH = $state(0);
  function loadEditorH(): number {
    if (typeof localStorage === 'undefined') return 240;
    const v = Number(localStorage.getItem('db.editorH'));
    return Number.isFinite(v) && v > 80 ? v : 240;
  }
  // Tallest the editor may grow to — viewport-relative so it can take most of the
  // pane on big screens, while always reserving room for the results grid so it
  // can never be squeezed off-screen. (≥180 keeps it sane on short windows.)
  function maxEditorH(): number {
    const vh = typeof window !== 'undefined' ? window.innerHeight : 1000;
    return Math.max(180, vh - 260);
  }
  function persistEditorH(): void {
    try {
      localStorage.setItem('db.editorH', String(Math.round(editorH)));
    } catch {
      /* ignore */
    }
  }
  function startResize(e: PointerEvent): void {
    e.preventDefault();
    resizing = true;
    const startY = e.clientY;
    const startH = editorH;
    const onMove = (ev: PointerEvent): void => {
      editorH = Math.max(100, Math.min(maxEditorH(), startH + (ev.clientY - startY)));
    };
    const onUp = (): void => {
      resizing = false;
      persistEditorH();
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }
  // Double-click the grip to expand the editor to (nearly) full height, and again
  // to restore the prior height — a quick way to focus on a long query.
  function toggleExpand(): void {
    const max = maxEditorH();
    if (editorH < max - 8) {
      prevEditorH = editorH;
      editorH = max;
    } else {
      editorH = prevEditorH > 80 ? prevEditorH : 240;
    }
    persistEditorH();
  }

  // ── Save query ──────────────────────────────────────────────────────────
  let saving = $state(false);
  let saveName = $state('');
  async function openSave(): Promise<void> {
    saveName = tab.name && tab.name !== 'Query' ? tab.name : '';
    saving = true;
    await tick();
  }
  async function confirmSave(): Promise<void> {
    const name = saveName.trim();
    if (!name) return;
    const saved = await database.saveQuery(name, tab.statement);
    if (saved) {
      tab.name = saved.name;
      saving = false;
      saveName = '';
    }
  }

  function explain(): void {
    const t = tab;
    let content = `Statement:\n${t.statement}`;
    if (t.result) {
      const cols = t.result.columns.map((c) => c.name).join(', ');
      content += `\n\nColumns: ${cols}\nRows returned: ${t.result.stats.row_count} in ${t.result.stats.duration_ms} ms`;
      const sample = t.result.rows.slice(0, 5);
      if (sample.length) content += `\n\nSample rows:\n${JSON.stringify(sample, null, 2)}`;
    } else if (t.error) {
      content += `\n\nError:\n${t.error}`;
    }
    void database.explainWithAgent(
      content,
      'Explain this query and its result. Suggest optimizations if relevant.',
      'Explain query',
    );
  }

  const canEdit = $derived(ws.myRole !== 'viewer');

  // ── Ask in English — verified NL→SQL (0001) ──────────────────────────────────
  // Available for SQL engines + Mongo (EXPLAIN-validation backs both); hidden for
  // Redis. The drafter loop only ever returns an EXPLAIN-validated READ query, so
  // nothing reaches the editor until it has a valid plan.
  const nlAvailable = $derived(
    database.queryLanguage === 'sql' || database.queryLanguage === 'mongo',
  );
  let nlOpen = $state(false); // the input row is toggled by the toolbar button
  let nlQuestion = $state('');
  let nlBusy = $state(false);
  let nlOutcome = $state<NlToSqlOutcome | null>(null);
  let nlError = $state<string | null>(null); // verbatim hint / message
  let nlPlanOpen = $state(false);

  async function askEnglish(): Promise<void> {
    const q = nlQuestion.trim();
    if (!q || nlBusy || !database.selectedConnId) return;
    nlBusy = true;
    nlOutcome = null;
    nlError = null;
    try {
      const out = await dbNlToSql(database.selectedConnId, {
        question: q,
        node: database.activeDb ?? undefined,
        max_attempts: 3,
      });
      nlOutcome = out;
      nlPlanOpen = false;
    } catch (e) {
      // Two expected 400s: no drafter configured, or the loop was exhausted
      // (its message carries the last engine error — show it verbatim).
      if (e instanceof ApiError && e.message.startsWith('NL-to-SQL is not configured')) {
        nlError = 'Ask AI is not set up on this server.';
      } else if (e instanceof ApiError) {
        nlError = e.message;
      } else {
        nlError = e instanceof Error ? e.message : String(e);
      }
    } finally {
      nlBusy = false;
    }
  }

  /** Put the validated SQL into the active tab; optionally run it via the normal
   *  run path. Closes the NL panel afterward. */
  function useNlSql(opts: { run: boolean }): void {
    if (!nlOutcome) return;
    database.setStatement(nlOutcome.sql);
    editorSel = { text: '', cursor: 0 };
    if (opts.run) void database.runQuery();
    nlOutcome = null;
    nlOpen = false;
    nlQuestion = '';
  }

  // ── Phone accordion ────────────────────────────────────────────────────────
  // On a phone the editor and the results are each collapsible, independently
  // scrolling blocks (tap a header to expand/minimise). Default: editor open,
  // results auto-open once a query has produced something. Inert on desktop —
  // the headers only render when isPhone.
  let editorOpen = $state(true);
  let resultsOpen = $state(true);
  const hasResult = $derived(!!tab.result || !!tab.error);
</script>

<div class="query-editor">
  <div class="qe-tabs" role="tablist" aria-label="Query tabs">
    {#each database.tabs as t, i (t.id)}
      <div
        class="qe-tab"
        class:active={i === database.activeTab}
        role="tab"
        tabindex="0"
        aria-selected={i === database.activeTab}
        onclick={() => database.switchTab(i)}
        ondblclick={() => startRename(i, t)}
        onkeydown={(e) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            database.switchTab(i);
          }
        }}
      >
        {#if renaming === i}
          <!-- svelte-ignore a11y_autofocus -->
          <input
            class="qe-tab-rename mono"
            bind:value={renameText}
            autofocus
            onclick={(e) => e.stopPropagation()}
            onblur={() => commitRename(t)}
            onkeydown={(e) => {
              e.stopPropagation();
              if (e.key === 'Enter') commitRename(t);
              else if (e.key === 'Escape') renaming = null;
            }}
          />
        {:else}
          <span class="qe-tab-label">{tabLabel(t, i)}</span>
          {#if t.running}<span class="qe-tab-dot running" title="Running"></span>
          {:else if t.error}<span class="qe-tab-dot error" title="Error"></span>{/if}
          {#if database.tabs.length > 1}
            <button
              class="qe-tab-close"
              title="Close tab"
              aria-label="Close tab"
              onclick={(e) => {
                e.stopPropagation();
                database.closeTab(i);
              }}
            >
              <Icon name="x" size={10} />
            </button>
          {/if}
        {/if}
      </div>
    {/each}
    <button class="qe-tab-new" title="New query tab" aria-label="New query tab" onclick={() => database.newTab()}>
      <Icon name="plus" size={12} />
    </button>
  </div>

  <div class="qe-toolbar">
    {#if tab.running}
      <button class="btn small stop" onclick={() => database.abortQuery()} title="Stop the running query">
        <Icon name="x" size={12} />
        Stop
      </button>
    {:else}
      <button class="btn small primary" onclick={run} disabled={!database.selectedConnId}>
        <Icon name="play" size={12} />
        Run
        <span class="kbd">⌘↵</span>
      </button>
    {/if}
    {#if canEdit}
      <button class="btn small" onclick={openSave} disabled={!tab.statement.trim()}>
        <Icon name="check" size={11} />Save
      </button>
    {/if}
    <button
      class="btn small ghost"
      onclick={() => void database.runExplain()}
      disabled={!tab.statement.trim() || tab.running}
      title="Run the real query plan (EXPLAIN / Mongo explain)"
    >
      <Icon name="zap" size={11} />Explain
    </button>
    <button class="btn small ghost" onclick={explain} disabled={!tab.statement.trim()} title="Ask an agent to explain this query">
      <Icon name="comment" size={11} />Ask AI
    </button>
    {#if nlAvailable}
      <button
        class="btn small ghost"
        class:on={nlOpen}
        onclick={() => (nlOpen = !nlOpen)}
        disabled={!database.selectedConnId}
        title="Describe what you want in plain English — the agent drafts a query and validates it with EXPLAIN before showing it"
      >
        <Icon name="comment" size={11} />Ask in English
      </button>
    {/if}
    {#if database.queryLanguage !== 'redis'}
      <button
        class="btn small ghost"
        onclick={() => {
          database.formatStatement();
          editorSel = { text: '', cursor: 0 };
        }}
        disabled={!tab.statement.trim() || tab.running}
        title="Format / beautify the SQL"
      >
        <Icon name="command" size={11} />Format
      </button>
    {/if}
    <span class="grow"></span>
    {#if database.capabilities?.sql && database.databaseNames.length > 0}
      <label class="qe-db" title="Active database — queries run scoped to it, so you don't need a db. prefix">
        <Icon name="db" size={11} />
        <select
          class="input"
          value={database.activeDb ?? ''}
          onchange={(e) => database.setActiveDb((e.currentTarget as HTMLSelectElement).value || null)}
        >
          <option value="">No active DB</option>
          {#each database.databaseNames as db (db)}
            <option value={db}>{db}</option>
          {/each}
        </select>
      </label>
    {:else if database.isRedis && database.keyspaces.length > 0}
      <label class="qe-db" title="Active Redis database — commands (GET, HGETALL, …) run against this DB">
        <Icon name="db" size={11} />
        <select
          class="input"
          value={database.activeDb ?? database.keyspaces[0]?.id ?? ''}
          onchange={(e) => database.setActiveDb((e.currentTarget as HTMLSelectElement).value || null)}
        >
          {#each database.keyspaces as ks (ks.id)}
            <option value={ks.id}>{ks.label}</option>
          {/each}
        </select>
      </label>
    {/if}
    <label class="qe-limit" title="Default row cap — an explicit LIMIT in your query overrides this">
      <span>Limit</span>
      <select
        class="input"
        value={database.rowLimit}
        onchange={(e) => database.setRowLimit(Number((e.currentTarget as HTMLSelectElement).value))}
      >
        {#each ROW_LIMIT_OPTS as o (o.value)}
          <option value={o.value}>{o.label}</option>
        {/each}
      </select>
    </label>
    <label class="qe-timeout" title="Per-statement timeout (ms) — 0 or blank = no limit; MySQL only">
      <span>Timeout</span>
      <input
        class="input qe-timeout-input"
        type="number"
        min="0"
        step="1000"
        placeholder="ms"
        value={tab.timeout_ms ?? ''}
        oninput={(e) => {
          const v = Number((e.currentTarget as HTMLInputElement).value);
          database.tab.timeout_ms = v > 0 ? v : null;
        }}
      />
    </label>
    <label
      class="qe-mask"
      class:active={tab.mask}
      title="Mask PII/prod — server redacts sensitive values (emails, tokens, keys) before returning results"
    >
      <input
        type="checkbox"
        class="sr-only"
        checked={tab.mask}
        onchange={(e) => { database.tab.mask = (e.currentTarget as HTMLInputElement).checked; }}
      />
      <Icon name="lock" size={11} />
      {#if tab.mask}<span class="qe-masked-badge">Masked</span>{:else}<span>Mask</span>{/if}
    </label>
    <span class="qe-lang mono">{database.queryLanguage}</span>
  </div>

  {#if nlAvailable && nlOpen}
    <div class="qe-nl">
      <div class="qe-nl-ask">
        <Icon name="comment" size={12} />
        <input
          class="input grow"
          placeholder="Ask in English — e.g. “top 10 customers by total order value last month”"
          bind:value={nlQuestion}
          spellcheck="false"
          disabled={nlBusy}
          onkeydown={(e) => {
            if (e.key === 'Enter') askEnglish();
            else if (e.key === 'Escape') nlOpen = false;
          }}
        />
        <button
          class="btn small primary"
          onclick={askEnglish}
          disabled={nlBusy || !nlQuestion.trim()}
        >
          {#if nlBusy}<span class="qe-nl-dot"></span>Generating…{:else}<Icon name="zap" size={11} />Generate{/if}
        </button>
        <button class="btn small ghost" onclick={() => (nlOpen = false)} title="Close" aria-label="Close">
          <Icon name="x" size={11} />
        </button>
      </div>

      {#if nlError}
        <div class="qe-nl-err mono">{nlError}</div>
      {/if}

      {#if nlOutcome}
        <div class="qe-nl-result">
          <pre class="qe-nl-sql mono">{nlOutcome.sql}</pre>
          {#if nlOutcome.warnings.length > 0}
            <div class="qe-nl-warn">{nlOutcome.warnings.join(' · ')}</div>
          {/if}
          <div class="qe-nl-actions">
            <button class="btn small primary" onclick={() => useNlSql({ run: true })}>
              <Icon name="play" size={11} />Run
            </button>
            <button class="btn small" onclick={() => useNlSql({ run: false })}>
              <Icon name="send" size={11} />Insert into editor
            </button>
            <span class="grow"></span>
            <button
              class="qe-nl-plan-toggle"
              onclick={() => (nlPlanOpen = !nlPlanOpen)}
              aria-expanded={nlPlanOpen}
              title="The EXPLAIN plan that proved this query runs as a read"
            >
              <Icon name={nlPlanOpen ? 'chevronDown' : 'chevronRight'} size={11} />
              Validated with EXPLAIN
              <span class="qe-nl-attempts">· {nlOutcome.attempts} attempt{nlOutcome.attempts === 1 ? '' : 's'}</span>
            </button>
          </div>
          {#if nlPlanOpen}
            <pre class="qe-nl-plan mono">{nlOutcome.plan}</pre>
          {/if}
        </div>
      {/if}
    </div>
  {/if}

  {#if queryVars.length > 0}
    <div class="qe-vars" bind:this={varsBarEl}>
      <Icon name="tag" size={11} />
      <span class="qe-vars-label">Variables</span>
      {#each queryVars as name (name)}
        {@const spec = tab.vars[name] ?? defaultVarSpec()}
        <div class="qe-var">
          <span class="qe-var-name mono">{name}</span>
          <input
            class="input qe-var-input"
            value={spec.value}
            placeholder={spec.type === 'number' ? '123' : 'value'}
            spellcheck="false"
            oninput={(e) =>
              database.setVar(name, { value: (e.currentTarget as HTMLInputElement).value })}
            onkeydown={(e) => {
              if (e.key === 'Enter') run();
            }}
          />
          <select
            class="input qe-var-type"
            value={spec.type}
            title="How to substitute this variable: string (quoted), number (raw), or raw (verbatim)"
            onchange={(e) =>
              database.setVar(name, {
                type: (e.currentTarget as HTMLSelectElement).value as VarSpec['type'],
              })}
          >
            <option value="string">string</option>
            <option value="number">number</option>
            <option value="raw">raw</option>
          </select>
          {#if spec.type === 'string'}
            <label class="qe-var-esc" title="Escape quotes inside the value">
              <input
                type="checkbox"
                checked={spec.escape}
                onchange={(e) =>
                  database.setVar(name, { escape: (e.currentTarget as HTMLInputElement).checked })}
              />
              esc
            </label>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  {#if saving}
    <div class="save-bar">
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="input grow"
        placeholder="Query name"
        bind:value={saveName}
        autofocus
        onkeydown={(e) => {
          if (e.key === 'Enter') confirmSave();
          else if (e.key === 'Escape') saving = false;
        }}
      />
      <button class="btn small primary" onclick={confirmSave} disabled={!saveName.trim()}>Save</button>
      <button class="btn small" onclick={() => (saving = false)}>Cancel</button>
    </div>
  {/if}

  {#if viewport.isPhone}
    <button class="qe-acc-head" onclick={() => (editorOpen = !editorOpen)} aria-expanded={editorOpen}>
      <Icon name={editorOpen ? 'chevronDown' : 'chevronRight'} size={14} />
      <span class="qe-acc-title">Editor</span>
    </button>
  {/if}
  <div class="qe-edit" class:qe-collapsed={viewport.isPhone && !editorOpen} style="height: {editorH}px">
    <CodeEditor
      path={editorPath}
      content={tab.statement}
      root={ws.current?.root_path ?? ''}
      language={lang}
      readOnly={false}
      minimal={true}
      completionSource={database.selectedConnId ? completionSource : null}
      onchange={(v) => database.setStatement(v)}
      onsubmit={run}
      onselect={(s) => (editorSel = s)}
    />
  </div>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="qe-splitter"
    class:resizing
    role="separator"
    aria-orientation="horizontal"
    aria-label="Drag to resize editor and results"
    title="Drag to resize · double-click to expand"
    onpointerdown={startResize}
    ondblclick={toggleExpand}
  ><span class="qe-grip"></span></div>

  {#if viewport.isPhone}
    <button class="qe-acc-head" onclick={() => (resultsOpen = !resultsOpen)} aria-expanded={resultsOpen}>
      <Icon name={resultsOpen ? 'chevronDown' : 'chevronRight'} size={14} />
      <span class="qe-acc-title">Results</span>
      {#if hasResult && tab.result}<span class="qe-acc-count">{tab.result.stats.row_count} rows</span>{/if}
      {#if tab.error}<span class="qe-acc-count err">error</span>{/if}
    </button>
  {/if}
  <div class="qe-results" class:qe-collapsed={viewport.isPhone && !resultsOpen}>
    <ResultsGrid
      result={tab.result}
      error={tab.error}
      statement={tab.statement}
      connectionId={database.selectedConnId}
    />
  </div>
</div>

<style>
  .query-editor {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .qe-tabs {
    display: flex;
    align-items: stretch;
    gap: 3px;
    margin-bottom: 8px;
    overflow-x: auto;
    scrollbar-width: thin;
    padding-bottom: 1px;
    border-bottom: 1px solid var(--border);
  }
  .qe-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    max-width: 220px;
    padding: 0 4px 0 11px;
    border: 1px solid transparent;
    border-bottom: none;
    border-top-left-radius: var(--radius-s);
    border-top-right-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11.5px;
    cursor: pointer;
    white-space: nowrap;
    user-select: none;
    flex: 0 0 auto;
    transition: background 0.12s, color 0.12s;
  }
  .qe-tab:hover {
    background: color-mix(in srgb, var(--text-dim) 7%, transparent);
    color: var(--text);
  }
  .qe-tab.active {
    background: var(--surface-2);
    border-color: var(--border);
    color: var(--text);
    font-weight: 600;
    /* sit on top of the strip's bottom border */
    margin-bottom: -1px;
    padding-bottom: 1px;
  }
  .qe-tab-label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .qe-tab-rename {
    height: 18px;
    width: 130px;
    padding: 0 4px;
    font-size: 11.5px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
  }
  .qe-tab-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    flex: 0 0 auto;
  }
  .qe-tab-dot.running {
    background: var(--accent);
    animation: qe-pulse 1s ease-in-out infinite;
  }
  .qe-tab-dot.error {
    background: var(--status-exited);
  }
  @keyframes qe-pulse {
    0%,
    100% {
      opacity: 0.35;
    }
    50% {
      opacity: 1;
    }
  }
  .qe-tab-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 17px;
    height: 17px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex: 0 0 auto;
    opacity: 0.6;
  }
  .qe-tab:hover .qe-tab-close,
  .qe-tab.active .qe-tab-close {
    opacity: 1;
  }
  .qe-tab-close:hover {
    background: color-mix(in srgb, var(--status-exited) 22%, transparent);
    color: var(--status-exited);
  }
  .qe-tab-new {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 26px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex: 0 0 auto;
  }
  .qe-tab-new:hover {
    background: var(--surface-2);
    border-color: var(--border);
    color: var(--accent);
  }
  .qe-toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 0 8px;
  }
  /* Query-level variables bar — shown only when the statement references
     :name / {name}. One labelled input per variable, values remembered per tab. */
  .qe-vars {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 0 0 8px;
    color: var(--text-dim);
  }
  .qe-vars-label {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .qe-var {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    padding: 2px 6px;
  }
  .qe-var-name {
    font-size: 11.5px;
    color: var(--accent);
  }
  .qe-var-name::before {
    content: ':';
    opacity: 0.6;
  }
  .qe-var-input {
    height: 22px;
    width: 120px;
    font-size: 12px;
  }
  .qe-var-type {
    height: 22px;
    font-size: 11px;
    padding: 0 2px;
  }
  .qe-var-esc {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 10.5px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.03em;
    cursor: pointer;
  }
  /* Ask-in-English (verified NL→SQL) panel — sits between toolbar and editor. */
  .qe-nl {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px 10px;
    margin: 0 0 8px;
    border: 1px solid color-mix(in srgb, var(--accent) 35%, var(--border));
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--accent) 5%, var(--surface-2));
  }
  .qe-nl-ask {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text-dim);
  }
  .qe-nl-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: currentColor;
    animation: qe-pulse 1s ease-in-out infinite;
  }
  .qe-nl-err {
    font-size: 11.5px;
    color: var(--status-exited);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .qe-nl-result {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .qe-nl-sql {
    margin: 0;
    padding: 8px 10px;
    font-size: 12px;
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
  }
  .qe-nl-warn {
    font-size: 11px;
    color: var(--text-dim);
  }
  .qe-nl-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .qe-nl-plan-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
  }
  .qe-nl-plan-toggle:hover {
    color: var(--text);
  }
  .qe-nl-attempts {
    opacity: 0.7;
  }
  .qe-nl-plan {
    margin: 0;
    padding: 8px 10px;
    max-height: 180px;
    overflow: auto;
    font-size: 11px;
    line-height: 1.4;
    white-space: pre-wrap;
    word-break: break-word;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text-dim);
  }
  /* Toggle highlight for the toolbar "Ask in English" button when its panel is open. */
  .btn.small.ghost.on {
    color: var(--accent);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border));
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .btn.stop {
    border-color: color-mix(in srgb, var(--status-exited) 55%, transparent);
    background: color-mix(in srgb, var(--status-exited) 16%, transparent);
    color: var(--status-exited);
    font-weight: 600;
  }
  .btn.stop:hover {
    background: color-mix(in srgb, var(--status-exited) 26%, transparent);
  }
  .kbd {
    font-size: 9.5px;
    opacity: 0.7;
    font-variant-numeric: tabular-nums;
  }
  .qe-lang {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    padding: 0 6px;
    height: 18px;
    line-height: 18px;
    border-radius: 999px;
    background: var(--surface-2);
  }
  .qe-limit,
  .qe-db,
  .qe-timeout {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .qe-limit select,
  .qe-db select {
    height: 24px;
    padding: 0 4px;
    font-size: 11px;
    width: auto;
    max-width: 160px;
  }
  .qe-timeout-input {
    height: 24px;
    padding: 0 4px;
    font-size: 11px;
    width: 72px;
  }
  /* Mask PII/prod toggle — styled like a small button, highlights when active. */
  .qe-mask {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-dim);
    height: 24px;
    padding: 0 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    cursor: pointer;
    user-select: none;
    transition: background 0.12s, color 0.12s, border-color 0.12s;
  }
  .qe-mask:hover {
    color: var(--text);
    border-color: var(--accent);
  }
  .qe-mask.active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    border-color: var(--accent);
    color: var(--accent);
  }
  .qe-masked-badge {
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }
  .save-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 0 8px;
  }
  .qe-edit {
    flex: 0 0 auto;
    min-height: 100px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  /* Draggable divider between editor and results. */
  .qe-splitter {
    flex: 0 0 auto;
    height: 11px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: row-resize;
    touch-action: none;
  }
  .qe-grip {
    width: 40px;
    height: 3px;
    border-radius: 2px;
    background: var(--border);
    transition: background 120ms ease-out;
  }
  .qe-splitter:hover .qe-grip,
  .qe-splitter.resizing .qe-grip {
    background: var(--accent);
  }
  .qe-results {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .grow {
    flex: 1;
  }

  /* ───────────────── Phone (≤640px) ─────────────────
     On desktop this is a fixed-height flex column (editor + flex:1 results)
     living inside the page's clipped viewport. On a phone that height chain
     is broken upstream (DatabasePage lets the page scroll) so here we make the
     editor a modest fixed height, let the dense toolbar WRAP instead of
     overflowing off-screen, and give the results their own bounded,
     internally-scrolling block so a query's rows are always reachable. */
  /* Phone accordion headers for the Editor / Results blocks. */
  .qe-acc-head {
    display: none;
  }
  /* Tablet (641–1024px): the side-by-side DB layout narrows the editor column,
     so wrap the dense toolbar (Run/Save/Explain/Ask-AI + Limit/Timeout/Mask)
     onto multiple rows instead of letting it overflow and get clipped — the same
     wrap the phone layout uses, but WITHOUT forcing the compact editor height. */
  @media (min-width: 641px) and (max-width: 1024px) {
    .qe-toolbar {
      flex-wrap: wrap;
      gap: 6px;
      row-gap: 6px;
    }
    .qe-toolbar .grow {
      flex-basis: 100%;
      height: 0;
      flex: 0 0 100%;
    }
  }

  @media (max-width: 640px) {
    .query-editor {
      height: auto;
      min-height: 0;
    }
    /* Editor: ignore the persisted desktop drag-height — keep it compact so the
       results sit just below it (the inline style sets height, so override it). */
    .qe-edit {
      height: 200px !important;
    }
    /* Dense toolbar → wrap onto multiple rows so nothing runs off the edge. */
    .qe-toolbar {
      flex-wrap: wrap;
      gap: 6px;
      row-gap: 6px;
    }
    /* The flexible spacer would force the controls onto a wider line — collapse
       it on mobile so the controls pack tightly and wrap naturally. */
    .qe-toolbar .grow {
      flex-basis: 100%;
      height: 0;
      flex: 0 0 100%;
    }
    /* Bigger tap targets / readable controls. */
    .qe-limit select,
    .qe-db select,
    .qe-timeout-input,
    .qe-mask {
      height: 32px;
      font-size: 12.5px;
    }
    .qe-limit,
    .qe-db,
    .qe-timeout {
      font-size: 12.5px;
    }
    .qe-tab {
      height: 32px;
      font-size: 13px;
      max-width: 60vw;
    }
    /* Collapsible Editor / Results accordion headers. */
    .qe-acc-head {
      display: flex;
      align-items: center;
      gap: 8px;
      width: 100%;
      min-height: 44px;
      padding: 8px 4px;
      border: none;
      border-top: 1px solid var(--border);
      background: transparent;
      color: var(--text-dim);
      cursor: pointer;
      text-align: start;
    }
    .qe-acc-title {
      font-size: 12.5px;
      font-weight: 600;
      text-transform: uppercase;
      letter-spacing: 0.04em;
    }
    .qe-acc-count {
      font-size: 11.5px;
      color: var(--text-dim);
      background: var(--surface-2);
      border-radius: 999px;
      padding: 1px 8px;
      font-variant-numeric: tabular-nums;
    }
    .qe-acc-count.err {
      color: var(--status-exited);
      background: color-mix(in srgb, var(--status-exited) 16%, transparent);
    }
    /* A collapsed block is removed from flow. */
    .qe-collapsed {
      display: none !important;
    }
    /* The drag splitter has no role on touch (we resize via the editor's fixed
       height + accordion) — hide it. */
    .qe-splitter {
      display: none;
    }
    /* Results: own bounded, internally-scrolling block — always reachable. A
       small result fits naturally; a large one caps at ~70vh and the grid
       scrolls inside it (its child .grid-scroll is overflow:auto) so the page
       doesn't grow unbounded. */
    .qe-results {
      flex: 0 0 auto;
      min-height: 340px;
      max-height: 70vh;
    }
    .qe-tabs {
      scrollbar-width: none;
    }
    .qe-tabs::-webkit-scrollbar {
      display: none;
    }
  }
</style>
