# Review lenses — what bad structure looks like in each pass

Run one pass per lens. Within a pass, keep only that lens active and walk the change against
its surrounding modules. This list is a prompt, not a cage — a real structural problem
outside the list still counts. Vocabulary (module, interface, seam, depth, coupling,
cohesion) is in `design-vocabulary.md`.

For every hit, you must be able to name a **future change it makes more expensive**. If you
can't, it isn't a finding.

## 1. Responsibility & cohesion
- A function/class/module that mixes layers: parsing + I/O + business rules + formatting in
  one body.
- The unit you can't name precisely *because* it does several jobs ("handle", "process",
  "manager", "util", "helper" that grows without limit).
- A class where different methods touch disjoint sets of fields — two classes wearing one
  name (low cohesion).
- A change that bolted a second, unrelated responsibility onto an existing unit instead of
  adding a new one.
- A "god" module everything imports because it accreted everyone's helpers.

## 2. Coupling & dependency direction
- A low-level module importing a high-level one; the dependency arrow pointing toward
  volatility instead of away from it.
- A domain/core type that knows about transport (HTTP, the DB driver, the queue, the
  framework) — the detail leaked inward.
- A change here that *forces* edits in N callers because they depend on internals, not the
  interface (shotgun surgery).
- An import cycle, or two modules that reach into each other's internals.
- "Feature envy": a method that operates mostly on another object's data — it belongs over
  there.
- Passing a fat object just to use one field (over-broad parameter coupling).

## 3. Abstraction — present, absent, or wrong
- **Leaky**: the interface hides nothing — the caller still has to know the internals, the
  call order, or the side effects to use it correctly.
- **Shallow wrapper**: a module/method that only forwards to another with no added behaviour
  (fails the deletion test — complexity just moves to the caller).
- **Premature**: an interface/port/base class with exactly one implementation and no second
  variant in sight — indirection taxing every reader for unused flexibility.
- **Missing**: two real variants already exist (prod + test, or two providers) handled by
  branching/duplication where a single seam belongs.
- **Wrong**: an abstraction that couples things which vary independently — the join is now
  the thing fighting every change.

## 4. Boundaries & layering
- Business rules in the controller/handler; SQL or the ORM in the HTTP layer; validation
  smeared across three layers instead of owned by one.
- Logic on the wrong side of an existing seam (formatting in the store, persistence in the
  domain object).
- A module reaching past another's public interface into its internals (breaking
  encapsulation).
- A new public surface that exposes internals it shouldn't — once callers depend on it, it's
  expensive to retract.
- Cross-cutting concern (auth, logging, tx, retries) hand-rolled inline where the codebase
  has an established place for it.

## 5. Size & shape
- A function/method too long to hold in your head; a class with far more methods than its
  siblings; a file that has become a junk drawer.
- A parameter list long enough that it should be a type (and several call sites pass the
  same cluster).
- Deep nesting / arrow code where early-return or extraction would flatten it.
- A `switch`/`if-else` chain on a type tag that grows with every feature, where the variants
  should carry their own behaviour (polymorphism / a map of strategies).
- Boolean parameters that fork the function into two functions wearing one name.

## 6. Duplication & the shared unit
- The same logic copy-pasted into a second (or third) place — it *will* drift; a bug fixed
  in one won't reach the other.
- A constant, shape, or piece of domain knowledge redefined instead of referenced.
- A third near-identical handler/repo/case that finally reveals the pattern worth
  extracting (the rule of three).
- **The inverse — wrong DRY**: two blocks that only *look* alike (same shape today, different
  reasons to change) being forced under one abstraction. Flag the forced join as the more
  expensive mistake.

## 7. Naming & intent
- A name that lies after a behaviour change (`getUser` that now also writes), or that hides
  intent (`data`, `info`, `temp`, `flag`, `doStuff`).
- A generic name where the codebase has a domain word for the concept (use the ubiquitous
  language).
- Stringly-typed state / magic strings / a bag of booleans where an enum or a small type
  models the real states.
- Inconsistent naming for the same concept across the change (and against its siblings).
- *Not a finding*: a name you'd merely have spelled differently. Only names that **mislead**
  or **hide intent** count.

## 8. Consistency with this codebase
- A new pattern where an established one already exists: error handling, config loading,
  dependency injection, logging, the repository/handler/store shape.
- A parallel structure that diverges from its siblings for no stated reason — the reader now
  has to learn two ways.
- Reinventing a helper/utility/abstraction that already lives in the tree.
- A folder/layout/naming scheme that breaks the project's convention.
- Contradicting a recorded decision (an ADR, a documented convention). Surface only when the
  friction is real enough to reopen it — and say which decision and why it's worth revisiting.
