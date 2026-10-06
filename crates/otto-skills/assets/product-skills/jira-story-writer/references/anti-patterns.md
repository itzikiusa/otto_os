# Story Anti-Patterns

Common story smells, why they cause problems, and how to fix them.
Use this catalogue when reviewing a story — name the pattern when you spot it.

---

## 1. Solutioning in the story

**The smell:**
The story describes implementation decisions — specific endpoints, database tables,
technology choices, code structure — rather than user-observable outcomes.

**Examples:**
> "Add a new column `withdrawal_status_v2` to the `transactions` table and backfill
> historical records using a migration script."

> "Implement a React hook that polls `/api/v3/balance` every 10 seconds."

**Why it hurts:**
Engineering loses the ability to choose the best implementation. Requirements become
brittle because they are tied to a specific approach. The story is harder to
estimate because it conflates *what* with *how*.

**Fix:**
Express the requirement as a user or business outcome:
> "Players see their current balance without a page refresh; updates reflect within
> 30 seconds of a balance change."

If a technical constraint is real (a specific integration required, a hard
compliance rule), state it as a constraint, not a design:
> **Constraint:** The solution must use the existing withdrawal webhook events from
> PaymentService v3; polling is not permitted under the current rate limits.

---

## 2. Untestable acceptance criteria

**The smell:**
AC that uses subjective or hedging language, making it impossible to confirm pass/fail.

**Examples:**
> "The experience should feel smooth and intuitive."
> "Performance should be good."
> "The UI should be clean and modern."
> "The system should handle errors gracefully."

**Why it hurts:**
Developers cannot know when they are done. Testers cannot write a test. "Done" becomes
a negotiation, not a verification.

**Fix:**
Replace every subjective phrase with an observable, measurable outcome:
> "The page renders completely within 2 seconds on a standard 4G connection (Chrome,
> no cache)."
> "When a payment fails, the user sees the message 'Payment could not be processed.
> Please try a different method.' — no raw error codes or stack traces are shown."

---

## 3. No user value (infrastructure story disguised as a feature)

**The smell:**
The story describes a technical task with no user-facing outcome. The value statement
is missing or is really a capability statement ("so that the system can do X").

**Examples:**
> "Migrate the player balance service to the new multi-tenant database schema."
> "As a developer, I want to refactor the notification module so that the code is
> easier to maintain."

**Why it hurts:**
The team cannot prioritise it against user-facing work. Stakeholders cannot understand
why it matters. It often hides the real requirement.

**Fix — option A:** Frame it around the user or business outcome the work enables:
> "As a player, I want my balance to update within 5 seconds of a transaction, so
> that I can trust I'm seeing an accurate figure."
> **Context:** The current schema is the blocker; the migration is the work, but this
> is the value it delivers.

**Fix — option B:** If no user outcome exists, keep it as a technical task on an
engineering card. Technical debt and infrastructure work are valid; they just belong
in a different card type, not disguised as a user story.

---

## 4. Scope creep baked in

**The smell:**
The story has grown to cover multiple independent deliverables. The "in scope" section
covers an entire feature area. There is no out-of-scope section, or the out-of-scope
section is empty.

**Examples:**
> "Players can register, verify their email, set notification preferences, complete
> KYC, and make their first deposit."

> (Out of scope): *(blank)*

**Why it hurts:**
The story is not estimable or completable in a sprint. Teams drag it across multiple
sprints, losing velocity and clarity. Definition of Done becomes blurry.

**Fix:**
Split along independently valuable milestones. Each split must satisfy INVEST on its
own. Add an explicit out-of-scope section to every story — even one item signals the
boundary was considered:
> **Out of scope:** Email verification is handled in PROJ-412. KYC flow is a separate
> epic. This story covers only the registration form and account creation.

---

## 5. Ambiguous personas and terms

**The smell:**
The story uses undefined terms or an overly broad persona. Words like "user", "admin",
"the system", "fast", "large", "many" appear without definition.

**Examples:**
> "As a user, I want to see my history…" (Which user? A player? A support agent? A VIP?)
> "The system should handle large volumes of transactions." (What volume? What is large?)
> "Admins can manage player accounts." (Which admin role? All admin actions?)

**Why it hurts:**
Different team members carry different mental models. Edge cases are missed. The story
cannot be adequately tested because the target state is undefined.

**Fix:**
Name the persona specifically. Define any term that could be interpreted more than one way:
> "As a **support agent** (role: Customer Care, read-only access), I want to view a
> player's last 90 days of transaction history…"
> "The feature must handle up to **10,000 concurrent active sessions** without
> degrading response time beyond the threshold in AC-3."

---

## 6. Missing failure paths

**The smell:**
All acceptance criteria describe the happy path. No criteria address what happens
when something goes wrong — bad input, network failure, timeout, insufficient funds,
permission denied.

**Why it hurts:**
Testers discover the error states late. Developers implement them inconsistently.
Users encounter rough edges that never had a specified behaviour.

**Fix:**
For every happy-path criterion, ask: "What if it fails?" Add at least one error-path
criterion per significant action:
> **Happy path:** When the player submits a valid withdrawal request, they see a
> confirmation message and receive a confirmation email.
> **Error path:** When the withdrawal amount exceeds the player's available balance,
> the form shows: "Insufficient balance. Your available balance is [amount]." The
> request is not submitted.

---

## 7. Open questions left implicit

**The smell:**
The story has unresolved decisions buried in the text or, worse, not mentioned at all.
The author has made assumptions that engineering will discover only during development.

**Examples:**
- The story mentions "send a notification" but does not say whether that is email,
  SMS, push, or in-app.
- The story says "display the player's tier" but tier data comes from a service that
  does not yet have an agreed contract.

**Why it hurts:**
Engineering stops mid-development waiting for an answer. Assumptions harden into
accidental requirements. Rework follows.

**Fix:**
Surface every open question in an explicit section. Assign an owner and a resolution
date. Nothing moves to In Progress until open questions are closed:
> **Open questions:**
> - [ ] Which notification channel? (Owner: PM — resolve before sprint start)
> - [ ] Is tier data available via the Player Profile API today, or does this depend
>   on PROJ-389 being shipped first? (Owner: Tech Lead — needs spike)
