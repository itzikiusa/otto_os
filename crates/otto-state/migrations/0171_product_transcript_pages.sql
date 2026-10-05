-- Stable keyset pages for transcript summaries and bounded explicit searches.
CREATE INDEX idx_product_transcripts_page
    ON product_transcripts(story_id, created_at DESC, id DESC);
