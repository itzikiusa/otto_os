# Rooms Arcade Implementation Plan

> **For agentic workers:** Use independent ownership and parallel implementation, test-first for simulation and transport, then integration and independent whole-branch review. User explicitly authorized autonomous execution without further questions.

**Goal:** Two complete 3D games in Rooms with human/computer opponents and three environments each.
**Architecture:** Pure host-owned TypeScript simulation, Three.js renderer, standalone capability-scoped Rust match relay, Svelte setup/HUD.
**Tech Stack:** Existing Rust/Axum, Svelte 5, Three.js, Blender 5.2, Playwright and node:test.
**Spec:** docs/superpowers/specs/2026-10-08-room-games-design.md

## Global Constraints
- Preserve terminal room behavior and user data; no production restart while developing.
- Two players; three shooter maps (station/foundry/dunes), three racer maps (coast/forest/neon).
- Computer Easy/Normal/Hard; switchable first/third-person shooter camera.
- CC0 or self-made assets with provenance; GLB Y-up, metres, +Z forward; <=20 MiB combined target.
- Contracts and TypeScript wire types together. Bounded retries/queues/TTL; no implicit public listener.
- No per-task full Cargo builds in parallel; one coordinated integration gate.

## Review Focus
- Guest credential must not authorize ordinary owner APIs or host state writes.
- Reconnect/rematch must reject old input/snapshots and clear held keys.
- Shooter collision, muzzle obstruction and camera obstruction use consistent geometry.
- Racing checkpoint order and reset cannot grant shortcut laps; AI completes every map.
- Asset/WebGL/audio load failures and view disposal release resources and show usable retry/exit.

## Task 1: Game-room transport
Owner: backend worker. Files: crates/otto-server/src/game_rooms/**, server route/context wiring, ui/src/lib/api/game-room-types.ts, docs/contracts/game-rooms.md, backend game-room tests. Interface: POST /game-rooms (owner), POST /game-room-join (public invite), WS /ws/game-rooms/{id} credential; exact schema recorded in contract before UI integration.
- [x] Write and run tests rejecting guest host-state/start, reused invite, third member and stale round.
- [x] Implement bounded ephemeral registry and socket routing; configuration game/map validated together.
- [x] Verify auth/capacity/disconnect/lifecycle tests and clippy; report protocol to UI owner.

## Task 2: Simulation and bot behavior
Owner: gameplay worker. Files: ui/src/modules/rooms/games/{types,maps,simulation,shooter,kart,bots}.ts and ui/unit/roomGamesSimulation.test.ts. Interface: createGame(config), stepGame(state, inputs, dt), defaultInput(), map definitions. Worker publishes exact typed interface before renderer integrates.
- [x] Tests first for blocked shooting, death/win, ordered checkpoints, bounded delta and rematch reset.
- [x] Implement shooter/kart simulations, three unique layouts each, bots using same input rules.
- [x] Run deterministic long simulations proving AI movement/completion on each map and difficulty.

## Task 3: Blender art pipeline
Owner: asset worker. Files: ui/assets-src/room-games/** and ui/public/room-games/**. Interface: asset manifest mapping characters/karts/kit and named clips/nodes documented in CONTRACT.md.
- [x] Author original Blender source models and record CC0 dedication/provenance.
- [x] Generate cohesive shooter fighters, drivers/karts, weapons, environment kit and previews.
- [x] Export GLBs; verify bounds, mesh counts, clips, node naming, bytes and hashes.

## Task 4: Playable renderer and Rooms UI
Owner: coordinator. Files: ui/src/modules/rooms/games renderer/input/audio/client/Svelte files; RoomLobby.svelte, RoomsRoute.svelte, App.svelte guest routing.
- [x] Exercise setup/launch and camera controls with browser tests before completion.
- [x] Implement lazy scene, complete six environments, animation/lighting/effects/audio, input/focus handling.
- [x] Add opponent/difficulty/map/character setup, human invitation/readiness, HUD/results/rematch.
- [x] Integrate network snapshots and local AI; handle loading/error/disconnect and teardown.

## Task 5: Verification and review
- [x] Run UI check/unit/build, changed-crate Rust checks and relevant named Playwright specs.
- [x] Capture light/dark gameplay and setup; record frame timing and real two-client evidence.
- [x] Independent review; fix confirmed issues with regression coverage.
- [x] Save reviewed feature with repository convention; record scope and verification limits.
