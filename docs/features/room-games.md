# Room games

Open **Rooms → Games**. Two 3D games built from Kenney CC0 models (see
[`ui/assets-src/room-games/PROVENANCE.md`](../../ui/assets-src/room-games/PROVENANCE.md)):

## Kart Grand Prix

Eight racers (humans plus CPU drivers on easy/normal/hard), seven tracks, three
laps with progress tracking, live ranking, minimap, lap times and a results
table. Dash panels, ice and barrier walls vary each track; leaving a grass
track rescues you. Controls: W/S gas and brake, A/D steer, Space drift/trick,
E item (hold S to throw a green shell or a bomb backwards), Q look back,
R rescue.

**Driving.** Hold Space while steering to drift. Sparks change colour as the
drift charges: blue (0.7 s), orange (1.45 s), purple (2.4 s). Letting go fires
a mini-turbo that gets longer and stronger at each tier (0.7 s / 1.2 s / 1.9 s
at ×1.30 / ×1.38 / ×1.46 top speed). Every boost shoves you forward the moment
it starts and its extra speed bleeds off gently afterwards. Any boost or a star
blasts through grass, sand and mud with no slowdown.

- **Rocket start**: press the gas as "2" fades and hold it through GO for a
  strong 1.4 s boost. Pressing later gives a smaller one. Flooring it from "3"
  is a burnout: the wheels spin in place for 0.8 s. Hard CPUs nail the start,
  normal ones get the smaller boost, easy ones none.
- **Slipstream**: tuck in 3–20 m behind a rival at speed. After about a second
  in their wake you get a draft boost past them.
- **Tricks**: press Space in the air off a kicker ramp, a launch, a raised
  drawbridge or a half-pipe lip. You land with a 1 s boost.
- **Rescues** take about a second. You land on the road where you fell off
  (before any jump run-up), keep your item, and are untouchable for 2 s. Hold
  the gas as you land for a short respawn boost.

**Items.** Item boxes roll ten items. The odds depend on your position. The
leader draws bananas, green shells and the odd bomb. The middle of the pack
gets shells and mushrooms. The back gets triple and golden mushrooms, stars,
lightning and the blue shell.

| Item | What it does |
|---|---|
| Turbo Mushroom | 1.5 s strong boost (×1.55 top speed) |
| Triple Mushroom | Three of them, one per press |
| Golden Mushroom | Unlimited mushrooms for 7.5 s after the first press |
| Banana | Dropped behind you; spins out whoever hits it |
| Green Shell | Fired straight ahead (or behind); bounces off walls and rails |
| Red Shell | Homes in on the racer just ahead of you |
| Blue Shell | Flies high over the pack and blasts whoever leads (never you) |
| Fuse Bomb | Lobbed ahead (or behind); blows up on contact or when its fuse runs out |
| Super Star | 7 s of invincibility and extra speed |
| Lightning | Strikes every opponent and never you. They spin, shrink and slow down (longest for the leaders) and lose their items. You keep your speed |

Nothing you fire or drop can hurt you. Your bomb's blast and your blue shell
spare you. Your banana is harmless to you for its first 1.5 s. A green shell
can only come back for you after 1 s, once it has bounced around.

