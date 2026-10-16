// Source manifest for the room-game kits: every pack is a Kenney CC0 1.0 pack.
// PROVENANCE.md mirrors this list with the download URLs and license text.
import { join } from 'node:path';

/** slug -> { url, model folder } — each pack ships License.txt stating CC0 1.0. */
export const PACKS = {
  'mini-characters': { url: 'https://kenney.nl/assets/mini-characters', dir: 'Models/GLB format' },
  'blaster-kit': { url: 'https://kenney.nl/assets/blaster-kit', dir: 'Models/GLB format' },
  'car-kit': { url: 'https://kenney.nl/assets/car-kit', dir: 'Models/GLB format' },
  'toy-car-kit': { url: 'https://kenney.nl/assets/toy-car-kit', dir: 'Models/GLB format' },
  'racing-kit': { url: 'https://kenney.nl/assets/racing-kit', dir: 'Models/GLTF format' },
  'nature-kit': { url: 'https://kenney.nl/assets/nature-kit', dir: 'Models/GLTF format' },
  'survival-kit': { url: 'https://kenney.nl/assets/survival-kit', dir: 'Models/GLB format' },
  'platformer-kit': { url: 'https://kenney.nl/assets/platformer-kit', dir: 'Models/GLB format' },
  'food-kit': { url: 'https://kenney.nl/assets/food-kit', dir: 'Models/GLB format' },
  'city-kit-suburban': { url: 'https://kenney.nl/assets/city-kit-suburban', dir: 'Models/GLB format' },
  'city-kit-industrial': { url: 'https://kenney.nl/assets/city-kit-industrial', dir: 'Models/GLB format' },
  'city-kit-commercial': { url: 'https://kenney.nl/assets/city-kit-commercial', dir: 'Models/GLB format' },
  'sci-fi-sounds': { url: 'https://kenney.nl/assets/sci-fi-sounds', dir: 'Audio' },
  'impact-sounds': { url: 'https://kenney.nl/assets/impact-sounds', dir: 'Audio' },
  'interface-sounds': { url: 'https://kenney.nl/assets/interface-sounds', dir: 'Audio' },
  'digital-audio': { url: 'https://kenney.nl/assets/digital-audio', dir: 'Audio' },
  'voiceover-pack': { url: 'https://kenney.nl/assets/voiceover-pack', dir: 'Audio' },
};

export const modelPath = (root, entry) => join(root, entry.pack, PACKS[entry.pack].dir, `${entry.file}.glb`);
const list = (pack, prefix, files) => files.map(file => ({ pack, file, key: `${prefix}-${file}` }));

export const KITS = {
  'shooter-kit': [
    ...list('city-kit-suburban', 'house', ['building-type-a', 'building-type-c', 'building-type-d', 'building-type-f', 'building-type-h', 'building-type-k', 'building-type-m', 'building-type-o', 'building-type-r', 'building-type-t', 'fence-1x3', 'fence-low', 'tree-large', 'planter']),
    ...list('city-kit-industrial', 'ind', ['building-a', 'building-h', 'building-i', 'building-j', 'building-k', 'building-o', 'shipping-container-a', 'shipping-container-b', 'shipping-container-c', 'detail-tank', 'water-tower', 'chimney-large']),
    ...list('survival-kit', 'surv', ['barrel', 'box-large', 'box', 'fence', 'fence-fortified', 'metal-panel-screws', 'rock-a', 'rock-b', 'rock-c', 'rock-sand-a', 'rock-sand-b', 'tent-canvas', 'tent', 'structure-metal', 'workbench', 'campfire-pit', 'tree', 'tree-tall', 'tree-autumn', 'tree-log', 'grass-large']),
    ...list('nature-kit', 'nat', ['tree_default', 'tree_oak', 'tree_detailed', 'tree_pineTallA', 'tree_pineRoundC', 'tree_palmTall', 'tree_palmBend', 'tree_palmDetailedShort', 'cactus_tall', 'cactus_short', 'rock_largeA', 'rock_largeC', 'rock_tallB', 'rock_tallE', 'stone_tallB', 'plant_bushLarge', 'plant_bush', 'grass_large', 'flower_redA', 'flower_yellowB', 'log_large', 'stump_old']),
    ...list('blaster-kit', 'gun', ['blaster-a', 'blaster-j', 'blaster-d', 'blaster-l', 'blaster-e', 'blaster-h', 'clip-small', 'clip-large', 'crate-wide', 'crate-medium', 'bullet-foam-tip-thick']),
    ...list('platformer-kit', 'plat', ['heart', 'jewel']),
    ...list('car-kit', 'car', ['sedan', 'suv', 'police', 'delivery']),
  ],
  'kart-kit': [
    ...list('car-kit', 'kart', ['kart-oobi', 'kart-oodi', 'kart-ooli', 'kart-oopi', 'kart-oozi']),
    ...list('toy-car-kit', 'toy', ['item-box', 'item-banana', 'item-cone', 'gate-finish', 'gate', 'tree-pine', 'tree']),
    ...list('racing-kit', 'race', ['grandStandCovered', 'grandStand', 'overheadLights', 'flagCheckers', 'flagRed', 'flagGreen', 'tent', 'tentLong', 'barrierRed', 'barrierWhite', 'billboard', 'billboardLow', 'lightPostLarge', 'pitsGarage', 'pitsOffice', 'bannerTowerRed', 'bannerTowerGreen', 'pylon', 'fenceStraight', 'treeLarge', 'treeSmall']),
    ...list('nature-kit', 'nat', ['tree_default', 'tree_oak', 'tree_detailed', 'tree_fat', 'tree_pineTallA', 'tree_pineRoundC', 'tree_palmTall', 'tree_palmBend', 'cactus_tall', 'cactus_short', 'rock_largeA', 'rock_largeC', 'rock_tallB', 'rock_tallE', 'rock_tallG', 'cliff_large_rock', 'cliff_block_rock', 'plant_bushLarge', 'plant_bush', 'flower_redA', 'flower_yellowB', 'flower_purpleA', 'fence_simple', 'log_stack', 'tent_detailedOpen', 'mushroom_redGroup', 'stump_old']),
    ...list('platformer-kit', 'plat', ['tree-pine-snow', 'tree-snow', 'tree-pine-snow-small', 'bomb', 'star', 'mushrooms', 'rocks', 'stones', 'block-snow-large', 'flag']),
    ...list('food-kit', 'food', ['banana', 'mushroom']),
    ...list('city-kit-commercial', 'city', ['building-a', 'building-b', 'building-c', 'building-d', 'building-e', 'building-f', 'building-g', 'building-h', 'building-i', 'building-j', 'building-k', 'building-l', 'building-skyscraper-a', 'building-skyscraper-b', 'building-skyscraper-c', 'building-skyscraper-d', 'detail-awning', 'detail-parasol-a']),
    ...list('city-kit-suburban', 'house', ['tree-large', 'tree-small']),
  ],
};

