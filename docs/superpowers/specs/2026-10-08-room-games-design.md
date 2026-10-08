# Rooms Arcade

User-approved scope: two fully playable 3D games in Rooms, each human 1v1 or computer 1v1, multiple environments, Blender-authored/adapted CC0 characters and environments. Shooter supports first/third-person switching. Racer has original kart-racing identity. User authorized autonomous choices and explicitly requested no further questions.

## Product
Rooms gains Games alongside existing collaboration rooms. A game does not need a coding session. Choose Arena Duel or Circuit Clash, opponent (computer/person), difficulty, character, and environment. Human match creates an invitation; opponent enters a display name, both ready, countdown starts. Matches end in a results/rematch view. Joining grants game access only, never terminal access.

Shooter: station/foundry/dunes arenas with distinct cover layouts. Move, sprint, jump, aim, fire, reload; first to seven eliminations or three-minute timer. Animated combatants, muzzle/impact feedback, spatial audio, health/ammo HUD. First/third-person chosen independently and switchable in play. Collision-aware camera and shot origin prevent shooting through cover.

Racer: coast/forest/neon circuits with distinct routes. Three laps with ordered checkpoint validation, drift charge, boost, pickups, off-track slowdown and recovery. Visible animated drivers, steering wheels/wheels, exhaust/trails, lap/position HUD. Computer opponents follow racing lines and have bounded reaction/accuracy parameters. Shooter AI navigates around cover and requires line of sight. Easy/normal/hard do not change player rules.

## Architecture
Existing Three.js dependency, lazy-loaded game scene; Svelte setup/HUD. Pure TypeScript fixed-step simulation shared by host and local computer mode. Host browser owns simulation; guest sends bounded input, host streams snapshots through a dedicated capability-scoped daemon game-room socket. This is friendly host-authoritative multiplayer, not competitive anti-cheat. Remote play uses existing reachable HTTPS deployment; no network listener or tunnel is enabled automatically. Guest route bypasses owner auth and never reads owner tokens. Simulation freezes on disconnect; bounded reconnect/rejoin, explicit end after grace. Host reload cannot silently create a new match. Computer mode works locally without room creation.

A dedicated server game-room registry is independent of terminal rooms, limited to two members, bounded room count/TTL/message sizes and rates. Opaque credentials live in memory, invite in URL fragment scrubbed on entry. Host-only state/start/end; guest-only input; ready state on server. Generation/round checks reject stale packets after rematches and reconnects. No input while unfocused or modal/menu is open.

## Asset contract
Retain Blender scripts and provenance/license files. Export GLB, metres, Y-up, forward +Z. Source CC0 assets may be adapted; original self-made additions permitted. Shared character and scenery kit reused across six maps. Shipped assets target <=20 MiB combined, textures <=1024 square, lazy loaded. Provide preview renders and machine-checked manifest (size/hash/animation/node names). Never claim AAA production scale; aim for cohesive detailed presentation, responsive controls, lighting, animation, audio and measured performance.

## Verification
Simulation tests: collision/line of sight, bounded dt, ammunition/death/score/win, AI across maps, checkpoint order/laps/items, reset/rematch. Transport tests: unauthenticated access, capacity, invitations, wrong-role packets, oversized/rate-limited messages, stale round/generation and disconnect cleanup. Browser: computer games across six maps, real two-context network match, camera toggle, keyboard release, results/rematch, WebGL cleanup, asset loading, desktop/mobile layout, light/dark screenshots. Native shell pointer-lock/fallback and frame timing exercised where available. Report WAN/hardware limits honestly.
