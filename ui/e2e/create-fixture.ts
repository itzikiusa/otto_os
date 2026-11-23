// Create (plain-words front door) E2E fixture, shared by desktop-create and
// mobile-create specs:
//  • a FAKE local Otto Forge: a tiny shell script that prints realistic
//    progress lines and copies a real rigged sample model, wired in as the
//    built-in provider's executable (so no test ever starts real GPU work);
//  • the owner's own scene as a canned Smart plan, served by an in-page mock
//    of the productions API (the server lands separately), whose results
//    point at real models and voice clips seeded in the isolated daemon;
//  • the "would a six-year-old get it" checks: no technical words, one
//    primary button, big touch targets, no sideways scrolling.
import { expect, type APIRequestContext, type Page, type Route } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';

const V1 = '/api/v1';
// REAL Otto Forge outputs (not stand-ins): the text-made knight with its
// stand/walk/run moves, and the chest.
export const SAMPLE = process.env.OTTO_E2E_SAMPLE_GLB ?? '/Users/itziklavon/otto-forge/out/text_to_asset/character/animlib.glb';
export const SAMPLE_PROP = process.env.OTTO_E2E_SAMPLE_PROP ?? '/Users/itziklavon/otto-forge/out/text_to_asset/prop_check/fixed.glb';
export const FORGE_ROOT = process.env.OTTO_E2E_FORGE_ROOT ?? '/Users/itziklavon/otto-forge';
// One man before and after a real look change, both REAL Otto Forge outputs from the second Smart
// mode acceptance run (real planner, real Forge; out/smart-acceptance-v2, see its README.md):
// models/man-v3.glb is the man made (generate, rig, animate: idle/walk/run) from the owner's scene's
// plan ("a rugged middle-aged male warrior, …", plans/plan-run1.json), and models/man-v6.glb is the
// same man made again after the comment "the man should look older and tired" (the planner's new
// prompt "a rugged old male warrior in his sixties, …" is in plans/plan-after-steer.json). The fake
// engine gives them back for those two prompts (the e2e daemon's fixed plan uses the same words),
// so the before/after screen shows a look change that really worked. Read in place, like the other
// Forge files here; never copied into the repo.
export const MAN_BEFORE = process.env.OTTO_E2E_MAN_BEFORE ?? join(FORGE_ROOT, 'out/smart-acceptance-v2/models/man-v3.glb');
export const MAN_AFTER = process.env.OTTO_E2E_MAN_AFTER ?? join(FORGE_ROOT, 'out/smart-acceptance-v2/models/man-v6.glb');
export const samplesPresent = (): boolean => [SAMPLE, SAMPLE_PROP, MAN_BEFORE, MAN_AFTER, join(FORGE_ROOT, 'bin/forge-library')].every((p) => existsSync(p));

/** Clean CC0 pieces from Otto Forge's hand-made library (forge-library), fetched at setup, never copied into the repo. */
export interface LibraryPieces {
  chest: string;
  key: string;
  coins: string;
  sword: string;
  /** A stand-in for the monster: the library's one-eyed alien (no generated monster exists yet). */
  monster: string;
}
let libCache: LibraryPieces | null = null;
export function libraryPieces(): LibraryPieces {
  if (libCache) return libCache;
  const out = mkdtempSync(join(tmpdir(), 'otto-e2e-library-'));
  const ids: Record<keyof LibraryPieces, string> = {
    chest: 'fantasy-props-megakit/chest-wood',
    key: 'fantasy-props-megakit/key-gold',
    coins: 'fantasy-props-megakit/coin-pile',
    sword: 'fantasy-props-megakit/sword-bronze',
    monster: 'modular-scifi-megakit/alien-cyclop',
  };
  const got = {} as LibraryPieces;
  for (const [k, id] of Object.entries(ids) as [keyof LibraryPieces, string][]) {
    const file = join(out, `${k}.glb`);
    execFileSync(join(FORGE_ROOT, 'bin/forge-library'), ['get', id, '--output', file], { stdio: 'ignore' });
    got[k] = file;
  }
  libCache = got;
  return got;
}

