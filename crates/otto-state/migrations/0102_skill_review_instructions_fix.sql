-- Skills Lab reviews: user-supplied extra reviewer instructions + the
-- apply-fixes agent.
--
-- `instructions` is free-form text appended to every reviewer/summarizer
-- prompt (e.g. "check recent commits — they fix the previous review round").
-- `fix_json` holds the single apply-fixes agent row (same shape as one
-- agents_json element): spawned on demand after a review completes to apply
-- the patch plan to the real skill directory.

ALTER TABLE skill_reviews ADD COLUMN instructions TEXT NOT NULL DEFAULT '';
ALTER TABLE skill_reviews ADD COLUMN fix_json TEXT;
