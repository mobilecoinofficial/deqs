-- This file should undo anything in `up.sql`

DROP INDEX idx__quotes__tombstone_block;
DROP INDEX idx__quotes__key_image;
DROP INDEX idx__quotes__base_range;
DROP INDEX idx__quotes__counter_token_id;
DROP INDEX idx__quotes__base_token_id;
DROP TABLE quotes;