/** Words a six-year-old should never meet in Create. */
export const FORBIDDEN = [
  'provider', 'polygon', 'triangle', 'triangles', 'LOD', 'LODs', 'GLB', 'glTF', 'rig', 'rigged', 'rigging', 'TTS',
  'seed', 'stage', 'stages', 'mesh', 'texture', 'preset', 'artifact', 'version', 'pipeline', 'job', 'API', 'retopo',
  // From the acceptance review: builder words, file names, the old steps.
  // "skeleton" is allowed: the owner wants a person's skeleton named where it was added ("Skeleton added · 3 moves").
  'Otto Forge', 'kit', 'Close-up', 'Object', 'assets', 'game folder', '\\.glb', 'Save to my game',
  'Save everything to my game', 'Where will you use it', 'Wants to', 'Moves today', 'Holds', 'Phone game',
  'Computer game', 'this computer', 'Tell me what you’ll change', 'Keep new', 'Go back to', 'I’ll do the rest',
  'husky', 'gravelly', 'Making it ready for games', 'Waiting its turn.*sound',
];

export interface CreateSeed {
  wsId: string;
  man: string;
  chest: string;
  pieces: { monster: string; sword: string; key: string; coins: string };
  /** The whole scene; without the key (its first try failed); with the man made bigger. */
  scenes: { full: string; noKey: string; bigMan: string };
  clips: { line1: string; line2: string; growl: string };
  /** The fake engine's folder (create-progress-fixture.ts paces its steps through it). */
  forgeDir: string;
}

// The man as the engine makes him, then his skeleton, then his moves, the chest and the
// coins, and the turnaround renders that stand for their drawn pictures: REAL Otto Forge
// outputs of the second Smart acceptance run, read in place (the making screen's walk-through).
const RUN2 = join(FORGE_ROOT, 'out/smart-acceptance-v2');
const RUN2_MODEL = (n: string) => join(RUN2, 'models', n);
const RUN2_RENDER = (n: string) => join(RUN2, 'renders', `${n}_turnaround_front.png`);
export const progressSamplesPresent = (): boolean =>
  ['man-v1.glb', 'man-v2.glb', 'man-v3.glb', 'chest-v1.glb', 'coins-v1.glb'].every((m) => existsSync(RUN2_MODEL(m))) &&
  ['man-v3', 'chest', 'coins'].every((r) => existsSync(RUN2_RENDER(r)));

/** The fake engine's folder (`bin/forge`; Forge's work folder `out/work/` beside it). A test that
 *  creates `gates/` in it holds each model's steps until it lets them go (create-progress-fixture.ts). */
