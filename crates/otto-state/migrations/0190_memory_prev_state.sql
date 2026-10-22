-- Memory forget/undo (data-integrity-3-01): the lifecycle state a memory had
-- when it was soft-forgotten, so Undo restores it (a pending `suggested`
-- memory stays pending, a `contradicted` one stays superseded) instead of
-- always `accepted`. Nullable, additive: NULL (rows forgotten by an older
-- build) restores as `accepted`, the previous behaviour.
ALTER TABLE memories ADD COLUMN prev_state TEXT;
