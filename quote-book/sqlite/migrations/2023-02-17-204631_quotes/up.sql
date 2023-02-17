-- Note about primary keys: For SQLite to auto-increment a PRIMARY KEY row, the column must be `INTEGER PRIMARY KEY`.
-- Even `INT PRIMARY KEY` won't work. It has to be `INTEGER PRIMARY KEY`. See https://www.sqlite.org/autoinc.html.
-- This has an annoying impact on Diesel - it forces the primary key column to be `i32` even though SQLite uses 64 bit for these columns.
-- Diesel also requires that a table has a primary key column, so each table must have one.
--
-- Note about signed/unsigned: SQLite does not support unsigned types. As such, everything is signed and gets represented
-- as i32/i64 on the rust side. Rust code can still safely cast to unsigned types without losing data, but this does mean
-- that SQLite functions will behave incorrectly - for example calling SUM(amount) should be avoided, since SQLite will be
-- summing signed values. The workaround is to do such operations in Rust, after casting to the unsigned type.
-- Where we want to do sorting/range comparisons (e.g. greater-than, less-than, etc) on u64s, we revert to storing
-- them as the big endian bytes. SQLite uses memcmp() to compare binary values, so this works.

CREATE TABLE quotes (
    -- Quote ID
    id BINARY PRIMARY KEY NOT NULL,

    -- The SCI, protobuf-encoded
    sci_protobuf BINARY NOT NULL,

    -- Base token id (stored as signed i64 since we can cast from rust and only do equality comparisons on it)
    base_token_id BIGINT NOT NULL,

    -- Counter token id (stored as signed i64 since we can cast from rust and only do equality comparisons on it)
    counter_token_id BIGINT NOT NULL,

    -- The range of base tokens offered by this quote. Stored as big endian bytes so that we don't run into signed/unsigned issues.
    base_range_min BINARY NOT NULL,
    base_range_max BINARY NOT NULL,

    -- The number of counter tokens needed to trade the max amount of base tokens.
    max_counter_tokens BINARY NOT NULL,

    -- Timestamp at which the quote arrived
    timestamp BIGINT NOT NULL
);

CREATE INDEX idx__quotes__base_token_id ON quotes (base_token_id);
CREATE INDEX idx__quotes__counter_token_id ON quotes (counter_token_id);
CREATE INDEX idx__quotes__base_range ON quotes (base_range_min, base_range_max);