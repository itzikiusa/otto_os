# Red Flags — Anti-Pattern Catalogue

When you spot one of these patterns, name it explicitly in the gaps section of
your overview. Cite the specific text that triggered it. Use the before/after
examples below to illustrate the fix.

---

## 1. The Vague Summary

**Pattern:** The story title or description uses abstract nouns or general verbs
("improve," "enhance," "support," "optimize") without saying what specifically
changes for whom.

**Before (weak):**
> Improve the reporting experience for back-office users.

**After (strong):**
> Back-office settlement analysts currently export the transaction table to Excel
> to apply date filters. This story adds a date-range filter directly in the UI
> so analysts can complete their daily reconciliation without leaving the app.

**How to spot it:** If you could apply the summary to three different features
without changing a word, it is too vague.

---

## 2. Missing or Unmeasurable Outcome

**Pattern:** The *why* is absent or written as a platitude ("deliver business
value," "align with strategy"). There is no success metric, no before/after
comparison, and no stated pain.

**Before (weak):**
> As a user, I want a dark mode so that the app looks better.

**After (strong):**
> As a user working late-night audit shifts, I want a dark mode so that screen
> glare does not cause eye strain during 4-hour sessions. Success: user
> preference is persisted and 80%+ of surveyed power users report the option
> meets their need.

**How to spot it:** Ask "how would we know this succeeded?" If there is no
plausible answer in the story, the outcome is missing.

---

## 3. The Phantom Persona

**Pattern:** The story says "users" or "admins" without specifying which role,
what workflow they are performing, or how frequently. Engineering builds for an
imagined user.

**Before (weak):**
> Users need to be able to see transaction history.

**After (strong):**
> Casino operations managers running end-of-day reconciliation need to view
> the full transaction history for a player within a date range, filtered by
> transaction type, without opening a separate reporting tool.

**How to spot it:** Replace "users" with "Bob, who is [specific role] doing
[specific task]." If that sentence would change the story meaningfully, the
persona is under-defined.

---

## 4. Untestable Acceptance Criteria

**Pattern:** Criteria use subjective language ("should be fast," "clean UI,"
"easy to use") or describe requirements rather than observable outcomes
("the system must store the preference").

**Before (weak):**
> - The page should load quickly.
> - The filter should be intuitive.
> - Data should be accurate.

**After (strong):**
> - The filtered results render within 1 second for datasets up to 10,000 rows
>   on a standard connection.
> - First-time users can apply a date filter without documentation (validated
>   by 5/5 usability participants completing the task in under 60 seconds).
> - The totals on the summary row match the sum of the displayed rows in all
>   tested scenarios, including partial-day filters.

**How to spot it:** For each criterion, ask "could two people independently test
this and agree on pass/fail?" If the answer is "it depends on their judgment,"
the criterion is untestable.

---

## 5. Scope Without Boundaries

**Pattern:** The story describes what to build but never states what is out of
scope. This leaves engineers filling in the blanks — usually in the direction of
building more than intended.

**Before (weak):**
> Add the ability for users to export their data.

**After (strong):**
> Add CSV export of the filtered transaction table.
>
> **Out of scope for this story:** Excel/PDF export, scheduled exports, bulk
> export of all players, export of data outside the current filter selection.

**How to spot it:** Try to write the out-of-scope section yourself. If you can
easily name five things that "might be" in scope, the story hasn't defined its
boundaries.

---

## 6. Hidden Assumptions

**Pattern:** The story assumes a state of the world that may not be true —
a data field exists, a service is live, a team has already done their work —
without stating these as dependencies or open questions.

**Before (weak):**
> Show the player's preferred language in the profile header.

**Hidden assumption:** "Preferred language is already stored per player in the
database."

**After (strong):**
> Show the player's preferred language in the profile header.
>
> **Assumption (unverified):** Player preferred language is stored in
> `MdlGm_tblPlayers.PreferredLanguage`. Needs confirmation from the data team
> before sprint planning. If the field does not exist, a data migration story
> must precede this.

**How to spot it:** For each data element or integration the story mentions,
ask "do we know for certain this exists today?" If the answer is "I think so,"
surface it as an assumption.

---

## 7. Undefined Terms

**Pattern:** The story uses domain-specific terms, product-specific names, or
internal acronyms without defining them. What "bonus balance," "settlement
status," or "active session" means is not obvious from the story.

**Before (weak):**
> Display the player's net position on the dashboard.

**Undefined:** "net position" — is this deposits minus withdrawals? Balance minus
bonus? A real-money only figure?

**After (strong):**
> Display the player's net position on the dashboard.
>
> **Definition:** Net position = total deposits minus total withdrawals (real
> money only; excludes bonus). This matches the definition used in the Finance
> reporting module.

**How to spot it:** Read the story as a new team member on their first week.
Every term you would need to ask about is an undefined term.

---

## 8. The Hidden Epic

**Pattern:** The story is too large to be a single story — it actually describes
a full user journey, multiple personas, or a feature area that would take many
sprints. It passes as a story because it is written in story format.

**Before (weak):**
> As a player, I want to manage my account so that I can update my details,
> set my communication preferences, change my password, and close my account.

**After (strong):**
> This is an epic. Suggested stories:
> 1. Update personal details (name, address, date of birth)
> 2. Manage communication preferences (email, SMS, push)
> 3. Change password with re-authentication
> 4. Request account closure with cool-off period

**How to spot it:** Count the number of distinct user goals. More than one
goal = candidate for splitting. If the story contains "and" in the value
statement, look harder.

---

## 9. Missing Error and Edge Cases in AC

**Pattern:** Acceptance criteria cover only the happy path. No error states,
empty states, permission boundaries, or data extremes are specified. Engineers
make their own decisions about these — which may not match user expectations.

**Before (weak):**
> - User can search for a player by email.
> - Results display in a table.

**Missing:** What happens when no results match? When the email format is invalid?
When the user lacks permission to view certain players? When there are 10,000 results?

**After (strong):**
> - User can search for a player by email; results display within 1s for up to 1,000 matches.
> - If no player matches the email, display "No players found. Try a different email."
> - If the email format is invalid (missing @), show inline validation before submission.
> - If the user's role restricts visibility, matching players they cannot access are excluded silently.
> - If more than 100 results match, display the first 100 with a "showing first 100 results — refine your search" notice.

**How to spot it:** After reading the AC, ask: what happens when the input is
wrong, the data is empty, the service is slow, or the user lacks permission?
If none of the criteria address these, edge cases are missing.
