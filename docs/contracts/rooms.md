# Session rooms

Canonical JSON DTOs are `crates/otto-core/src/api/rooms.rs`, mirrored by
`ui/src/lib/api/room-types.ts`. All IDs are strings and all timestamps are UTC
RFC3339. State is ephemeral; daemon restart ends every room.

## HTTP

Owner bearer authentication, Sessions feature edit permission, session ownership
and workspace editor permission are required for management. Scoped, MCP,
managed-agent and impersonation credentials cannot manage rooms. Root does not
bypass ownership. Only an already-live, local agent/shell session is eligible.
No room route starts or resumes a process.

| Method/path (under `/api/v1`) | Request | Response |
| --- | --- | --- |
| POST `/sessions/{id}/room` | `{name}` | `{room_id,member_id,token}` |
| GET `/rooms` | — | admitted host `RoomSnapshot[]`, owned rooms only |
| DELETE `/rooms/{id}` | — | 204 |
| POST `/rooms/{id}/invites` | `{role:"viewer"\|"editor"}` | `{invite,url,expires_at}` |
| POST `/room-join` | `{room_id,invite,name}` | `{room_id,member_id,token}` |

Join authenticates with a single-use invitation rather than an owner token.
Maximums: 32 rooms per daemon, one per session, four admitted participants
including host, eight pending joins and eight outstanding invites per room.
Room lifetime is eight hours, invitation lifetime ten minutes, pending admission
two minutes. Full rooms reject joins without consuming invitations. Admission
atomically checks capacity. Rejection/removal destroys the member capability.
Credentials contain 256 random bits; only SHA256 hashes stay in the registry.
They are never stored in AuthRepo and cannot authenticate ordinary Otto APIs.

Invitation URLs use an explicit origin and a fragment containing room ID and
invitation. The default daemon loopback origin works only on the same computer;
a user must provide a configured reachable HTTPS origin for remote access.
Invites are copied, never automatically sent. Normal errors use `{code,message}`.

## WebSockets

`/ws/rooms/{id}` and `/ws/rooms/{id}/terminal` accept exactly the subprotocols
`otto-room, <member-token>`, and negotiate `otto-room`. Query-string tokens and
owner bearer tokens are not accepted. Capabilities are authenticated before
upgrade. Main socket reconnect increments `RoomMember.generation`; old sockets
are fenced. Heartbeat every five seconds detects loss after ten seconds.
Thirty seconds of reconnect grace retain membership; disconnect immediately
withdraws driver, microphone, presenter and annotation grants. Reconnect never
restores capture/control. Host grace expiration ends the room.

Room events are discriminated by `type`. The initial and resync event is
`{type:"snapshot",room:RoomSnapshot}`. A pending snapshot contains **only**
`room_id,member_id,admission:"pending"`. Admitted snapshots include session title,
provider, host and driver member IDs, `grant_epoch`, expiration, members,
messages, presentations and annotation grants. Only the host sees pending
members. Credentials, filesystem paths, workspace metadata and invitations
never appear in a snapshot. Driver/epoch reflect the authoritative session
manager, including deliberate host input from another terminal pane.

See canonical `RoomAction` for the complete exact client union and `RoomEvent`
for server events. Unknown fields (including spoofed actor IDs), oversized
messages and unknown actions are rejected. Host-only actions include admission,
rejection, role changes, control grant, mute, present grant, removal and end.
A role is separate from the driver seat. Viewers cannot request or receive
terminal control. Guest signaling routes only through the host: `signal.to`
is the target member and `signal.generation` is that target's current connection
generation. Relayed events carry the sender's current generation. `media` is
bounded opaque WebRTC signaling, never raw media bytes.

`chat` has `{text,nonce}`. Attribution, sequence and time come from the server.
Maximum text is 4096 UTF8 bytes; history retains 200 messages. Nonce retries
are deduplicated per member in that bounded window. Limit is 5 messages/second,
burst 10. Chat does not invoke PTY input. Snapshot messages acknowledge nonce.

## Terminal socket

Only admitted members with a live main socket may open the terminal socket.
It is pinned to the main connection generation and the original PTY spawn
sequence. Session exit/replacement ends sharing without attaching a new PTY.
Room closure does not terminate the session or evict ordinary terminal viewers.

Server output uses binary PTY chunks. Initial/resync text frames:
`{type:"status",status:"running",epoch:<spawn_seq>}` and
`{type:"scrollback",data:<base64>,cols,rows,epoch:<spawn_seq>}`.
Clients send JSON `{type:"input",data:<string>,grant_epoch:<number>}` or
`{type:"resize",cols,rows,grant_epoch:<number>}`. Binary input is rejected.
The final PTY write validates both current driver and grant epoch under the
same session lock as grant/revoke. Every grant/revoke/takeover advances epoch,
including regrant to the same member. Passive input/resize never reclaims.

## Presentations and annotations

