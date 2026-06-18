# Severity & finding shape

## Severity ladder (calibrated to *friction*, not correctness)

Rank every finding by the friction it inflicts on the next developer, not by how much it
offends your taste. A miscalibrated label costs trust both ways: a naming nit marked
*blocker* cries wolf; a silent-transposition footgun marked *nit* ships an interface every
caller will misuse.

| Severity | Definition | Examples |
|---|---|---|
| **blocker** | A footgun that *will* cause real, silent misuse, or a breaking change to a depended-on surface shipped with no migration path. Don't ship the interface as-is. | Two adjacent same-typed params a caller will transpose silently; an unsafe default (`verify=false`) on the easy path; a removed/renamed public field with no deprecation, breaking existing callers on upgrade. |
| **major** | Real, repeated friction that leaves developers stuck or guessing — they'll hit it often. | An error that states the problem but not the field or fix; a new public endpoint/function with no docs or runnable example; a required-vs-optional split every caller fights; a stringly-typed field whose valid set is only in the source. |
| **minor** | Genuine friction, but narrow or low-traffic — an awkward edge few callers reach. | A clunky param order on a rarely-called overload; a slightly-off error on an unlikely path; missing example for an advanced-only flag. |
| **nit** | Small but real. Report it — the brief is to catch friction down to the smallest — but label it so it's easy to skip. | A name one degree off (`getList` vs `listItems`); a doc typo that mildly misleads; a message that could be one word clearer; an inconsistent flag casing. |

When unsure between two levels, state the trigger ("blocker *if* external callers exist;
minor if internal-only") and pick the lower — the author re-ranks with the facts in hand.

## Off-lens findings

You are the devex lens. If a pass surfaces a correctness bug, a race, a leak, a missing index,
or an internal-coupling smell, do **not** write it up as a devex finding. Note it in a single
line under an **"Off-lens (FYI)"** heading — `file:line` + one clause — and move on. Let
grill / correctness-review / architecture-review own it. Exception: an internal-structure
problem that *leaks into the surface the caller touches* (e.g. an internal type bleeding into a
public signature) IS in your lane — review it as ergonomics.

## Evidence bar (every finding must clear it)

1. **Location.** `path/to/file.ext:line`, or the named surface (`POST /v1/orders`, `--retries`
   flag, the `OrderError` shape). No location → not a finding.
2. **The friction, shown — not asserted.** Pick the strongest available:
   - the **awkward call written out** (the actual line a caller types, and the mistake it
     invites),
   - the **bad message quoted verbatim** (and what the reader still doesn't know after it), or
   - the **divergent sibling** placed beside it (so the inconsistency is concrete).
3. **Victim & moment.** *Who* hits this and *when* — the integrator at the call site, the
   on-call reading the log at 3am, the developer upgrading past the rename.
4. **Confidence + how you know.** One of:
   - **confirmed** — you wrote the bad call / quoted the real error / found the real divergence.
   - **likely** — strong read, not demonstrated end to end.
   - **question** — you couldn't verify the convention or the caller set. Phrase it as a
     question, say what you'd need. Never present a *question* as a confirmed footgun.
5. **The improvement.** A concrete better signature / name / message / default / doc — written
   out, adoptable. Not "make this more ergonomic."

## False-positive filter (run before you submit)

For each finding, ask:
- Is this **friction a real caller hits**, or my **personal taste**? If I can't show the
  awkward call, the bad message, or the divergence, it's taste — cut it or downgrade to a
  labelled *suggestion*.
- Is the surface **actually public/consumed**, or private code no other developer touches? If
  private, there's no devex finding — drop it.
- Am I straying into **correctness or internal structure**? If so, move it to "Off-lens".
- Did I **verify the convention**, or pattern-match? If I can't point to the sibling that sets
  the convention, downgrade to *question*.

A short list where every item survives this filter beats a long one that doesn't.

## Finding template

Emit each finding in this shape. One finding, one friction point.

```
### [SEVERITY] <short, specific title>   — `path/to/file.ext:line` (or named surface)

**What:** <the friction, in one or two sentences>
**Friction for:** <who hits it and when — the caller / on-call / upgrader>
**Shown:** <the awkward call written out | the message quoted | the divergent sibling>
**Confidence:** <confirmed | likely | question> — <how you know>
**Improvement:**
```diff
- <the awkward surface as-is>
+ <the better surface>
```
<or a precise instruction if a diff doesn't fit>
```

## Good vs. bad findings

**Good (confirmed footgun — blocker):**

> ### [blocker] `resize` transposes width/height silently — `image/ops.rs:88`
> **What:** `pub fn resize(w: u32, h: u32)` — two adjacent `u32`s, no type distinction.
> **Friction for:** every caller; a swapped call compiles and silently ships a squashed image.
> **Shown:** `resize(1080, 1920)` vs `resize(1920, 1080)` — both compile; nothing flags the
> wrong one. Three of the five existing call sites guess differently.
> **Confidence:** confirmed — read all five call sites; two pass `(h, w)`.
> **Improvement:**
> ```diff
> - pub fn resize(w: u32, h: u32) -> Image
> + pub fn resize(size: Dimensions) -> Image   // Dimensions { width, height } — named, un-transposable
> ```

**Good (vague error — major):**

> ### [major] `parse_config` error names no field — `config/load.rs:142`
> **What:** On any bad field the function returns `Err("invalid config".into())`.
> **Friction for:** the developer whose deploy fails — they get `invalid config` and must
> bisect the file by hand to find which key is wrong.
> **Shown:** quoted output: `Error: invalid config`. Clears tier 1 weakly, tiers 2 and 3 not at
> all (no field, no value, no remedy).
> **Confidence:** confirmed — ran it against a config with `port: 0`.
> **Improvement:** `Err(format!("config field `{field}` is invalid: {reason} (got {value:?})"))`
> → `config field `port` is invalid: must be 1–65535 (got 0)`.

**Bad (taste dressed as a finding — cut it):**

> ~~"I'd prefer this function were called `fetchUsers` instead of `getUsers`."~~ — no friction
> shown, no convention cited. If the codebase consistently uses `fetch*` and this breaks it,
> *that's* the finding (cite the siblings, label it nit). Otherwise it's preference — drop it.

**Bad (off-lens — move it):**

> ~~"This function could deadlock if two threads call it."~~ — that's a correctness/concurrency
> finding. One line under "Off-lens (FYI)", then move on. Not a devex finding.