function fakeForge(step: number, lib: LibraryPieces): string {
  const dir = mkdtempSync(join(tmpdir(), 'otto-e2e-fake-forge-'));
  mkdirSync(join(dir, 'bin'));
  const path = join(dir, 'bin', 'forge');
  writeFileSync(
    path,
    `#!/bin/sh
# Fake Otto Forge for the Create E2E: real progress lines, a real sample model.
root="$(cd "$(dirname "$0")/.." && pwd)"; gates="$root/gates"
# Help is answered at once, like the real engine (the daemon reads it to learn the flags).
for a in "$@"; do [ "$a" = --help ] && { echo "usage: forge {generate,rig,animate,library} --height-m H --longest-side-m L"; exit 0; }; done
stage="$1"; shift
out=""; mesh=""; prompt=""; key="$stage"; pic=""
while [ $# -gt 0 ]; do
  case "$1" in
    --output) out="$2"; shift 2;;
    --mesh) mesh="$2"; shift 2;;
    --prompt) prompt="$2"; shift 2;;
    *) shift;;
  esac
done
# With gates/ (a test that paces the making), each step of a model waits after its line
# until the test lets it go: gates/at-<key> says where it waits, gates/go-<key> lets it on.
p() {
  printf '{"stage":"%s","pct":%s,"message":"%s"}\\n' "$1" "$2" "$3" >&2
  if [ -d "$gates" ] && [ "$stage" != library ]; then
    echo "$1" > "$gates/at-$key"
    while [ -d "$gates" ] && [ ! -e "$gates/go-$key" ]; do sleep 0.1; done
    rm -f "$gates/at-$key" "$gates/go-$key"
  else
    sleep ${step}
  fi
}
# The making screen's walk-through: its man (a statue, then a skeleton, then moves), chest and coins.
case "$prompt" in
  *"tired fighter"*) key=man; src="${RUN2_MODEL('man-v1.glb')}"; pic="${RUN2_RENDER('man-v3')}";;
  *"small wooden treasure chest"*) key=chest; src="${RUN2_MODEL('chest-v1.glb')}"; pic="${RUN2_RENDER('chest')}";;
  *"shiny gold coins"*) key=coins; src="${RUN2_MODEL('coins-v1.glb')}"; pic="${RUN2_RENDER('coins')}";;
esac
case "$stage" in
  generate)
    [ -n "$prompt" ] && p concept 2 "drawing the concept picture (FLUX.2 klein 4B)"
    # Forge draws the concept picture first, in its work folder for this job (out/work/<job>-…).
    if [ -n "$pic" ]; then work="$root/out/work/$(basename "$out" .glb)-fake"; mkdir -p "$work"; cp "$pic" "$work/concept.png"; fi
    p matte 15 "cutting out the background (BiRefNet-lite)"
    p generate 20 "building the 3D model (Pixal3D)"
    # Only real results: the Forge-made man before and after "older and
    # tired", the Forge-made knight, the Forge-made chest for one thing, and
    # clean library pieces for the scene (the alien stands in for the
    # monster). Anything else fails. No real red-armoured knight exists, so
    # "make the armor red" gives the same knight back, and Create says so.
    [ -n "$src" ] || case "$prompt" in
      *"in his sixties"*) src="${MAN_AFTER}";;
      *"middle-aged male warrior"*) src="${MAN_BEFORE}";;
      *beast*|*monster*|*Monster*) src="${lib.monster}";;
      *sword*|*Sword*) src="${lib.sword}";;
      *key*|*Key*) src="${lib.key}";;
      *coin*|*Coin*) src="${lib.coins}";;
      *"iron bands"*) src="${lib.chest}";;
      *chest*|*Chest*) src="${SAMPLE_PROP}";;
      *knight*|*Knight*|*fighter*|*man,*|*woman*|*person*|*character*) src="${SAMPLE}";;
      *) printf '{"error":"could not make this one"}\\n' >&2; exit 1;;
    esac
    p finish 80 "AssetFurnace Finish"
    p gameready 90 "game-ready pass, 3 LODs, collision"
    cp "$src" "$out";;
  rig)
    p rig 5 "fitting a skeleton and skin weights (SkinTokens)"; p rig 60 "cleaning weights"
    if cmp -s "$mesh" "${RUN2_MODEL('man-v1.glb')}"; then cp "${RUN2_MODEL('man-v2.glb')}" "$out"; else cp "$mesh" "$out"; fi;;
  animate)
    p animate 5 "retargeting idle"; p animate 45 "retargeting walk"
    if cmp -s "$mesh" "${RUN2_MODEL('man-v2.glb')}"; then cp "${RUN2_MODEL('man-v3.glb')}" "$out"; else cp "$mesh" "$out"; fi;;
  *) p "$stage" 50 "working"; cp "$mesh" "$out";;
esac
printf '{"fake":true}' > "$out.manifest.json"
`,
  );
  chmodSync(path, 0o755);
  writeFileSync(join(dir, 'bin', 'forge-doctor'), '#!/bin/sh\necho ok\n');
  chmodSync(join(dir, 'bin', 'forge-doctor'), 0o755);
  // The real sound and music makers, beside the fake engine, only when asked
  // (they run the real models: about 90 s a sound, under a minute for music).
  if (process.env.OTTO_CREATE_REAL_SOUND === '1') {
    for (const tool of ['forge-sfx', 'forge-music']) {
      writeFileSync(join(dir, 'bin', tool), `#!/bin/sh\nexec "${join(FORGE_ROOT, 'bin', tool)}" "$@"\n`);
      chmodSync(join(dir, 'bin', tool), 0o755);
    }
  }
  return path;
}

export async function ok<T>(p: Promise<import('@playwright/test').APIResponse>, what: string): Promise<T> {
  const r = await p;
  expect(r.ok(), `${what}: ${r.status()} ${await r.text().catch(() => '')}`).toBeTruthy();
  return (await r.json()) as T;
}

export async function glbArtifact(ctx: APIRequestContext, base: string, wsId: string, title: string, file: string, kind?: string): Promise<string> {
  const a = await ok<{ artifact: { id: string } }>(
    ctx.post(`${base}${V1}/design/artifacts`, {
      data: { workspace_id: wsId, format: 'glb', studio: '3d', title, content_b64: readFileSync(file).toString('base64'), meta: kind ? { create: { kind, idea: title, where: 'pc' } } : {} },
    }),
    `seed ${title}`,
  );
  return a.artifact.id;
}

async function clip(ctx: APIRequestContext, base: string, wsId: string, text: string, who: string, artifactId: string): Promise<string> {
  const c = await ok<{ id: string }>(
    ctx.post(`${base}${V1}/product/tts/synthesize`, {
      data: { workspace_id: wsId, provider_id: 'macos-say', voice_id: 'Daniel', text, title: text.slice(0, 40), character_name: who, artifact_id: artifactId, format: 'wav' },
    }),
    'seed voice line',
  );
  return c.id;
}

