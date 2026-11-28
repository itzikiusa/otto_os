<script lang="ts">
  // One notebook cell's outputs: streams and tracebacks with ANSI colours,
  // images (SVG as an <img> data URI, so its scripts are inert), HTML in a
  // script-less sandboxed iframe, text, fallback notes (widgets, JavaScript),
  // and the one-line field a running cell's input() / getpass() asks for.
  import type { NormalizedNotebookOutput } from './ideNotebookModel';

  interface Props {
    outputs: NormalizedNotebookOutput[];
    cellIndex: number;
    cellId: string;
    /** The kernel's pending `input_request` for this cell. */
    input: { prompt: string; password: boolean } | null;
    inputValue: string;
    onsubmitinput: () => void;
  }

  let { outputs, cellIndex, cellId, input, inputValue = $bindable(), onsubmitinput }: Props = $props();

  let inputEl = $state<HTMLInputElement | null>(null);
  $effect(() => {
    if (input && inputEl) inputEl.focus();
  });
</script>

<div class="nb-outputs" data-testid={`ide-nb-outputs-${cellIndex}`}>
  {#each outputs as out, oi (oi)}
    {#if out.kind === 'stream' || out.kind === 'error'}
      <pre class="nb-out-pre" class:is-stderr={out.kind === 'stream' && out.streamName === 'stderr'} class:is-error={out.kind === 'error'}>{#each out.segments ?? [{ text: out.text }] as seg, si (si)}<span class="ansi {seg.color ? `ansi-${seg.color}` : ''}" class:ansi-bold={seg.bold}>{seg.text}</span>{/each}</pre>
    {:else if out.kind === 'image' || out.kind === 'svg'}
      <div class="nb-out-img-wrap">
        <img class="nb-out-img" src={out.dataUri} alt="Notebook cell figure output" />
      </div>
    {:else if out.kind === 'html'}
      <iframe
        class="nb-out-html"
        sandbox=""
        title={`Cell ${cellIndex + 1} HTML output`}
        srcdoc={`<!doctype html><html><head><style>body{margin:6px;font-family:-apple-system,sans-serif;line-height:1.4;color:CanvasText;background:transparent}table{border-collapse:collapse;font-size:inherit}th,td{border:1px solid color-mix(in srgb,CanvasText 20%,transparent);padding:4px 8px;text-align:start}</style></head><body>${out.text}</body></html>`}
      ></iframe>
    {:else if out.text}
      <pre class="nb-out-pre">{out.text}</pre>
    {/if}
    {#if out.note}
      <p class="nb-out-note">{out.note}</p>
    {/if}
  {/each}
  {#if input}
    <form class="nb-input" onsubmit={(e) => { e.preventDefault(); onsubmitinput(); }} data-testid="ide-nb-input">
      <label class="nb-input-label" for={`nb-input-${cellId}`}>{input.prompt || 'Input'}</label>
      <input
        id={`nb-input-${cellId}`}
        class="input nb-input-field"
        type={input.password ? 'password' : 'text'}
        autocomplete="off"
        bind:this={inputEl}
        bind:value={inputValue}
      />
    </form>
  {/if}
</div>

<style>
  .nb-out-note { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .nb-input { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .nb-input-label { font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); white-space: pre-wrap; }
  .nb-input-field { flex: 1; min-inline-size: 160px; font-family: var(--font-mono); font-size: var(--fs-xs); }
  .ansi-danger { color: var(--danger); }
  .ansi-success { color: var(--success); }
  .ansi-warning { color: var(--warning); }
  .ansi-info { color: var(--info); }
  .ansi-accent { color: var(--accent-text); }
  .ansi-dim { color: var(--text-dim); }
  .ansi-bold { font-weight: 600; }
  .nb-outputs {
    display: flex; flex-direction: column; gap: 6px; padding: 8px 10px;
    border-block-start: 1px solid var(--border); background: var(--surface-2);
  }
  .nb-out-pre {
    margin: 0; font-family: var(--font-mono); font-size: var(--fs-xs); line-height: 1.45;
    color: var(--text); white-space: pre-wrap; word-break: break-word;
  }
  .nb-out-pre.is-stderr { color: var(--warning); }
  .nb-out-pre.is-error { color: var(--danger); background: var(--danger-soft); padding: 6px 8px; border-radius: var(--radius-s); }
  .nb-out-img-wrap { padding: 4px; background: var(--surface); border-radius: var(--radius-s); border: 1px solid var(--border); }
  .nb-out-img { max-inline-size: 100%; block-size: auto; display: block; }
  .nb-out-html { inline-size: 100%; min-block-size: 140px; border: 1px solid var(--border); border-radius: var(--radius-s); background: var(--surface); }
</style>
