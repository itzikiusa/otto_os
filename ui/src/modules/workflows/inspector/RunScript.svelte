<script lang="ts">
  // script — "Run script" (engine.md §7.2): a command under /bin/sh with the
  // login PATH, in each repository's worktree or the run folder, with a time
  // limit. Values reach it as environment variables, never in the command.
  import type { WorkflowNode } from '../../../lib/api/types';
  import { paramBool, paramNum, paramStr, type SetParam } from './util';

  let { node, set }: { node: WorkflowNode; set: SetParam } = $props();
  const ids = $props.id();
  const params = $derived((node.params ?? {}) as Record<string, unknown>);
  const where = $derived(params.foreach === 'repos' ? 'repos' : params.cwd === 'context' ? 'context' : 'worktree');
  const outFile = $derived(paramStr(node, 'output_file'));
  /** A bare file name in the run folder (engine.md §7.2, DR-002). */
  const outBad = $derived(!!outFile && (/[\\/\0]/.test(outFile) || outFile.includes('..')));

  function setWhere(v: string): void {
    set('foreach', v === 'repos' ? 'repos' : undefined);
    set('cwd', v === 'context' ? 'context' : undefined);
  }
</script>

<label for="{ids}-cmd">Command</label>
<textarea dir="ltr" id="{ids}-cmd" class="rs-cmd" rows="4" spellcheck="false" placeholder={'bash ./scripts/gates.sh "$OTTO_WORKTREE" "$OTTO_BASE"'}
  value={paramStr(node, 'command')} oninput={(e) => set('command', e.currentTarget.value)}></textarea>
<p class="node-hint">Runs with <code>/bin/sh -c</code>. Use <code>$OTTO_WORKTREE</code>, <code>$OTTO_BASE</code>, <code>$OTTO_BASE_BRANCH</code>, <code>$OTTO_SOURCE</code>,
  <code>$OTTO_REPO</code>, <code>$OTTO_REPO_NAME</code>, <code>$OTTO_CONTEXT_DIR</code>, <code>$OTTO_RUN_ID</code> and <code>$OTTO_OUT</code> (the output file) — never paste values into the command.</p>
<label for="{ids}-where">Run in</label>
<select id="{ids}-where" value={where} onchange={(e) => setWhere(e.currentTarget.value)}>
  <option value="repos">Each repository’s worktree, one after another</option>
  <option value="worktree">The run’s working folder, once</option>
  <option value="context">The run folder (its step files), once</option>
</select>
<label for="{ids}-limit">Time limit (minutes)</label>
<input id="{ids}-limit" type="number" min="1" max="120" value={Math.round(paramNum(node, 'timeout_s', 900) / 60)}
  oninput={(e) => { const m = Number(e.currentTarget.value); set('timeout_s', m > 0 ? Math.min(7200, Math.max(10, Math.round(m * 60))) : 900); }} />
<label for="{ids}-out">Output file (optional)</label>
<input dir="ltr" id="{ids}-out" type="text" placeholder="gates.md" value={outFile} aria-invalid={outBad ? 'true' : undefined}
  oninput={(e) => set('output_file', e.currentTarget.value.trim())} />
{#if outBad}<p class="rs-err">Just a file name — no folders, no “..”. It is written in the run folder.</p>
{:else}<p class="node-hint">Its text becomes the step’s answer and what the next steps read.</p>{/if}
<label for="{ids}-fail">When it exits with an error</label>
<select id="{ids}-fail" value={paramBool(node, 'allow_failure') ? 'record' : 'fail'} onchange={(e) => set('allow_failure', e.currentTarget.value === 'record' || undefined)}>
  <option value="fail">Fail the step</option>
  <option value="record">Record it and continue</option>
</select>

<style>
  .rs-cmd {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .rs-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
</style>
