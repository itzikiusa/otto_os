# Plan Template

Model your output on this filled example. It is intentionally small, ordered, and
TDD-flavored where behavior is testable. Notice: every task has a Goal, ~3–6 checkbox
steps emitted as `- [ ]`, and a concrete Verify. No speculative work.

---

## Template shape

```
### Task N: <short, outcome-oriented title>

**Goal:** <what this achieves; "Depends on Task X" if applicable>

- [ ] <concrete step>
- [ ] <concrete step — failing test first when doing TDD>
- [ ] <concrete step>

**Verify:** <exact command or observable result proving the task is done>
```

---

## Filled example — "Let a PO bookmark a story"

**Story context (refined):** A Product Owner can bookmark a story so it appears in a
"Bookmarked" filter. Bookmarks are per-user. Toggling is instant and persists.

---

### Task 1: Persist bookmarks in the database

**Goal:** Add storage for per-user story bookmarks so a toggle survives a reload.

- [ ] Write a failing repo test: bookmarking a story then listing returns it; unbookmarking removes it
- [ ] Add a `story_bookmarks` table (`user_id`, `story_id`, `created_at`) via migration
- [ ] Implement `set_bookmark(user, story, on)` and `list_bookmarks(user)`
- [ ] Make the repo test pass

**Verify:** `cargo test -p otto-state bookmarks::` passes.

### Task 2: Expose toggle + list endpoints

**Goal:** Add HTTP routes so the UI can toggle and read bookmarks. Depends on Task 1.

- [ ] Add `POST /stories/{id}/bookmark` (Editor) calling `set_bookmark`
- [ ] Add `GET /bookmarks` (Viewer) calling `list_bookmarks`
- [ ] Add a handler test asserting toggle then list reflects the change

**Verify:** the handler test passes; `curl` toggle then list shows the story.

### Task 3: Bookmark control + filter in the UI

**Goal:** Let the PO toggle a bookmark and filter to bookmarked stories. Depends on Task 2.

- [ ] Add a store method `toggleBookmark(id)` posting to the endpoint and updating state
- [ ] Add a star control on each story row that calls it (optimistic update)
- [ ] Add a "Bookmarked" filter that lists only bookmarked stories

**Verify:** clicking the star persists across reload; the filter shows only bookmarked stories.

### Task 4: End-to-end check

**Goal:** Confirm the whole flow works together. Depends on Tasks 1–3.

- [ ] Bookmark two stories, reload, confirm both stay starred
- [ ] Apply the filter and confirm only those two appear
- [ ] Unbookmark one and confirm it leaves the filter immediately

**Verify:** all three manual checks pass; no console or server errors.

---

Keep your real plan to as many tasks as the story genuinely needs — usually 3 to 8.
Resist padding it with tasks the story did not ask for.