`request_present` and host `grant_present` are separate from terminal roles.
Unused grants expire after 60 seconds. `start_present` publishes metadata for
one locally selected source/member (four total), never starts remote capture.
Source IDs are unique across stop/restart. Each source carries its generation,
dimensions and `clear_epoch`. Owner or host may stop an individual source.
Subscriptions carry source ID/generation and `hidden|preview|grid|full` tier;
they are private events to the trusted host coordinator, never room snapshots.

Annotations use normalized finite [0,1] points, source generation and clear
epoch. Presenter approval is required for another member; host revocation
blocks regrant until the host lifts it. Annotations are server-attributed,
vector-only, and never inject terminal or native input. Pointer expiry is one
second, pen/highlight expiry ten seconds. Bounds/rate limits apply before
broadcast; clear increments epoch so delayed pre-clear vectors fail closed.

## Reachability, relay setup and audio moderation

Root-only `GET /api/v1/room-settings` returns
`{public_origin,stun_urls:string[],turn_urls:string[],relay_only,turn_secret_configured}`.
Root-only `PUT /api/v1/room-settings` accepts
`{public_origin,stun_urls:string[],turn_urls:string[],relay_only,turn_secret?:string}`.
Omitting the secret retains it; an empty string removes it. The shared secret
is stored in Keychain under `otto:rooms:turn-shared-secret`, and never returned
or stored in settings JSON. Settings contain only the opaque key reference.
Origin must be HTTPS or loopback HTTP, with no credentials/path/query/fragment;
`tauri.localhost` is not a remote room origin. Invitation links have the exact
format `https://host/#/room/<room_id>/<invite>`.

An admitted member sends `{type:"request_ice"}` on the membership socket and
receives `{type:"ice",ice_servers:[{urls:string[],username?,credential?}],
relay_configured,relay_only,expires_at}`. TURN REST credentials use HMAC-SHA1
and expire after ten minutes. They are distinct from the persistent shared
secret. No STUN/TURN operator is provisioned automatically. `relay_only` is
explicitly supported for cross-network testing.

Snapshots carry `audio_epoch` and `audio_enforced_epoch`. The host media client
sends `{type:"audio_applied",epoch}` only after applying the mixer change.
If an active room call has an unacknowledged change for three seconds, the
server closes audio state for everyone and reports `audio_unavailable`.
Room mute is separate from local mute; lifting moderation leaves the source
locally muted. Host disconnect clears every presentation and annotation grant,
as its relay is unavailable. Reconnect requires fresh capture/grants.

Annotation grants carry `epoch`; vector actions must include `grant_epoch` as
well as source generation and clear epoch. Admitted snapshots include only
unexpired `annotations` and `annotations_enabled`. `undo_annotation {source_id}`
removes the actor's last mark; `annotations_enabled {enabled}` is host-only.
`annotation_removed {source_id,annotation_id}` removes an individual overlay.
Source traffic is capped at 128 live marks; vectors at 64 points and 8 KiB,
32 events/second with burst48, pointers at20/second. Pointer/vector events are
deltas, not full room snapshots. Clearing/revocation filters queued stale
annotations before dispatch.

Terminal `input.data` uses **base64** (the existing terminal protocol), with a
32 KiB decoded input limit. Read-only `{type:"scrollback",lines?:number}` and
`{type:"snapshot"}` are supported, at1/second burst2; history is capped at10,000
lines. All admitted viewers may send `{type:"pause"}` and `{type:"resume"}`
without a driver grant. Pause stops output to that terminal socket only; it
does not stop the PTY or change driver authority. Resume sends one full
scrollback snapshot if output was skipped, otherwise streaming simply continues.
A repeated pause renews the two-second auto-resume deadline.
After discarding local renderer backlog, a viewer sends
`{type:"resync",lines?:number}` to request a full recovery snapshot even when
the server has no pending output. Resync clears the viewer's pause without
changing driver authority; a trailing resume does not duplicate the snapshot.
The first resync answers immediately. Further requests coalesce into one
pending recovery (latest requested history depth, capped at 10,000 lines), at
most twice per second. They are delayed rather than rejected by the history
rate limiter. Live output waits behind the pending snapshot, and its new
subscription starts atomically after the replay. Other terminal
frames, including binary input, are rejected. Input
already in an OS write cannot be retracted: the writer checks revocation before
each subsequent bounded256-byte write attempt. This does not undo commands
already delivered to the process.

Room capabilities are independent ephemeral credentials. Revoking the owner's
ordinary login/API token does not itself terminate an existing room; end the
room to revoke its member capabilities. Lifecycle and permission decisions are
audited without chat text, display capture, SDP, microphone data or credentials.

`update_present {source_id,width,height}` is presenter-only. An actual content
geometry change increments the existing source's generation and clear epoch,
clears its marks, and invalidates other members' annotation approvals. Host
blocks remain in force. The presenter's default self grant receives a new epoch.
An encoding quality tier change that preserves content geometry should not send
this action. The source ID, and therefore the viewer's personal pin, is retained.

Recap capture and durable owner archives: [room-recaps.md](room-recaps.md).
