// Contract mirror: agent-runtime. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md ("Structured agent runtime") + ws.md
// (`agent_runtime_event`) — change them together.

// ── Structured agent runtime (docs/features/threads-structured-runtime.md) ──
// Mirror of crates/otto-core/src/agent_runtime.rs, itself a port of T3 Code's
// provider runtime model (MIT, Copyright (c) 2026 T3 Tools Inc.:
// `ProviderAdapterV2Event` + the `OrchestrationV2*` entities). Events are
// entity upserts — an item is re-sent WHOLE with its accumulated text and a
// `streaming` flag; `command_output.delta` is the only true delta. Field
// names are snake_case where T3 Code uses camelCase.

// The types live in ./agent-runtime/ (events.ts: entities, events and replay;
// sessions.ts: sessions, councils, peers and request bodies).

export * from './agent-runtime/events';
export * from './agent-runtime/sessions';