/** One isolated workspace + the fake engine + the owner's scene's real pieces. */
export async function seedCreate(opts: { step?: number } = {}): Promise<CreateSeed> {
  const { ctx, base } = await apiCtx();
  try {
    const wsId = await seedWorkspace(ctx, base, 'My game');
    const lib = libraryPieces();
    const forge = fakeForge(opts.step ?? 1.2, lib);
    await ok(ctx.patch(`${base}${V1}/design/3d/providers/otto-forge`, { data: { cli_binary: forge, enabled: true } }), 'wire the fake engine');
    const man = await glbArtifact(ctx, base, wsId, 'The man', SAMPLE, 'character');
    const chest = await glbArtifact(ctx, base, wsId, 'Treasure chest', lib.chest, 'object');
    const pieces = {
      monster: await glbArtifact(ctx, base, wsId, 'The monster', lib.monster, 'character'),
      sword: await glbArtifact(ctx, base, wsId, 'Sword', lib.sword, 'object'),
      key: await glbArtifact(ctx, base, wsId, 'Key', lib.key, 'object'),
      coins: await glbArtifact(ctx, base, wsId, 'Gold coins', lib.coins, 'object'),
    };
    const obj = (id: string, name: string, x: number, z: number, s = 1) => ({
      id: `o-${name}`, name, type: 'gltf', src: `otto://design/${id}@latest`, position: [x, 0, z], rotation: [0, x > 0 ? -30 : 20, 0], scale: [s, s, s],
    });
    // Sized by the server's scale rule (productions/scene.rs): a person
    // 1.7 m, a big character 2.7 m, an object's longest side 0.6 m, a small
    // one 0.25 m (the man is 0.88 units, the alien 2.0, the chest 1.28 long,
    // the sword 1.13, the key 0.12, the coins 0.2), on the server's ground.
    const pieces3d = (manScale: number, key: boolean) => [
      obj(man, 'man', -1.0, 0.3, manScale),
      obj(pieces.sword, 'sword', -1.8 - (manScale - 1.93) * 0.4, 0.5, 0.53),
      obj(pieces.monster, 'monster', 1.3, -0.2, 1.34),
      // The key hangs at the monster's neck (85 % of its height, at its front), like the server puts it.
      ...(key ? [{ ...obj(pieces.key, 'key', 0.9, 0.5, 2.1), position: [0.9, 2.15, 0.5] }] : []),
      obj(chest, 'chest', 0.1, -0.9, 0.47),
      obj(pieces.coins, 'coins', 0.7, -0.6, 1.25),
      { id: 'ground', name: 'Ground', type: 'plane', position: [0, 0, 0], rotation: [-90, 0, 0], scale: [7, 7, 1], material: { color: '#a39a8a', roughness: 1 } },
    ];
    const sceneOf = async (objects: unknown[]) =>
      (
        await ok<{ artifact: { id: string } }>(
          ctx.post(`${base}${V1}/design/artifacts`, {
            data: {
              workspace_id: wsId,
              format: 'scene3d',
              studio: '3d',
              title: 'The sword, the monster and the gold',
              content: JSON.stringify({
                type: 'otto-scene3d', version: 2, background: '#d9d7d2', grid: false,
                environment: { preset: 'studio-soft', intensity: 1, background: true },
                camera: { position: [3, 2.5, 5], target: [0, 0.8, 0], fov: 50 },
                lights: [{ id: 'sun', name: 'Sun', type: 'directional', position: [4, 8, 6], target: [0, 0.8, 0], intensity: 1.5, shadow: true }, { id: 'sky', name: 'Sky', type: 'hemisphere', intensity: 0.8 }],
                objects,
                groups: [],
              }),
            },
          }),
          'seed scene',
        )
      ).artifact.id;
    // "Make the man bigger": 1.7 m → 2.3 m, the sword moved out of his way.
    const scenes = { full: await sceneOf(pieces3d(1.93, true)), noKey: await sceneOf(pieces3d(1.93, false)), bigMan: await sceneOf(pieces3d(2.6, true)) };
    const clips = {
      line1: await clip(ctx, base, wsId, 'This sword is mine now.', 'The man', man),
      line2: await clip(ctx, base, wsId, 'Stand aside, beast.', 'The man', man),
      growl: await clip(ctx, base, wsId, 'Grrr. Rrraaah!', 'The monster', pieces.monster),
    };
    return { wsId, man, chest, pieces, scenes, clips, forgeDir: dirname(dirname(forge)) };
  } finally {
    await ctx.dispose();
  }
}

// ── The owner's scene as a canned Smart plan ────────────────────────────────

