<script lang="ts">
  // The Tools view's catalog of Otto's own tools (otto.*): one row per
  // category, collapsed ("Git · 5 of 10 on"), searchable — a search opens
  // every category it matches — and WINDOWED (ListWindow), so 299 tools never
  // mount at once. A tool row says its state in words (Off · Allowed · Asks
  // first · Runs without asking) and its description's first sentence; the
  // full text and the per-tool auto-approve switch are behind Details. The
  // category's ⋯ turns it all on / off and carries its auto-approve rule.
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { ListWindow } from '../../lib/listWindow.svelte';
  import { toastError } from '../../lib/toastError';
  import type { McpAutoApproveRef, McpOttoToolInfo, UpdateMcpOttoServerReq } from '../../lib/api/types';
  import { mcpCpExtraApi } from './cp-api';
  import { catalogRows, firstSentence, toolState } from './toolCatalog';

  interface Props {
    tools: McpOttoToolInfo[];
    /** Saves through the page (it owns the status the catalog reads). */
    patch: (body: UpdateMcpOttoServerReq) => Promise<boolean>;
    /** After an auto-approve rule changed: reload the catalog and the rules. */
    reload: () => Promise<void>;
    disabled: boolean;
    approvalsOn: boolean;
  }
  let { tools, patch, reload, disabled, approvalsOn }: Props = $props();

  let filter = $state('');
  let open = $state(new Set<string>());
  let busy = $state(false);
  let detailName = $state<string | null>(null);
  const detail = $derived(detailName ? (tools.find((t) => t.name === detailName) ?? null) : null);

  const rows = $derived(catalogRows(tools, open, filter));
  const enabledNames = $derived(new Set(tools.filter((t) => t.enabled).map((t) => t.name)));
  const changing = $derived(tools.filter((t) => t.enabled && t.mutating).length);
  const lw = new ListWindow('.tc-row', { min: 60, overscan: 10, rowH: 52 });
  const win = $derived(lw.range(rows.length));
  const off = $derived(disabled || busy);

  function toggleOpen(cat: string): void {
    const next = new Set(open);
    if (next.has(cat)) next.delete(cat);
    else next.add(cat);
    open = next;
  }

  async function toggleTool(name: string, input: HTMLInputElement): Promise<void> {
    const next = new Set(enabledNames);
    if (next.has(name)) next.delete(name);
    else next.add(name);
    // One-way checkbox: a failed save puts it back.
    if (!(await patch({ tools: [...next] }))) input.checked = enabledNames.has(name);
  }

  async function setCategory(list: McpOttoToolInfo[], enable: boolean): Promise<void> {
    const next = new Set(enabledNames);
    for (const t of list) {
      if (enable) next.add(t.name);
      else next.delete(t.name);
    }
    await patch({ tools: [...next] });
  }

  const globalToolRule = (t: McpOttoToolInfo): McpAutoApproveRef | undefined =>
    t.auto_approved_by?.find((r) => r.scope === 'global' && r.target_kind === 'tool');
  function globalCategoryRule(cat: string, list: McpOttoToolInfo[]): McpAutoApproveRef | undefined {
    for (const t of list) {
      const rule = t.auto_approved_by?.find((r) => r.scope === 'global' && r.target_kind === 'category' && r.target === cat);
      if (rule) return rule;
    }
    return undefined;
  }
  const otherRules = (t: McpOttoToolInfo): McpAutoApproveRef[] => (t.auto_approved_by ?? []).filter((r) => r.id !== globalToolRule(t)?.id);
  function ruleBadge(rule: McpAutoApproveRef): string {
    const where = rule.scope === 'global' ? 'everywhere' : rule.scope === 'workspace' ? 'in a workspace' : 'for one session';
    return rule.target_kind === 'category' ? `${rule.target} writes ${where}` : where;
  }

  async function confirmAuto(what: string, irreversible: boolean): Promise<boolean> {
    const first = await confirmer.ask(
      `Auto-approve ${what} everywhere? Agents and external clients will run it without a human approval. Every call is still audited as “Auto approved”, and you can turn this off here at any time.`,
      { title: 'Auto-approve', confirmLabel: 'Auto-approve', danger: true },
    );
    if (!first || !irreversible) return first;
    return confirmer.ask(`${what} is IRREVERSIBLE — nobody can undo it from Otto once it runs. Run it without asking anyway?`, {
      title: 'Irreversible tool',
      confirmLabel: 'Run without asking',
      danger: true,
    });
  }

  /** A global auto-approve rule for one tool (`target` a tool) or a
   *  category; a cancelled confirm or failed save restores `input`. */
  async function setRule(kind: 'tool' | 'category', target: string, rule: McpAutoApproveRef | undefined, on: boolean, what: string, irreversible: boolean, input?: HTMLInputElement): Promise<void> {
    if (on && !(await confirmAuto(what, irreversible))) {
      if (input) input.checked = false;
      return;
    }
    busy = true;
    try {
      if (on) await mcpCpExtraApi.createAutoApprove({ scope: 'global', target_kind: kind, target, allow_irreversible: irreversible ? true : undefined });
      else if (rule) await mcpCpExtraApi.deleteAutoApprove(rule.id);
      await reload();
    } catch (e) {
      if (input) input.checked = !!rule;
      toastError('Couldn’t update the setting', e);
    } finally {
      busy = false;
    }
  }

  function catMenu(e: MouseEvent, cat: string, list: McpOttoToolInfo[]): void {
    const gated = list.some((t) => t.mutating && t.enabled);
    const rule = globalCategoryRule(cat, list);
    ctxMenu.showAt(
      e.currentTarget as Element,
      [
        { label: 'Turn all on', icon: 'check', disabled: off, action: () => void setCategory(list, true) },
        { label: 'Turn all off', icon: 'x', disabled: off, action: () => void setCategory(list, false) },
        ...(gated
          ? [
              { separator: true },
              {
                label: `Auto-approve every ${cat} write`,
                checked: !!rule,
                disabled: off || !approvalsOn,
                title: 'One rule for the category: its tools that change things (and future ones) run without asking — never an irreversible tool.',
                action: () => void setRule('category', cat, rule, !rule, `every ${cat} write (except irreversible tools)`, false),
              },
            ]
          : []),
      ],
      { align: 'end' },
    );
  }

  /** Markdown-ish descriptions: `inline code` renders as code. */
  const descParts = (text: string | null | undefined) =>
    (text ?? '').split('`').map((t, i) => ({ text: t, code: i % 2 === 1 })).filter((p) => p.text !== '');
  const stateTone = (s: string) => (s === 'Runs without asking' ? 'warn' : s === 'Off' ? 'dim' : '');
