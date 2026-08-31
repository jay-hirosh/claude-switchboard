-- v14 -> v15: add device_id to session_events and session_compactions so a
-- "local only" vs "all synced devices" filter can be applied to dashboard
-- queries. Same provenance concept 0014 added to transcript_lines/
-- file_snapshots, extended to the derived (parsed) tables the report UI
-- actually reads. Neither table's existing UNIQUE constraint (event_id /
-- uuid) needs to change — both are already globally unique identifiers that
-- don't collide across devices, so this is a plain ADD COLUMN on both.
ALTER TABLE session_events ADD COLUMN device_id TEXT NOT NULL DEFAULT '';
ALTER TABLE session_compactions ADD COLUMN device_id TEXT NOT NULL DEFAULT '';
