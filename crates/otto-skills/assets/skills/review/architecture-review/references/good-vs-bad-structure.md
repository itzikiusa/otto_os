# Good vs bad structure — worked examples

Concrete before/after pairs for the lenses in `review-lenses.md`. Each shows the smell, the
future cost, and the deepening — so you can recognize the shape and write a finding that
proposes the *right* move, not a rewrite. Vocabulary is in `design-vocabulary.md`.

The point of every "after" is **depth**: more behaviour behind a smaller interface, change
concentrated in one place (locality), the caller learning less. Don't praise an "after" that
is merely more files — praise one where the interface got *narrower* over real behaviour.

---

## 1. Shallow wrapper — fails the deletion test

```ts
// BEFORE — a "service" that only forwards. Delete it and nothing concentrates;
// the complexity just moves to the caller unchanged.
class UserService {
  constructor(private repo: UserRepo) {}
  getUser(id) { return this.repo.findById(id); }
  saveUser(u)  { return this.repo.save(u); }
}
```
**Future cost:** every reader learns two interfaces to reach one behaviour; the layer
implies a seam that isn't real, so people add to it instead of to the repo, and it slowly
accretes pass-throughs.
**Refactor direction:** delete the wrapper; call the repo directly — *until* there's real
orchestration (validation, events, a second data source) to make the module deep. Only then
does the seam earn its keep.

---

## 2. Leaky abstraction — the interface hides nothing

```ts
// BEFORE — caller must know the open/use/close order AND that step() mutates.
const conn = pool.acquire();
conn.begin();
try { conn.step(stmt); conn.commit(); } finally { conn.release(); }
```
```ts
// AFTER — a deep interface: one call, the protocol hidden behind the seam.
await db.inTransaction(tx => tx.run(stmt));
```
**Future cost (before):** every call site re-implements the protocol; one that forgets
`release()` leaks, and the knowledge lives in N places (no locality).
**Refactor direction:** move the acquire/begin/commit/release protocol *inside* the module;
expose `inTransaction`. The caller learns one method, not a five-step ritual.

---

## 3. Premature abstraction — one adapter, hypothetical seam

```ts
// BEFORE — an interface + factory with exactly one implementation and no second in sight.
interface PricingStrategy { price(cart): Money }
class DefaultPricingStrategy implements PricingStrategy { /* the only one */ }
const strategy = PricingStrategyFactory.create();
```
**Future cost:** every reader chases an indirection to find the one real implementation; the
flexibility is unused, the tax is permanent.
**Refactor direction:** inline to a single `price(cart)` function/module. Introduce the
interface when a **second** real variant appears — *two adapters justify a seam; one does
not.* This is the wrong-abstraction risk in reverse: don't pay for flexibility nobody uses.

---

## 4. Missing seam — two variants already branching

```ts
// BEFORE — the "second adapter" already exists, smuggled in as a branch.
function send(msg) {
  if (process.env.NODE_ENV === 'test') { sentMessages.push(msg); return; }
  smtp.send(msg);
}
```
**Future cost:** the test path and the prod path are tangled in one body; a change to either
risks the other, and the test fake can't be reused. Tests reach *past* the interface (env
var) to control behaviour — the seam is in the wrong place.
**Refactor direction:** define a `Mailer` port at the seam; an SMTP adapter for prod and an
in-memory adapter for tests. Now *two real adapters* justify the interface — and the
interface is the test surface.

---

## 5. Mixed responsibility — low cohesion in one body

```ts
// BEFORE — parse + validate + business rule + persist + format, all in the handler.
async function handleSignup(req, res) {
  const body = JSON.parse(req.body);
  if (!body.email.includes('@')) return res.status(400)...;
  const discount = body.referral ? 0.1 : 0;          // business rule
  await db.query('INSERT INTO users ...', [...]);     // persistence
  res.json({ welcome: `Hi ${body.name}` });           // formatting
}
```
**Future cost:** you can't change the discount rule, the storage, or the response shape
without re-reading the whole function and risking the others; it can't be unit-tested
without HTTP + a DB; the next handler copies the tangle.
**Refactor direction:** the handler orchestrates; push validation to a parser, the rule into
a `Signup` domain module, persistence behind a repo. Each becomes deep and testable through
its own interface. (This is a *move*, not a rewrite — same behaviour, better seams.)

