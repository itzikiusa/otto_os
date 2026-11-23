<script lang="ts">
  // "Change something" for one made thing, with the same box as everywhere:
  // say what should be different, tap the model to pin notes on places, then
  // "Here's what I'll change" in one sentence (with the notes) and "Make the
  // changes". Each note is also kept as a pinned design comment (the 3D Studio
  // shows them). Otto can't edit a finished model in place yet, so a change
  // honestly makes it again from the idea plus the notes — and says so.
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { createComment, getArtifact } from '../../../lib/api/design';
  import { list3dJobs } from '../../../lib/api/media';
  import type { DesignArtifact, Media3dJob } from '../../../lib/api/types';
  import type { Pin3dAnchor } from '../../../lib/design/scene3d';
  import type { Pin3dInput } from '../../../lib/design/scene3d/viewportInspection';
  import { loadErrorText } from '../../../lib/loadError';
  import { router } from '../../../lib/router.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import CreateModel from './CreateModel.svelte';
  import SteerBox, { type SteerPin } from './SteerBox.svelte';
  import { createStore, type CreationMeta } from './createStore.svelte';
  import { WHERE, WHERE_ORDER, changePrompt, createHref, estimateSecs, timeText, type UseWhere } from './plan';

  interface Props {
    id: string;
  }
  let { id }: Props = $props();

  let artifact = $state<DesignArtifact | null>(null);
  let source = $state<Media3dJob | null>(null);
  let error = $state<string | null>(null);
  let pins = $state<(SteerPin & { anchor: Pin3dAnchor })[]>([]);
  /** The change asked for, waiting for "Make the changes". */
  let asked = $state<{ comment: string; notes: (SteerPin & { anchor: Pin3dAnchor })[] } | null>(null);
  let words = $state('');
  let applying = $state(false);
  const meta = $derived((artifact?.meta as { create?: CreationMeta } | undefined)?.create ?? null);
  const kind = $derived<'character' | 'object'>(meta?.kind === 'character' || source?.then_stages?.includes('rig') ? 'character' : 'object');
  const fromPicture = $derived(!!meta?.picture || (!!source && !source.prompt));
  const idea = $derived(source?.prompt || words.trim() || meta?.idea || artifact?.title || '');
  const where = $derived<UseWhere>(meta?.where ?? WHERE_ORDER.find((w) => WHERE[w].triangles === source?.poly_budget) ?? 'pc');
  const pinInputs = $derived<Pin3dInput[]>(pins.map((p) => ({ id: String(p.n), n: p.n, body: p.note, muted: false, anchor: p.anchor })));
  const what = $derived(kind === 'character' ? 'person' : 'thing');

  $effect(() => {
    const aid = id;
    untrack(() => void load(aid));
  });

  async function load(aid: string): Promise<void> {
    error = null;
    try {
      const [d, jobs] = await Promise.all([getArtifact(aid), list3dJobs({ artifact_id: aid, limit: 50 }).catch(() => [])]);
      artifact = d.artifact;
      source = jobs.find((j) => j.pipeline_stage === 'generate' || (j.then_stages?.length ?? 0) > 0) ?? null;
      words = (d.artifact.meta as { create?: CreationMeta })?.create?.idea ?? '';
    } catch (e) {
      error = loadErrorText(e);
    }
  }

  function addPin(anchor: Pin3dAnchor): void {
    const n = (pins.at(-1)?.n ?? 0) + 1;
    pins = [...pins, { n, on: null, note: '', anchor }];
    queueMicrotask(() => document.getElementById(`sb-pin-${n}`)?.focus());
  }

  const allNotes = $derived(asked ? [asked.comment, ...asked.notes.map((n) => n.note)].filter((x) => x.trim()) : []);

  async function apply(): Promise<void> {
    const a = artifact;
    const provider = createStore.engine.provider;
    const wsId = ws.currentId;
    if (!a || !provider || !wsId || !asked || applying) return;
    applying = true;
    try {
      for (const p of asked.notes) {
        await createComment(a.id, { body: p.note.trim(), version_id: a.head_version_id ?? undefined, pin_3d: { ...p.anchor, selection_kind: 'point' } });
      }
      const prompt = changePrompt(fromPicture ? words.trim() || a.title : idea, allNotes.map((text) => ({ text })));
      const w = WHERE[where];
      const job = await createStore.start(
        {
          workspace_id: wsId,
          provider_id: provider.id,
          stage: 'generate',
          then_stages: kind === 'character' ? ['rig', 'animate'] : [],
          target_artifact_id: a.id,
          prompt,
          output_format: 'glb',
          poly_budget: w.triangles,
          preset: w.preset,
        },
        { kind, idea: prompt, where, before: a.head_seq ?? undefined },
      );
      router.go(createHref('job', job.id));
    } catch {
      toasts.warn('I couldn’t start the change', 'Please try again in a moment.');
    } finally {
      applying = false;
    }
  }
