# DX principles — the standard you judge against

These are the laws behind the hunt list. Every finding should trace back to one of them.
Internalize them; don't enumerate them at the author. Use them to decide *whether* something
is friction and *how bad*, and to phrase the improvement.

## The pit of success

> "We want customers to simply fall into winning practices." — Rico Mariani

Make the right thing the easy thing and the wrong thing hard (ideally impossible). A great
surface is one a tired developer uses *correctly by default*, without reading the docs. When
you find a footgun, the fix is rarely "document it" — it's "redesign so the mistake can't
happen": replace two same-typed positional params with named/typed ones, replace a stringly
field with an enum, replace a leakable handle with a scoped helper, flip an unsafe default.

A footgun a caller will hit silently (no error, wrong result) is the **blocker** of this lens —
it's the API equivalent of shipping a bug, except *every* caller ships it.

## Easy to use right, hard to use wrong

The two halves are separate tests, both required:

- **Easy to use right** — the common case is short and obvious; the types/signature guide the
  caller to a correct call without thinking.
- **Hard to use wrong** — the incorrect call doesn't compile, or is visibly wrong at the call
  site. Same-typed adjacent params, order-dependence with no guard, and footgun defaults all
  fail this half: the wrong call looks exactly like the right one.

## The error-message three-tier model

Every error a developer reads is a moment of pain. Judge each emitted error/exception/log
against three tiers — a good message clears all three:

1. **What went wrong** — the specific failure, in plain language, naming the offending thing.
   ("`config.port` is out of range.")
2. **Why / which input** — the value or condition that caused it. ("got `0`; valid range is
   1–65535.")
3. **What to do next** — the remedy, ideally with a pointer. ("Set a port in 1–65535, e.g.
   `PORT=8080`. See docs/config#port.")

Gold standard: Rust/Elm compiler errors and Stripe's API errors — they tell you the problem,
the cause, and the fix, often with the exact edit. A bare `panic("bad input")` or
`400 invalid` clears *none* of the tiers and strands the developer. Severity scales with how
stuck the reader is left and how often they'll hit it.

## Progressive disclosure

The simple case should be one line and *production-ready* (not a toy); the complex case should
use the *same* API, just with more knobs. SwiftUI's `Button("Save") { save() }` scaling up to
full customization through the same type is the model. Red flags: a "simple" API that can't do
the real thing (so everyone drops to the "advanced" one immediately), or two parallel APIs for
the same task where one is a dead end.

## Opinionated defaults + escape hatches

Strong opinions, loosely held. **Opinionated defaults are a feature** — they let the 95% caller
write nothing. But *every default needs an override*. A default with no escape hatch is a
trust-killer: the first caller who needs the other behavior is stuck forking or hacking. When
you flag a missing default, propose the sane one; when you flag a rigid default, propose the
override.

## Time to first working call (TTHW)

How long from "developer arrives at this surface" to "first correct call returns"? Shorter is
strictly better — it's the single strongest adoption signal. Judge changes by their effect on
it: a new required config step, an extra concept to learn, a non-copy-pasteable example, or a
breaking rename all *lengthen* TTHW. A change that shortens it (a sane default replacing a
required arg, a working quickstart) is a real win worth calling out, not just the absence of a
problem.

## Journey wholeness

DX is the whole arc: discover → evaluate → install → first call → integrate → debug → upgrade.
A change can be locally fine and still create a gap in the journey — great signature, zero
docs (discover/integrate gap); clean new behavior, no migration note (upgrade gap); works in
the demo, useless error when it fails (debug gap). When you review a surface, ask which stage
of the journey this change touches and whether it leaves a hole there.

## Context-switch cost

Every time a developer has to *leave* the surface to make progress — read the source to learn
the valid enum values, ask a teammate which arg order is right, search for what an error means
— you've cost them the thread (and 10–20 minutes). Findings that eliminate an exit (self-
documenting types, an error that contains its own fix, an inline example) are high value;
phrase the improvement as "so the caller never has to leave to figure this out."

## Consistency is ergonomics

A caller learns your conventions once and expects them to transfer. An inconsistent surface
(divergent error shape, mixed arg styles, a sibling that returns a different envelope) forces
them to re-learn per-call. Treat inconsistency with an established local pattern as real
friction, not a style nit — the cost is paid by every caller, every time.

## The seven characteristics of good DX (a checklist of what "great" means)

Use as a coverage check when scoring a surface; not every one applies to every change.

| Characteristic | What it means for the surface you're reviewing |
|---|---|
| **Usable** | Simple to call; intuitive signature; fast, clear feedback. |
| **Credible** | Predictable, consistent, no surprises; safe defaults; clear deprecation. |
| **Findable** | The caller can discover it and get help from within (docs, `--help`, examples). |
| **Useful** | Solves the real problem, not a toy slice of it (show real auth/error handling). |
| **Valuable** | Measurably saves the caller time/steps/lines vs. doing it themselves. |
| **Accessible** | Works for the junior and the principal; sensible for both common and advanced use. |
| **Desirable** | A developer *wants* to use it, not tolerates it — the call reads cleanly. |
