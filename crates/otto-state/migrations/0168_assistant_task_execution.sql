-- A task controls only the thread operation that created/resumed it.
ALTER TABLE assistant_tasks ADD COLUMN thread_execution_id TEXT;
