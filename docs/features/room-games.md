# Room games

Open **Rooms → Games**. Two 3D games built from Kenney CC0 models (see
[`ui/assets-src/room-games/PROVENANCE.md`](../../ui/assets-src/room-games/PROVENANCE.md)):

## Kart Grand Prix

Eight racers (humans plus CPU drivers on easy/normal/hard), four tracks (Sunny
Meadow, Downtown Dash, Canyon Run, Frostbite Pass), three laps with progress
tracking, live ranking, minimap, lap times and a results table. Drift (Space
while steering) charges three mini-turbo levels; jump ramps allow a mid-air
trick boost; dash panels, ice and barrier walls vary each track; leaving a grass
track rescues you. Item boxes roll six items weighted by position: Turbo and
Triple Mushroom, Banana, Homing Bomb, Super Star and Thunderbolt. Controls:
W/S gas and brake, A/D steer, Space drift/trick, E item, Q look back, R rescue.

## Storm Royale

Battle royale for up to 16 survivors (up to 4 people, bots fill the rest) on
Palm Isle or Red Mesa. Drop from the balloon, skydive and glide, loot six
weapons (sidearm, SMG, assault blaster, pump scatter, sniper rail, rocket tube)
in five rarities, ammo, heals and shields, open chests, and outlast a shrinking
storm. HUD: health/shield, inventory with ammo, minimap with storm circles, kill
feed, damage numbers and hit markers; results show placement, eliminations and
damage. Controls: WASD, mouse aim, click fire, right-click aim, 1–3 weapons,
E pick up / hold to open chests, R reload, Space jump/drop, Shift sprint.

## Playing together

Choose **Invite people**, create a room and share the invitation; up to 8
(karts) or 4 (shooter) people join with it before the host starts. The host
runs the simulation; the daemon relays bounded inputs and snapshots (see the
[game-room contract](../contracts/game-rooms.md)). Guests pick their character
in the lobby. Browser verification and screenshots live in
[`docs/testing/room-games`](../testing/room-games/).
