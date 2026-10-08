-- Match history's pinned-first ordering so a bounded page does not sort the
-- owner's full history. The id tie-break makes equal timestamps deterministic.
CREATE INDEX idx_assistant_thread_page ON assistant_threads(
    owner_user_id, (space_slot IS NULL), space_slot, updated_at DESC, id DESC
);
