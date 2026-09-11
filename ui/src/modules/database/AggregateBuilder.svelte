<script lang="ts">
  // Stage-by-stage aggregate pipeline builder (Mongo): pick stages from a
  // menu, fill their bodies (each pre-seeded with a template), reorder /
  // remove them, and watch the `db.<coll>.aggregate([...])` statement take
  // shape in the live preview. Insert writes it into the editor; Run runs it;
  // every stage row also has ▶ to run the pipeline UP TO that stage — the
  // quickest way to see what a `$group` or `$lookup` receives. The draft is
  // kept per connection in localStorage so closing the modal loses nothing.
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { formatMongo } from './mongo-format';
  import { isBalancedObject } from './query-filter';

  type StageOp =
    | '$match'
    | '$project'
    | '$sort'
    | '$limit'
    | '$skip'
    | '$group'
    | '$unwind'
    | '$lookup'
    | '$addFields'
    | '$count'
    | 'raw';
  interface Stage {
    op: StageOp;
    body: string;
  }

  interface Props {
    /** The collection the pipeline targets; null → editable, pre-filled from
     *  the selected schema node (if it is a collection). */
    collection: string | null;
    open: boolean;
    onclose: () => void;
    oninsert: (stmt: string) => void;
    onrun: (stmt: string) => void;
    /** Draft persistence key scope. */
    connId: string;
  }
  let { collection, open, onclose, oninsert, onrun, connId }: Props = $props();

  // Template per op — what a fresh stage's body starts as. `raw` is a whole
  // stage object for anything not in the menu (`$facet`, `$bucket`, …).
  const TEMPLATES: Record<StageOp, string> = {
    $match: '{ field: value }',
    $project: '{ _id: 0, field: 1 }',
    $sort: '{ field: -1 }',
    $limit: '10',
    $skip: '0',
    $group: '{ _id: "$field", count: { $sum: 1 } }',
    $unwind: '"$field"',
    $lookup: '{ from: "other", localField: "field", foreignField: "_id", as: "joined" }',
    $addFields: '{ field: "$other" }',
    $count: '"count"',
    raw: '{ $facet: { } }',
  };
  const OPS = Object.keys(TEMPLATES) as StageOp[];

  // Per-USER, per-connection — same shape as the store's `otto_db_tabs` /
  // `otto_db_view` keys, so two accounts sharing a device never inherit each
  // other's draft pipeline (root keeps the unnamespaced key).
  const LS_KEY = $derived(
    auth.isRoot
      ? `otto_db_pipeline:${connId}`
      : `otto_db_pipeline:user:${auth.me?.id ?? 'anonymous'}:${connId}`,
  );

  /** Last path segment of the selected schema node when it is a collection
   *  (`db:x/coll:orders` → `orders`); empty otherwise. */
  function selectedCollection(): string {
    const last = (database.selectedObjectPath ?? '').split('/').pop() ?? '';
    const m = last.match(/^(?:coll|collection):(.+)$/);
    return m ? m[1] : '';
  }

  let coll = $state('');
  let stages = $state<Stage[]>([]);
  let loadedFor: string | null = null;

  // (Re)load the per-connection draft whenever the modal opens. An explicit
  // `collection` prop always wins over the draft's remembered one.
  $effect(() => {
    if (!open) {
      loadedFor = null;
      return;
    }
    if (loadedFor === LS_KEY) return;
    loadedFor = LS_KEY;
    let draft: { collection?: string; stages?: Stage[] } = {};
    try {
      draft = JSON.parse(localStorage.getItem(LS_KEY) ?? '{}') as typeof draft;
    } catch {
      /* corrupt draft → start empty */
    }
    const valid = (draft.stages ?? []).filter(
      (s): s is Stage => !!s && typeof s.body === 'string' && OPS.includes(s.op),
    );
    stages = valid.length > 0 ? valid : [{ op: '$match', body: TEMPLATES.$match }];
    coll = collection ?? draft.collection ?? selectedCollection();
  });
  // Persist on every change while open.
  $effect(() => {
    if (!open || loadedFor !== LS_KEY) return;
    const snapshot = JSON.stringify({ collection: coll, stages: stages.map((s) => ({ op: s.op, body: s.body })) });
    try {
      localStorage.setItem(LS_KEY, snapshot);
    } catch {
      /* quota / private mode — the draft simply isn't remembered */
    }
  });

  /** Why a stage body can't be emitted, or null when it is fine. */
  function stageError(s: Stage): string | null {
    const b = s.body.trim();
    if (!b) return 'empty';
    switch (s.op) {
      case '$limit':
      case '$skip':
        return /^\d+$/.test(b) ? null : 'must be a non-negative integer';
      case '$unwind':
        return isBalancedObject(b) || /^(["']\$[^"']+["'])$/.test(b) ? null : 'must be "$field" or an { … } object';
      case '$count':
        return /^(["'])[^"']+\1$/.test(b) ? null : 'must be a quoted output field name';
      default:
        return isBalancedObject(b) ? null : 'must be a balanced { … } object';
    }
  }
  const errors = $derived(stages.map(stageError));
  const firstError = $derived.by(() => {
    const i = errors.findIndex((e) => e !== null);
    return i < 0 ? null : `Stage ${i + 1} (${stages[i].op}): ${errors[i]}`;
  });
  const canEmit = $derived(coll.trim() !== '' && stages.length > 0 && firstError === null);

  /** `db.<coll>.aggregate([...])` over the first `n` stages (all by default). */
  function statement(n = stages.length): string {
    const body = stages
      .slice(0, n)
      .map((s) => (s.op === 'raw' ? s.body.trim() : `{ ${s.op}: ${s.body.trim()} }`))
      .join(', ');
    return formatMongo(`db.${coll.trim()}.aggregate([${body}])`);
  }
  const preview = $derived(coll.trim() ? statement() : '');

  function addStage(op: StageOp): void {
    stages = [...stages, { op, body: TEMPLATES[op] }];
  }
  function move(i: number, d: -1 | 1): void {
    const j = i + d;
    if (j < 0 || j >= stages.length) return;
    const next = [...stages];
    [next[i], next[j]] = [next[j], next[i]];
    stages = next;
  }
  function remove(i: number): void {
    stages = stages.filter((_, k) => k !== i);
  }
  /** Reseed a stage's body when its op changes (only if still the old template). */
  function setOp(i: number, op: StageOp): void {
    const s = stages[i];
    const untouched = s.body.trim() === TEMPLATES[s.op].trim();
    stages[i] = { op, body: untouched ? TEMPLATES[op] : s.body };
  }
  function runUpTo(i: number): void {
    if (!coll.trim() || errors.slice(0, i + 1).some((e) => e !== null)) return;
    onrun(statement(i + 1));
    onclose();
  }
</script>

{#if open}
  <Modal title="Aggregate pipeline" width={760} {onclose}>
    <div class="ab">
      <label class="ab-row">
        <span class="ab-label">Collection</span>
        <input class="input mono ab-coll" bind:value={coll} placeholder="collection" spellcheck="false" />
        <span class="ab-hint">db.<strong class="mono">{coll.trim() || '…'}</strong>.aggregate([ … ])</span>
      </label>

      <ol class="ab-stages" aria-label="Pipeline stages">
        {#each stages as s, i (i)}
          <li class="ab-stage" class:err={errors[i] !== null}>
            <div class="ab-stage-head">
              <span class="ab-n mono">{i + 1}</span>
              <select class="input ab-op mono" value={s.op} onchange={(e) => setOp(i, e.currentTarget.value as StageOp)} aria-label="Stage {i + 1} operator">
                {#each OPS as op (op)}<option value={op}>{op === 'raw' ? 'raw stage' : op}</option>{/each}
              </select>
              {#if errors[i]}<span class="ab-err">{errors[i]}</span>{/if}
              <span class="grow"></span>
              <button class="icon-btn" title="Run the pipeline up to this stage" aria-label="Run up to stage {i + 1}" disabled={!coll.trim() || errors.slice(0, i + 1).some((e) => e !== null)} onclick={() => runUpTo(i)}><Icon name="play" size={11} /></button>
              <button class="icon-btn" title="Move up" aria-label="Move stage {i + 1} up" disabled={i === 0} onclick={() => move(i, -1)}><Icon name="arrowUp" size={11} /></button>
              <button class="icon-btn" title="Move down" aria-label="Move stage {i + 1} down" disabled={i === stages.length - 1} onclick={() => move(i, 1)}><Icon name="arrowDown" size={11} /></button>
              <button class="icon-btn" title="Remove stage" aria-label="Remove stage {i + 1}" onclick={() => remove(i)}><Icon name="trash" size={11} /></button>
            </div>
            <textarea class="input mono ab-body" rows={s.op === '$limit' || s.op === '$skip' || s.op === '$count' || s.op === '$unwind' ? 1 : 3} bind:value={s.body} spellcheck="false" aria-label="Stage {i + 1} body"></textarea>
          </li>
        {/each}
      </ol>

      <div class="ab-add">
        <Icon name="plus" size={11} />
        <select class="input ab-op mono" value="" onchange={(e) => { const v = e.currentTarget.value as StageOp | ''; if (v) addStage(v); e.currentTarget.value = ''; }} aria-label="Add stage">
          <option value="" disabled>Add stage…</option>
          {#each OPS as op (op)}<option value={op}>{op === 'raw' ? 'raw stage' : op}</option>{/each}
        </select>
      </div>

      <div class="ab-preview-head">Preview{#if firstError}<span class="ab-err"> · {firstError}</span>{/if}</div>
      <pre class="ab-preview mono">{preview || '— name the collection to see the statement —'}</pre>
    </div>

    {#snippet footer()}
      <button class="btn" onclick={onclose}>Cancel</button>
      <button class="btn" disabled={!canEmit} onclick={() => { oninsert(statement()); onclose(); }} title="Write the statement into the editor (not run)">Insert</button>
      <button class="btn primary" disabled={!canEmit} onclick={() => { onrun(statement()); onclose(); }} title="Write the statement into the editor and run it">Run</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .ab {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .ab-row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .ab-label {
    flex: 0 0 76px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .ab-coll {
    flex: 1 1 160px;
    min-width: 0;
  }
  .ab-hint {
    font-size: 11px;
    color: var(--text-dim);
  }
  .ab-stages {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .ab-stage {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .ab-stage.err {
    border-color: color-mix(in srgb, var(--status-exited) 55%, transparent);
  }
  .ab-stage-head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ab-n {
    width: 18px;
    text-align: right;
    font-size: 11px;
    color: var(--text-dim);
  }
  .ab-op {
    height: 24px;
    padding: 0 6px;
    font-size: 12px;
  }
  .ab-err {
    font-size: 11px;
    color: var(--status-exited);
  }
  .ab-body {
    width: 100%;
    box-sizing: border-box;
    font-size: 12px;
    resize: vertical;
  }
  .ab-add {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-dim);
  }
  .ab-preview-head {
    font-size: 11px;
    color: var(--text-dim);
    font-weight: 600;
  }
  .ab-preview {
    margin: 0;
    padding: 8px 10px;
    max-height: 220px;
    overflow: auto;
    font-size: 11.5px;
    line-height: 1.45;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .grow {
    flex: 1;
  }
</style>