**Coins** lie in lines of five down every route, with more on the branches.
Each coin up to ten raises your top speed a little (about 6 % at ten). A
spin-out or a rescue costs three. Collected coins come back after 12 seconds.
The HUD shows your purse beside the lap counter.

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
branch's side, and a sign stands where the roads split. No route drives into
a wall. Barriers, rails, curbs and skirts open wide at every fork and merge
mouth. Tunnel tubes and bridge decks start only once the roads have separated.
A branch keeps the main road's height until the two have pulled apart. Props,
gantry posts and the landscape stay clear of every route's tarmac. The
placement lives in pure functions (`kart/layout.ts`). A unit test sweeps every
route's road, five lanes wide, against all of it. It also drives ghost lines
and CPU drivers at three speeds through every branch, and asserts no wall
contact, stop or fall (`ui/unit/roomGamesKartLayout.test.ts`). Launches, drawbridges
and half-pipes have their own banners. On the minimap, shortcuts are dashed
yellow, boost-only cuts are dashed orange and the other routes are light blue.
Each fork has a dot, and jumps show as breaks. Each drawbridge is a bar that
turns red while its leaves are up. Branches rejoin the main road, and lap
progress, positions, wrong-way warnings and rescues all work on them. CPUs
choose a route each lap, and harder CPUs take shortcuts more often. A CPU takes
a boost-only cut only when it holds a mushroom or a star, and fires it in the
mud. A CPU shoved wide past a branch's mouth, up against the main road's wall,
gives that branch up for the lap. A CPU that gains under 8 m in 4 s (pinned
somewhere) calls the rescue itself (`respawn` event value 2). The play-tests
assert that this never happens, on the base grid and six nudged ones per track
(`ui/unit/roomGamesPlaytest.test.ts`).

**Half-pipes**: the road edges curve up into riding walls. Ride high on a wall
off the end lip and you get a trick jump; trick in the air for a boost when you
land. The flat floor in the middle just drives through.

Water and lava behave differently. Under water the kart is slower and floatier,
the view tints blue and bubbles trail behind. Falling into water off a bridge,
the boardwalk, thin ice or an open drawbridge, falling into lava, or dropping
into a chasm rescues you a little way back. A rescue always lands before a
jump's or a drawbridge's run-up, and you get no progress credit for the fall.
Launch ramps throw you over their gap at racing speed. The glider bleeds off a
boost you carry into it, so a boosted launch still lands where an unboosted one
does. A roller or crusher that knocks you down leaves you untouchable for one of
its cycles, long enough to get clear before it comes round again. Everything
that moves (rollers, crushers and drawbridge leaves) follows the race clock, so
every player sees the same thing with no extra network traffic. Snapshots (version 4)
carry each racer's coins, golden-mushroom window, rescue invulnerability,
slipstream charge and burnout. They also carry each shell's kind and the list
of collected coins. New events: `zap` (an opponent struck by lightning),
`draft` (slipstream boost) and `burnout`. The `bolt` event belongs to the user
and its value is the number of racers struck.

### Kart look and feel

The renderer (`kart/render.ts` plus `kart/render/*.ts`) builds everything
from code and the Kenney kit; nothing in it changes the simulation or the
network.

- **Road** (`render/road.ts`, `render/surfaces.ts`): the tarmac is world-space
  asphalt with its own normal and roughness maps (aggregate, sealed cracks,
  patches), so a branch and the main road are the same pixels where they
  overlap and forks and merges join seamlessly. On top: worn racing grooves,
  tyre marks through every real bend, edge lines that break at fork mouths,
  and raised rumble curbs with a sloped profile. Open roads have curbs all the
  way; walled roads have them through the corners. Banked bends use a second
  colour pair. Off-road patches are sand, snow, cinders or mud by theme, and
  ice is glossy with cracks. The lagoon's bridges are boardwalk planks and the
  skyway's flyovers are steel plate. Dash panels have a frame and chevrons that
  scroll forward. Behind the start line, a chequered band and eight painted
  grid boxes sit exactly where the racers are seated.
- **Trackside** (`render/dressing.ts`, pure and unit-tested): sponsor banners
  on the barrier walls along the home straight and long straights, a neon strip
  along the walls at dusk and on the volcano, chevron boards round the outside
  of sharp bends, floodlights down the home straight, extra grandstands at the
  sharpest corners, and a crowd on every grandstand that bounces and cheers
  harder when the race passes and at the finish. Seats are found by casting
  rays onto the stand model. Tunnels get ribs, and glass tubes are glossy.
- **World** (`render/atmosphere.ts`): a sky dome with a sun disc and halo,
  drifting procedural clouds and stars at dusk. Sun shadows are filtered, the
  sun sits low at dusk, and each theme has its own fog. Water has rolling
  waves, reflects the sky at grazing angles, glitters in the sun and is tinted
  by depth from a baked depth map, with foam lapping every shore. Below the
  surface, caustics play over the seabed and the sunken roads. Lava flows,
  its cracks glow and pulse, and it lights the rock just above it. The terrain
  has a detail texture per theme: grass, sand, snow, rock or paving.
