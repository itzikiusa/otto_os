// arch-2-08: one room-game catalog. `rooms/games/catalog.json` is read by the
// daemon (game_rooms/types.rs include_str!s it to validate POST /game-rooms)
// and by the chooser (games/catalog.ts). This pins that the client's real map
// implementations cover exactly the catalog's ids, so a map added on one side
// only fails here instead of shipping a map the relay refuses (or hides).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { TRACK_DEFS } from '../src/modules/rooms/games/kart/tracks/defs.ts';
import { SHOOTER_MAPS } from '../src/modules/rooms/games/shooter/catalog.ts';

const UI = join(import.meta.dirname, '..');
const read = (path: string) => readFileSync(join(UI, path), 'utf8');
const CATALOG = JSON.parse(read('src/modules/rooms/games/catalog.json')) as {
  games: Record<string, { capacity: number; maps: string[] }>;
};

test('catalog lists exactly the two wire game kinds', () => {
  assert.deepEqual(Object.keys(CATALOG.games).sort(), ['kart', 'shooter']);
  assert.equal(CATALOG.games.kart.capacity, 8);
  assert.equal(CATALOG.games.shooter.capacity, 4);
});

test('every kart track and shooter map the client implements is in the catalog, and only those', () => {
  assert.deepEqual([...CATALOG.games.kart.maps].sort(), Object.keys(TRACK_DEFS).sort());
  assert.deepEqual([...CATALOG.games.shooter.maps].sort(), SHOOTER_MAPS.map(m => m.id).sort());
});

test('the chooser and the daemon read the catalog file instead of a hand-synced copy', () => {
  assert.match(read('src/modules/rooms/games/catalog.ts'), /from '\.\/catalog\.json'/);
  const rust = readFileSync(join(UI, '..', 'crates/otto-server/src/game_rooms/types.rs'), 'utf8');
  assert.match(rust, /include_str!\("\.\.\/\.\.\/\.\.\/\.\.\/ui\/src\/modules\/rooms\/games\/catalog\.json"\)/);
  // No literal map list survives in the Rust enum.
  assert.doesNotMatch(rust, /"meadow",\s*"downtown"/);
  // The wire map union still names every catalog id (P06 owns that file).
  const wire = read('src/lib/api/game-room-types.ts');
  for (const id of [...CATALOG.games.kart.maps, ...CATALOG.games.shooter.maps]) assert.match(wire, new RegExp(`'${id}'`));
});