---

## 6. Wrong dependency direction — detail leaked inward

```ts
// BEFORE — the domain type imports the web framework and the DB row shape.
import { Request } from 'express';
class Order { constructor(public req: Request, public row: DbRow) {} }
```
**Future cost:** the core can't be used off the web path, can't be tested without Express,
and a DB-schema change ripples into business logic. Every new caller inherits the coupling.
**Refactor direction:** invert it — the domain `Order` depends on nothing outward; the web
and persistence layers depend on *it* and map to/from their own shapes at the seam. The
arrow points away from volatility.

---

## 7. Type-tag switch that grows with every feature

```ts
// BEFORE — every new shape edits this function (and usually three others like it).
function area(s) {
  switch (s.kind) {
    case 'circle': return Math.PI * s.r ** 2;
    case 'rect':   return s.w * s.h;
    // ...add a case here every time, forever
  }
}
```
**Future cost:** adding a variant means editing N switch statements scattered across the
codebase; miss one and you ship a silent gap. Open/closed violated.
**Refactor direction:** give each variant its own `area()` (polymorphism, or a map of
`kind → fn`). New variants *add* a unit instead of *editing* every switch. Note the
trade-off: if there's genuinely one switch and variants are stable, leave it — don't pay for
extensibility you won't use.

---

## 8. Premature DRY — the wrong shared unit

```ts
// BEFORE — two blocks that look alike today get forced under one helper...
function format(x, kind) {
  const base = x.toFixed(2);
  return kind === 'invoice' ? `$${base}` : `${base}%`;  // they vary for different reasons
}
```
**Future cost:** invoice formatting and percentage formatting change for *independent*
reasons; the shared `format` now fights both — every change to one risks the other, and the
`kind` flag multiplies. The coupling is more expensive than the duplication it removed.
**Refactor direction:** split back into `formatMoney` and `formatPercent`. **DRY is about a
single source of *knowledge*, not similar-looking lines.** Duplicate until a *third* case
reveals the real shared concept (rule of three).

---

## Classifying dependencies before you propose a seam

When a finding says "put a seam here," classify what's behind it — the category decides
whether a seam is even the right move and how it gets tested (adapted from ports & adapters):

| Dependency | What it is | Seam guidance |
|---|---|---|
| **In-process** | Pure computation, in-memory state, no I/O | No seam needed — merge and test through the interface directly. A port here is pure indirection. |
| **Local-substitutable** | Has a real local stand-in (in-memory DB/fs, fake clock) | Use an *internal* seam; test with the stand-in. Don't expose it on the public interface. |
| **Remote but owned** | Your own services across a network | A port + two adapters (HTTP for prod, in-memory for tests) is justified. |
| **True external** | Third-party you don't control (Stripe, Twilio) | Inject a port; mock adapter in tests. The seam protects you from their interface. |

The rule stays the same across all four: **one adapter is a hypothetical seam; two make it
real.** If you can't name the second adapter, don't ask for the port.

## When the design is genuinely contested — design it twice

For a *significant* structural finding (a new public module, a seam you're unsure about),
the strongest move is not to assert one answer but to **design the interface two or three
radically different ways and compare them** — because the first idea is rarely the best:

- **Minimal**: fewest entry points, maximum leverage per call.
- **Flexible**: supports many use cases and future extension.
- **Common-case-first**: the default path is trivial; advanced use costs more.

Then compare on **depth** (leverage at the interface), **locality** (where change
concentrates), and **seam placement**. Recommend one, or a hybrid — opinionated, not a menu.
In a review, you usually present this as *"here are two shapes this could take, and why I'd
pick B"* rather than a single decree. Reserve it for findings where the seam is load-bearing
and the right shape isn't obvious; for everyday smells, just point at the move.