export const OWNER_DESCRIPTION =
  'man pulling sword stuck in the ground, and fights a monster which guards a chest, key is on the neck of the monster, a lot of gold coins in chest, relevant voices around, man has deep voice, sort of broken, and sounds around the scene should fit, also for the fight scene, swords sounds and so on.';

type Item = Record<string, any>;
function ownerItems(): Item[] {
  const it = (id: string, kind: string, name: string, description: string, details: Record<string, unknown> = {}): Item => ({
    id, kind, name, description, enabled: true, details, status: 'waiting', progress_pct: 0, message: null, result: null,
  });
  return [
    it('man', 'character', 'The man', 'A tired fighter in worn armor.', {
      voice: 'deep and a bit broken', holds: [{ item: 'sword', socket: 'right_hand' }], actions: ['pull_sword', 'swing_sword'], moves: ['idle', 'walk', 'run'],
    }),
    it('monster', 'character', 'The monster', 'A big beast with a key on a cord around its neck. It guards the chest.', { holds: [{ item: 'key', socket: 'neck' }], moves: ['idle', 'walk'] }),
    it('sword', 'object', 'Sword', 'An old sword stuck in the ground.'),
    it('chest', 'object', 'Treasure chest', 'A wooden chest full of gold coins.'),
    it('key', 'object', 'Key', 'A big iron key on a cord.'),
    it('coins', 'object', 'Gold coins', 'A pile of shiny gold coins inside the chest.', { attach_to: { item: 'chest', where: 'inside' } }),
    it('line1', 'voice_line', 'The man: “This sword is mine now.”', 'He says it in a deep, broken voice.', { speaker: 'man', text: 'This sword is mine now.', voice: 'deep and a bit broken' }),
    it('line2', 'voice_line', 'The man: “Stand aside, beast.”', 'He says it when he sees the monster.', { speaker: 'man', text: 'Stand aside, beast.' }),
    it('growl', 'voice_line', 'The monster growls', 'A deep growl.', { speaker: 'monster', nonverbal: true }),
    it('clash', 'sound', 'Sword clashes', 'Metal hitting metal.'),
    it('place', 'sound', 'Wind in a cave', 'Wind and dripping water.', { loop: true }),
    it('music', 'music', 'Brave music', 'Fast, brave drums.', { loop: true }),
  ];
}

/** Seconds per piece, as the server counts them (sounds left out: no sound maker in these runs). */
const PIECE_SECS: Record<string, number> = { character: 780, object: 330, voice_line: 5 };
export const etaOf = (items: Item[]): number =>
  items
    .filter((i) => i.enabled && (i.status === 'waiting' || i.status === 'working'))
    .reduce((n, i) => n + Math.round(((PIECE_SECS[i.kind] ?? 0) * (100 - (i.status === 'working' ? i.progress_pct : 0))) / 100), 0);

const SOUND_NOTE = 'Sounds and music need the sound maker, which isn’t on this computer yet. I’ll leave them for now.';

/** The productions API, served in the page while the server lands. */
export class ProductionMock {
  status = 'planning';
  polls = 0;
  items = ownerItems();
  revisions: Item[] = [];
  scene: string | null = null;
  keyTried = false;
  /** The man was made bigger (a size change: the scene's, nothing made again). */
  bigMan = false;
  /** The pieces the last change makes again. */
  remake: string[] = [];
  calls: { method: string; path: string; body: unknown }[] = [];
  constructor(private seed: CreateSeed) {}

  private body(): Item {
    return {
      id: 'prod-owner',
      workspace_id: this.seed.wsId,
      description: OWNER_DESCRIPTION,
      use: 'pc',
      status: this.status,
      // Like the server: what the pieces still to make take, the one being made by how far along it is.
      eta_secs: this.status === 'running' || this.status === 'ready' ? etaOf(this.items) : null,
      plan: this.status === 'planning' && !this.revisions.length ? null : {
        title: 'The sword, the monster and the gold',
        summary: 'A tired man pulls a sword out of the ground and fights the monster that guards a chest of gold.',
        items: this.items,
        notes: ['People can stand, walk and run today, so the man stands ready next to the sword.', SOUND_NOTE],
        layout: [{ item: 'man', place: 'center' }, { item: 'monster', place: 'front', facing: 'man' }, { item: 'chest', place: 'back' }, { item: 'key', place: 'front', attach_to: { item: 'monster', where: 'neck' } }],
      },
      scene_artifact_id: this.scene,
      revisions: this.revisions,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    };
  }

