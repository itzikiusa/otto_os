# Game rooms

Game rooms are ephemeral two-person game relays, independent of terminal rooms,
sessions, workspaces and persisted credentials. Solo computer play makes no room
requests. The browser host owns game simulation; this is friendly multiplayer,
not a competitive anti-cheat service.

## HTTP

- `POST /api/v1/game-rooms` requires a normal authenticated owner credential
  (no impersonation, managed-session, share or MCP-only credentials). Body:
  `{"name":"Alex","config":{"game":"shooter","map":"station"}}`.
  Returns `GameRoomCredential` (TypeScript: `ui/src/lib/api/game-room-types.ts`):
  `room_id`, `member_id`, `role:"host"`, `token`, `config`, `invite`, `invite_url`.
  Names trim to 1–40 characters, no controls. Shooter maps: station/foundry/dunes;
  kart maps: coast/forest/neon. Mismatched maps return 400.
- `POST /api/v1/game-room-join` is public, authorized only by its body:
  `{"room_id":"…","invite":"…","name":"Sam"}`. Returns the same credential
  fields with `role:"guest"`, omitting invite fields. Invite consumption is atomic,
  expires after ten minutes, and allows exactly one guest. Invalid/expired tokens
  return 401; full/closed rooms return 409; missing rooms return 404.

Tokens and invites contain 256 random bits; only SHA-256 hashes are retained by
the registry. They cannot authenticate any ordinary owner API, terminal, room,
or other game. Responses containing credentials use `Cache-Control: no-store`.
Creation is limited to 32 rooms globally and two per owner. Rooms expire after
one hour and are removed after close. Credentials do not survive daemon restart.

Invitations use the configured `room_public_origin` (otherwise the daemon's
loopback base URL), validated by the same rules as terminal rooms. Example:
`https://otto.example/#/game-room/UUID?invite=SECRET`. The client removes the
fragment credential before opening a socket. This does not create a listener or
tunnel. Remote play requires an already reachable HTTPS deployment. Never put
credentials in query parameters, logs, local storage, or owner auth headers.

## WS /ws/game-rooms/{id}

`/ws/game-rooms/{room_id}` uses exactly `['otto-game', token]` WebSocket
subprotocols. Only `otto-game` is echoed. URL queries are rejected. Owner tokens
are not accepted in place of a game token. All messages are tagged JSON objects.

Server sends `{type:"state",room:{id,config,round,generation,phase,paused,members}}`.
Members contain `id,name,role,connected,ready`; phase is
`lobby|playing|finished|closed`. Round starts at 1. Generation advances whenever a
socket connects or disconnects. This state always precedes subsequent packets
from that generation. Clients discard packets whose round/generation differs.

Client commands:

| Command | Fields | Permission / effect |
| --- | --- | --- |
| ready | ready:boolean | Lobby only, own ready flag |
| start | round,generation | Host only; both connected and ready; playing |
| snapshot | round,generation,seq,data | Host only while playing and unpaused; relay to guest |
| input | round,generation,seq,data | Guest only while playing and unpaused; relay to host |
| finish | round,generation,result | Host only while playing and unpaused; finished |
| rematch | none | Finished only; both must request; round increments, lobby, ready resets |
| ping | none | Server responds pong; send every five seconds |
| leave | none | Closes room immediately for both players |

`seq` is a positive monotonically increasing integer per sender, reset on any
generation or round change. Data is JSON, with snapshot max 16 KiB, guest input
max 1 KiB, result max 4 KiB, measured serialized. Snapshot/input data must be
objects. Client simulation validates its input shape before using it. The server
validates envelope, sender role, round/generation, connection identity, sequence,
size and rate, but does not interpret game physics. Host and guest character/camera
preferences are local; game/map configuration is immutable for the room.

Snapshot/input events echo command fields to the peer only. Finish broadcasts
`{type:"finished",round,generation,result}` and the new state. On reconnect a
finished result is sent again. Errors are `{type:"error",message}`. Closure is
`{type:"closed",reason}`. State changes are broadcast to both players.

Socket frame/message limit: 20 KiB. Input/snapshot rate: 60/sec, burst 90;
control rate: 5/sec, burst 15. Invalid envelopes, binary data, rate violations,
and queue lag disconnect the sender. Outbound queue is bounded to 64 messages;
a stalled send times out after two seconds. A socket without incoming messages
for 15 seconds is disconnected. Ping/pong is application-level JSON.

Only one live socket per member is accepted. Disconnect clears readiness,
advances generation and pauses a running game. Reconnecting within 20 seconds
uses the same in-memory member token; clients must retain their simulation and
resume only after the new state. Reload does not auto-create or auto-start a new
match. After grace, the room closes for both members. Members who obtain a
credential but never connect must connect within 20 seconds. No automatic guest
replacement or host migration. Clients clear held input whenever paused or
unfocused, and clear remote input after generation changes.

The bundled kart snapshot includes `players[].steering` (finite number), the
authoritative eased steering input. The client validates it before rendering;
participants should reload together after upgrading the game client. Rendering
smoothing stays local and does not modify authoritative positions or scores.
