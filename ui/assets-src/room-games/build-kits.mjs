#!/usr/bin/env node
// Builds the shipped room-game model kits from Kenney CC0 packs.
//
//   node ui/assets-src/room-games/build-kits.mjs <packs-dir>
//
// <packs-dir> holds each pack extracted into a folder named after its slug
// (see PACKS in sources.mjs; `fetch-packs.sh` downloads them). Every source
// model becomes one top-level node named by its kit key; descendant nodes are
// renamed `<key>:<name>` so names stay unique inside a kit (three.js would
// otherwise suffix duplicates). Characters ship as separate files on Kenney's
// shared mini-character rig; only `char-anims.glb` keeps the animation clips,
// which bind to any character by bone name.
//
// Output: ui/public/room-games/*.glb (meshopt-compressed, quantized) and
// ui/public/room-games/manifest.json (per-model bounds, sizes, hashes).
import { createHash } from 'node:crypto';
import { mkdirSync, readFileSync, writeFileSync, statSync, copyFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { Document, NodeIO } from '@gltf-transform/core';
import { ALL_EXTENSIONS, EXTMeshoptCompression } from '@gltf-transform/extensions';
import { dedup, prune, quantize, meshopt, mergeDocuments, getBounds, weld } from '@gltf-transform/functions';
import { MeshoptEncoder, MeshoptDecoder } from 'meshoptimizer';
import { KITS, CHARACTERS, ANIMATIONS, SOUNDS, modelPath } from './sources.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, '../../public/room-games');
const src = process.argv[2];
if (!src || !existsSync(src)) { console.error('usage: build-kits.mjs <packs-dir>'); process.exit(2); }
mkdirSync(out, { recursive: true });
mkdirSync(join(out, 'sfx'), { recursive: true });

await MeshoptEncoder.ready; await MeshoptDecoder.ready;
const io = new NodeIO().registerExtensions(ALL_EXTENSIONS).registerDependencies({ 'meshopt.encoder': MeshoptEncoder, 'meshopt.decoder': MeshoptDecoder });
const manifest = { generated: 'build-kits.mjs', license: 'CC0-1.0', kits: {}, characters: {}, sounds: {} };
const round = v => Math.round(v * 1000) / 1000;

async function finish(doc, file, { keepAnimations = false } = {}) {
  if (!keepAnimations) for (const a of doc.getRoot().listAnimations()) a.dispose();
  await doc.transform(dedup(), prune({ keepLeaves: false }), weld(), quantize({ quantizeNormal: 10, quantizePosition: 14 }), meshopt({ encoder: MeshoptEncoder, level: 'medium' }));
  doc.createExtension(EXTMeshoptCompression).setRequired(true);
  const bytes = await io.writeBinary(doc);
  writeFileSync(join(out, file), bytes);
  return { bytes: bytes.byteLength, sha256: createHash('sha256').update(bytes).digest('hex') };
}

for (const [kit, entries] of Object.entries(KITS)) {
  const doc = new Document();
  doc.createBuffer();
  const scene = doc.createScene(kit);
  const models = {};
  for (const entry of entries) {
    const source = await io.read(modelPath(src, entry));
    const map = mergeDocuments(doc, source);
    const root = doc.createNode(entry.key);
    for (const sceneIn of source.getRoot().listScenes()) {
      const merged = map.get(sceneIn);
      for (const child of merged.listChildren()) {
        const rename = n => { n.setName(`${entry.key}:${n.getName()}`); n.listChildren().forEach(rename); };
        rename(child);
        root.addChild(child);
      }
      merged.dispose();
    }
    // Remove the merged (now empty) source buffers so one buffer remains.
    scene.addChild(root);
    const b = getBounds(root);
    models[entry.key] = { pack: entry.pack, file: entry.file, min: b.min.map(round), max: b.max.map(round) };
  }
  for (const buffer of doc.getRoot().listBuffers().slice(1)) {
    for (const accessor of doc.getRoot().listAccessors()) if (accessor.getBuffer() === buffer) accessor.setBuffer(doc.getRoot().listBuffers()[0]);
    buffer.dispose();
  }
  for (const s of doc.getRoot().listScenes()) if (s !== scene) s.dispose();
  doc.getRoot().setDefaultScene(scene);
  manifest.kits[kit] = { ...(await finish(doc, `${kit}.glb`)), models };
  console.log(kit, Object.keys(models).length, 'models', manifest.kits[kit].bytes, 'bytes');
}

for (const character of CHARACTERS) {
  const doc = await io.read(modelPath(src, character));
  const b = getBounds(doc.getRoot().listScenes()[0]);
  manifest.characters[character.key] = { pack: character.pack, file: character.file, name: character.name, min: b.min.map(round), max: b.max.map(round), ...(await finish(doc, `char-${character.key}.glb`)) };
}
{
  const doc = await io.read(modelPath(src, ANIMATIONS.source));
  for (const a of doc.getRoot().listAnimations()) if (!ANIMATIONS.keep.includes(a.getName())) a.dispose();
  // Clips only: drop meshes so the file stays tiny; nodes keep bone names.
  for (const node of doc.getRoot().listNodes()) { node.setMesh(null); node.setSkin(null); }
  manifest.animations = { pack: ANIMATIONS.source.pack, file: ANIMATIONS.source.file, clips: ANIMATIONS.keep, ...(await finish(doc, 'char-anims.glb', { keepAnimations: true })) };
}
for (const sound of SOUNDS) {
  const from = join(src, sound.pack, sound.file);
  const to = join(out, 'sfx', `${sound.key}.ogg`);
  copyFileSync(from, to);
  const bytes = readFileSync(to);
  manifest.sounds[sound.key] = { pack: sound.pack, file: sound.file, bytes: statSync(to).size, sha256: createHash('sha256').update(bytes).digest('hex') };
}
writeFileSync(join(out, 'manifest.json'), JSON.stringify(manifest, null, 1) + '\n');
console.log('wrote', join(out, 'manifest.json'));
