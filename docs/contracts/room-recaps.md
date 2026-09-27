# Room recap contract

Recaps are owner-local durable records under `data_dir/room-recaps/<uuid>`.
Ordinary owner authentication is required for every endpoint below. Room member
credentials never authorize these HTTP routes. Scoped, MCP, managed-session and
impersonation credentials are rejected. Even root cannot read another owner's
archive. Settings mutation additionally requires root. No API-key fallback exists.

## HTTP

All paths are below `/api/v1`; request/response bodies are JSON unless stated.
Rust definitions in `crates/otto-core/src/api/recap.rs` are canonical; UI mirrors
are in `ui/src/lib/api/room-recap-types.ts`.

| Method/path | Request | Response |
|---|---|---|
| POST `/rooms/{id}/recaps` | `{allow_unavailable_speech:false}` | `RecapMetadata` |
| GET `/room-recaps` | — | `RecapMetadata[]`, newest first |
| GET `/room-recaps/{id}?after=0&limit=200` | — | `RecapDetail`; limit 1–1000 |
| GET `/room-recaps/{id}/export` | — | streamed `application/x-ndjson`, complete event timeline |
| GET `/room-recaps/{id}/images/{image_id}` | — | JPEG, `Cache-Control:no-store` |
| POST `/room-recaps/{id}/audio` | `RecapAudioReq` | `{queued:true}` |
| POST `/room-recaps/{id}/screen` | `RecapScreenReq` | `{queued:true}` |
| POST `/room-recaps/{id}/gap` | `RecapGapReq` | `{queued:true}` |
| POST `/room-recaps/{id}/summary` | — | `{queued:true}` |
| DELETE `/room-recaps/{id}/summary` | — | 204; abort running summary |
| GET/PUT `/room-recap-settings` | PUT `RecapEngineSettings` | `RecapEngineSettings` |
| GET `/room-recap-capabilities` | — | capability object below |

Export retains every accepted event independently of summary context. Screen
records contain `image_id`; image bytes are downloaded through the owner-only
image endpoint. Export is a timeline, not a video or self-contained image bundle.
No archive deletion or automatic retention expiry is implemented.

`RecapEngineSettings` contains `whisper_executable:string`, `whisper_model:string`,
`language:"auto"|"en"|"he"`, `threads:1..4`. Paths must be absolute (empty means
unconfigured). Capabilities contain `speech_ready:boolean`,
`executable_ready:boolean`, `model_ready:boolean`, `configuration_error:string|null`,
`codex_subscription_ready:boolean`, `screen_text_ready:boolean`,
`languages:string[]`, `setup:string`. Missing speech recognition prevents create
unless the host explicitly sets `allow_unavailable_speech:true`; metadata retains
the missing-recognizer error. Local recognition failures produce gap events.

`RecapMetadata` fields: `id`, `owner_id`, `room_id`, `session_id`, `session_title`,
`created_at`, `updated_at` (strings); `status:RecapState`, `capture_epoch`,
`bytes_used`, `quota_bytes`, `last_seq` (integers); `speech_available:boolean`,
`speech_error:string|null`, `summary_status`, `summary_error:string|null`,
`summary_through_seq:integer|null`. `RecapState` is `awaiting_consent|ready|capturing|finalizing|paused|stopped`.
Summary status is `idle|queued|running|ready|error|cancelled`.

`RecapDetail` is `{metadata,events:RecapEvent[],next_cursor:number|null,draft:RecapDraft|null}`.
Continue with `after=next_cursor`; this is an exclusive event sequence cursor.
A sparse in-memory offset index seeks near the requested sequence (128-event
spacing), rebuilt lazily from the journal on the first owner detail request.
Archive-list startup reads metadata only; initialization and recovery run off the
async executor. Journal recovery reconstructs the sequence and storage count.
An incomplete/corrupt suffix is preserved untouched on disk; complete prior
events remain browseable, with an `archive_recovery` gap projected into detail,
summary and export. Export emits the complete prefix plus that gap. Metadata durability is batched
every 64 data events or one second of active appends; lifecycle changes persist
immediately.
`RecapDraft` contains `overview:string`, `decisions:string[]`, `actions:string[]`,
`open_questions:string[]`, `coverage:string[]`, `source_event_ids:number[]`.
Drafts are attributed model output and never execute or publish actions.
`summary_through_seq` is the final event included in that invocation, not the
current live archive tail. Summary input above 3 MiB fails explicitly before
loading/model invocation, directing the owner to export and summarize sections.
All validated stored image references are passed to the engine, which selects
representative images and discloses selection coverage. Summary processing has
its own single-job lane; it does not hold the local ASR/OCR processor.

## Consent and live projection

