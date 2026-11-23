# ACP agents — recorded capabilities

What each installed ACP agent advertised to Otto on **2026-11-23**, captured
with Otto's own handshake (`initialize` with Otto's client capabilities, then
`session/new` with a stdio probe MCP server; no prompt, no quota). The live
tests in `crates/otto-agent-protocol/tests/acp_live.rs` (all `#[ignore]`)
repeat the handshake through the real runtime and add the turns listed below.
Re-record after a CLI update: run the live tests with `--nocapture` (each
prints its capability line) and update this file.

Part of [Threads — structured agent runtime](./threads-structured-runtime.md).

## Summary

| | gemini | agy | grok | opencode | muse |
|---|---|---|---|---|---|
| Version | 0.43.0-preview.0 | 1.5.15 | 1.0.92 | 2.3.0 | 1.4.25 |
| `session/new` | **refused** (see below) | ok | ok (after `authenticate cached_token`) | ok | ok |
| Images / audio / embedded files | yes / yes / yes | yes / no / yes | yes / no / yes | yes / – / yes | yes / no / yes |
| `loadSession` | yes | yes | yes | yes | yes |
| `sessionCapabilities` | – | resume, close, load, list, additionalDirectories | list, resume, close | resume, close, delete, fork, list, additionalDirectories | list, resume, additionalDirectories |
| MCP over http / sse | yes / yes | – | yes / yes | yes / no | yes / no |
| Session modes | – | default, accept-edits, auto, plan | – | – (`mode` option: build, plan) | allowAll, promptUnmatched, onRequest, denyUnmatched |
| Config options | – | model, thought_level, mode | model (1 value), reasoning_effort | model (144), effort, mode | mode, model |
| Slash commands at `session/new` | – | yes | yes | yes | yes |
| Uses Otto's `fs/*` / `terminal/*` when advertised (one live turn) | – | no (own tools) | **yes** (`fs/read_text_file`, `terminal/create` → `wait_for_exit` → `output` → `release`) | no (own tools) | no (own tools) |

## What the live turns showed

One tiny prompt per agent ("read README.md, run `echo …`, call the probe's
`probe_echo`") plus the `acp_live` tests (command + probe tool, a cancelled
turn, close + reopen through `session/load`):

| | agy | grok | opencode | muse |
|---|---|---|---|---|
| Otto's MCP server delivered (probe answered with its env) | yes | yes (full access) | yes (through its code-mode `execute` tool) | yes |
| Approval-required asks (`session/request_permission`) | yes, command + MCP tool | **no**: every unapproved tool is "denied by prompt policy (tool not pre-approved)" and no request is sent. Grok runs `grok agent` sessions headless with `permission_mode: None` (its debug log); `--permission-mode` is a top-level flag `agent stdio` rejects, and Grok also reads Claude Code's `permissions.defaultMode` (`dontAsk` on the test machine). The thread now says once: "Grok denies tools without asking when ~/.claude/settings.json sets permissions.defaultMode to dontAsk · Use Full access for this thread". | yes (after the `OPENCODE_CONFIG_CONTENT` fix; 2.3.0 ignores `OPENCODE_PERMISSION`) | no for a workspace shell write: `promptUnmatched` is applied (`current_mode_update`) but Muse's own rules cover it |
| Options offered on a request | allow_once, reject_once | – | allow_once, allow_always, reject_once | – |
| Cancel → interrupted | yes | yes | yes | yes |
| Close + reopen keeps the agent's session (`session/load`) | yes | yes | yes | yes |
| Notices, compaction, non-text answer content, `terminal` tool content | none sent | none sent (vendor `_x.ai/*` notifications instead) | none sent | none sent |

Gemini CLI 0.43 refuses `session/new` for a personal Google account with
`-32000` "This client is no longer supported for Gemini Code Assist for
individuals. To continue using Gemini, please migrate to the Antigravity suite
of products". Otto shows that message on the thread (`gemini_live_handshake`).

## Raw dump

