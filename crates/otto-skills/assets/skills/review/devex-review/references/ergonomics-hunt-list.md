# Ergonomics hunt list — what to hunt in each pass

Run one pass per lens. Within a pass, keep only that lens active and walk every exposed
surface as the *caller* who will use it. This list is a prompt, not a cage — real friction
outside the list still counts. Each item, when you hit it, must clear the evidence bar in
`severity-and-finding.md` (show the awkward call, quote the bad message, or name the divergent
sibling) before it becomes a finding.

## 1. Easy to use right, hard to use wrong (pit of success)

- **Silent transposition** — two or more adjacent params of the *same type* a caller can swap
  with no compile/type error: `(width, height)`, `(host, port)` both strings, two `bool`s,
  two `id`s. The classic blocker of this lens.
- **Boolean-blindness at the call site** — `f(true, false, true)` where the call reads as
  noise. An enum or named/options argument would make the call self-documenting.
- **Stringly-typed surface** — accepting a `String` where a closed set (enum/union) is meant;
  the caller has to guess the magic value and finds the valid set only by reading source.
- **Order-dependence with no guard** — must call `init()` before `use()`, `open()` before
  `write()`, set field A before B — and nothing (type state, builder, constructor) enforces
  it; the wrong order compiles and fails (or corrupts) at runtime.
- **Footgun default** — the default value/behavior is the *dangerous* one (retries off,
  validation off, `verify=false`, unbounded), so the easy path is the unsafe path.
- **Easy-to-leak resource** — exposes an `open`/`acquire` with no RAII/`with`/`defer`/scoped
  helper, so every caller must remember to release. Make the safe usage the default shape.
- **Mutable thing handed out** — returns an internal mutable reference/slice/map the caller can
  corrupt; or takes ownership ambiguously so the caller doesn't know if they still own it.

## 2. Naming & clarity

- **Misleading verb** — `get*` that mutates or does I/O; `validate*` that also persists;
  `parse*` that throws vs returns; a name that implies cheap when it's expensive (a network
  call named like a getter).
- **Unit-less / scale-less** — `timeout: 30`, `size: 1024`, `delay`, `limit` with no unit in
  the name or type. Is it ms or s, bytes or KB, count or bytes? Encode the unit
  (`timeout_ms`, `Duration`, a typed quantity).
- **Insider abbreviation** — a name only the author's head expands (`procCtx`, `tmpHdlr`,
  `do2`). The next developer reads it cold.
- **Convention break** — the name/casing/shape diverges from the established local convention
  for the same kind of thing (the rest of the module uses `fetch_x`, this adds `getX`).
- **Negative / double-negative flags** — `disableNoCache`, `skipUnlessForce`; the caller has
  to think twice to know what `true` means.

## 3. Defaults & required-vs-optional

- **Common case is the hard case** — the most frequent call requires the most setup; the 95%
  caller pays for the 5% caller's flexibility. Provide a sane default / convenience overload.
- **Too many required positional params** — 4+ required args, especially mixed types. Reach
  for an options object / builder / struct-of-args so call sites stay readable and additive.
- **Surprising default** — a default that violates least-astonishment (silently truncates,
  picks prod, swallows errors). State it and justify it, or change it.
- **No escape hatch** — an opinionated default with no override. Opinionated defaults are a
  feature; *no way to override them* is a trust-killer at scale (`dx-principles.md`).
- **Required arg that's almost always the same value** — make it default; let the rare caller
  override.

## 4. Error messages (judge against the three-tier model in dx-principles.md)

- **Bare-string panic/throw** — `panic("bad input")`, `throw Error("failed")`: no field, no
  value, no next step.
- **Lost offending value** — "invalid id" without *which* id; "parse error" without the input
  or position. The developer can't act without it.
- **Generic API error** — `400 Bad Request` / `{"error":"invalid"}` with no field path or
  reason; a `500` that hides a validation failure the caller could fix.
- **No "what to do next"** — the message states the problem but not the remedy ("config
  invalid" vs "config invalid: `port` must be 1-65535, got 0 — set a valid port").
- **Wrong audience** — an internal stack trace / Rust panic / SQL error surfaced to an API
  caller who can do nothing with it; or, inversely, a one-liner where the developer needs the
  trace.
- **Inconsistent error shape** — this error is a string while siblings return a typed/coded
  error object the caller already handles; now they need two code paths.

## 5. Discoverability & docs (for *new or changed* public surface only)

- **Undocumented public surface** — a new exported function/type/endpoint/flag with no
  doc-comment, no schema description, no `--help` line.
- **Example that won't run** — a snippet missing an import, using placeholder values that
  don't work, or omitting required setup (auth, config). "Hello world is a lie" — the example
  should copy-paste-run, including the real auth/error handling a caller actually needs.
- **`--help` that doesn't help** — flags listed with no explanation of what they do, what the
  default is, or which are required; no usage example.
- **Buried capability** — a genuinely useful/"magical" feature placed where no one will find
  it (no mention in README/quickstart, no discoverable entry point).
- **Stale doc on a changed surface** — the change altered behavior/signature but the
  doc-comment/README/example still describes the old one (now actively misleading).

## 6. Migration & onboarding friction

- **Breaking change, no path** — changed/removed a public signature, flag, env var, response
  field, or status code with no deprecation, no alias, no migration note. Existing callers
  break on upgrade with no guidance.
- **Silent new requirement** — added a now-required env var / config key / setup step without
  surfacing it; the upgrading developer's first sign is a runtime failure.
- **Rename with no alias** — renamed a public thing and deleted the old name in one step; a
  deprecation window (old name → warning → removal) would cost the author little and save every
  caller a scramble.
- **TTHW regression** — the change makes "time to first working call" longer: more steps, more
  config, more concepts to learn before the first success (`dx-principles.md`).
- **Upgrade fear** — a change to a widely-depended-on surface with no changelog entry, so
  callers can't tell if upgrading is safe. Upgrades should be *boring*.

## 7. Consistency of the surface

- **Divergent shape** — this endpoint/function returns a different envelope/error/pagination
  style than its siblings; the caller can't reuse what they learned next door.
- **Mixed conventions** — positional args here, options object there; `snake_case` JSON here,
  `camelCase` there; `--no-foo` here, `--foo=false` there. Pick the house style.
- **Reinvented surface** — a new bespoke way to do something the codebase already exposes a
  standard way to do (config loading, auth, error wrapping); now callers learn two.
- **Asymmetry** — a `create` with no matching `delete`, an `encode` whose `decode` takes
  different args, a setter without a getter. Pairs should mirror.