- **Speed and hits** (`render/fx.ts`, `render/camera.ts`): the chase camera
  trails on a critically damped spring and pulls back on a boost. On walled
  roads it stays on the road side of the barrier, so a kart drifting along a
  wall never swings it into pit canopies, grandstands or buildings. The field of
  view widens with speed and kicks harder for stronger boosts. Speed lines rush
  past at top speed and on every boost. The screen shakes on landings,
  spin-outs, crushers and nearby blasts. Boost flames have a white-hot core and
  take the colour of what fired them: the drift tier, orange-red for a
  mushroom, gold all through a golden-mushroom window, pale blue for a
  slipstream. Each rear wheel glows in the drift tier's colour and flares on
  every new tier. There is dust, snow or cinder spray off-road, a rooster tail
  of spray when skimming water, tyre smoke on a burnout, and wind streaks while
  a draft charges. A kart struck by lightning shrinks while it is slowed, and
  a rescued kart blinks while it is untouchable. Lightning comes down as a
  jagged bolt on each racer it strikes, with a sky flash. Green, red and
  spiked blue shells have their own models, and the blue one flies high over
  the pack. Blasts throw a fireball and a shockwave ring, and shells shatter
  in their colour. Fireworks go off over the line at the finish.
- **HUD** (`KartGame.svelte`, `kart/hud.ts`): the item box sits top-left. It is
  a roulette that spins through all ten items and settles with a pop. Shells
  are drawn in their colour, the golden mushroom glows gold, and a gold ring
  counts down its window. Bottom-left shows coins, lap and race time over the
  minimap. The minimap is drawn at double resolution from a cached layer: an
  outlined road, a chequered start, coloured shell dots and your arrow
  pointing the way you drive. Bottom-right is a big outlined position numeral
  in gold, silver, bronze or blue, which pops when you change place. A boost
  warms the screen edges. Lightning flashes the screen: white for the user's
  own bolt with a positive "Lightning! N struck", and electric blue with
  "Struck by lightning!" only for the racers it hits.

## Storm Royale

Five modes (below): the battle royale, Team Deathmatch, Domination, Zombies
and Apocalypse Royale. The battle royale is for up to 16 survivors (up to 4
people, bots fill the rest) on a
480 m island, **solo, in duos or in squads of four** (chosen in the setup next
to the bot count): Palm Isle (Harbor Town, the Rustworks factory, Whispering
Woods, Loot Lake and its river to the sea, Lookout Hill, Pine Farm, Container
Port, Center Camp), Red Mesa (Dust Town, Oil Works, the Oasis, Cactus Grove,
Mesa Top, Canyon Camp, Crossroads) or Dead City (a ruined downtown, see
below). Towns and landmarks have walk-in,
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
WASD, mouse aim, click fire / place / use, right-click aim down sights, 1–5
hotbar, F pickaxe, Q build (1 wall, 2 floor, 3 ramp, R material, G edit), G
frag / molotov, T smoke / flash, X killstreak, C crouch (slide out of a
sprint), E pick up / hold to open chests and supply drops / hold to revive /
buy from a wall, R reload (fire cancels), Space jump / drop / mantle, Shift
sprint (double-tap: tactical sprint), 6–9 respawn class, M map.

Guests see their own build preview at once (it follows the local camera and
piece choice from the freshest snapshot instead of waiting for the host's
echo). Structures are drawn as one persistent mesh per material whose buffers
are rewritten in place (only the materials that changed; health-only changes
touch the colours), so building does not reallocate geometry. For browser
tests, `window.__ottoGameTest = { stormScale: 0.06 }` (set before the page
loads) shortens every storm wait and shrink so a match ends in about a minute.

### Modes

The setup picks a mode (`shooter/modes/`); the list order is the wire value.

