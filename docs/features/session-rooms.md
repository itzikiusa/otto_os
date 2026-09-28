# Session rooms

Session rooms let a session owner invite up to three people into one running
local agent or shell session. Guests use room credentials; they do not sign in
to the host's Otto account. The host approves entry and remains in charge of
terminal input, presenting permissions and moderation.

## Start and join

1. Open a running local session and choose **Start room** from its More menu.
   On desktop, the room opens in its own native window; your workspace and
   sidebar stay where they are. **Open room** in the Rooms lobby focuses the
   existing room window. Browser hosting keeps the inline room view.
   Enter the name that participants should see. The shared terminal includes
   its available history; anyone granted control can execute commands with
   that session's permissions.
2. Choose **Invite someone**, select view-only or editor access, and copy the
   invitation. Each invitation is single-use and expires after ten minutes.
3. In another Otto, open **Rooms → Join room**, paste the invitation and review
   its destination. Enter a display name and wait for the host to admit you.
4. The host can admit, reject, remove or change a participant's role. An editor
   requests control; the host grants the exclusive driver seat. Viewers cannot
   type. The host can take control back at any time, including by typing in
   the original session pane.
5. **End room** disconnects participants and ends sharing. The underlying
   session and agent continue running.

A room shares one already-running local process. Connection terminals, remote
SSH/database sessions and resource-bound agents are ineligible. A process exit
or replacement ends sharing instead of silently attaching guests to a new
process. Rooms last at most eight hours and are not restored after daemon restart.

## Chat and voice

Chat is separate from terminal input and never becomes an agent prompt. Messages
are attributed by the server. A room retains up to 200 messages, with a maximum
of 4096 UTF-8 bytes per message. Chat history disappears when the room ends.

Voice requires an explicit **Join audio** gesture and microphone permission.
The host coordinates room audio; guests join after the host. Microphone mute,
room moderation and leaving audio are separate from terminal access. The host
mixes audio for each guest without sending that guest their own microphone.

## Presenting and highlighting

The host can grant presenting permission to multiple participants. Each person
can share one source at a time, selected through the platform's capture picker.
The available screen, window, application or tab choices depend on the browser
and macOS capture implementation. A particular IDE window can be shared when
that picker offers it. Otto does not silently select a capture source.

Each viewer chooses their own grid or pinned screen. Pinning does not change
anyone else's layout. Hidden views unsubscribe from video; a host still needs
to receive tracks that it forwards to other viewers.

A participant can ask the presenter for annotation permission. Approved people
can point, draw a pen stroke or mark a rectangle over the shared image. The
presenter approves access to their source; the host can revoke or block it.
Marks are tied to the source and its permission generation, expire quickly,
and can be cleared. They are overlays in Otto, not edits to the presented IDE
or drawings on the sharer's desktop. A mark does not follow a line of code when
that line scrolls away.

## Connecting different computers

The daemon remains loopback-only by default. Starting a room does not enable a
network listener, create a tunnel, or publish the host's machine. Remote guests
need a reachable HTTPS origin serving the host's Otto UI, HTTP routes and
WebSocket routes. A loopback invitation works only on the same computer.

A root administrator opens **Rooms → Connection settings** to set the public
HTTPS origin and STUN/TURN URLs. This setting describes existing network
infrastructure; it does not provision a certificate, reverse proxy or relay.
The TURN shared secret is saved in the host Keychain and is never returned by
the settings API. Admitted participants receive short-lived relay credentials.
The optional relay-only setting requires TURN for voice and video.

WebRTC voice and video also need network connectivity between participants.
STUN helps discover a reachable address; TURN relays media when direct paths
are unavailable. Use infrastructure operated for this purpose. Chat and the
terminal can still work when media connectivity fails. Room credentials do not
grant ordinary API access, and invitations carry their secret in the URL
fragment, which the guest page immediately removes from visible history.

Same-computer rooms still use WebRTC for voice and screens. An empty ICE
configuration may fail to gather usable addresses even on the same computer,
depending on the browser and its permissions. Configure an appropriate STUN or
TURN service in **Connection settings** if media cannot connect; Otto does not
silently add a third-party service. A working room page or chat connection does
not establish that the media connection is ready.

Native guest windows are ephemeral and isolated from the local app's native
capabilities. They are confined to the invited origin and room. Credentials live
only in window memory, so reloading or closing the window may require a new
invitation. Disconnecting withdraws control and capture grants immediately;
reconnecting does not restart the microphone or screen automatically.

Hosted desktop rooms use a separate authenticated window loading only the room
UI. Navigating the main sidebar does not remount that window, disconnect audio,
or stop screen sharing. Closing its view disconnects the host and pauses capture;
it does not end the underlying session. Reopen it from the originating window's
Rooms lobby. **End room** remains the explicit action that removes guest access.
Host room credentials cross native IPC in memory, never a URL, local storage or
the saved window registry. Closing the room window releases its native context;
the source window retains the capability for reopening while it stays alive.

