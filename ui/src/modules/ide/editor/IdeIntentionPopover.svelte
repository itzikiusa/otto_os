<script lang="ts">
  // GoLand / IntelliJ `⌥⏎` Intention & Quick-Fix Popover with Live `- / +` Diff Preview
  // AND 4-Mode Dependency Update Preview on manifest files (`go.mod`, `pom.xml`, etc.).
  // Deterministic fixes apply concrete code edits in `<5ms` (undoable with `⌘Z`), never forcing Agent!
  import Icon from '../../../lib/components/Icon.svelte';
  import { detectManifestEcosystem, executeIdeDepAction, type IdeDepItem } from '../../../lib/components/cm-ide-deps';
  import { previewConcreteEdits, type IdeConcreteTextEdit, type IdeQuickFixAction } from '../../../lib/components/cm-ide-quickfix';
  import type { ClientDiagQuickFix } from '../../../lib/components/cm-ide-semantic';
  import type { IdeCodeAction } from '../store/ideTypes';

  interface Props {
    filePath: string;
    root: string;
    connectionId?: string | null;
    projectFolders?: string[];
    docText: string;
    line: number;
    col: number;
    anchorX: number;
    anchorY: number;
    readOnly: boolean;
    clientFixes: ClientDiagQuickFix[];
    deterministicFixes: IdeQuickFixAction[];
    serverActions: IdeCodeAction[];
    manifestDep?: IdeDepItem | null;
    onapplyclientfix: (fix: ClientDiagQuickFix) => void;
    onapplyedits: (edits: IdeConcreteTextEdit[]) => void;
    onapplycontent: (nextContent: string) => void;
    onapplyserveraction: (act: IdeCodeAction) => void;
    onrename: () => void;
    onfindusages: () => void;
    onformat: () => void;
    onaskagent: () => void;
    onclose: () => void;
  }

  let {
    filePath, root, connectionId = null, projectFolders = [], docText, line, anchorX, anchorY, readOnly,
    clientFixes, deterministicFixes, serverActions, manifestDep = null,
    onapplyclientfix, onapplyedits, onapplycontent, onapplyserveraction,
    onrename, onfindusages, onformat, onaskagent, onclose,
  }: Props = $props();

  interface IntentionRow {
    id: string;
    label: string;
    hint?: string;
    kind: 'fix' | 'dep' | 'refactor' | 'agent';
    disabled?: boolean;
    previewMinus: string[];
    previewPlus: string[];
    previewNote?: string;
    run: () => void;
  }

  const eco = $derived(detectManifestEcosystem(filePath));
  const clampedX = $derived(typeof window !== 'undefined' ? Math.max(12, Math.min(anchorX, window.innerWidth - 580)) : anchorX);
  const clampedY = $derived(typeof window !== 'undefined' ? Math.max(12, Math.min(anchorY, window.innerHeight - 360)) : anchorY);

  function splitPreview(edits: IdeConcreteTextEdit[]): { before: string[]; after: string[] } {
    const p = previewConcreteEdits(docText, edits);
    return {
      before: p.beforeSnippet ? p.beforeSnippet.split('\n') : [],
      after: p.afterSnippet ? p.afterSnippet.split('\n') : [],
    };
  }

  function clientFixPreview(qf: ClientDiagQuickFix): { before: string[]; after: string[] } {
    if (qf.edits?.length) return splitPreview(qf.edits);
    const lines = docText.split(/\r?\n/);
    const slice = lines.slice(Math.max(0, qf.line - 1), Math.min(lines.length, qf.endLine));
    if (qf.deleteLine) return { before: slice, after: [] };
    return { before: slice, after: qf.replacement != null ? qf.replacement.split('\n') : slice };
  }

  function runDepAction(action: string, dep?: IdeDepItem | null, version?: string | null): void {
    onclose();
    void executeIdeDepAction({ root, connectionId, projectFolders, path: filePath, content: docText, action, dep, version }).then((res) => {
      if (res.updated_content && res.updated_content !== docText) onapplycontent(res.updated_content);
    });
  }

  const rows = $derived.by((): IntentionRow[] => {
    const out: IntentionRow[] = [];
    if (eco) {
      if (manifestDep?.latest_version && manifestDep.latest_version !== manifestDep.current_version) {
        const d = manifestDep;
        out.push({
          id: `dep-up-${d.name}`,
          label: `Upgrade ${d.name} to ${d.latest_version}`,
          hint: 'Quick Fix',
          kind: 'dep',
          disabled: readOnly,
          previewMinus: [`${d.name} ${d.current_version}`],
          previewPlus: [`${d.name} ${d.latest_version}`],
          previewNote: d.vulnerabilities?.length ? `Fixes ${d.vulnerabilities[0].id}: ${d.vulnerabilities[0].summary}` : `Updates single dependency in ${filePath.split('/').pop()}`,
          run: () => runDepAction('update_single', d, d.latest_version),
        });
      }
      out.push(
        {
          id: 'dep-update-all',
          label: 'Update all dependencies',
          kind: 'dep',
          disabled: readOnly,
          previewMinus: ['# Current pinned dependency versions'],
          previewPlus: [eco === 'go' ? 'go get -u ./... && go mod tidy' : eco === 'maven' ? 'mvn versions:use-latest-releases' : 'Upgrade all direct & transitive packages to latest compatible releases'],
          previewNote: 'Mode 1/4: Upgrades every dependency in this manifest',
          run: () => runDepAction('update_all'),
        },
        {
          id: 'dep-update-direct',
          label: 'Update direct dependencies only',
          kind: 'dep',
          disabled: readOnly,
          previewMinus: ['# Direct dependencies only (keeps indirect untouched)'],
          previewPlus: [eco === 'go' ? 'go list -f "{{if not .Indirect}}{{.Path}}@latest{{end}}" -m all | xargs go get' : 'Upgrade direct dependencies only'],
          previewNote: 'Mode 2/4: Upgrades direct dependencies without bumping indirect pins',
          run: () => runDepAction('update_direct'),
        },
        {
          id: 'dep-tidy-sync',
          label: eco === 'go' ? 'Run go mod tidy' : eco === 'maven' ? 'Sync / Reload Maven Project' : 'Sync Project Dependencies',
          kind: 'dep',
          disabled: readOnly,
          previewMinus: ['# Unsynced / unused module graph entries'],
          previewPlus: [eco === 'go' ? 'go mod tidy (prunes unused & adds missing sums)' : 'Re-resolves lockfile and workspace classpath'],
          previewNote: 'Mode 3/4: Synchronizes dependency graph and lockfile',
          run: () => runDepAction(eco === 'go' ? 'tidy' : 'sync'),
        },
        {
          id: 'dep-vuln-check',
          label: 'Scan manifest for known CVEs',
          kind: 'dep',
          previewMinus: [],
          previewPlus: [eco === 'go' ? 'govulncheck ./...' : 'Advisory & OSV vulnerability check across pinned versions'],
          previewNote: 'Mode 4/4: Audits dependencies against security advisories',
          run: () => runDepAction('vuln_scan'),
        },
      );
    }

    for (const [idx, qf] of clientFixes.entries()) {
      const p = clientFixPreview(qf);
      out.push({
        id: `cf-${idx}-${qf.title}`,
        label: qf.title,
        hint: 'Quick Fix',
        kind: 'fix',
        disabled: readOnly,
        previewMinus: p.before,
        previewPlus: p.after,
        run: () => { onapplyclientfix(qf); onclose(); },
      });
    }

    for (const df of deterministicFixes) {
      if (out.some((r) => r.label === df.title)) continue;
      const p = splitPreview(df.edits);
      out.push({
        id: df.id,
        label: df.title,
        hint: df.isPreferred ? 'Preferred' : 'Intention',
        kind: 'fix',
        disabled: readOnly,
        previewMinus: p.before,
        previewPlus: p.after,
        run: () => { onapplyedits(df.edits); onclose(); },
      });
    }

    for (const sa of serverActions) {
      const sEdits = (sa as IdeCodeAction & { edits?: IdeConcreteTextEdit[] }).edits;
      if (!sEdits?.length && !sa.replacement_content) continue;
      if (out.some((r) => r.label === sa.title)) continue;
      const p = sEdits?.length ? splitPreview(sEdits) : { before: [`Line ${line}`], after: [sa.edit_preview ?? 'Apply server code action'] };
      out.push({
        id: `srv-${sa.id}`,
        label: sa.title,
        hint: 'LSP Fix',
        kind: 'fix',
        disabled: readOnly,
        previewMinus: p.before,
        previewPlus: p.after,
        run: () => {
          if (sEdits?.length) onapplyedits(sEdits);
          else onapplyserveraction(sa);
          onclose();
        },
      });
    }

    out.push(
      { id: 'ref-rename', label: 'Rename symbol…', hint: '⇧F6', kind: 'refactor', disabled: readOnly, previewMinus: [], previewPlus: [], previewNote: 'Workspace-wide symbol rename with preview', run: () => { onclose(); onrename(); } },
      { id: 'ref-usages', label: 'Find usages', hint: '⌥F7', kind: 'refactor', previewMinus: [], previewPlus: [], previewNote: 'Search all references across workspace', run: () => { onclose(); onfindusages(); } },
      { id: 'ref-format', label: 'Reformat code', hint: '⌥⌘L', kind: 'refactor', disabled: readOnly, previewMinus: [], previewPlus: [], previewNote: 'Format current file with language formatter', run: () => { onclose(); onformat(); } },
      { id: 'ask-agent', label: 'Ask Agent to fix…', hint: '⌘I', kind: 'agent', previewMinus: [], previewPlus: [], previewNote: 'Optional: open inline AI assistant on current line', run: () => { onclose(); onaskagent(); } },
    );
    return out;
  });

  let selectedIdx = $state(0);
  const activeRow = $derived(rows[Math.min(selectedIdx, Math.max(0, rows.length - 1))] ?? null);

  function handleKeydown(e: KeyboardEvent): void {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onclose(); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); selectedIdx = (selectedIdx + 1) % Math.max(1, rows.length); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); selectedIdx = (selectedIdx - 1 + rows.length) % Math.max(1, rows.length); }
    else if (e.key === 'Enter' && activeRow && !activeRow.disabled) { e.preventDefault(); activeRow.run(); }
  }

  let popEl = $state<HTMLDivElement | null>(null);
  $effect(() => { popEl?.focus(); });
