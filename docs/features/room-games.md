# Room games

Open **Rooms → Games** (or the Games command in ⌘K). Game rooms are independent
of coding sessions. Choose **Arena Duel** or **Circuit Clash**, an environment,
and an Azure or Ember character. Play immediately against the computer, or
create a two-person room and copy its invitation.

## Arena Duel

A stylized 3D sci-fi duel in **Station**, **Foundry**, or **Dunes**. The first
player to seven eliminations wins; after three minutes, the higher score wins
(equal scores draw). Each elimination needs four unshielded rifle hits. Players
respawn after two seconds with a short protective shield.

| Control | Action |
|---|---|
| WASD / arrows | Move |
| Mouse | Aim (click the canvas first; dragging also works without pointer lock) |
| Left click / right click | Fire / aim with lower sensitivity |
| Shift / Space | Sprint / jump |
| R | Reload the 24-round magazine |
| V or camera button | Switch first/third-person view |
| Escape | Release mouse capture |

Easy gives over two seconds to react after the bot acquires you, fires brief
bursts with long pauses, moves more slowly and can miss at close range. Normal
and Hard progressively shorten those pauses and improve tracking. Bots navigate arena
cover; bullets and players cannot pass through it. The third-person crosshair
projects the actual firing ray, including cover obstruction.

## Circuit Clash

An original kart racer on **Coast**, **Forest**, or **Neon**. Complete three laps
through ordered checkpoints. Computer opponents use the same track progress, pickups, off-track slowdown and recovery
rules as human players.

| Control | Action |
|---|---|
| WASD / arrows | Accelerate, brake/reverse, steer |
| Space while steering | Drift; release to spend accumulated boost |
| Shift | Release a charged drift boost |
| E | Use a collected Turbo or Pulse item |
| R | Recover to the last checkpoint without gaining progress |

Steering builds gradually, including when reversing direction. Grass and the
end of a boost slow the kart progressively. Nearby grass does not teleport you;
use R to recover, or automatic recovery applies far beyond the circuit.

Touch devices show movement and action buttons; the shooter also has a drag-to-aim
pad. Sound starts after an interaction and can be muted. Menu pauses a computer
match. During a human match, Menu leaves your player idle while play continues.

## Playing with another person

1. Choose **Another person**, enter your display name and create a room.
2. Copy the invitation. The guest opens it, enters a name and joins.
3. Both players select **Ready**; the host selects **Start match**.
4. After results, both select **Rematch**, then ready up for a fresh round.

For a different computer, configure the reachable HTTPS origin in Rooms
connection settings. A loopback invitation works on the host Mac only. Creating
a room does not enable a listener or create a tunnel. Guests receive only a
game-scoped capability, not access to sessions or the owner's APIs. Invitation
secrets are removed from the address bar and kept only in memory; reloading a
guest page requires reopening the original invitation.

The host browser runs the simulation; the daemon relays bounded inputs and
snapshots. This is friendly multiplayer, without competitive anti-cheat,
rollback prediction, matchmaking or persistent rankings. Keep the host game
visible: browsers suspend animation frames in background tabs. Rooms are
in-memory and end on daemon restart or after one hour. An unused invitation
expires after ten minutes. A broken connection pauses play; clients make at
most five consecutive reconnect attempts, within a twenty-second server grace
period. Leaving explicitly ends the room. At most two people can join.

## Assets and troubleshooting

All characters, vehicles and scenery are original Blender-generated assets
with a CC0 dedication. Six GLBs total about 5.65 MiB; previews bring the complete
asset folder to about 8.13 MiB. Reproducible scripts, model contracts, provenance,
and export validation live in [`ui/assets-src/room-games`](../../ui/assets-src/room-games/).
The renderer loads only when starting a game and releases its GPU resources on exit.

- **Models cannot load:** use **Retry**; check that `/room-games/*.glb` is served
  by the same origin as the UI.
- **3D graphics unavailable:** enable hardware acceleration in the browser.
- **Invitation cannot connect:** verify the configured HTTPS origin reaches the
  daemon and forwards WebSocket upgrades; use a fresh invitation after expiry.
- **Waiting for connection:** keep the host tab visible and reconnect within the
  grace period. After the room closes, create a new match.

The authoritative wire protocol and limits are in the
[game-room contract](../contracts/game-rooms.md). Browser verification and
screenshots are recorded in [`docs/testing/room-games`](../testing/room-games/).