```json
{
 "gemini": {
  "agentInfo": "0.43.0-preview.0",
  "agentCapabilities": {
   "loadSession": true,
   "promptCapabilities": {
    "image": true,
    "audio": true,
    "embeddedContext": true
   },
   "mcpCapabilities": {
    "http": true,
    "sse": true
   }
  },
  "authMethods": [
   "oauth-personal",
   "gemini-api-key",
   "vertex-ai",
   "gateway"
  ],
  "session/new": {
   "error": "This client is no longer supported for Gemini Code Assist for individuals. To continue using Gemini, please migrate to the Antigravity suite of products: https://antigravity.google"
  },
  "notifications": []
 },
 "agy": {
  "agentInfo": "1.5.15",
  "agentCapabilities": {
   "loadSession": true,
   "promptCapabilities": {
    "image": true,
    "audio": false,
    "embeddedContext": true
   },
   "sessionCapabilities": {
    "resume": {},
    "close": {},
    "load": {},
    "list": {},
    "additionalDirectories": {}
   },
   "auth": {
    "logout": {}
   },
   "_meta": [
    "antigravity"
   ]
  },
  "authMethods": [
   "oauth-personal",
   "oauth-business",
   "gemini-api-key",
   "agent-platform"
  ],
  "session/new": {
   "modes": [
    "default",
    "accept-edits",
    "auto",
    "plan"
   ],
   "configOptions": [
    "model (model, 11 values)",
    "thought_level (thought_level, 4 values)",
    "mode (mode, 4 values)"
   ],
   "models": "27 (session/set_model)"
  },
  "notifications": [
   "session/update:available_commands_update",
   "session/update:usage_update"
  ]
 },
 "grok": {
  "agentInfo": "1.0.92",
  "agentCapabilities": {
   "loadSession": true,
   "promptCapabilities": {
    "image": true,
    "audio": false,
    "embeddedContext": true
   },
   "mcpCapabilities": {
    "http": true,
    "sse": true
   },
   "sessionCapabilities": {
    "list": {},
    "resume": {},
    "close": {}
   },
   "auth": {},
   "_meta": [
    "x.ai/fs_notify",
    "x.ai/hooks",
    "x.ai/capabilities"
   ]
  },
  "authMethods": [
   "cached_token",
   "grok.com"
  ],
  "session/new": {
   "modes": null,
   "configOptions": [
    "model (model, 1 values)",
    "reasoning_effort (thought_level, 4 values)"
   ],
   "models": "1 (session/set_model)"
  },
  "notifications": [
   "_x.ai/session/setup",
   "_x.ai/mcp/servers_updated",
   "_x.ai/models/update",
   "_x.ai/settings/update",
   "_x.ai/announcements/update",
   "_x.ai/mcp/init_progress",
   "session/update:available_commands_update",
   "_x.ai/mcp/server_status",
   "_x.ai/session_notification"
  ]
 },
 "opencode": {
  "agentInfo": "2.3.0",
  "agentCapabilities": {
   "loadSession": true,
   "mcpCapabilities": {
    "http": true,
    "sse": false
   },
   "promptCapabilities": {
    "embeddedContext": true,
    "image": true
   },
   "sessionCapabilities": {
    "additionalDirectories": {},
    "close": {},
    "delete": {},
    "fork": {},
    "list": {},
    "resume": {}
   },
   "_meta": [
    "opencode/child-session-updates"
   ]
  },
  "authMethods": [
   "opencode-login"
  ],
  "session/new": {
   "modes": null,
   "configOptions": [
    "model (model, 144 values)",
    "effort (thought_level, 7 values)",
    "mode (mode, 2 values)"
   ],
   "models": null
  },
  "notifications": [
   "session/update:available_commands_update"
  ]
 },
 "muse": {
  "agentInfo": "1.4.25",
  "agentCapabilities": {
   "loadSession": true,
   "promptCapabilities": {
    "image": true,
    "audio": false,
    "embeddedContext": true
   },
   "mcpCapabilities": {
    "http": true,
    "sse": false
   },
   "sessionCapabilities": {
    "list": {},
    "resume": {},
    "additionalDirectories": {}
   }
  },
  "authMethods": [
   "muse-login"
  ],
  "session/new": {
   "modes": [
    "allowAll",
    "promptUnmatched",
    "onRequest",
    "denyUnmatched"
   ],
   "configOptions": [
    "mode (mode, 4 values)",
    "model (model, 5 values)"
   ],
   "models": null
  },
  "notifications": [
   "session/update:available_commands_update"
  ]
 }
}
```
