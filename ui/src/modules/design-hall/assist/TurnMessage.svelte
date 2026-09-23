<script lang="ts">
  // One exchange in the Otto thread: what the person asked (when this device
  // remembers it, else the committed version's prompt summary) and Otto's
  // answer — attributed (provider mark, "Otto", AGENT label, time), its live
  // state in words, the one-line summary, the verified citations as chips
  // (unverified ones flagged, never clickable), findings with "Fix" (which is
  // a NEW turn the person starts), the team rules the turn applied, and — for
  // a conflict — the side draft with Compare / Apply on top / Keep current.
  // A committed turn is a new version; Compare offers Restore of the previous
  // one, so nothing an agent did is ever final without a person.
  import Icon, { asIcon } from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import StatusDot from '../../../lib/components/StatusDot.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import type { DesignAssistTurn, DesignVersion } from '../../../lib/api/types';
  import type { Ask } from './asks.svelte';
  import { findingsOf, provenanceChips, statusInfo, type Finding, type ProvChip } from './model';

  interface Props {
    turn: DesignAssistTurn;
    ask: Ask | null;
    /** The version this turn committed (`turn.version_id`), when loaded. */
    version: DesignVersion | null;
    /** The version it started from (for Compare / seq labels). */
    base: DesignVersion | null;
    headSeq: number | null;
    /** Team-rule key → its text (from the learned rules), when known. */
    ruleText: (key: string) => string | null;
    dismissed: ReadonlySet<string>;
    /** The conflict draft was set aside by the person. */
    keptCurrent: boolean;
    readonly: boolean;
    /** Another turn is running (fix / apply are disabled meanwhile). */
    busy: boolean;
    onopenref: (chip: ProvChip) => void;
    oncompare: (leftVersionId: string, rightVersionId: string) => void;
    onfix: (findings: Finding[], a11y: boolean) => void;
    ondismiss: (f: Finding) => void;
    onapplyconflict: () => void;
    onkeepcurrent: () => void;
    onopensession: (sessionId: string) => void;
  }
  let {
    turn,
    ask,
    version,
    base,
    headSeq,
    ruleText,
    dismissed,
    keptCurrent,
    readonly,
    busy,
    onopenref,
    oncompare,
    onfix,
    ondismiss,
    onapplyconflict,
    onkeepcurrent,
    onopensession,
  }: Props = $props();

  const st = $derived(statusInfo(turn.status, turn.mode));
  const chips = $derived(provenanceChips(turn, version?.provenance ?? null));
  const findings = $derived(findingsOf(turn).filter((f) => !dismissed.has(`${turn.turn_id}:${f.index}`)));
  const open = $derived(findings.filter((f) => !f.fixed));
  const fixedList = $derived(findingsOf(turn).filter((f) => f.fixed));
  const isA11y = $derived(ask?.intent === 'a11y_check' || turn.mode === 'a11y');
  const summary = $derived(
    typeof version?.provenance?.prompt_summary === 'string' ? (version.provenance.prompt_summary as string) : null,
  );
  const asked = $derived(ask?.prompt ?? summary);
  const modeLabel = $derived(
    ({ generate: 'Generate', refine: 'Refine', critique: 'Review', a11y: 'Accessibility fix', variant: 'Variant' } as Record<string, string>)[
      turn.mode
    ] ?? turn.mode,
  );
  const rules = $derived(turn.team_rules.map((k) => ({ key: k, text: ruleText(k) ?? k.replace(/[_:]/g, ' ') })));
  const provider = $derived(turn.provider || 'claude');
</script>

