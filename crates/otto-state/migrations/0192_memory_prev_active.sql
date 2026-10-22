-- Memory forget/undo (backend-correctness-4-01): the `active` flag a memory had
-- when it was soft-forgotten, next to `prev_state` (0190). `active` is set
-- independently of `state` (DELETE /memories/{id}, PATCH {"active": false}),
-- so an accepted-but-deactivated memory must come back deactivated, not into
-- recall. Nullable, additive: NULL (rows forgotten by an older build) falls
-- back to deriving `active` from the restored state, the previous behaviour.
ALTER TABLE memories ADD COLUMN prev_active INTEGER;