- **Storm Royale** (`royale`): the battle royale above, now with grenades,
  attachments and killstreak supply drops in the loot.
- **Team Deathmatch** (`tdm`): two teams of up to eight (people first,
  together) in an arena of 78 m round the first town, spawning on opposite
  sides. Everyone spawns with a class loadout and respawns 4 s after dying,
  at the spawn spot farthest from the enemy. Each elimination scores; the first
  team to the limit (twice the fighters, between 10 and 30) or the team ahead
  after 5 minutes wins.
- **Domination** (`dom`): the same arena with three capture points: A near one
  spawn, B in the middle, C near the other. Standing on a point takes it over
  6 s (faster with more teammates; an enemy on it freezes the capture). The
  enemy's point is neutralised first. Every two seconds each held point scores
  one; the first team to 150 (or the team ahead after 6 minutes) wins.
- **Zombies** (`zombies`): co-op survival round the map's Zombies arena (its
  survivor starts, infected spawn ring, wall-buys and extraction pad from
  `map.anchors`; otherwise the first town and derived spots). Everyone starts
  with a pistol, two frags, a flash, 150 wood and 500 points. Rounds bring
  more infected (`waveSize`), harder kinds and more health; between rounds
  there is a 10 s breather with wood, a grenade and 100 points. Hits pay 10,
  kills pay by kind (+40 for a headshot). Spend points at buy-walls (E; a gun
  you own refills its ammo at half price). Build barricades (Q); the infected
  claw them down, and swinging the pickaxe at your team's piece repairs it
  for points. Knocked survivors can be revived, and the dead come back at the
  next round with a pistol. After the last round (8, `ShooterSetup.rounds`)
  a helicopter is called. Hold out 40 s while it flies in, then be on the
  landing zone (12 m) when it leaves 20 s later to be extracted. If nobody is
  left standing, the crew is overrun.
- **Apocalypse Royale** (`apoc`): the battle royale under a toxic cloud that
  hits half as hard again as the storm. A horde of up to 22 infected roams
  the island from the start and is topped up away from everyone. They hunt
  whoever comes within about 28 m (60 m once provoked) and sometimes drop
  ammo or armour.

Arena modes and Zombies use the storm as a fixed boundary: it hurts outside
and never moves. There are no knocks in the arena modes.

### Gunplay, gear and killstreaks

- **Aim down sights.** Right-click raises the sights over each weapon's ADS
  time (`WeaponDef.ads`; a scope is slower). This zooms in, moves you slower,
  and narrows the spread from hip fire to the aimed spread.
- **Recoil.** Each weapon has a recoil pattern: a fixed climb and sideways
  drift, the same every spray. The kick adds to every shot, and the view
  climbs with it, so you pull the mouse down to stay on target. The kick
  recovers once you let go. A grip cuts it to 62 %; crouching cuts it to 80 %.
- **Hip-fire bloom** grows with each shot. Moving, being airborne or sliding
  widens the spread; crouching tightens it. A flash blinds your aim too.
- **Headshots and falloff** are kept from the royale.
- **Reloads.** A reload with rounds still in the mag is 15 % quicker.
  Pulling the trigger with rounds left, or breaking into a tactical sprint,
  cancels it. A click just before the weapon is ready is kept (out of a
  sprint, between semi-auto shots).
- **Movement.** Shift sprints; double-tap Shift for a tactical sprint (9.4 m/s,
  4 s of stamina that refills while walking). C crouches. C while sprinting
  slides at 11 m/s, and you can fire during the slide. Space at a ledge 0.7 to
  2.35 m high ahead mantles onto it; jumping at one and holding forward grabs
  it on the way up.
- **Grenades.** G throws a frag or molotov, T a smoke or flash, in an arc.
  Frags bounce and explode after 2.1 s. Molotovs and the spitter's acid burst
  on impact into a burning or corroding patch. Smoke blocks bots' sight for
  12 s. A flash blinds whoever can see it, more the more directly they look,
  and the thrower at half. You carry up to 3 of each kind, found as floor,
  chest and supply-drop loot in the royale modes.