</script>

<svelte:window onkeydown={handleKeydown} onmousedown={(e) => { if (popEl && !popEl.contains(e.target as Node)) onclose(); }} />

<div
  bind:this={popEl}
  class="ide-intention-popover"
  role="dialog"
  aria-label="Context Actions and Quick Fixes"
  tabindex="-1"
  style:inset-inline-start="{clampedX}px"
  style:inset-block-start="{clampedY}px"
  data-testid="ide-intention-popover"
  data-dep-popover={eco ? 'true' : undefined}
>
  {#if eco}<span class="sr-only" data-testid="ide-dep-intention-popover"></span>{/if}
  <div class="ide-int-list" role="listbox" aria-label="Available actions">
    {#each rows as row, idx (row.id)}
      {#if row.kind === 'agent' || (idx > 0 && rows[idx - 1].kind !== row.kind && (row.kind === 'refactor' || row.kind === 'fix'))}
        <div class="ide-int-divider" role="separator"></div>
      {/if}
      <button
        type="button"
        role="option"
        aria-selected={idx === selectedIdx}
        class="ide-int-item"
        class:active={idx === selectedIdx}
        class:is-agent={row.kind === 'agent'}
        disabled={row.disabled}
        onmouseenter={() => (selectedIdx = idx)}
        onclick={() => row.run()}
        data-testid="ide-intention-option"
      >
        <Icon name={row.kind === 'fix' ? 'bulb' : row.kind === 'dep' ? 'refresh' : row.kind === 'agent' ? 'sparkle' : 'edit'} size={13} />
        <span class="ide-int-label">{row.label}</span>
        {#if row.hint}<span class="ide-int-hint">{row.hint}</span>{/if}
      </button>
    {/each}
  </div>

  {#if activeRow && (activeRow.previewMinus.length > 0 || activeRow.previewPlus.length > 0 || activeRow.previewNote)}
    <div class="ide-int-preview" dir="ltr" data-testid="ide-intention-preview">
      <div class="ide-int-prev-head">Preview</div>
      {#if activeRow.previewNote}<div class="ide-int-prev-note">{activeRow.previewNote}</div>{/if}
      {#each activeRow.previewMinus as m, i (`m-${i}`)}
        <div class="ide-int-diff is-minus">- {m}</div>
      {/each}
      {#each activeRow.previewPlus as p, i (`p-${i}`)}
        <div class="ide-int-diff is-plus">+ {p}</div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .sr-only { position: absolute; inline-size: 1px; block-size: 1px; overflow: hidden; clip: rect(0, 0, 0, 0); }
  .ide-intention-popover {
    position: fixed; z-index: var(--z-popover); display: flex; align-items: stretch;
    max-inline-size: min(560px, calc(100vw - 24px)); max-block-size: min(340px, calc(100vh - 24px));
    background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-m);
    box-shadow: var(--glass-shadow); overflow: hidden; outline: none;
  }
  .ide-int-list { display: flex; flex-direction: column; padding: var(--sp-2); min-inline-size: 250px; overflow-y: auto; }
  .ide-int-divider { block-size: 1px; background: var(--border); margin-block: var(--sp-1); flex-shrink: 0; }
  .ide-int-item {
    display: flex; align-items: center; gap: var(--sp-2); padding: 4px var(--sp-3);
    background: transparent; border: none; border-radius: var(--radius-s);
    color: var(--text); font-size: var(--fs-xs); text-align: start; cursor: pointer;
  }
  .ide-int-item.active, .ide-int-item:hover:not(:disabled) { background: var(--accent-soft); color: var(--accent-text); }
  .ide-int-item.is-agent { color: var(--text-dim); }
  .ide-int-item:disabled { opacity: var(--disabled-opacity); cursor: default; }
  .ide-int-label { flex: 1; min-inline-size: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ide-int-hint { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); flex-shrink: 0; }
  .ide-int-preview {
    display: flex; flex-direction: column; gap: var(--sp-1); padding: var(--sp-3);
    inline-size: 270px; border-inline-start: 1px solid var(--border); background: var(--bg);
    overflow: auto; font-family: var(--font-mono); font-size: var(--fs-xs);
  }
  .ide-int-prev-head { font-family: var(--font-ui); font-weight: 600; color: var(--text-dim); margin-block-end: var(--sp-1); }
  .ide-int-prev-note { font-family: var(--font-ui); color: var(--text); margin-block-end: var(--sp-2); white-space: normal; }
  .ide-int-diff { padding: 2px var(--sp-2); border-radius: var(--radius-s); white-space: pre-wrap; word-break: break-all; }
  .ide-int-diff.is-minus { background: var(--danger-soft); color: var(--danger); }
  .ide-int-diff.is-plus { background: var(--success-soft); color: var(--success); }
</style>
