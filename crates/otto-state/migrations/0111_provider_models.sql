-- Dynamic model catalog (personal-agents design §3): per-provider model lists
-- discovered at runtime (CLI probe / docs scrape / models.dev fallback) plus
-- user-added manual entries — no hardcoded lists, no API keys.
--
--   * `source` records where a row came from: 'cli' | 'scrape' | 'catalog'
--     (models.dev) | 'manual'. A refresh replaces only the NON-manual rows of a
--     provider, and only on a successful fetch — a failed fetch never wipes the
--     last good list (the repo enforces this; `fetched_at` staleness is
--     surfaced to the UI instead).
--   * Order of appearance matters (docs list newest/most-relevant first), so
--     readers ORDER BY rowid — the implicit rowid preserves insert order.

CREATE TABLE provider_models (
    provider   TEXT NOT NULL,
    model_id   TEXT NOT NULL,
    label      TEXT NOT NULL DEFAULT '',
    notes      TEXT NOT NULL DEFAULT '',
    source     TEXT NOT NULL CHECK (source IN ('cli', 'scrape', 'catalog', 'manual')),
    fetched_at TEXT NOT NULL,
    PRIMARY KEY (provider, model_id)
);