- **Attachments.** Royale loot, and part of every class: a scope (further zoom,
  tighter aimed spread), a grip (less recoil), an extended mag (half again as
  many rounds) and a suppressor. Firing without a suppressor puts you on
  the enemy minimap for 1.6 s.
- **Classes** (TDM and Domination; 6–9 while respawning): Assault, Rusher,
  Marksman and Demolition (`CLASSES` in weapons.ts).
- **Killstreaks.** You earn them by eliminations without dying: 3, 5 and 7
  (Zombies counts four infected as one; Apocalypse counts three). X calls one
  in, and earned streaks are kept until used. A **UAV** shows every enemy on
  the team's map for 22 s. An **Airstrike** marks the point under the
  crosshair, then drops five bombs 7 m apart along your aim. An **Attack
  Drone** orbits over you for 25 s and shoots enemies and infected within
  32 m. Royale supply drops hand their opener a streak.
- **Feedback.** A hitmarker shows on every hit (red on a kill), with damage
  numbers and the headshot / kill sounds. After death a recap shows who
  killed you, with what, from how far, how much health they had left and the
  damage each of you did. You then watch your killer, or a teammate.

### The infected

The infected live in `shooter/zombies.ts`. There are four kinds:

- **Walkers**
- **Runners**: fast and fragile.
- **Brutes**: 420 health, tear barricades down six times as fast.
- **Spitters**: keep their distance and lob acid.

They pick a target on a staggered think timer. A fixed spatial hash, rebuilt
in place each tick, keeps them apart. They use the fighters' collision. A
blocked infected claws a structure in its way or climbs a low ledge. In
Zombies they path-find with a flow field: a breadth-first distance to the
nearest survivor over a 1.25 m walkability grid of the static props, rebuilt
twice a second. That leads them round buildings and in through the doors.
Barricades are left out of the field on purpose, so they walk into them. A
straggler that has not reached anyone for 25 s comes back at the nearest
spawn. 64 infected chasing four survivors cost well under a millisecond a
tick.

The computer difficulty softens their claws: 60 % on easy, 80 % on normal.

They are drawn by `shooter/modes/layer.ts`, wired in with one call from the
renderer. Their bodies are instanced, one draw call per part. They walk, claw
and stagger, and are smoothed between snapshots. The same layer draws:

- grenades in flight
- smoke, fire and acid
- airstrike warnings
- drones
- Domination flags and rings, which fill in the capturing team's colour
- buy-walls
- the landing-zone beacon and the helicopter

### Bots in the modes

Bots fight the infected as well as fighters. In Zombies the infected are their
only enemy. Bots kite away from anything in arm's reach, brutes from farther.
They aim against the recoil: 30 %, 60 % or 85 % of the kick by difficulty.
They throw frags and molotovs at groups of infected or at mid-range enemies.
They flash before closing in and smoke a teammate before reviving them. They
chase, or flank, an enemy they lost sight of, and hunt whoever the team's UAV
shows. They call in streaks at once (an airstrike only on an enemy in sight),
crouch for long shots, slide into cover and tactical-sprint long runs. They
see nothing through smoke and nothing while flashed.

By mode, they play the objective:

- **Domination**: take the points the team does not hold, spreading across
  them.
- **TDM**: push through the middle toward the enemy side.
- **Zombies**:
  - buy from walls when it is an upgrade
  - fetch ammo
  - hunt the last few infected of a round
  - otherwise hold the base
  - run for the landing zone once the helicopter is called.

### Network

Snapshots are version 4 (`SNAPSHOT_VERSION` in `shooter/net.ts`, and
`KART_SNAPSHOT_VERSION` in `kart/net.ts`; the encoder and decoder both use it),
still under the 16 KiB relay cap. They carry:

- the mode
- the mode state (scores, limit, clock, Zombies round / phase / timer /
  infected left / landing zone / survivors aboard)
