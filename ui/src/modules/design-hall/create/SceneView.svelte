<script lang="ts">
  // The finished set as ONE scene: every made piece together, the camera
  // stepping back to see all of it, and any piece that isn't there named in
  // a plain outline on the picture. Voices, sounds and music play beside it.
  // Changing it is the same box as everywhere else, plus tap-to-pin notes.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { Pin3dAnchor } from '../../../lib/design/scene3d';
  import type { Scene3dDoc } from '../../../lib/design/scene3d/types';
  import type { Pin3dInput } from '../../../lib/design/scene3d/viewportInspection';
  import { scrollBehavior } from '../../../lib/motion';
  import { router } from '../../../lib/router.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import type { Production, ProductionItem } from '../../../lib/api/types';
  import AudioLine, { stopAll } from './AudioLine.svelte';
  import CreateModel from './CreateModel.svelte';
  import ItemThumb from './ItemThumb.svelte';
  import RigLine from './RigLine.svelte';
  import SteerBox, { type SteerPin } from './SteerBox.svelte';
  import { createHref, slugName, steerExample, theName } from './plan';
  import { saveClip, saveModel, saveProblem } from './saveToGame';
  import { onDestroy } from 'svelte';
  import { ScenePlayer, playOrder, prepareClips, type PlayLine } from './scenePlay';

  type ScenePin = SteerPin & { item_id?: string; artifact_id?: string; point?: [number, number, number]; anchor: Pin3dAnchor };

  interface Props {
    production: Production;
    busy: boolean;
    soundReady: boolean;
    /** What the last change did, said once above the picture: what already shows, and what came out the same. */
    said?: { same: boolean; done?: string | null; text: string } | null;
    onsteer: (comment: string, pins: ScenePin[]) => void;
    /** Make the pieces that didn't work again. */
    onretry: () => void;
    /** The scene so far, while the rest is still being made: no changing or saving yet. */
    making?: boolean;
    /** Words to start the change box with (notes written while it was being made). */
    initialSteer?: string;
    /** Open one piece to look at it (its moves and skeleton). */
    onlook?: (item: ProductionItem) => void;
  }
  let { production, busy, soundReady, said = null, onsteer, onretry, making = false, initialSteer = '', onlook }: Props = $props();

  const items = $derived((production.plan?.items ?? []).filter((i) => i.enabled));
  // A person standing in the scene while its skeleton is still coming (`made: "mesh"`) is there too.
  const inScene = (i: ProductionItem) => i.status === 'done' || i.made === 'mesh';
  const made = (id: string | undefined) => !id || items.some((i) => i.id === id && inScene(i));
  // A line whose speaker wasn't made isn't offered (no growl without a monster).
  const voices = $derived(items.filter((i) => i.kind === 'voice_line' && i.status === 'done' && i.result?.clip_id && made(i.details?.speaker)));
  const sounds = $derived(items.filter((i) => i.kind === 'sound' || i.kind === 'music'));
  const madeSounds = $derived(sounds.filter((i) => i.status === 'done' && i.result?.clip_id));
  const unmadeSounds = $derived(sounds.filter((i) => i.status !== 'done'));
  const pieces = $derived(items.filter((i) => i.kind === 'character' || i.kind === 'object'));
  // While it is being made, a person already in the scene whose skeleton is still coming is in the picture too.
  const models = $derived(pieces.filter((i) => i.result?.artifact_id && (inScene(i) || (making && i.status === 'working'))));
  const missing = $derived(pieces.filter((i) => !models.includes(i)));
  const title = $derived(production.plan?.title || 'My scene');
  /** What is still coming while it is being made (the pieces, voices and sounds not made yet). */
  const coming = $derived(making ? items.filter((i) => (i.status === 'waiting' && i.made !== 'mesh') || i.status === 'working') : []);
  // A new piece in the scene (or a new look of one) draws the scene again; the camera stays where it was.
  const sceneKey = $derived(models.map((m) => `${m.id}:${m.result?.version_id ?? ''}`).join(','));
  /** The scene object a made piece is (its model reference names the artifact). */
  const objectOf = (i: ProductionItem) => sceneDoc?.objects.find((o) => !!i.result?.artifact_id && ((o as { src?: string }).src ?? '').includes(i.result.artifact_id));
  /** Where a piece that sits on another one is, so a small one (a key at a neck) can be found —
   *  only when the scene really shows it there (near its host and up off the ground, or beside it). */
  function where(i: ProductionItem): string {
    const at = production.plan?.layout?.find((l) => l.item === i.id);
    // The plan's relation (`rel`), else its older words.
    const a = at?.rel ? { item: at.rel.to, where: at.rel.how === 'worn' ? (at.rel.part ?? 'worn') : at.rel.how } : (at?.attach_to ?? i.details?.attach_to);
    const host = a && items.find((x) => x.id === a.item);
    const [me, it] = host ? [objectOf(i), objectOf(host)] : [];
    if (!a || !host || !me || !it) return i.name;
    const apart = Math.hypot(me.position[0] - it.position[0], me.position[2] - it.position[2]);
    const t = theName(host.name);
    // In a container is said as in, even when it rests on the container's floor.
    const inside = /^(in|inside)$/i.test(a.where.trim());
    // At the front of a wearer whose neck the scene couldn't find, as it says.
    const put = (production.plan?.notes ?? []).map((x) => x.toLowerCase()).find((x) => x.startsWith(`i put ${theName(i.name)} `));
    if (put?.includes(' at the front of ')) return `${i.name}, at the front of ${t}`;
    if (apart < 1 && (inside || me.position[1] > it.position[1] + 0.05)) return `${i.name}, ${/neck/i.test(a.where) ? `on ${t}’s neck` : a.where === 'held' ? `in ${t}’s hand` : inside ? `in ${t}` : `on ${t}`}`;
    // "Beside" also when the scene says it put it there (a piece it couldn't hang at a big wearer's neck).
    const said = (production.plan?.notes ?? []).some((x) => x.toLowerCase().startsWith(`i put ${theName(i.name)} beside`));
    return apart < 1.5 || said ? `${i.name}, beside ${t}` : i.name;
  }
  /** Lines whose speaker isn't made yet, said once per speaker instead of hidden. */
  const waitingLines = $derived.by(() => {
    const by = new Map<string, number>();
    for (const v of items) if (v.kind === 'voice_line' && v.details?.speaker && !made(v.details.speaker)) by.set(v.details.speaker, (by.get(v.details.speaker) ?? 0) + 1);
    const count = (n: number) => (n === 1 ? 'line' : `${['two', 'three', 'four', 'five'][n - 2] ?? n} lines`);
    return [...by].map(([id, n]) => {
      const who = items.find((x) => x.id === id);
      const name = who ? theName(who.name) : 'they';
      const owner = name.charAt(0).toUpperCase() + name.slice(1);
      return `${owner}${owner.endsWith('s') ? '’' : '’s'} ${count(n)} will be here once ${name} is made.`;
    });
  });

  // Play the whole scene: the lines in order over the place's sounds, every person moving, the one speaking outlined.
  const order = $derived(playOrder(items));
  let player: ScenePlayer | null = null;
  let playing = $state(false);
  let played = $state(false);
  let speaking = $state<PlayLine | null>(null);
  const speaker = $derived(speaking?.speaker ? items.find((i) => i.id === speaking!.speaker) : undefined);
  // The clips are fetched ahead: a phone plays sound only from inside the tap.
  let urls = $state<Map<string, string> | null>(null);
  const clipKey = $derived(making ? '' : [...order.lines, ...order.beds].map((c) => c.clipId).join(','));
  $effect(() => {
    if (!clipKey) return;
    let gone = false;
    void prepareClips(order).then((u) => (gone ? revoke(u) : (urls = u)));
    return () => {
      gone = true;
      player?.stop();
      if (urls) revoke(urls);
      urls = null;
    };
  });
  const revoke = (u: Map<string, string>) => u.forEach((url) => url.startsWith('blob:') && URL.revokeObjectURL(url));
  function playScene(): void {
    if (!urls) return;
    stopAll();
    player?.stop();
    player = new ScenePlayer(
      (l) => (speaking = l),
      () => ((playing = false), (played = true)),
    );
    playing = true;
    player.start(order, urls);
  }
  onDestroy(() => player?.stop());

  let pins = $state<ScenePin[]>([]);
  let steerTyped = $state(false);
  let sceneDoc = $state<Scene3dDoc | null>(null);
  let playTick = $state(0);
  let saving = $state(false);
  const pinInputs = $derived<Pin3dInput[]>(pins.map((p) => ({ id: String(p.n), n: p.n, body: p.note, muted: false, anchor: p.anchor })));

  /** Which made piece a tapped scene object is (its model reference names the artifact). */
  function itemFor(anchor: Pin3dAnchor): ProductionItem | null {
    const obj = sceneDoc?.objects.find((o) => o.id === anchor.object_id);
    const src = (obj as { src?: string } | undefined)?.src ?? '';
    return models.find((m) => m.result?.artifact_id && src.includes(m.result.artifact_id)) ?? null;
  }

  function addPin(anchor: Pin3dAnchor): void {
    const n = (pins.at(-1)?.n ?? 0) + 1;
    const it = itemFor(anchor);
    pins = [...pins, { n, on: it ? it.name.toLowerCase() : null, note: '', anchor, item_id: it?.id, artifact_id: it?.result?.artifact_id ?? undefined, point: anchor.position }];
    queueMicrotask(() => document.getElementById(`sb-pin-${n}`)?.focus());
  }

  function whyMissing(i: ProductionItem): string {
    if (making && i.status !== 'failed' && i.status !== 'skipped') return 'still coming';
    return i.status === 'failed' ? 'it didn’t come out right this time' : 'not made yet';
  }

  /** "Say it differently": the box, in view and ready to type in. */
  function sayAgain(): void {
    const box = document.getElementById('create-steer');
    box?.scrollIntoView({ block: 'center', behavior: scrollBehavior() });
    box?.focus({ preventScroll: true });
  }

  async function saveAll(): Promise<void> {
    if (saving) return;
    saving = true;
    const folder = `assets/${slugName(title, 'my-scene')}`;
    try {
      for (const m of models) {
        await saveModel(m.result!.artifact_id!, (n) => `${folder}/${slugName(m.name, m.kind)}${n > 1 ? `-${n}` : ''}.glb`, m.result?.version_id);
      }
      for (const v of [...voices, ...madeSounds]) {
        await saveClip(v.result!.clip_id!, (n) => `${folder}/${slugName(v.name, 'sound')}${n > 1 ? `-${n}` : ''}.wav`);
      }
      toasts.success('Saved', 'Everything is in your game’s folder.');
    } catch (e) {
      toasts.warn('I couldn’t save everything', saveProblem(e));
    } finally {
      saving = false;
    }
  }
