# Design vocabulary — the language of a structural review

Use these terms exactly. A design review is only as sharp as its language: "this is too
complex" is a vibe; "this module is **shallow** — its interface is nearly as wide as its
implementation" is a finding. Reach for these words in every finding so the author and the
next reviewer mean the same thing.

## Core terms

**Module** — anything with an interface and an implementation: a function, a class, a file,
a package, a tier-spanning slice. Scale-agnostic on purpose. The unit you are judging the
*shape* of.

**Interface** — *everything a caller must know to use the module correctly.* Not just the
type signature — also its invariants, ordering constraints, error modes, required
configuration, and performance characteristics. When you say an interface is "wide," you
mean a caller has to learn a lot to use it safely.

**Implementation** — the body of code behind the interface; what the caller does *not* have
to know.

**Depth** — leverage at the interface: how much behaviour a caller (or test) gets per unit
of interface they have to learn.
- **Deep module** = small interface, lots of behaviour behind it. Callers learn little, get
  a lot. *This is the goal.*
- **Shallow module** = the interface is nearly as complex as the implementation; the module
  forwards more than it hides. A pass-through wrapper, a one-line "service" that just calls
  the repo, a class whose every method maps to one field. *This is the smell.*

**Seam** *(Michael Feathers)* — a place where behaviour can vary without editing in that
place; the *location* where a module's interface lives. *Where* the seam goes is its own
design decision, separate from *what* sits behind it. Most boundary findings are really
"the seam is in the wrong place" or "there is no seam where two things already vary."

**Adapter** — a concrete thing that satisfies an interface at a seam (the Postgres repo and
the in-memory fake are two adapters at the same seam). Names the *role* a thing plays, not
what's inside it.

**Coupling** — how much one module must know about another's internals. *Low coupling*:
modules interact only through narrow interfaces. *High coupling*: a change here forces edits
there; a module reaches past another's interface into its guts. High coupling is the cost
multiplier behind most "this will be expensive to change" findings.

**Cohesion** — how related the things inside one module are. *High cohesion*: everything in
the unit serves one purpose. *Low cohesion*: a `utils`/`manager`/`helpers` grab-bag of
unrelated functions that happen to live together. Low cohesion is the smell behind most
"this does too much" findings.

**Leverage** — what callers get from depth: more capability per unit of interface learned.
One deep implementation pays back across N call sites and M tests.

**Locality** — what maintainers get from depth: change, bugs, and knowledge concentrate in
*one* place instead of spreading across callers. "Fix once, fixed everywhere." When you
argue a refactor's benefit, argue it in terms of locality and leverage — those are the
concrete payoffs.

## Principles you review against

- **Depth is a property of the interface, not the implementation.** A deep module can be
  internally composed of small, swappable parts — they just aren't part of its interface.
  Don't reward a fat implementation; reward a *narrow* interface over real behaviour.

- **The deletion test.** For any abstraction the change adds (or that you'd add), imagine
  deleting it. If complexity *concentrates* somewhere when it's gone, it earned its keep. If
  complexity just *moves* to the callers unchanged, it was a pass-through — indirection, not
  abstraction. Run this before you praise *or* demand any layer.

- **One adapter means a hypothetical seam. Two adapters means a real one.** Do not introduce
  (or ask for) an interface/port unless something actually varies across it — typically a
  production adapter *and* a test or second-variant adapter. A single-implementation
  interface is indirection that taxes every reader for a flexibility no one is using.

- **The interface is the test surface.** Callers and tests cross the same seam. If a test
  has to reach *past* the interface to set up or assert, the module is probably the wrong
  shape — the seam is in the wrong place. Hard-to-test-through-its-interface is a design
  finding, not just a testing one.

- **Right abstraction beats early abstraction.** The wrong abstraction is more expensive
  than the duplication it replaced, because it couples things that vary independently and is
  costly to back out. Two similar blocks are often cheaper left as two until a third tells
  you the real shape. Prefer "duplicate until the pattern is obvious" over a speculative
  base class.

## Words to avoid (and what to say instead)

| Avoid | Say | Why |
|---|---|---|
| "component" / "service" / "unit" | **module** | One scale-agnostic word; don't smuggle in a tier. |
| "API" / "signature" | **interface** | Those are only the type surface; interface includes invariants, errors, ordering, config. |
| "boundary" | **seam** | "Boundary" is overloaded (DDD bounded context). Seam is precise: where the interface lives. |
| "it's too complex / not clean" | shallow / low-cohesion / high-coupling | Name the specific property so the author can act on it. |
| "add an interface here" | "two adapters justify a seam here" (or don't) | Forces the deletion test before you ask for indirection. |