<div class="exchange" data-testid="design-assist-turn" data-status={turn.status}>
  {#if asked}
    <div class="msg human">
      <div class="who"><span class="av" aria-hidden="true">Y</span> <strong>You</strong>
        {#if ask}<span class="when" title={new Date(ask.at).toLocaleString()}>{rel(ask.at)}</span>{/if}</div>
      {#if ask?.selectionLabel}<span class="chip ctx"><Icon name="target" size={11} /> {ask.selectionLabel}</span>{/if}
      <p class="bubble">{asked}</p>
    </div>
  {/if}

  <div class="msg agent" aria-live={st.working ? 'polite' : undefined}>
    <div class="who">
      <ProviderIcon {provider} size={16} />
      <strong>Otto</strong>
      <span class="chip agent-label">AGENT</span>
      <span class="when" title={new Date(turn.started_at).toLocaleString()}>{rel(turn.started_at)}</span>
      <span class="grow"></span>
      <span class="st tone-{st.tone}" data-testid="design-assist-status">
        {#if st.working}<StatusDot status="working" />{/if}
        {st.label}
      </span>
    </div>
    <div class="meta">{modeLabel}{#if turn.provider} · {turn.provider}{/if}{#if base} · from v{base.seq}{/if}</div>

    {#if st.working}
      <p class="body dim">
        {turn.status === 'starting' ? 'Starting an agent session…' : 'Working on the saved version. Each valid save shows live on the canvas.'}
      </p>
    {:else if turn.status === 'failed'}
      <p class="body err"><Icon name="warning" size={12} /> {turn.error ?? 'The turn failed.'} Nothing was committed — the design is unchanged.</p>
    {:else if turn.message}
      <p class="body">{turn.message}</p>
    {/if}

    {#if turn.session_id}
      <button class="linkbtn" onclick={() => onopensession(turn.session_id!)} data-testid="design-assist-session">
        <Icon name="terminal" size={11} /> {st.working ? 'View live session' : 'Open session'}
      </button>
    {/if}

    {#if turn.status === 'done' && turn.version_id}
      <div class="result">
        <Icon name="commit" size={12} />
        <span>Saved as <strong>v{version?.seq ?? '…'}</strong> (Otto){#if base} — v{base.seq} stays in history{/if}.</span>
        {#if base}
          <button class="linkbtn" onclick={() => oncompare(base.id, turn.version_id!)}>Compare</button>
        {/if}
      </div>
    {/if}

    {#if turn.status === 'conflict' && turn.version_id}
      <div class="conflict" role="status" data-testid="design-assist-conflict">
        <p><Icon name="warning" size={12} /> Someone saved {headSeq != null ? `v${headSeq}` : 'a newer version'} while Otto worked, so Otto’s result
          was kept as a separate draft. The current version is untouched.</p>
        {#if keptCurrent}
          <p class="dim">You kept the current version. The draft stays in history.</p>
        {:else}
          <div class="row">
            {#if base}<button class="btn small" onclick={() => oncompare(base.id, turn.version_id!)}>Compare</button>{/if}
            <button class="btn small" disabled={readonly || busy} onclick={onapplyconflict}>Apply on top…</button>
            <button class="btn small ghost" disabled={readonly} onclick={onkeepcurrent}>Keep current</button>
          </div>
        {/if}
      </div>
    {/if}

    {#if open.length}
      <ul class="findings" aria-label="Findings">
        {#each open as f (f.index)}
          <li>
            <Icon name={f.severity === 'error' || f.severity === 'high' || f.severity === 'warning' ? 'warning' : 'info'} size={12} />
            <span class="ftext">
              {#if f.rule}<span class="rule">{f.rule}</span>{/if}
              {f.message}{#if f.fix} <span class="dim">→ {f.fix}</span>{/if}
            </span>
            <button class="icon-btn" disabled={readonly} onclick={() => ondismiss(f)} aria-label="Dismiss this finding" title="Dismiss — Otto notes it">
              <Icon name="x" size={11} />
            </button>
          </li>
        {/each}
      </ul>
      {#if turn.mode === 'critique'}
        <button class="btn small primary-soft" disabled={readonly || busy} onclick={() => onfix(open, isA11y)} data-testid="design-assist-fix">
          <Icon name="sparkle" size={12} /> {open.length === 1 ? 'Fix it' : `Fix all ${open.length}`}
        </button>
      {/if}
    {/if}
    {#if fixedList.length && turn.mode !== 'critique'}
      <ul class="findings fixed" aria-label="Fixed">
        {#each fixedList as f (f.index)}
          <li><Icon name="check" size={12} /> <span class="ftext">{#if f.rule}<span class="rule">{f.rule}</span>{/if}{f.message}</span></li>
        {/each}
      </ul>
    {/if}

    {#if chips.length}
      <div class="prov" aria-label="Provenance">
        <span class="k">Provenance</span>
        {#each chips as c (c.key)}
          {#if c.verified && c.artifactId}
            <button class="chip as-btn" class:brand={c.kind === 'brand'} title={c.title} onclick={() => onopenref(c)} data-testid="design-prov-chip">
              <Icon name={asIcon(c.kind === 'brand' ? 'palette' : 'link')} size={11} /> {c.label}
            </button>
          {:else}
            <span class="chip unverified" title={c.title} data-testid="design-prov-unverified">
              <Icon name="warning" size={11} /> {c.label} · not verified
            </span>
          {/if}
        {/each}
      </div>
    {/if}

    {#if rules.length}
      <p class="rules"><Icon name="bulb" size={12} /> Applied your team rules:
        {#each rules as r, i (r.key)}<a href="#/design/learned/rules">{r.text}</a>{i < rules.length - 1 ? ', ' : '.'}{/each}
      </p>
    {/if}
  </div>
</div>

<style>
  .exchange {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .msg {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .av {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: var(--fs-xs);
    font-weight: 700;
    background: var(--surface-3);
    color: var(--text-dim);
  }
  .when,
  .meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .bubble {
    margin: 0;
    align-self: flex-start;
    max-width: 100%;
    padding: 6px 10px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .ctx {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .msg.agent {
    border-inline-start: 2px solid var(--border-strong);
    padding-inline-start: 10px;
  }
  .agent-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    letter-spacing: 0.04em;
  }
  .st {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
  }
  .tone-ok {
    color: var(--success);
  }
  .tone-bad {
    color: var(--danger);
  }
  .tone-warn {
    color: var(--warning);
  }
  .tone-info {
    color: var(--info);
  }
  .body {
    margin: 0;
    font-size: var(--fs-s);
    line-height: 1.45;
    overflow-wrap: anywhere;
  }
  .dim {
    color: var(--text-dim);
  }
  .err {
    color: var(--danger);
  }
  .linkbtn {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .linkbtn:hover {
    text-decoration: underline;
  }
  .result {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .conflict {
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    font-size: var(--fs-s);
  }
  .conflict p {
    margin: 0 0 6px;
  }
  .conflict :global(svg) {
    color: var(--warning);
    vertical-align: -2px;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .findings {
    list-style: none;
    margin: 4px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .findings li {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .findings li > :global(svg) {
    flex: none;
    margin-block-start: 3px;
    color: var(--warning);
  }
  .findings.fixed li > :global(svg) {
    color: var(--success);
  }
  .ftext {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .rule {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-inline-end: 4px;
  }
  .primary-soft {
    align-self: flex-start;
    background: var(--accent-soft);
    color: var(--accent-text);
    border-color: transparent;
  }
  .prov {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin-block-start: 2px;
  }
  .prov .k {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-inline-end: 2px;
  }
  .prov .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .unverified {
    color: var(--warning);
    background: var(--warning-soft);
    border-color: transparent;
  }
  .rules {
    margin: 2px 0 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .rules :global(svg) {
    vertical-align: -2px;
  }
  .rules a {
    color: var(--accent-text);
    font-style: italic;
  }
  .as-btn {
    cursor: pointer;
    font-family: inherit;
  }
  .as-btn:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .as-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
