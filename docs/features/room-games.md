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

**Coins** lie in lines of five down every route, with more on the branches.
Each coin up to ten raises your top speed a little (about 6 % at ten). A
spin-out or a rescue costs three. Collected coins come back after 12 seconds.
The HUD shows your purse under the lap counter.

The four classic circuits run 1.4–1.9 km a lap. Each now has a route choice
and a new set-piece:

- **Sunny Meadow** has a lake. The main road crosses it on a **drawbridge**,
  and the Ford branch dips under the water beside it. When the leaves are up,
  the drawbridge is a jump: crest it at speed. A kart stopped in the gap
  between the leaves drops into the lake.
- **Downtown Dash** has an interchange: a climbing 270° loop that flies back
  over its own entry road. It also has a skate-park **half-pipe** on the
  boulevard and the Market Arcade. The arcade is a boost-only shortcut through
  deep mud that only pays with a mushroom or a star.
- **Canyon Run** has a gorge launch over the dry river and a half-pipe in the
  gulch. The Mine Shaft is a tunnel with crushers, about as long as the
  winding canyon road it replaces.
- **Frostbite Pass** has a banked glacier spiral that passes under its own
  first turn, and a bobsled half-pipe out of the spiral. Thin Ice is a
  rail-less, icy shortcut straight across the frozen lake.

The three Grand Prix tracks run 2.3–2.9 km a lap, about 4–6 minutes a race.
Each has three routes:

- **Coral Lagoon** is a figure eight. Its seabed road dives under the lagoon and
  passes beneath the bridge that later carries you back over it. There is a dune
  jump, banked jungle sweepers and the rail-less Boardwalk shortcut across a
  cove. On the east side you choose between the sea-cliff glider and the Reef
  Tunnel, a glass tube under the cove with rolling sea mines.
- **Magma Peak** has a lava-river launch and rolling boulders. A banked
  corkscrew tower climbs two and a quarter turns to a crater tunnel with
  crushers. From the summit you either glide down to the lava lake or take the
  Switchback Trail, a banked serpentine road with boulders. At the lava lake
  the main road loops round and the rail-less Obsidian Ridge cuts across.
- **Neon Skyway** is a sunset city with a freeway flyover and a broken-freeway
  jump. A banked corkscrew garage ramp drops into the old town, where the Back
  Alley shortcut runs over gravel past rolling barrels. You cross the harbour
  either through the glass tunnel under it or over the Harbour Bridge and its
  drawbridge.

Every fork is signed. A gantry about 70 m before the fork points to the
branch's side, and a sign stands where the roads split. Launches, drawbridges
and half-pipes have their own banners. On the minimap, shortcuts are dashed
yellow, boost-only cuts are dashed orange and the other routes are light blue.
Each fork has a dot, and jumps show as breaks. Each drawbridge is a bar that
turns red while its leaves are up. Branches rejoin the main road, and lap
progress, positions, wrong-way warnings and rescues all work on them. CPUs
choose a route each lap, and harder CPUs take shortcuts more often. A CPU takes
a boost-only cut only when it holds a mushroom or a star, and fires it in the
mud.

**Half-pipes**: the road edges curve up into riding walls. Ride high on a wall
off the end lip and you get a trick jump; trick in the air for a boost when you
land. The flat floor in the middle just drives through.

Water and lava behave differently. Under water the kart is slower and floatier,
the view tints blue and bubbles trail behind. Falling into water off a bridge,
the boardwalk, thin ice or an open drawbridge, falling into lava, or dropping
into a chasm rescues you a little way back. A rescue always lands before a
jump's or a drawbridge's run-up, and you get no progress credit for the fall.
Launch ramps throw you over their gap at racing speed. Everything that moves
(rollers, crushers and drawbridge leaves) follows the race clock, so every
player sees the same thing with no extra network traffic. Snapshots (version 3)
add each racer's coins and the list of collected coins.

## Storm Royale

