# RFC Anti-Patterns — Smells and Fixes

These are the most common ways RFCs fail to support good decisions. For each pattern:
a description of the smell, why it harms the reader, and a concrete fix.

Flag an anti-pattern during review only when it **genuinely hinders the decision** — do
not enforce every item as a checklist. Use judgment.

---

## 1. No Real Alternatives

**The smell:**
The options section lists one real option and one or two options that are obviously
unworkable — included to create the appearance of deliberation without the reality.
Sometimes the "alternatives" section is simply missing.

**Why it harms:**
Readers who disagree with the recommendation have no place to engage. Future readers
cannot understand what was considered. The decision feels predetermined.

**Fix:**
Include at least two genuine alternatives that a reasonable person could advocate for.
If a third option was considered and quickly dismissed, a one-sentence explanation of why
it was ruled out is sufficient. "Do nothing" counts as a real option when inaction is
genuinely on the table.

---

## 2. Buried Decision

**The smell:**
The recommendation appears at the end of a long document, after pages of analysis, often
hedged with language like "given the above considerations, it seems reasonable to
conclude that Option B may be the most appropriate path forward."

**Why it harms:**
Decision-makers who skim (all of them) miss or misread the recommendation. Authors who
bury the decision often do not fully believe it themselves. The RFC cannot be acted upon
without a re-read.

**Fix:**
State the recommendation in the first sentence of the Proposed Decision section, not the
last. Use a decision marker if the document is long. "We recommend X" is stronger than
"X appears to be the preferred approach."

---

## 3. Missing Non-Goals

**The smell:**
The Goals section lists what the proposal achieves. There is no Non-Goals section, or it
is a placeholder ("out of scope: everything not listed above").

**Why it harms:**
Without explicit non-goals, every reader imagines a different scope. Some will assume the
proposal covers things it does not; others will ask for additions that the author never
intended. Discussions become long and repetitive.

**Fix:**
List the things a reader would *reasonably expect* to be in scope but are not. Give a
brief rationale or a pointer to future work. Non-goals that surprise readers (things they
assumed were included) are the most valuable ones to write down.

---

## 4. Hand-Wavy Rollout

**The smell:**
The rollout section says something like "we will roll this out gradually" or "we will
monitor the impact" without specifying what gradually means, what will be monitored, or
what threshold triggers a rollback.

**Why it harms:**
Reviewers cannot evaluate whether the rollout plan is realistic. Engineering cannot
execute it. If something goes wrong, there is no agreed signal for when to stop.

**Fix:**
Name the phases (if any), the population or percentage in each phase, the metric or
signal being watched, the threshold for proceeding vs. rolling back, and who owns the
go/no-go decision. If backout is hard or impossible, say so explicitly — this is critical
information.

---

## 5. Problem Statement in Solution Language

**The smell:**
The problem statement describes what will be built rather than what is wrong. "We need a
notification deduplication service" is a solution, not a problem.

**Why it harms:**
Anchoring the problem to a specific solution closes off alternatives before the options
section. Readers who think a different solution would work have no foundation to argue
from.

**Fix:**
Describe the user or system behavior that is broken, the consequence of that breakage,
and why it matters — without naming a solution. "On-call engineers receive duplicate
alerts from three systems during incidents, spending 20 min/day triaging noise. This
contributed to five missed SLA breaches in Q1."

---

## 6. Goals Written as Features

**The smell:**
Goals are a list of things that will be built ("build a deduplication layer", "add an
admin dashboard") rather than outcomes the proposal aims to achieve.

**Why it harms:**
Feature goals conflate means and ends. They make it impossible to know whether the work
succeeded. They also invite scope creep — if the goal is "build X," any addition to X
sounds relevant.

**Fix:**
Write goals as observable outcomes: "On-call engineers receive a single alert per
incident." If a feature is the only way to achieve the outcome, the feature can appear in
the rollout section, not the goals.

---

## 7. False Certainty in Open Questions

**The smell:**
The RFC has no Open Questions section, or the section lists only trivial implementation
details while real uncertainties go unacknowledged.

**Why it harms:**
Reviewers encounter unstated assumptions and either silently accept them or relitigate
them in comments. The RFC appears more decided than it is. Stakeholders who had input to
give were not asked.

**Fix:**
List every question where the answer could change the decision or the rollout plan.
Name who should answer each question and whether it blocks approval or can be resolved
during implementation.

---

## 8. Options with Asymmetric Depth

**The smell:**
The recommended option gets three detailed paragraphs of analysis. The alternatives get
one sentence each.

**Why it harms:**
Readers infer (often correctly) that the alternatives were not seriously considered. The
analysis reads as advocacy rather than deliberation. Trust in the recommendation drops.

**Fix:**
Give each option roughly equal treatment. If the recommended option genuinely has more
to say, a brief note ("Option A was the runner-up; see the appendix for full analysis")
is more credible than lopsided coverage.

---

## 9. Scope Creep via Implicit Goals

**The smell:**
The goals section is short but the rollout section quietly includes capabilities,
integrations, or work that were not named as goals.

**Why it harms:**
Reviewers approve a small proposal but discover in implementation that the actual scope
was much larger. Timelines slip, teams are surprised, trust erodes.

**Fix:**
Every deliverable in the rollout section should map back to a named goal or non-goal.
If something shows up in rollout but not in goals, either add it to goals (if it is truly
in scope) or move it to a follow-on (if it is not).