</script>

<section class="catalog" aria-labelledby="mcp-tools-title" data-testid="mcp-tool-catalog">
  <div class="tc-head">
    <div class="tc-title">
      <h3 id="mcp-tools-title">Otto tools</h3>
      <span class="dim small" data-testid="mcp-tool-counts">{enabledNames.size} of {tools.length} on · {changing} can change things</span>
    </div>
    <input dir="ltr" class="input filter" type="search" placeholder="Search tools…" bind:value={filter} aria-label="Search tools" />
  </div>
  <p class="dim small note">
    Agents and external clients may call the tools that are on. A tool that can change things asks a person first unless an
    auto-approve rule covers it; every call is audited.
  </p>
  {#if !approvalsOn}
    <p class="warn" data-testid="mcp-approvals-globally-off">
      Approval prompts are turned off globally (<code>mcp_require_approval_dangerous</code>), so no otto.* call asks first — the
      auto-approve rules have no effect until that is turned back on.
    </p>
  {/if}
  <div class="tc-list" {@attach lw.attach}>
    {#if win.top}<div style:height="{win.top}px" aria-hidden="true"></div>{/if}
    {#each rows.slice(win.start, win.end) as r (r.key)}
      {#if r.kind === 'cat'}
        <div class="tc-row tc-cat" data-testid="mcp-cat-{r.cat}">
          <button class="tc-fold" aria-expanded={r.open} disabled={!!filter.trim()} onclick={() => toggleOpen(r.cat)}>
            <Icon name={r.open ? 'chevronDown' : 'chevronRight'} size={12} />
            <span class="grp-name">{r.cat}</span>
            <span class="dim small">{r.on} of {r.total} on</span>
          </button>
          <button class="icon-btn" aria-label="{r.cat}: turn on, off or auto-approve" title="{r.cat}: turn on, off or auto-approve" onclick={(e) => catMenu(e, r.cat, r.tools)}>
            <Icon name="more" size={14} />
          </button>
        </div>
      {:else}
        {@const t = r.tool}
        {@const state = toolState(t, approvalsOn)}
        <div class="tc-row tc-tool">
          <input type="checkbox" checked={t.enabled} disabled={off} aria-label="Agents may use {t.name}" onchange={(event) => void toggleTool(t.name, event.currentTarget)} />
          <span class="t-meta">
            <span class="t-name"><span class="mono">{t.name}</span>{#if t.mutating}<span class="mut">can change things</span>{/if}</span>
            <span class="t-desc">{firstSentence(t.description)}</span>
          </span>
          <span class="t-state {stateTone(state)}">{state}</span>
          <button class="icon-btn" aria-label="Details for {t.name}" title="Details" data-testid="mcp-tool-details-{t.name}" onclick={() => (detailName = t.name)}>
            <Icon name="info" size={14} />
          </button>
        </div>
      {/if}
    {/each}
    {#if win.bottom}<div style:height="{win.bottom}px" aria-hidden="true"></div>{/if}
  </div>
  {#if filter.trim() && !rows.length}<p class="dim small">No tools match “{filter}”.</p>{/if}
</section>

{#if detail}
  {@const own = globalToolRule(detail)}
  <Modal title={detail.name} width={560} onclose={() => (detailName = null)}>
    <p class="t-full">{#each descParts(detail.description) as part, i (i)}{#if part.code}<code>{part.text}</code>{:else}{part.text}{/if}{/each}</p>
    <p class="dim small">{toolState(detail, approvalsOn)}{detail.mutating ? ' · can change things' : ' · read-only'}{detail.irreversible ? ' · irreversible' : ''}</p>
    {#if detail.mutating && detail.enabled}
      <label class="ask-toggle" title="On: calls run without asking a person — still audited. Off: a person approves each call.">
        <input type="checkbox" data-testid="mcp-auto-{detail.name}" checked={!!own} disabled={off || !approvalsOn}
          onchange={(event) => void setRule('tool', detail.name, own, event.currentTarget.checked, detail.name, !!detail.irreversible, event.currentTarget)} />
        <span>Auto-approve everywhere</span>
      </label>
      {#each otherRules(detail) as rule (rule.id)}<p class="noask small" title={rule.name}>Auto-approved {ruleBadge(rule)}</p>{/each}
    {/if}
  </Modal>
{/if}

<style>
  .catalog {
    display: flex;
    flex-direction: column;
    gap: var(--sp-4);
    min-inline-size: 0;
  }
  .tc-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: var(--sp-4) var(--sp-5);
  }
  .tc-title {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: var(--sp-2) var(--sp-5);
  }
  h3 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .filter {
    flex: 0 1 240px;
  }
  .note,
  .warn {
    margin: 0;
  }
  .warn {
    font-size: var(--fs-m);
    color: var(--warning);
    background: var(--warning-soft);
    border-radius: var(--radius-s);
    padding: var(--sp-4) var(--sp-5);
  }
  .tc-list {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
  }
  /* Every row is one pitch (52 px): the window measures one and trusts it. */
  .tc-row {
    display: flex;
    align-items: center;
    gap: var(--sp-5);
    block-size: 52px;
    box-sizing: border-box;
    padding-inline: var(--sp-5) var(--sp-3);
    border-block-end: 1px solid var(--separator);
    min-inline-size: 0;
  }
  .tc-cat {
    background: var(--surface);
  }
  .tc-tool:hover {
    background: var(--hover);
  }
  .tc-fold {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    min-inline-size: 0;
    block-size: 100%;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .tc-fold:disabled {
    cursor: default;
  }
  .grp-name {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .t-meta {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    min-inline-size: 0;
  }
  .t-name {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    font-size: var(--fs-m);
    min-inline-size: 0;
  }
  .t-desc {
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mut {
    flex: none;
    font-size: var(--fs-xs);
    color: var(--warning);
  }
  .t-state {
    flex: none;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .t-state.warn {
    color: var(--warning);
    font-weight: 600;
  }
  .t-state.dim {
    color: var(--text-dim);
  }
  .t-full {
    margin-block-start: 0;
    overflow-wrap: anywhere;
    line-height: 1.5;
  }
  .t-full code,
  .warn code {
    font-size: var(--fs-xs);
    padding: 0 var(--sp-2);
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .ask-toggle {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
    cursor: pointer;
  }
  .noask {
    color: var(--warning);
  }
  .mono {
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-s);
  }
  @media (max-width: 640px) {
    .filter {
      flex-basis: 100%;
    }
    .t-state {
      display: none;
    }
  }
  @media (pointer: coarse) {
    .tc-row {
      block-size: 56px;
    }
  }
</style>