- the capture points
- the infected (flat, integer decimetres and centiradians)
- grenades, areas and drones
- UAV time per team
- per fighter: sights, kick, stance, tactical stamina, grenades, killstreak
  progress and earned streaks, class, points, respawn, flash and deaths
- per held weapon: its attachments

Loot is flat with id deltas. A full floor of loot rides with at most the
Apocalypse horde. Zombies keeps at most 60 floor items so the full horde of
64 fits. The worst case, every list at its cap, is about 15 KB.

Inputs are ten numbers. They add crouch, tactical sprint, lethal, tactical,
killstreak and class.

New events: `nade`, `smoke`, `fire`, `flash`, `streak`, `capture`, `round`,
`zhit`, `zkill`, `buy`, `extract`, `spawn`, `mantle`, `slide`, `repair`.
Kill codes 9–14: frag, molotov, airstrike, drone, the infected, acid.

### Tests

These cover every mechanic above:

- `ui/unit/roomGamesShooterModes.test.ts` (unit tests)
- headless play-tests where each mode reaches an end state within bounded
  ticks, with a guest decoding every snapshot under the cap:
  - TDM and Domination with 16 bots
  - Zombies with 4 bots and three rounds: extraction or overrun
  - Apocalypse Royale with 16 bots

### Dead City

A burning, ash-choked downtown on a six-by-six street grid inside a rubble
berm (`shooter/maps/deadcity.ts`). Landmarks:

- **The Spire**: a five-storey stump of a skyscraper. Its fallen upper floors
  lie across Second Avenue as a stepped rubble ramp you can run up, straight
  through a blown-out wall into the third storey (with a chest on the ramp).
- **Broken Tower**: a three-storey stump beside a climbable mound of rubble.
- **Mercy Hospital**: three tiled storeys, ambulances, a triage camp and
  sandbags at the doors.
- **Eastgate Mall**: a glass-and-steel hall with a mezzanine and kiosks, and a
  car park full of burnt-out cars.
- **Line 3 Subway**: stairs down a cut below the street to a platform with a
  derailed carriage you can walk through (loot and a chest underground).
- **Checkpoint Bravo**: two guard towers, sandbag nests, tents, a container
  wall, trucks and a barrier chicane across the avenue.
- **Ashwood Park**: overgrown woods round a fountain, a gazebo and benches.
- **Old Downtown** (four blown-out tenements) and the **Ironside Depot**.

The other blocks hold ruined walk-ins (two to seven storeys, with blown-out
walls on upper floors you can walk through, broken parapets and jagged
stubs), office blocks beside rubble heaps, or burnt-out lots. Streets have
wrecks (some still burning), jersey barriers, heaved asphalt, lamp posts and
bomb craters; weeds and street trees grow through the cracks. About 40 fires
burn across the map.

### Mode anchors

Every map exports, from the generated world (`shooter/maps/anchors.ts`,
`map.anchors`): 24 free-for-all respawns spread far apart; two team bases
with eight spawns each, facing each other; three Domination flags (A near the
first base, B in the middle, C near the second); a Zombies arena (four
survivor starts, a ring of 16 infected spawns, the ground-floor doors and
windows to board up, six wall-buys on outer walls and an extraction pad);
and a street nav graph for bots (`navNext(map, x, z, goalX, goalZ)` gives
the next waypoint, A* over open ground about 16 m apart; walk-ins keep their
`climbStep` stairs). Every walk-in also records its ground-floor doors and
windows (`map.openings`). Dead City places its bases at the checkpoint and the
depot, the flags on three crossings and the Zombies arena round the hospital,
with the helicopter landing in the park.

### Look and feel