  /** Each poll moves the work along a little, like the server would. */
  private tick(): void {
    this.polls++;
    if (this.status === 'planning' && this.polls > 2) {
      // Like the server: a piece made again waits with its old result kept aside;
      // with nothing to make (only a size changed) the new scene is there at once.
      for (const i of this.items) if (this.remake.includes(i.id)) Object.assign(i, { status: 'waiting', progress_pct: 0, previous_result: i.result, result: null });
      const made = this.items.some((i) => i.status === 'done');
      this.status = made && !this.remake.length ? 'done' : 'ready';
      if (this.status === 'done') this.scene = this.sceneNow();
    }
    if (this.status !== 'running') return;
    const work = this.items.filter((i) => i.enabled && i.status !== 'done' && i.status !== 'skipped' && i.status !== 'failed');
    const cur = work[0];
    if (!cur) {
      this.status = 'done';
      this.scene = this.sceneNow();
      return;
    }
    if (cur.kind === 'sound' || cur.kind === 'music') {
      Object.assign(cur, { status: 'skipped', message: 'Left for later — the sound maker isn’t on this computer yet.' });
      return;
    }
    if (cur.id === 'key' && !this.keyTried) {
      // The first try at the key fails, so the screens show "Try again".
      this.keyTried = true;
      Object.assign(cur, { status: 'failed', message: 'could not make this one' });
      return;
    }
    cur.status = 'working';
    cur.progress_pct = Math.min(100, cur.progress_pct + (cur.kind === 'voice_line' ? 100 : 50));
    if (cur.progress_pct >= 100) Object.assign(cur, { status: 'done', result: this.result(cur.id) });
  }

  /** The server's scene holds only made pieces, at their current sizes. */
  private sceneNow(): string {
    const s = this.seed.scenes;
    if (this.items.find((i) => i.id === 'key')!.status !== 'done') return s.noKey;
    return this.bigMan ? s.bigMan : s.full;
  }

  /** A piece made again comes back the same: the fake engine only has the one real model of each. */
  private result(id: string): Item {
    const s = this.seed;
    switch (id) {
      case 'man':
        return { artifact_id: s.man, version_id: null };
      case 'monster':
        return { artifact_id: s.pieces.monster, version_id: null };
      case 'sword':
        return { artifact_id: s.pieces.sword, version_id: null };
      case 'key':
        return { artifact_id: s.pieces.key, version_id: null };
      case 'coins':
        return { artifact_id: s.pieces.coins, version_id: null };
      case 'line1':
        return { clip_id: s.clips.line1 };
      case 'line2':
        return { clip_id: s.clips.line2 };
      case 'growl':
        return { clip_id: s.clips.growl };
      default:
        return { artifact_id: s.chest, version_id: null };
    }
  }

  async install(page: Page): Promise<void> {
    await page.route('**/api/v1/design/productions**', async (route: Route) => {
      const req = route.request();
      const url = new URL(req.url());
      const body = req.postDataJSON?.() ?? null;
      this.calls.push({ method: req.method(), path: url.pathname, body });
      const json = (v: unknown, status = 200) => route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(v) });
      const tail = url.pathname.replace(/^.*\/design\/productions/, '');
      if (req.method() === 'POST' && tail === '') {
        this.status = 'planning';
        return json({ id: 'prod-owner', status: 'planning' }, 201);
      }
      if (req.method() === 'GET' && tail === '') {
        const b = this.body();
        return json(this.status === 'planning' ? [] : [{ id: b.id, workspace_id: b.workspace_id, description: b.description, use: b.use, status: b.status, title: b.plan?.title ?? '', created_at: b.created_at, updated_at: b.updated_at }]);
      }
      if (req.method() === 'GET') {
        this.tick();
        return json(this.body());
      }
      if (req.method() === 'PATCH') {
        for (const u of (body as { items: Item[] }).items) Object.assign(this.items.find((i) => i.id === u.id)!, u);
        return json(this.body());
      }
      if (tail.endsWith('/start')) {
        // Like the server: start makes every piece that isn't done (a failed one again).
        for (const i of this.items) if (i.enabled && i.status === 'failed') Object.assign(i, { status: 'waiting', progress_pct: 0, message: null });
        this.status = 'running';
        return json(this.body());
      }
      if (tail.endsWith('/cancel')) {
        this.status = 'cancelled';
        for (const i of this.items) if (i.status === 'waiting' || i.status === 'working') i.status = 'skipped';
        return json(this.body());
      }
      if (tail.endsWith('/steer')) {
        const b = body as { comment: string; pins: { item_id?: string; note: string }[] };
        if (this.items.some((i) => i.status === 'done')) {
          // "Make the man bigger" is his size; a pinned note is about the piece it is on
          // (a note on no piece is about the man), and that piece is made again with it.
          this.bigMan = true;
          this.remake = [...new Set(b.pins.map((p) => p.item_id ?? 'man'))];
          const names = this.remake.filter((id) => id !== 'man').map((id) => this.items.find((i) => i.id === id)!.name.toLowerCase());
          const man = this.remake.includes('man') ? 'make the man again, bigger and with your note' : 'make the man bigger';
          const summary = `I’ll ${man}${names.length ? `, and make ${names.join(' and ')} again with your note` : ''}.`;
          this.revisions.push({ id: `rev-${this.revisions.length + 1}`, comment: b.comment, summary, changed: [...new Set(['man', ...this.remake])], added: [], removed: [], done: ['The man is bigger.'], at: new Date().toISOString() });
        } else {
          // A change to the plan itself, said in words: "a bigger pile of coins".
          const coins = this.items.find((i) => i.id === 'coins')!;
          coins.description = 'A big pile of shiny gold coins next to the chest.';
          delete coins.details.attach_to;
          this.revisions.push({ id: `rev-${this.revisions.length + 1}`, comment: b.comment, summary: 'I’ll make a bigger pile of coins.', changed: ['coins'], added: [], removed: [], at: new Date().toISOString() });
        }
        this.status = 'planning';
        this.polls = 0;
        return json(this.body());
      }
      if (tail.endsWith('/revert')) {
        for (const i of this.items) if (this.remake.includes(i.id) && i.previous_result) Object.assign(i, { status: 'done', result: i.previous_result });
        this.bigMan = false;
        this.status = 'done';
        this.scene = this.sceneNow();
        return json(this.body());
      }
      return route.fallback();
    });
  }
}

