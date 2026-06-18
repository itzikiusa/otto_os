# Access control, secrets & sensitive-data exposure

The bugs that taint tracing won't catch: a **missing** check, a **leaked** secret, an
**over-shared** field. These are about what the code *doesn't* do. Walk them deliberately.

---

## Authentication & authorization

### Authentication — is the caller who they claim to be?
- Is every new/changed endpoint behind the auth middleware? A refactor that moves a route, or
  registers it on a different router, can drop it outside the guard. Check the *effective*
  routing, not the intent.
- Is auth enforced **before** the sensitive work, not after (no work done, then rejected)?
- Token handling: is the signature/expiry actually verified? `alg:none` accepted? A token
  from one tenant/audience accepted by another? A long-lived token where a short one belongs?
- No **auth bypass** branch: a debug header, a default-admin account, a `if env == "dev"`
  shortcut, an empty-password path.

### Authorization — may *this* caller do *this* to *this* object?
Authentication ≠ authorization. Logged in is not the same as allowed.
- **Function-level:** does the handler check the caller's **role/permission** for the action
  (delete, refund, export, admin)? A new admin action with no role check is a blocker.
- **Object-level (IDOR):** the handler takes an `id`/`accountId`/`orderId` from the request —
  does it verify the **current user owns or may access that object**, or does it just fetch by
  id and return it? "Fetch by id, return" with no ownership check is **IDOR** — the classic,
  most common real-world finding. Trigger: change the `id` in the URL to a victim's.
- **Mass assignment:** does the handler bind request fields straight onto a model, letting a
  caller set `isAdmin`, `role`, `balance`, `ownerId`? Bind an explicit allow-list of fields.
- **Indirect references / enumeration:** sequential or guessable ids that leak existence;
  consider whether the *count* or *error difference* leaks data (user-exists oracle).
- **Consistency:** if a sibling endpoint checks ownership and the new one doesn't, that's a
  strong signal the check was forgotten.

### Where authz gaps hide
- A new endpoint copied from an existing one but the `requirePermission(...)` line wasn't.
- A GraphQL resolver / batch endpoint that checks the parent but not each child.
- A "internal only" route reachable from outside because it's on the public listener.
- A check on the **read** path but not the **write/delete** path (or vice versa).

---

## Secrets & credential handling

- **Hard-coded secrets:** API keys, passwords, private keys, tokens, connection strings with
  credentials, signing secrets committed in source or config. Even in tests/examples, a real
  secret is a leak — rotate it.
- **Secrets in logs:** a token/password/key logged at any level, an entire request/headers
  object logged (auth header rides along), a secret in an exception message.
- **Secrets in URLs / query strings:** they land in access logs, browser history, proxies, and
  the `Referer` header. Put secrets in headers or the body, never the URL.
- **Secrets to the client:** an API response or error that echoes an internal token/key; a JWT
  with sensitive claims returned where it can be inspected.
- **Weak secret lifecycle:** no rotation path, a secret defaulted to a known dev value when the
  env var is unset (so prod silently runs with the dev secret).

---

## Sensitive-data exposure

- **PII / financial / health data in logs:** full card numbers, account numbers, SSNs, emails
  at debug volume, full request bodies. Mask or drop before logging.
- **Verbose errors to the client:** stack traces, SQL errors, internal hostnames/paths, library
  versions returned in a 500 — they map the attack surface. Return a generic message; log the
  detail server-side.
- **Over-broad responses:** an endpoint that serializes the whole DB row (password hash,
  internal flags, other users' fields) instead of an explicit DTO/field allow-list.
- **Caching/transport:** sensitive responses cacheable (`Cache-Control`), or sent over a
  downgraded/`verify=false` connection.

---

## Cryptography & randomness

- **Weak randomness:** `Math.random()`, `rand()`, time-seeded PRNG used for tokens, session
  ids, password-reset codes, nonces, or anything security-bearing. Use a CSPRNG
  (`crypto.randomBytes`, `secrets`, `/dev/urandom`).
- **Password storage:** plain, encrypted-not-hashed, MD5/SHA-1, fast/unsalted hash. Use a slow
  KDF (bcrypt/scrypt/argon2) with a salt.
- **Symmetric crypto misuse:** ECB mode (patterns leak), static/zero IV/nonce, IV reuse with
  CTR/GCM, encrypt-without-authenticate (no MAC/AEAD), a key derived from a low-entropy value.
- **Transport / verification disabled:** `verify=false`, `InsecureSkipVerify: true`, accepting
  any TLS cert, disabling host-name checks, downgrade to HTTP.
- **Homemade crypto:** a hand-rolled cipher, XOR "encryption", a custom token format with no
  signature. Use vetted primitives.
- **Comparison:** non-constant-time compare of secrets/HMACs/tokens (timing oracle) — use a
  constant-time compare.

---

## Unsafe defaults & configuration

- **CORS:** `Access-Control-Allow-Origin: *` together with `Allow-Credentials: true`, or
  reflecting the `Origin` header unchecked.
- **Debug/verbose:** debug mode, stack traces, or a profiler enabled in a path that can reach
  production.
- **Open by default:** a new feature flag / route / admin tool that defaults to enabled or
  unauthenticated; a queue/bucket/DB exposed without auth.
- **Permissions:** files written world-readable/writable; a temp file with a secret in a shared
  dir; an over-broad IAM scope in code.
- **Dependencies:** a new dependency pinned to a known-vulnerable version, or pulled from an
  untrusted source. (Flag if visible in the diff; deep SCA is out of scope.)
