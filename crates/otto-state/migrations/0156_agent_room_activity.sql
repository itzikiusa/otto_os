-- Agent rooms perf (R1 + R5).
--
-- R1: the room tail / after-cursor reads order by the table's insertion
-- `rowid`, but the only room index was `(room_id, id)` — ordered by ULID, so
-- every read sorted the whole room in a temp B-tree. An index on `room_id`
-- alone stores its entries as `(room_id, rowid)`: both reads become range
-- scans that stop after `limit` rows.
CREATE INDEX IF NOT EXISTS idx_arm_room_seq ON agent_room_messages(room_id);

-- R5: the rooms list (and the room-name lookups behind the MCP room tools)
-- used to COUNT/MAX every message in the workspace on each call. The activity
-- is now denormalized onto the room row, maintained by `add_message` in the
-- same transaction as the insert (and recounted after retention prunes).
ALTER TABLE agent_rooms ADD COLUMN message_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE agent_rooms ADD COLUMN last_message_at TEXT;

UPDATE agent_rooms
   SET message_count = (SELECT COUNT(*) FROM agent_room_messages m
                         WHERE m.room_id = agent_rooms.id),
       last_message_at = (SELECT MAX(m.created_at) FROM agent_room_messages m
                           WHERE m.room_id = agent_rooms.id);
