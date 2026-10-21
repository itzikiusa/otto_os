# Room games

Open **Rooms → Games**. Two 3D games built from Kenney CC0 models (see
[`ui/assets-src/room-games/PROVENANCE.md`](../../ui/assets-src/room-games/PROVENANCE.md)):

## Kart Grand Prix

Eight racers (humans plus CPU drivers on easy/normal/hard), seven tracks, three
laps with progress tracking, live ranking, minimap, lap times and a results
table. Drift (Space while steering) charges three mini-turbo levels; jump ramps
allow a mid-air trick boost; dash panels, ice and barrier walls vary each track;
leaving a grass track rescues you. Item boxes roll six items weighted by
position: Turbo and Triple Mushroom, Banana, Homing Bomb, Super Star and
Thunderbolt. Controls: W/S gas and brake, A/D steer, Space drift/trick, E item,
Q look back, R rescue.

The four classic circuits (Sunny Meadow, Downtown Dash, Canyon Run, Frostbite
Pass) run about 1.3 km a lap. The three Grand Prix tracks run 2.3–2.9 km a lap,
about 4–6 minutes a race:

- **Coral Lagoon** is a figure eight. Its seabed road dives under the lagoon and
  passes beneath the bridge that later carries you back over it. It also has a
  glider launch off the sea cliffs, a dune jump, banked jungle sweepers and a
  rail-less Boardwalk shortcut across a cove.
- **Magma Peak** has a lava-river launch and rolling boulders. A banked
  corkscrew tower climbs two and a quarter turns to a crater tunnel with
  crushers. A summit glider descends to a lava lake, where the main road loops
  round and the rail-less Obsidian Ridge cuts across.
- **Neon Skyway** is a sunset city with a freeway flyover and a broken-freeway
  jump. A banked corkscrew garage ramp drops into the old town, where the Back
  Alley shortcut runs over gravel past rolling barrels. A glass tunnel then
  runs under the harbour.

Branches rejoin the main road, and lap progress, positions, wrong-way warnings
and rescues all work on them. The minimap shows branches dashed and jumps as
breaks. CPUs choose a route each lap; harder CPUs take shortcuts more often.

Water and lava behave differently. Under water the kart is slower and floatier,
the view tints blue and bubbles trail behind. Falling into water off a bridge
or the boardwalk, falling into lava, or dropping into a chasm rescues you a
little way back. A rescue always lands before a jump's run-up, and you get no
progress credit for the fall. Launch ramps throw you over their gap at racing
speed. A trick in the air gives a boost on landing.

## Storm Royale

Battle royale for up to 16 survivors (up to 4 people, bots fill the rest) on a
480 m island: Palm Isle (Harbor Town, the Rustworks factory, Whispering Woods,
Loot Lake and its river to the sea, Lookout Hill, Pine Farm, Container Port,
Center Camp) or Red Mesa (Dust Town, Oil Works, the Oasis, Cactus Grove, Mesa
Top, Canyon Camp, Crossroads). Towns and landmarks have walk-in, multi-storey
buildings with stairs, windows and parapet roofs; the factory has a catwalk;
water slows you to a swim. Drop from the balloon, skydive and glide, then loot
a five-slot hotbar: six weapons (sidearm, SMG, assault blaster, pump scatter,
sniper rail, rocket tube) in five rarities, ammo, and heals / shields
(bandages, med kits, mini shields, shield potions — hold fire to use). Chests
add a weapon, ammo, a consumable and wood.

The pickaxe (F) harvests wood from trees, brick from rocks and metal from
vehicles. Build mode (Q): walls, floors and ramps on a 4 m grid in wood, brick
or metal (10 each); pieces grow to full health, can be shot or broken, and
anything they were holding up collapses. G edits your own piece (wall →
doorway → window, flip a ramp). The storm closes in seven phases (about seven
minutes). Bots loot, heal, harvest, rotate with the storm, use cover to
reload and wall up or ramp when shot. HUD: health/shield, materials, hotbar,
build bar, minimap and full map (M), storm phase timer, kill feed, damage
numbers; after elimination you spectate; results show placement,
eliminations and damage. Controls: WASD, mouse aim, click fire / place,
right-click aim, 1–5 hotbar, F pickaxe, Q build (1 wall, 2 floor, 3 ramp, R
material, G edit), E pick up / hold to open chests, R reload, Space jump/drop,
Shift sprint, M map.

## Playing together

Choose **Invite people**, create a room and share the invitation; up to 8
(karts) or 4 (shooter) people join with it before the host starts. The host
runs the simulation; the daemon relays bounded inputs and snapshots (see the
[game-room contract](../contracts/game-rooms.md)). Guests pick their character
in the lobby. Browser verification and screenshots live in
[`docs/testing/room-games`](../testing/room-games/).
