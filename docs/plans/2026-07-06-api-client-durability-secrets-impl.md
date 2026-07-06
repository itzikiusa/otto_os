# API Client Durability & Secrets — Implementation Plan

> Implements `docs/superpowers/specs/2026-07-04-api-client-durability-secrets-design.md`.
> Date: 2026-07-06 · Branch: `worktree-api-client-durability-secrets` (worktree).

## Corrections to the spec discovered during code study

1. **`mcp_servers` already has the secret columns.** Migration `0077` (Control
   Plane) added `secret_ref`, `secret_env_keys`, `headers_json`, … to the SAME
   `mcp_servers` table the workspace CRUD uses — the two surfaces share rows.
   So part D needs **no migration**; it adopts the existing columns and the
   existing Control-Plane conventions: keychain ref `mcp-{id}`
   (`McpService::secret_ref`) holding a `{"env":{…},"headers":{…}}` blob. Using
   the spec's proposed `otto.wsmcp.<id>` ref would fork the source of truth for
   the same row; we deliberately reuse `mcp-{id}` and merge-preserve the
   `headers` part of an existing blob.
2. **The interactive script engine is browser-side** (`new Function` in
   `ui/src/lib/api/scripts.ts`). "Run scripts with the same engine" for
   server-side automations means reproducing the same `pm` API surface
   server-side. We add `boa_engine` (pure Rust, no native build deps) to
   `otto-server` and define the `pm` object in a JS prelude ported from
   `scripts.ts`, so semantics match the interactive runner.
3. **Next free migration was `0100` at implementation time** (0099 = skill_reviews) — renumbered to `0101` at merge because another feature landed `0100_review_agent_prompts.sql` on main first.

## A. Persist the draft-only request fields (`extras`)

- Migration `0101_api_client_durability_secrets.sql`:
  - `ALTER TABLE api_requests ADD COLUMN extras_json TEXT;` (NULL = never set)
  - `ALTER TABLE api_environments ADD COLUMN secret_keys_json TEXT;` (for B)
- `otto-core::domain::ApiRequest` gains `#[serde(default)] pub extras: Option<Value>`;
  `UpsertApiRequestReq` mirrors it. Shape (UI-owned, server validates only
  object-ness + ≤256 KiB):
  `{v:1, transport, graphql_variables, docs_md, scripts:{pre,post}, settings:{timeout_ms,follow_redirects,tls_verify}}`
- `otto-state::api_client` round-trips `extras_json` in create/update/get/list
  exactly like `auth_json` (None ⇄ NULL).
- UI (`apiClient.svelte.ts`): `saveDraft` serializes the draft's kind /
  graphql_variables / docs / scripts / settings into `extras` (omit empty →
  null); `loadRequestIntoDraft` restores them; the tab dirty indicator (if one
  exists) must cover them.
- **Automations** (`run_automation`/`run_step`):
  - pre script (extras.scripts.pre) runs before send: may mutate
    method/url/headers/body and set vars (chained map).
  - graphql: body wrapped as `{query, variables}` (extras.graphql_variables),
    matching the interactive path.
  - extras.settings map onto ExecuteApiReq timeout_ms/follow_redirects/verify_ssl.
  - post script runs after: reads response, sets vars, `pm.test` results are
    appended to the step's assertions (`script: <name>`) and affect `ok`.
  - Engine: `crates/otto-server/src/api_scripts.rs` (boa + JS prelude), with
    runtime loop limits so a bad script can't hang the daemon.
- **Exports**:
  - OpenAPI (`collection_to_openapi`): `description` ← docs_md; `x-otto-settings`,
    `x-otto-graphql-variables` extensions when present.
  - Postman (UI `importers.ts` `pmItem`): `event[]` prerequest/test from scripts,
    `request.description` ← docs_md, `body.graphql.variables`,
    `protocolProfileBehavior {followRedirects, strictSSL}` + `_otto.settings`
    for timeout. Importer (`detectAndParse` Postman path) reads these back so
    git-sync round-trips are lossless.
  - Secret markers in exported auth are serialized as the literal placeholder
    string `{{otto:secret}}` — never a resolved value (see B).