Battle royale for up to 16 survivors (up to 4 people, bots fill the rest) on a
480 m island, **solo, in duos or in squads of four** (chosen in the setup next
to the bot count): Palm Isle (Harbor Town, the Rustworks factory, Whispering
Woods, Loot Lake and its river to the sea, Lookout Hill, Pine Farm, Container
Port, Center Camp) or Red Mesa (Dust Town, Oil Works, the Oasis, Cactus Grove,
Mesa Top, Canyon Camp, Crossroads). Towns and landmarks have walk-in,
multi-storey buildings with stairs, windows and parapet roofs; the factory has
a catwalk; water slows you to a swim. Drop from the balloon, skydive and
glide, then loot a five-slot hotbar: six weapons (sidearm, SMG, assault
blaster, pump scatter, sniper rail, rocket tube) in five rarities, ammo, heals
/ shields (bandages, med kits, mini shields, shield potions — hold fire to
use) and launch pads. Chests add a weapon, ammo, a consumable and wood.
Supply drops fall under a balloon (with a beam, and a square on the map) when
storm phases 1, 3 and 5 start closing; hold E on a landed one for an epic or
legendary weapon, ammo, two shield potions, metal and sometimes a launch pad.

**Teams.** Seats fill teams in order, so people who join a room play
together and bots fill the rest (a seat can also be pinned to a team). There
is no friendly fire; shots pass through teammates. A survivor dropped to zero
while a teammate still stands is knocked instead of eliminated: they crawl,
cannot shoot, build or loot, and bleed out over about 33 s (credited to
whoever knocked them). A teammate holding E next to them for 4 s revives them
with 30 health. When the last one standing goes down, the knocked go with
them, and the whole team shares its place. The last team standing wins
together (Victory Royale for every member). Once you are eliminated you
spectate a teammate, and the match runs on for as long as anyone on your team
is still in it (in solo it ends shortly after the last person is out). Teammates get a ring under their
feet and a name label in the team colour (visible through walls, red while
knocked), a dot on the map, and a row in the team panel (health and shield,
or the bleed-out bar); the results table marks teams by colour.

The pickaxe (F) harvests wood from trees, brick from rocks and metal from
vehicles, but every prop holds a limited amount (60 wood, 72 brick, 54 metal):
a spent tree falls (it stops blocking shots and survivors), a spent rock or
car stays but gives nothing, and both are whole again 60 s after the last
swing. Build mode (Q): walls, floors and ramps on a 4 m grid in wood, brick or
metal (10 each); pieces grow to full health, can be shot or broken, and
anything they were holding up collapses. G edits your own piece (wall →
doorway → window, flip a ramp). Launch pads (set pieces on the camp /
crossroads square, at the foot of each hill and in town, or deployed from the
hotbar onto open ground ahead) throw you about 20 m up; the glider opens on
the way down. The storm closes in seven phases (about seven minutes).

Bots loot (supply drops first), heal, harvest (never a spent tree), rotate
with the storm, use cover to reload, wall up or ramp when shot, and ramp-rush
toward enemies holding high ground. They go in and out of buildings by the
doors they can reach and climb the stairs to loot, chests and, late in the
match, rooftops. In duos and squads they drop with their team, stay near the
team's lead (a human teammate first), revive knocked teammates (by the stairs
if need be) and finish knocked enemies. HUD: health/shield, materials,
hotbar, build bar, minimap and full map (M) with supply drops and pads, storm
phase timer, alive / teams count, kill feed (knocks, revives, eliminations),
damage numbers, revive / bleed-out bars; after elimination you spectate (a
teammate first); results show placement, eliminations and damage. Controls:
WASD, mouse aim, click fire / place / use, right-click aim, 1–5 hotbar, F
pickaxe, Q build (1 wall, 2 floor, 3 ramp, R material, G edit), E pick up /
hold to open chests and supply drops / hold to revive, R reload, Space
jump/drop, Shift sprint, M map.

Guests see their own build preview at once (it follows the local camera and
piece choice from the freshest snapshot instead of waiting for the host's
echo). Structures are drawn as one persistent mesh per material whose buffers
are rewritten in place (only the materials that changed; health-only changes
touch the colours), so building does not reallocate geometry. For browser
tests, `window.__ottoGameTest = { stormScale: 0.06 }` (set before the page
loads) shortens every storm wait and shrink so a match ends in about a minute.

## Playing together

Choose **Invite people**, create a room and share the invitation; up to 8
(karts) or 4 (shooter) people join with it before the host starts. The host
runs the simulation; the daemon relays bounded inputs and snapshots (see the
[game-room contract](../contracts/game-rooms.md)). Guests pick their character
in the lobby. Browser verification and screenshots live in
[`docs/testing/room-games`](../testing/room-games/).
