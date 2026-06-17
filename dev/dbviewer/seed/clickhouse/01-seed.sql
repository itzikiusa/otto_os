-- Seed the Otto DB Explorer dev ClickHouse instance.
-- Runs once on first container start (docker-entrypoint-initdb.d).
-- INSERT VALUES kept on a single line each (the init runner splits on ';').

CREATE DATABASE IF NOT EXISTS analytics;

CREATE TABLE IF NOT EXISTS analytics.events (event_id UInt64, event_type LowCardinality(String), user_id UInt32, path String, ts DateTime, revenue_cents UInt64 DEFAULT 0) ENGINE = MergeTree ORDER BY (event_type, ts);

INSERT INTO analytics.events (event_id, event_type, user_id, path, ts, revenue_cents) VALUES (1, 'page_view', 1, '/', '2026-06-15 10:00:00', 0), (2, 'add_to_cart', 1, '/p/SKU-1', '2026-06-15 10:02:00', 0), (3, 'checkout', 1, '/checkout', '2026-06-15 10:05:00', 14400), (4, 'page_view', 3, '/', '2026-06-15 11:00:00', 0), (5, 'checkout', 3, '/checkout', '2026-06-15 11:03:00', 1500);

CREATE TABLE IF NOT EXISTS analytics.daily_sales (day Date, orders UInt32, revenue_cents UInt64) ENGINE = SummingMergeTree ORDER BY day;

INSERT INTO analytics.daily_sales VALUES ('2026-06-14', 3, 56300), ('2026-06-15', 2, 15900);

CREATE VIEW IF NOT EXISTS analytics.revenue_by_type AS SELECT event_type, sum(revenue_cents) AS revenue FROM analytics.events GROUP BY event_type;
