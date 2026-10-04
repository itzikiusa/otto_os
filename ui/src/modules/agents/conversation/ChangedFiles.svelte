<script lang="ts">
  // "2 changed files +29 −12 · Show files · Open diff" under a settled
  // response (t3code's changed-files card): what the response did to the
  // tree, from its edit / write calls. A file row opens that file's diff in
  // the side panel; renderable files (md, html, svg, json, csv, images) also
  // get a Preview.
  import { getContext } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { changedDiff, relPath, type ChangedFile } from './format';
  import { previewKind } from './preview';
  import { CONV_CTX, type ConvContext } from './context';

  interface Props {
    files: ChangedFile[];
  }
  let { files }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);
  const cwd = $derived(ctx.cwd ?? null);
  let open = $state(false);
  const add = $derived(files.reduce((n, f) => n + f.add, 0));
  const del = $derived(files.reduce((n, f) => n + f.del, 0));

  function split(p: string): { dir: string; base: string } {
    const r = relPath(p, cwd);
    const at = r.lastIndexOf('/');
    return at < 0 ? { dir: '', base: r } : { dir: r.slice(0, at + 1), base: r.slice(at + 1) };
  }
  function openDiff(focus: string | null = null): void {
    const n = files.length;
    ctx?.openPreview?.({ kind: 'diff', title: `${n} changed ${n === 1 ? 'file' : 'files'}`, diff: changedDiff(files), focus });
  }
  function preview(f: ChangedFile): void {
    ctx?.openPreview?.({ kind: 'file', path: f.path, content: f.content, diff: changedDiff([f]) });
  }
</script>

<div class="cf" class:open data-changed-files={files.length}>
  <div class="cf-head">
    <button class="cf-toggle" onclick={() => (open = !open)} aria-expanded={open} title={open ? 'Hide the files' : 'Show the files'}>
      <span class="cf-caret" aria-hidden="true"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} /></span>
      <Icon name="edit" size={12} />
      <span class="cf-title">{files.length} changed {files.length === 1 ? 'file' : 'files'}</span>
      <span class="cf-stat mono"><span class="add">+{add}</span> <span class="del">−{del}</span></span>
      <span class="cf-show">{open ? 'Hide files' : 'Show files'}</span>
    </button>
    <button class="btn small cf-diff" onclick={() => openDiff()} title="Open this response's changes in the side panel"><Icon name="split" size={12} /> Open diff</button>
  </div>
  {#if open}
    <ul class="cf-list">
      {#each files as f (f.path)}
        {@const p = split(f.path)}
        <li class="cf-row">
          <button class="cf-file" onclick={() => openDiff(f.path)} title="Diff of {f.path}">
            <Icon name="file" size={12} />
            <span class="cf-path mono"><span class="cf-dir">{p.dir}</span>{p.base}</span>
            {#if f.created}<span class="cf-new">new</span>{/if}
            <span class="cf-stat mono"><span class="add">+{f.add}</span> <span class="del">−{f.del}</span></span>
          </button>
          {#if previewKind(f.path) !== 'code'}
            <button class="icon-btn cf-prev" onclick={() => preview(f)} aria-label="Preview {p.base}" title="Preview {p.base}"><Icon name="eye" size={12} /></button>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .cf {
    margin: 6px 0 2px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    min-width: 0;
    overflow: hidden;
  }
  .cf-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-block: 3px; padding-inline: 3px 5px;
    min-width: 0;
  }
  .cf-toggle {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: none;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
  }
  .cf-toggle:hover {
    background: var(--hover);
  }
  .cf-toggle:focus-visible,
  .cf-file:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .cf-caret {
    display: inline-flex;
    color: var(--text-dim);
  }
  .cf-title {
    font-weight: 600;
    white-space: nowrap;
  }
  .cf-stat {
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .add {
    color: var(--success);
    font-weight: 600;
  }
  .del {
    color: var(--danger);
    font-weight: 600;
  }
  .cf-show {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .cf-diff {
    flex-shrink: 0;
    gap: 5px;
  }
  .cf-list {
    list-style: none;
    margin: 0;
    padding: 2px 4px 5px;
    border-top: 1px solid var(--border);
  }
  .cf-row {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
  }
  .cf-file {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: none;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
  }
  .cf-file:hover {
    background: var(--hover);
    color: var(--text);
  }
  .cf-path {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-xs);
    color: var(--text);
    direction: ltr;
    text-align: start;
  }
  .cf-dir {
    color: var(--text-dim);
  }
  .cf-new {
    font-size: var(--fs-xs);
    color: var(--success);
    border: 1px solid color-mix(in srgb, var(--success) 40%, transparent);
    border-radius: 99px;
    padding: 0 6px;
  }
  .cf-prev {
    flex-shrink: 0;
  }
  @container (max-width: 420px) {
    .cf-show {
      display: none;
    }
  }
</style>