## Performance and current limits

The media topology uses the host as the WebRTC coordinator. Screens are forwarded
as received video tracks, without canvas recapture in the production client.
Bandwidth profiles favor the pinned screen and reduce grid/preview resolution.

The synthetic native probe establishes track forwarding in WKWebView with four
simulated participants. It does **not** establish production performance across
four Macs, real screen capture, microphone quality or TURN. In particular,
WKWebView accepted requested preview frame-rate limits but did not reliably
honor them in outbound statistics. Those limits are best effort; the specified
CPU, memory, frame-rate and latency goals require further hardware validation.

One source per presenter is the initial limit. Multiple simultaneous sources
from the same presenter and TeamViewer-style mouse/keyboard control of the
computer are not included. Terminal control applies only to the shared session.
No unattended desktop access, file transfer or clipboard synchronization is added.

## Troubleshooting

- **Invitation expired or already used:** ask the host for a fresh invitation.
- **Room full:** four admitted people includes the host; the host can remove a
  participant to make room.
- **Cannot type:** view-only access cannot receive control. Editors must be the
  current driver. Reconnects and permission changes invalidate old input.
- **Chat works but media does not:** check capture/microphone permissions,
  secure-origin support, and STUN/TURN configuration, including for same-computer
  tests. Joining audio requires a deliberate click to resume the browser audio
  context.
- **Sharing stops after a connection loss:** this is deliberate. Rejoin media
  and request the relevant permission again after reconnecting.
- **Room closed after host disconnect:** membership has a thirty-second grace
  period; expiry ends the room while leaving the session alive.

The authoritative wire protocol is [Session rooms](../contracts/rooms.md).
Ordinary terminal input also follows the room authority rules documented in
[WebSockets](../contracts/ws.md).

## Session recap and full transcript

A recap combines timestamped speech, room chat, terminal output, source changes,
shared-screen samples and annotations. It produces a saved activity timeline
and an attributed Codex draft with an overview, decisions, action items and open
questions. The full accepted transcript is retained independently of the summary.

The host prepares a recap, then every connected admitted participant opts in.
Capture starts only after the host selects Start. A new participant, consent
withdrawal or disconnection pauses capture and invalidates pending media work.
Resuming requires fresh consent and a host action. Joining voice or presenting
alone does not enable recap capture. A visible room banner shows its state.
Only content already shared into this room is eligible for capture.

Finish stops new capture and shows the remaining recognition jobs while already
accepted speech finishes, for up to three minutes. Ending the room waits for
that finalization. Consent withdrawal still cancels pending work immediately;
timeout or interrupted processing is shown as a coverage gap.

Recognition runs on the host. Install [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
and download a **multilingual** GGML model using its documented model-download
instructions. In recap settings, select absolute paths to `whisper-cli` and the
model, choose Auto, English or Hebrew, and use 1–4 recognition threads. English
only `.en` models are unsuitable for Hebrew. Otto does not bundle a large speech
model, install it silently, or fall back to a cloud transcription API. A missing
recognizer is shown before capture; the host can explicitly continue without
speech, and the archive records that coverage gap.

The host records separate shared microphone streams, so speaker attribution
comes from room membership, not voice identification. Muted microphones are
excluded. Audio chunks are recognized locally and temporary audio inputs are
removed afterward. Speech recognition can mishear names, code and mixed-language
conversation. Review the transcript and generated commitments before relying on
them. Apple SpeechAnalyzer is not the sole engine because its language and OS
coverage do not cover this feature's requirements.

Shared screens are sampled at most once every 15 seconds per available source,
with unchanged frames skipped. macOS Vision extracts visible text locally; saved
images preserve visual context. This is sampled coverage, **not continuous video
recording**. Hidden/unavailable streams and queue overload produce coverage gaps.
Personal pins remain personal. Recap capture does not secretly switch them.

Select Generate summary to send captured evidence and representative shared
images to Codex through your existing ChatGPT sign-in. This uses subscription
allowances and needs no API key. Run `codex login` if setup reports no subscription
sign-in. The summary process disables action-taking tools and cannot operate the
shared terminal. Summaries remain drafts, and generation errors leave the full
transcript available. Long inputs are processed in chunks; requests exceeding the
bounded summary-input limit fail explicitly instead of summarizing only the end.
Local speech recognition and screen OCR do not send media to a transcription API.

Archives are local to the host under Otto's data directory, `room-recaps/`, and
remain after the room ends. Owner-only APIs provide paginated browsing and export.
No automatic archive deletion is performed. Capture stops at the per-archive
512 MiB quota. Participants do not receive access to host archives merely by
joining a room. Export/copy is a deliberate host action.

Synthetic local speech, screen-text and Codex tests are documented in
[recap validation](../testing/room-recap.md). Actual multi-person microphone
transcription quality and capture-plus-recognition performance remain acceptance
checks on the target hardware.