// ── "Would a six-year-old know what to do next?" ────────────────────────────

export async function expectPlain(page: Page, step: string): Promise<void> {
  const root = page.locator('.create-page');
  await expect(root, `${step}: Create is on screen`).toBeVisible();
  const text = await root.innerText();
  for (const w of FORBIDDEN) {
    expect(new RegExp(`\\b${w}\\b`, 'i').test(text), `${step}: technical word “${w}” is visible`).toBe(false);
  }
  const primaries = await page.locator('.create-page .cr-primary:visible, .create-page .btn.primary:visible').count();
  expect(primaries, `${step}: at most one main button`).toBeLessThanOrEqual(1);
  const small = await page.evaluate(() => {
    const out: string[] = [];
    for (const el of document.querySelectorAll<HTMLElement>('.create-body button, .create-body input, .create-body textarea, .create-body [role="switch"]')) {
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height || getComputedStyle(el).visibility === 'hidden') continue;
      if (el.closest('.s3d-viewport')) continue;
      if (r.height < 43.5 || r.width < 43.5) out.push(`${el.tagName} “${(el.textContent || el.getAttribute('aria-label') || '').trim().slice(0, 30)}” ${Math.round(r.width)}×${Math.round(r.height)}`);
    }
    return out;
  });
  expect(small, `${step}: touch targets under 44 px`).toEqual([]);
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow, `${step}: sideways scrolling`).toBeLessThanOrEqual(2);
  // The main button never sits on top of anything else (checked by geometry).
  const covered = await page.evaluate(() => {
    const btn = document.querySelector<HTMLElement>('.create-page .cr-primary');
    if (!btn || !btn.offsetParent) return [];
    const b = btn.getBoundingClientRect();
    const out: string[] = [];
    for (const el of document.querySelectorAll<HTMLElement>('.create-body h1, .create-body h2, .create-body p, .create-body li, .create-body label, .create-body textarea, .create-body input, .create-body canvas, .create-body .cr-chip')) {
      if (el === btn || btn.contains(el) || el.contains(btn) || !el.offsetParent) continue;
      const r = el.getBoundingClientRect();
      const w = Math.min(r.right, b.right) - Math.max(r.left, b.left);
      const h = Math.min(r.bottom, b.bottom) - Math.max(r.top, b.top);
      if (w > 1 && h > 1) out.push(`${el.tagName} “${(el.textContent || '').trim().slice(0, 30)}”`);
    }
    return out;
  });
  expect(covered, `${step}: the main button covers content`).toEqual([]);
}

