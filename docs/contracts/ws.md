# Otto WebSocket Contract (FROZEN)

Two WS endpoints. Auth for both: `?token=<bearer token>` query parameter, validated
BEFORE the upgrade completes; invalid token → HTTP 401, no upgrade.

## 1. Terminal stream — `WS /ws/term/{session_id}`

Role: workspace **viewer** may attach (read-only); **editor**+ may send input/resize.
Input frames from viewers are silently dropped server-side (and a single JSON
`{"type":"error","code":"forbidden"}` is sent once).

### Client → server (JSON text frames)

```json
{"type":"input","data":"<base64 bytes>"}
{"type":"resize","cols":120,"rows":32}
{"type":"scrollback","lines":2000}
```

### Server → client

- **Binary frames**: raw PTY output bytes — write straight into xterm.
- **JSON text frames**:

```json
{"type":"scrollback","data":"<base64 bytes>"}      // response to scrollback request; send BEFORE live bytes resume
{"type":"status","status":"working"}                // running|working|idle|exited|reconnectable
{"type":"exit","code":0}                            // child exited; socket stays open
{"type":"error","code":"forbidden","message":"..."}
```

Multiple clients may attach to one session simultaneously; all receive the same
output broadcast. Input is interleaved in arrival order. On attach the server
sends current `status` immediately.

## 2. Event stream — `WS /ws/events`

Server → client only. Each message is one JSON-serialized `otto_core::event::Event`
(see crate; tag field `type`, snake_case). The server filters events: a client only
receives session events for workspaces it is a member of (root receives all).
`Notice` events are delivered to all authenticated clients.

Client→server messages on this socket are ignored. Ping/pong handled by the
transport layer (axum auto-responds to pings; server sends a ping every 30s).