</script>

{#if error}
  <EmptyState variant="page" tone="error" icon="warning" title="I couldn’t open this" body={error} actionLabel="Try again" actionIcon="refresh" onaction={() => void load(id)} />
{:else if artifact && asked}
  <h1 class="cr-h1">Here’s what I’ll change</h1>
  <div class="cv-say" data-testid="create-revision">
    <p>I’ll make your {what} again with these changes:</p>
    <ol>{#each allNotes as n (n)}<li>{n}</li>{/each}</ol>
  </div>
  <p class="cr-sub">It can come out a bit different. {timeText(estimateSecs(kind, false))}. Your old one stays safe.</p>
  {#if fromPicture}
    <label class="cv-label" for="cv-words">This was made from a picture. Say in words what it is:</label>
    <input id="cv-words" class="cv-input" dir="auto" maxlength="600" bind:value={words} placeholder="A knight in blue armor" />
  {/if}
  <div class="cr-sticky">
    <button type="button" class="cr-primary" disabled={applying || !createStore.engine.provider} onclick={() => void apply()} data-testid="create-apply">{applying ? 'Starting…' : 'Make the changes'}</button>
  </div>
  <button type="button" class="cr-quiet" onclick={() => (asked = null)}>Never mind</button>
{:else if artifact}
  <h1 class="cr-h1">Change something</h1>
  <p class="cr-sub">Tap the place you want to change, or just say what you want.</p>
  <div class="cr-two">
    <div class="cr-stage">
      <CreateModel artifactId={artifact.id} label={artifact.title} pickable pins={pinInputs} onpick={addPin} />
    </div>
    <SteerBox bind:pins canPin placeholder="Make the armor red…" onsend={(comment, notes) => (asked = { comment, notes: notes as (SteerPin & { anchor: Pin3dAnchor })[] })} onremovepin={(n) => (pins = pins.filter((x) => x.n !== n))} />
  </div>
{:else}
  <div class="cv-calm"><span class="spinner" role="img" aria-label="Loading"></span></div>
{/if}

<style>
  .cv-calm {
    display: grid;
    justify-items: center;
    gap: var(--sp-2);
    text-align: center;
    padding-block: var(--sp-7);
    margin-block-start: var(--sp-5);
  }
  .cv-say {
    font-size: var(--fs-l);
    line-height: 1.45;
    background: var(--accent-soft);
    border-radius: var(--radius-l);
    padding: var(--sp-4);
    margin: var(--sp-3) 0;
  }
  .cv-say p {
    margin: 0;
  }
  .cv-say ol {
    margin: var(--sp-2) 0 0;
    padding-inline-start: var(--sp-6);
  }
  .cv-label {
    display: block;
    margin-block: var(--sp-3) var(--sp-1);
    font-size: var(--fs-m);
  }
  .cv-input {
    inline-size: 100%;
    min-block-size: var(--cr-hit);
    padding: var(--sp-2);
    border: 1.5px solid var(--control-border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
  }
</style>