Only admitted room snapshots have optional `recap`:
`{id,state,epoch,pending_jobs:number,consented_member_ids:string[],reason:string|null,started_at:string|null}`.
Pending guests see no recap state or archive contents. Capture is off by default.
Create establishes epoch 1; each connected admitted participant sends
`{type:"recap_consent",epoch,allow:true}`. The host must then send
`{type:"recap_start",epoch}`. Ready state never starts automatically.
Host action `{type:"recap_pause"}` immediately pauses and discards unfinished
processing. `{type:"recap_stop"}` enters `finalizing`: no new capture/uploads are
accepted, but already accepted same-epoch recognition and archive writes drain.
`pending_jobs` shows outstanding recognition. Both recognition and writer counts
must reach zero before `stopped`; after 180 seconds, remaining work is cancelled
with an explicit cutoff reason. Withdrawal/disconnect/removal during finalization
still cancel immediately. The UI defers deliberate End room until finalization
ends; forced room termination cancels capture immediately.
Withdrawal (`allow:false`), admitted membership changes, disconnect/reconnect,
and forced room end invalidate the epoch and clear consent. Stopped recaps cannot
restart; an owner may create a new archive. Old archive callbacks are fenced
by archive ID as well as epoch. Daemon restart finalizes live archives and marks
unfinished summaries failed; archives remain owner-accessible after room end.

## Ingestion and limits

`RecapAudioReq`:
`{capture_epoch,member_id,member_generation,sequence,offset_ms,duration_ms,wav_base64}`.
Audio is canonical RIFF/WAVE PCM16, 16 kHz, one channel, <=30 seconds/960044 bytes.
Declared duration must match samples within 1 ms. Attribution comes from the
server's current admitted member. A bounded final pre-mute chunk is accepted
only within that member's shared audible interval (500 ms clock tolerance),
same connection generation and consent epoch. Reconnection/removal/withdrawal
invalidates it. Uploads never grant microphone or OS input access.

`RecapScreenReq`:
`{capture_epoch,source_id,source_generation,sequence,offset_ms,jpeg_base64}`.
Only current shared source generations are accepted. JPEG <=256 KiB, maximum
side 1280 pixels and area 1280×720 (portrait allowed). One frame per 15 seconds
per source generation; first frame after source change is immediate. UI applies
perceptual change filtering. Sampling is not full video capture.

`RecapGapReq`:
`{capture_epoch,kind,member_id:string|null,source_id:string|null,reason}`.
Kind is 1–64 bytes, reason 1–1024 bytes. Optional member/source must exist.
Only the owner submits ingestion and gaps, never arbitrary room guests.

Offsets are milliseconds since the current capture `started_at`, bounded by
server elapsed time plus 5 seconds of clock slack. Sequences start at 1 and
increase per member/source per epoch; duplicate/reordered uploads are rejected.
There are eight pending media jobs and one local ASR/OCR processor. Queue pressure
returns 409; callers show/report a gap and continue live room media. The archive
writer has a bounded 128-event data queue and a separate latest-state control
lane. Full data queues pause capture without delaying PTY or media handling.
Quota is 512 MiB/archive; reaching it stops capture visibly. Directory/file
permissions are 0700/0600 for newly created archive data.

Consent and source generation are rechecked before inference and before storage.
Consent invalidation cancels queued/in-flight local recognition futures promptly,
kills the recognition child, and releases the single processor.
Withdrawal invalidates queued work without waiting for disk locks. An already
in-flight disk write cannot be retroactively cancelled; image writes invalidated
before event commit are discarded. Raw audio is a temporary inference input and
is not retained in the archive. PTY capture subscribes only on explicit start;
no pre-start scrollback or raw input bytes are archived. Chat, source transitions
and annotations are captured from accepted server events. No SDP, room tokens,
TURN secrets, auth headers or arbitrary guest archive records are recorded.

## Timeline events

Every event is `{seq,created_at,capture_epoch,payload}`. `payload.type` selects:

- `capture`: `state,reason,started_at`.
- `participants`: `members:RoomMember[]`.
- `chat`: `message:RoomMessage`.
- `terminal`: `data_base64` (actual PTY output).
- `presentation`: `operation:"current"|"start"|"update"|"stop",presentation:RoomPresentation`.
- `annotation`: `annotation:RoomAnnotation`.
- `annotations_cleared`: `source_id,clear_epoch`.
- `annotation_removed`: `source_id,annotation_id`.
- `audio_pending`: `member_id,member_name,sequence,offset_ms,duration_ms`.
- `speech`: `member_id,member_name,sequence,offset_ms,segments:[{start_ms,end_ms,text}]`.
- `screen`: `source_id,source_generation,title,image_id,offset_ms,text` (OCR text).
- `gap`: `kind,member_id,source_id,reason`.

Speech segment offsets are relative to that audio chunk. Pending audio without a
speech completion indicates interrupted processing; capture state/gaps provide
coverage context. Media failures never fabricate transcript or OCR content.
Errors use the ordinary API envelope: 400 invalid formats/offsets, 401 missing
ordinary credentials, 403 wrong owner/expired consent, 409 capacity/state conflicts.