</script>

<div class="sv">
  <div class="sv-head">
    <h1 class="cr-h1">{title}</h1>
    <p class="cr-sub">{making ? 'Here is what I made so far. The rest is still coming.' : missing.length ? 'Here is what I made so far.' : 'Here is everything together.'} Drag to look around.</p>
    {#if said}
      <div class="cr-note sv-said" data-testid="create-said">
        <Icon name={said.same ? 'info' : 'check'} size={16} />
        <div>
          {#if said.done}<p>{said.done}</p>{/if}
          <p>{said.text}</p>
        </div>
      </div>
      {#if said.same}
        <button type="button" class={steerTyped ? 'cr-secondary' : 'cr-primary'} onclick={sayAgain} data-testid="create-say-again">Say it differently</button>
      {/if}
    {/if}
  </div>

  <div class="sv-main">
    {#if production.scene_artifact_id && models.length}
      <div class="cr-stage sv-stage">
        {#key sceneKey}<CreateModel
          artifactId={production.scene_artifact_id}
          as="scene"
          viewKey={production.id}
          label={title}
          pickable
          thumbnail
          pins={pinInputs}
          onpick={addPin}
          onscene={(d) => (sceneDoc = d)}
          playing={playing}
          highlight={speaker ? (objectOf(speaker)?.id ?? null) : null}
        />{/key}
      </div>
      {#if !making && (order.lines.length || order.beds.length)}
        <div class="sv-play">
          <button type="button" class="cr-secondary" disabled={!urls} onclick={() => (playing ? player?.stop() : playScene())} data-testid="create-play-scene">
            <Icon name={playing ? 'stop' : 'play'} size={16} /> {playing ? 'Stop' : played ? 'Play again' : 'Play the scene'}
          </button>
          <p class="sv-caption" aria-live="polite" data-testid="create-play-caption">{#if speaking}<b>{speaker?.name ?? 'Narrator'}:</b> {speaking.text ?? ''}{/if}</p>
        </div>
      {/if}
      <ul class="sv-pieces" aria-label="In the picture">
        {#each models as m (m.id)}
          <li data-testid="create-piece">
            <button type="button" class="sv-piece" onclick={() => onlook?.(m)} aria-label={`${where(m)}: look at it`}>
              <Icon name={m.kind === 'character' ? 'user' : 'box'} size={14} /> {where(m)}
            </button>
            {#if m.kind === 'character'}<RigLine artifactId={m.result!.artifact_id!} versionId={m.result?.version_id} />{/if}
          </li>
        {/each}
      </ul>
      {#if missing.length && !making}
        <ul class="sv-missing" aria-label="Not in the picture">
          {#each missing as m (m.id)}
            <li data-testid="create-missing-piece"><Icon name={m.kind === 'character' ? 'user' : 'box'} size={16} /> {m.name}: {whyMissing(m)}</li>
          {/each}
        </ul>
        <button type="button" class="cr-secondary sv-retry" disabled={busy} onclick={onretry} data-testid="create-retry">
          <Icon name="refresh" size={16} /> Try again{missing.length > 1 ? ` (${missing.length} pieces)` : `: ${missing[0].name.toLowerCase()}`}
        </button>
      {/if}
    {:else if models.length}
      <ul class="sv-models">
        {#each models as m (m.id)}
          <li><ItemThumb artifactId={m.result!.artifact_id!} icon={m.kind === 'character' ? 'user' : 'box'} size={96} /><span>{m.name}</span></li>
        {/each}
      </ul>
    {/if}
  </div>

  <!-- Beside the picture where there is room: the box first, then the voices and Save. -->
  <div class="sv-side">
    {#if making}
      <div class="cr-card" data-testid="create-coming">
        <h2 class="cr-h2 sv-first"><Icon name="clock" size={16} /> Still coming</h2>
        <ul class="sv-coming">
          {#each coming as c (c.id)}<li>{c.name}</li>{/each}
        </ul>
        <p class="sv-later">When everything is made, you can change it or save it here.</p>
      </div>
    {:else}
    <SteerBox bind:pins bind:hasText={steerTyped} canPin={!!production.scene_artifact_id && models.length > 0} {busy} placeholder={steerExample(items)} onsend={(c, p) => onsteer(c, p as ScenePin[])} onremovepin={(n) => (pins = pins.filter((x) => x.n !== n))} initial={initialSteer} />
    {/if}
    {#if !making || voices.length || waitingLines.length || madeSounds.length || unmadeSounds.length}
    <div class="cr-card sv-card">
      {#if voices.length || waitingLines.length}
        <h2 class="cr-h2 sv-first"><Icon name="mic" size={16} /> Voices</h2>
        {#each voices as v (v.id)}
          <AudioLine clipId={v.result!.clip_id!} title={v.name} detail={v.details?.text && !v.name.includes(v.details.text) ? v.details.text : undefined} />
        {/each}
        {#each waitingLines as w (w)}<p class="sv-later" data-testid="create-lines-waiting">{w}</p>{/each}
      {/if}
      {#if madeSounds.length}
        <h2 class="cr-h2"><Icon name="volume" size={16} /> Sounds and music</h2>
        {#if madeSounds.length > 1}
          <button type="button" class="cr-secondary sv-together" onclick={() => (stopAll(), playTick++)}>
            <Icon name="play" size={16} /> Play them all together
          </button>
        {/if}
        {#each madeSounds as s (s.id)}
          <AudioLine clipId={s.result!.clip_id!} title={s.name} loop={!!s.details?.loop} {playTick} />
        {/each}
      {/if}
      {#if unmadeSounds.length}
        <p class="cr-note sv-notyet"><Icon name="info" size={16} /> <span><b>Not yet:</b> {soundReady ? `I couldn’t make ${unmadeSounds.map((s) => s.name.toLowerCase()).join(', ')}.` : 'Sounds and music aren’t ready yet.'}</span></p>
      {/if}
      <!-- Save is the one blue button only when the scene is whole, nothing is being typed and no change came out the same. -->
      {#if !making}
      <button type="button" class={missing.length || steerTyped || said?.same ? 'cr-secondary sv-save' : 'cr-primary sv-save'} disabled={saving || (!models.length && !voices.length && !madeSounds.length)} onclick={() => void saveAll()} data-testid="create-save-all">
        <Icon name="download" size={16} /> {saving ? 'Saving…' : 'Save'}
      </button>
      <!-- The next scene with the same people, things and place. -->
      <button type="button" class="cr-secondary sv-next" onclick={() => router.go(createHref('in', production.project_id ?? production.id))} data-testid="create-new-scene">
        <Icon name="plus" size={16} /> New scene in this project
      </button>
      {/if}
    </div>
    {/if}
  </div>
</div>

<style>
  .sv-play {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    flex-wrap: wrap;
    margin-block-start: var(--sp-2);
  }
  .sv-caption {
    margin: 0;
    min-block-size: 1.4em;
    font-size: var(--fs-m);
    color: var(--text-dim);
  }
  /* The title, the picture and what goes with it: one column on a narrow
     screen, the picture beside the rest on a wide one, and on a phone on its
     side the picture on the left with everything else on the right, so the
     whole picture fits the screen. */
  .sv {
    display: grid;
    grid-template-columns: minmax(0, 1.25fr) minmax(0, 1fr);
    grid-template-areas: 'head head' 'main side';
    gap: 0 var(--sp-4);
    align-items: start;
  }
  .sv-head {
    grid-area: head;
  }
  .sv-main {
    grid-area: main;
  }
  .sv-side {
    grid-area: side;
    display: grid;
    gap: var(--sp-4);
  }
  .sv-side :global(.sb),
  .sv-side :global(.sb-open) {
    margin-block-start: 0;
  }
  @media (max-width: 1024px) {
    .sv {
      grid-template-columns: minmax(0, 1fr);
      grid-template-areas: 'head' 'main' 'side';
    }
    .sv-side {
      margin-block-start: var(--sp-4);
    }
  }
  @media (max-height: 500px) and (orientation: landscape) {
    .sv {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
      grid-template-areas: 'main head' 'main side';
    }
    .sv-side {
      margin-block-start: var(--sp-3);
    }
  }
  .sv-said p {
    margin: 0;
  }
  .sv-said p + p {
    margin-block-start: var(--sp-1);
  }
  .sv-stage {
    position: relative;
  }
  .sv-missing {
    margin: var(--sp-2) 0 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
  }
  .sv-missing li {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    padding: var(--sp-1) var(--sp-3);
    border: 2px dashed var(--border-strong);
    border-radius: var(--radius-m);
    background: none;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .sv-said {
    margin-block: var(--sp-2) var(--sp-3);
  }
  .sv-pieces {
    margin: var(--sp-2) 0 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1) var(--sp-3);
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .sv-pieces li {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0 var(--sp-2);
  }
  .sv-piece {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    min-block-size: var(--cr-hit);
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    text-decoration: underline;
    text-underline-offset: 3px;
    cursor: pointer;
  }
  .sv-piece:focus-visible {
    outline: 3px solid var(--accent-text);
  }
  .sv-coming {
    margin: 0;
    padding-inline-start: var(--sp-5);
    font-size: var(--fs-m);
  }
  .sv-later {
    margin: var(--sp-3) 0 0;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .sv-retry {
    margin-block-start: var(--sp-2);
  }
  .sv-together {
    inline-size: 100%;
    margin-block-end: var(--sp-2);
  }
  .sv-first {
    margin-block-start: 0;
  }
  .sv-notyet {
    margin-block: var(--sp-3) 0;
  }
  .sv-save {
    margin-block-start: var(--sp-5);
  }
  .sv-next {
    margin-block-start: var(--sp-2);
  }
  .sv-models {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-3);
  }
  .sv-models li {
    display: grid;
    justify-items: center;
    gap: var(--sp-1);
  }
</style>
