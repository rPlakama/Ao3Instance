-- AO3 blurb/search fidelity: fields the work page already exposes.
-- Nullable on purpose: rows cached before this migration fall back to
-- fetched_at and unknown status until their next refresh.

ALTER TABLE works ADD COLUMN published_at TEXT;
ALTER TABLE works ADD COLUMN complete INTEGER;
ALTER TABLE works ADD COLUMN hits INTEGER;
ALTER TABLE works ADD COLUMN bookmarks INTEGER;