/** Evidence screenshots (only when OTTO_CREATE_EVIDENCE names a folder). */
export async function evidence(page: Page, project: string, name: string, opts: { keepScroll?: boolean } = {}): Promise<void> {
  const dir = process.env.OTTO_CREATE_EVIDENCE;
  if (!dir) return;
  if (!opts.keepScroll) await page.locator('.create-page .page-body').evaluate((el) => el.scrollTo(0, 0)).catch(() => {});
  const theme = await page.evaluate(() => localStorage.getItem('otto_scheme') ?? 'light');
  await page.waitForLoadState('networkidle').catch(() => {});
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  await page.screenshot({ path: join(dir, `${project}-${theme}-${name}.png`), fullPage: false, animations: 'disabled' });
  // A long screen is also captured whole: the window is made as tall as the page for one shot.
  const extra = await page.locator('.create-page .page-body').evaluate((el) => el.scrollHeight - el.clientHeight).catch(() => 0);
  if (extra > 4) {
    const vp = page.viewportSize()!;
    await page.setViewportSize({ width: vp.width, height: vp.height + extra });
    await page.locator('.create-page .page-body').evaluate((el) => el.scrollTo(0, 0)).catch(() => {});
    await page.waitForLoadState('networkidle').catch(() => {});
    await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
    await page.screenshot({ path: join(dir, `${project}-${theme}-${name}-full.png`), fullPage: false, animations: 'disabled' });
    // The tall window raised the shell's keyboard-free height (lib/viewportFit.ts keeps the
    // tallest seen at a width), so a focused field would later read as an on-screen keyboard
    // and hide the app frame. A width change starts a new baseline.
    const frame = () => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
    await page.setViewportSize({ width: vp.width + 1, height: vp.height });
    await frame();
    await page.setViewportSize(vp);
    await frame();
  }
}

export async function openCreate(page: Page, seed: CreateSeed, path = '', opts: { where?: string | null; theme?: 'light' | 'dark' } = {}): Promise<void> {
  await page.addInitScript(
    ([ws, where, theme]) => {
      localStorage.setItem('otto_workspace', ws as string);
      if (theme) localStorage.setItem('otto_scheme', theme as string);
      if (where) localStorage.setItem('otto.create.where', where as string);
      else if (where === null) localStorage.removeItem('otto.create.where');
    },
    [seed.wsId, opts.where === undefined ? 'pc' : opts.where, opts.theme ?? process.env.OTTO_CREATE_THEME ?? null],
  );
  await page.goto(`/#/design/create${path ? `/${path}` : ''}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.create-page')).toBeVisible({ timeout: 20_000 });
}

/** The 3D picture shows a real, coloured surface, not white shards (hidden game helpers). */
export async function expectColourful(page: Page, step: string, which = 0): Promise<void> {
  const canvas = page.locator('[data-testid="create-model"] canvas').nth(which);
  await expect(canvas).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  // Read the picture as a person sees it (a screenshot), decoded in the page.
  const shot = (await canvas.screenshot()).toString('base64');
  const { subject, white, colour } = await page.evaluate(async (b64) => {
    const img = new Image();
    img.src = `data:image/png;base64,${b64}`;
    await img.decode();
    const c = document.createElement('canvas');
    c.width = img.width;
    c.height = img.height;
    const g2 = c.getContext('2d')!;
    g2.drawImage(img, 0, 0);
    const d = g2.getImageData(0, 0, c.width, c.height).data;
    // The background, read inside the rounded corner (the corner pixel is the page's).
    const o = (24 * c.width + 24) * 4;
    const bg = [d[o], d[o + 1], d[o + 2]];
    // The ground the scene stands on, read at the bottom middle, is not a model either.
    const go = ((c.height - 8) * c.width + Math.round(c.width / 2)) * 4;
    const ground = [d[go], d[go + 1], d[go + 2]];
    const near = (r: number, g: number, b: number, x: number[]) => Math.abs(r - x[0]) + Math.abs(g - x[1]) + Math.abs(b - x[2]) < 40;
    let subject = 0;
    let white = 0;
    let colour = 0;
    for (let i = 0; i < d.length; i += 4) {
      const [r, g, b] = [d[i], d[i + 1], d[i + 2]];
      if (near(r, g, b, bg) || near(r, g, b, ground)) continue;
      subject++;
      if (Math.min(r, g, b) > 215) white++;
      if (Math.max(r, g, b) - Math.min(r, g, b) > 35) colour++;
    }
    return { subject, white, colour };
  }, shot);
  expect(subject, `${step}: something is drawn`).toBeGreaterThan(500);
  expect(white / subject, `${step}: share of near-white pixels on the model`).toBeLessThan(0.3);
  expect(colour / subject, `${step}: share of coloured pixels on the model`).toBeGreaterThan(0.15);
}