/** Kenney mini characters share one rig; names are this game's character names. */
export const CHARACTERS = [
  ['male-a', 'Dex'], ['male-b', 'Bruno'], ['male-c', 'Officer Kai'], ['male-d', 'Milo'], ['male-e', 'Doc'], ['male-f', 'Theo'],
  ['female-a', 'Nova'], ['female-b', 'Sunny'], ['female-c', 'Rex'], ['female-d', 'Ivy'], ['female-e', 'Juno'], ['female-f', 'Rosa'],
].map(([id, name]) => ({ pack: 'mini-characters', file: `character-${id}`, key: id, name }));

export const ANIMATIONS = {
  source: { pack: 'mini-characters', file: 'character-male-a' },
  keep: ['idle', 'walk', 'sprint', 'jump', 'fall', 'die', 'drive', 'emote-yes', 'holding-both', 'holding-both-shoot', 'holding-right', 'holding-right-shoot', 'pick-up'],
};

const sfx = (pack, file, key) => ({ pack, file, key });
export const SOUNDS = [
  sfx('sci-fi-sounds', 'Audio/laserSmall_000.ogg', 'pistol'),
  sfx('sci-fi-sounds', 'Audio/laserSmall_002.ogg', 'smg'),
  sfx('sci-fi-sounds', 'Audio/laserRetro_002.ogg', 'rifle'),
  sfx('sci-fi-sounds', 'Audio/laserLarge_000.ogg', 'shotgun'),
  sfx('sci-fi-sounds', 'Audio/laserLarge_003.ogg', 'sniper'),
  sfx('sci-fi-sounds', 'Audio/thrusterFire_000.ogg', 'rocket'),
  sfx('sci-fi-sounds', 'Audio/explosionCrunch_000.ogg', 'explosion'),
  sfx('sci-fi-sounds', 'Audio/forceField_000.ogg', 'storm'),
  sfx('sci-fi-sounds', 'Audio/doorOpen_001.ogg', 'chest'),
  sfx('sci-fi-sounds', 'Audio/forceField_002.ogg', 'shield'),
  sfx('impact-sounds', 'Audio/impactPunch_medium_000.ogg', 'hit'),
  sfx('impact-sounds', 'Audio/impactGlass_light_000.ogg', 'shieldhit'),
  sfx('impact-sounds', 'Audio/impactBell_heavy_000.ogg', 'headshot'),
  sfx('impact-sounds', 'Audio/impactMetal_light_000.ogg', 'reload'),
  sfx('impact-sounds', 'Audio/footstep_grass_000.ogg', 'step'),
  sfx('impact-sounds', 'Audio/impactPlate_light_000.ogg', 'bump'),
  sfx('impact-sounds', 'Audio/impactSoft_heavy_000.ogg', 'land'),
  sfx('interface-sounds', 'Audio/tick_001.ogg', 'tick'),
  sfx('interface-sounds', 'Audio/confirmation_002.ogg', 'item'),
  sfx('interface-sounds', 'Audio/drop_002.ogg', 'drop'),
  sfx('interface-sounds', 'Audio/select_001.ogg', 'select'),
  sfx('digital-audio', 'Audio/powerUp2.ogg', 'pickup'),
  sfx('digital-audio', 'Audio/phaserUp3.ogg', 'boost'),
  sfx('digital-audio', 'Audio/zapThreeToneUp.ogg', 'turbo'),
  sfx('digital-audio', 'Audio/phaserDown2.ogg', 'spin'),
  sfx('digital-audio', 'Audio/zapThreeToneDown.ogg', 'eliminated'),
  sfx('digital-audio', 'Audio/threeTone1.ogg', 'lap'),
  sfx('digital-audio', 'Audio/twoTone2.ogg', 'finallap'),
  sfx('voiceover-pack', 'Male/3.ogg', 'voice-3'),
  sfx('voiceover-pack', 'Male/2.ogg', 'voice-2'),
  sfx('voiceover-pack', 'Male/1.ogg', 'voice-1'),
  sfx('voiceover-pack', 'Male/go.ogg', 'voice-go'),
  sfx('voiceover-pack', 'Male/you_win.ogg', 'voice-win'),
  sfx('voiceover-pack', 'Male/game_over.ogg', 'voice-over'),
  sfx('voiceover-pack', 'Male/hurry_up.ogg', 'voice-hurry'),
  sfx('voiceover-pack', 'Male/congratulations.ogg', 'voice-congrats'),
];