## B. Keychain-backed API-client secrets

- **Marker**: a secret member's value is replaced by `{"$secret":"<ref>"}`.
  Refs: `otto.api.request.<request_id>` / `otto.api.env.<env_id>`. One keychain
  entry per request/env holding a flat JSON map `member → value` (requests) or
  `key → value` (environments), via the daemon's existing `SecretStore`
  (`ctx.secrets`).
- **Secret auth members** (per type): bearer→`token`; basic→`password`;
  api_key→`value`; oauth2→`client_secret`,`refresh_token`,`password`,`access_token`.
- **Lazy migration on save**: create/update request handlers scan the incoming
  auth; plaintext secret members move to the keychain and the row stores
  markers. Markers arriving from the UI (unchanged fields) are kept as-is
  (ref validated to be this request's own). Stale blob members are dropped on
  auth-type change; the blob (and entry) is deleted when empty / on request
  delete.
- **Environments**: `ApiEnvironment.secret_keys: Vec<String>` (+ column). The
  row's `variables` holds NON-secret pairs only; secret values live in the
  keychain blob. `UpsertApiEnvironmentReq` gains `secret_keys` +
  **write-only** `secret_values: {k:v}` (only new/changed values are sent).
  GET never returns secret values (keys only). Delete env → delete blob.
- **Execute-time resolution** (only in `execute` + automations, in-memory):
  - auth markers resolved from the keychain **after validating the ref belongs
    to this workspace** (parse ref → load request/env → workspace check);
  - active/selected environment's secret blob is merged into the variable map;
  - `oauth2/token` endpoint accepts marker-or-string for
    `client_secret`/`password`/`refresh_token` and resolves the same way.
- **History redaction**: the executor's request snapshot replaces every secret
  auth member (plaintext or marker) with `"***"` before insert — success and
  failure paths both.
- **`POST /workspaces/{wid}/api-client/secure-all`** (ws editor; existing
  policy prefix ⇒ ApiClient/Edit): sweeps every request (moves plaintext secret
  auth members) and every environment (secret-shaped keys — name matches
  token/secret/password/api[-_]?key/authorization — become `secret_keys` and
  move to the keychain). Returns
  `{requests_secured, env_keys_secured}`. Idempotent.
- **UI**: auth inputs render markers masked (`•••••• stored in Keychain`,
  typing replaces); `codegen.ts`/`toCurl` emit `***` for markers; EnvSelector
  gains a per-row lock toggle (secret rows masked; save sends
  variables/secret_keys/secret_values); an explicit "Secure secrets" action
  triggers secure-all.

## C. Per-workspace cookie jar

- Replace the `OnceLock` global jar with a module-level
  `Mutex<HashMap<Id, Arc<CookieStoreMutex>>>` (same idiom as `tunnel_cache`).
- The shared reqwest client becomes per-workspace (`http_client_for(wid)`,
  cached per wid); `build_settings_client` takes the wid jar too;
  `oauth2_token` uses the caller's workspace client.
- `list_cookies`/`clear_cookies` operate on the caller's workspace jar only.
- Contract wording: "shared jar" → "workspace jar". Jars stay in-memory.

## D. Workspace MCP server env secrets

- Domain `McpServer` gains `#[serde(default)] secret_env_keys: Vec<String>` and
  `#[serde(default)] secret_ref: Option<String>` (read from existing 0077
  columns). `env` keeps non-secret pairs only.
- `CreateMcpServerReq`/`UpdateMcpServerReq` gain write-only `secret_env`
  (create: map, default empty; update: `Option<map>`, None = unchanged,
  Some = full replacement of the secret-env set).
- Routes: on create/update, secret values go to the keychain under `mcp-{id}`
  (merge-preserving the blob's `headers` part written by the Control Plane);
  the row stores key names in `secret_env_keys`. GET returns names only.
  Delete already cascades; route also deletes the blob.
- `McpServersRepo`: read the two columns; add `set_secret_meta`; drop
  secret-named keys from `env_json`.
- `DbMcpServerProvider` gains the `SecretStore`; `enabled_servers` resolves the
  blob at `.mcp.json` merge time and overlays `env`. The rendered `.mcp.json`
  still contains real values (documented residual). `ottod/main.rs` passes
  `secrets.clone()`.
- UI Settings MCP editor: secret env rows (masked, names listed from
  `secret_env_keys`, values write-only).
- Docs: remove the "plaintext for now" caveat (`connections-ssh-sftp.md` §7),
  document the `.mcp.json` residual explicitly.

## Contracts & docs

- `docs/contracts/api.md`: Request/Upsert `extras`; auth `$secret` markers +
  lazy-migration semantics; Environment `secret_keys` (+ write-only
  `secret_values`); `POST …/secure-all`; cookies per-workspace wording;
  McpServer `secret_env_keys` + write-only `secret_env`. All additive.
- `ui/src/lib/api/types.ts` in lockstep.
- `docs/features/api-client.md` §3/§9/§10 + troubleshooting rewritten where the
  draft-only limitation / plaintext auth was flagged.

## Testing (focused on this change)

- otto-state: `extras_json` + `secret_keys_json` round-trips (mirroring
  existing tests).
- otto-server unit: marker split/resolve/redact helpers; secure-all heuristics;
  boa pm-engine (pre mutates request + vars; post extracts var; pm.test
  pass/fail; console capture; hostile script hits the loop limit, not a hang);
  cookie-jar isolation (two wids → distinct stores; Set-Cookie in one not
  visible in the other).
- otto-server integration test (`tests/api_client_secrets.rs`): repo+keychain
  (FileStore) flow — save request with plaintext auth → row holds marker, blob
  holds value; history insert redacted; secure-all idempotent; export path
  (OpenAPI fn) never contains the secret. (Live `execute` against loopback is
  SSRF-blocked by design, so send-path coverage is at the unit seam.)
- UI: `npm run check`; focused Playwright spec
  `ui/e2e/desktop-api-durability.spec.ts` — save request with
  scripts/docs/settings/graphql vars → reload → still present after loading
  the saved request; env secret lock round-trip renders masked; plus rerun of
  `desktop-api-tabs-persist.spec.ts` (adjacent surface).
- Automations: engine-level test that a stored pre script's variable feeds the
  next step's substitution (unit seam on `run_step`'s helpers).

## Requirement coverage matrix (spec § → plan)

| Spec requirement | Covered by |
|---|---|
| extras persisted (scripts/docs/settings/gql vars/transport) | A: migration+core+state+UI save/load |
| unsaved-changes dot covers extras | A: UI dirty indicator |
| automations run stored scripts | A: api_scripts.rs + run_step |
| exports serialize docs/gql/settings; scripts → Postman events | A: exporters both sides |
| extras is `Value`, server validates object + 256 KiB cap | A: handler validation |
| auth secret members → Keychain markers, one entry per request | B |
| env `secret_keys` + `otto.api.env.<id>` blob, masked, never in GET | B |
| history snapshot redacts to `"***"` | B |
| lazy migration on save + explicit `secure-all` (editor, counts, idempotent, no boot sweep) | B |
| resolution only at execute/automation, in-memory; exports get markers only | B |
| per-workspace cookie jar; list/clear scoped; wording updated | C |
| McpServer secret_env_keys+secret_ref; env non-secret only | D (existing 0077 columns) |
| create/update accept secret_env → keychain, never row; GET names only | D |
| .mcp.json writer resolves at merge; residual documented | D |
| contracts + types.ts lockstep | Contracts section |
| migrations append-only, next-free number verified (=0101 at merge) | A |
| tests per spec Testing section | Testing section |
| Out of scope untouched (OAuth2 auth-code, persistent cookies, SQLite encryption, binary body, bidi gRPC) | not implemented |
