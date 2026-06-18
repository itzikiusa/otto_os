-- Add tags column to product_stories (comma-separated list, e.g. "auth,payments,mvp")
ALTER TABLE product_stories ADD COLUMN tags TEXT NOT NULL DEFAULT '';