- **Light and weather.** Physically based materials with soft shadows. Each
  surface family (plaster, brick, concrete, glass, rust, sandbags, rubble…)
  has its own generated texture and finish. Six presets
  (`render/weather.ts`): clear day, dusk, night (your flashlight, and beams
  from everyone else's), rain (streaks and lightning), fog and ash (drifting
  ash and embers, Dead City's default). Override with
  `localStorage['otto.games.weather']` (`day`, `dusk`, `night`, `rain`,
  `fog`, `ash`) or `RenderOptions.weather`.
- **Soldiers.** Fighters are articulated soldiers in fatigues, vests and
  helmets or caps, one look per survivor, with a team-coloured armband in
  team modes. They run, crouch, slide feet first, raise the gun to the
  shoulder when aiming, kick with recoil, sprint with the gun across the
  chest, chop with the pickaxe, crawl when knocked and fall when eliminated
  (`render/pose.ts`). All of them are drawn as one instanced crowd.
- **First person.** Aiming blends into first person (or stay there with
  `CameraState.firstPerson` or `localStorage['otto.games.fpv'] = '1'`): the gun sits in gloved hands low and right,
  comes up to the sights, sways when you turn, bobs as you walk, kicks back
  and up on each shot with a muzzle flash, dips to reload and lowers to
  sprint. Iron sights sit just under the crosshair; with a scope (or the
  sniper) the gun is put away once the scope reticle is up. Name tags fade
  out within 10 m and while aiming, so a teammate beside you never covers
  the middle of the view.
- **Combat effects.** Tracers fly down each shot. Muzzle flashes and short
  light flashes go with them, and impacts leave bullet holes and dust. Hits
  spray blood onto the ground and the wall behind; set
  `localStorage['otto.games.blood'] = '0'` for sparks instead. Explosions
  throw a fireball, a ground shockwave and tumbling debris, leave a scorch
  mark, light the area and shake the camera. Player structures crack as they
  lose health.
- **The storm** is a billowing cloud wall with crackling sparks: purple, or
  green as the toxic cloud of Dead City and Apocalypse Royale.
- **Performance.** Kit props are batched per 120 m chunk, so off-screen chunks
  are culled and small decor in chunks more than about 200 m away is hidden. Architecture is one
  merged mesh per surface family. Fighters are five instanced draw calls in
  all. Effects come from fixed pools: no allocation per frame, and the light
  count never changes, so no shader recompiles.

## Playing together

Choose **Invite people**, create a room and share the invitation; up to 8
(karts) or 4 (shooter) people join with it before the host starts. The host
runs the simulation; the daemon relays bounded inputs and snapshots (see the
[game-room contract](../contracts/game-rooms.md)). Guests pick their character
in the lobby. Browser verification and screenshots live in
[`docs/testing/room-games`](../testing/room-games/).

**One catalog.** The game kinds, their capacity and the map ids live in
`ui/src/modules/rooms/games/catalog.json`. The daemon compiles the same file in
(`game_rooms/types.rs`) to validate `POST /game-rooms`, and the chooser reads it,
so a map cannot be offered that the relay refuses. `unit/roomGamesCatalog.test.ts`
checks that the kart tracks and shooter maps implement exactly these ids.

**Capabilities.** Game rooms and session rooms share one set of capability
primitives (`otto-server/src/rooms/capability.rs`): 43-character tokens stored
only as SHA-256 digests and compared in constant time, the
`Sec-WebSocket-Protocol: otto-game, <token>` pair, and per-member token
buckets. The browser side is one `RelaySocket` (`ui/src/lib/relaySocket.ts`).

**Reconnecting.** A dropped connection retries five times (1, 2, 4, 5 and 5 s).
After that the lobby and the match say "Couldn’t reconnect to the game room."
with a **Reconnect** button. Coming back online, or returning to the tab, also
starts a fresh set of retries.

**Accessibility.** Each game has one screen-reader status for its edges: the
countdown start and GO, call-outs, round start, helicopter inbound, respawning,
paused, and the result headline. Countdown digits and timers stay visual only.
Esc on the game opens the menu, and focus moves to **Resume**. In the shooter,
the first Esc releases the mouse. Leaving a game puts focus back on the selected
game's title. Result and pause screens scroll on short screens; at 640 px and
below the result table is compact.

**Graphics context.** If the browser reclaims the game's WebGL context, the
renderer remounts on the live race or match state, at most twice a minute.
After that the game shows Retry. This uses the shared `ui/src/lib/three/stage.ts`.
