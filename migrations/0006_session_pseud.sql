-- W6: the default pseud used when posting comments. AO3 validates that the
-- pseud belongs to the account, so we discover and remember it once per session.

ALTER TABLE sessions ADD COLUMN pseud_id INTEGER;
ALTER TABLE sessions ADD COLUMN pseud_name TEXT;
