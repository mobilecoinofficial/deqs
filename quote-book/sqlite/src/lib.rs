// Copyright (c) 2023 MobileCoin Inc.

mod models;
mod schema;
mod sql_types;

use std::{
    ops::{Bound, RangeBounds},
    path::Path,
    time::Duration,
};

use deqs_quote_book_api::{Error, Quote, QuoteBook};
use diesel::{
    connection::SimpleConnection,
    insert_into,
    prelude::*,
    r2d2::{ConnectionManager, Pool, PooledConnection},
    SqliteConnection,
};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

use crate::sql_types::VecU64;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/");

pub type Conn = PooledConnection<ConnectionManager<SqliteConnection>>;

#[derive(Debug)]
pub struct ConnectionOptions {
    pub enable_wal: bool,
    pub busy_timeout: Option<Duration>,
}

impl diesel::r2d2::CustomizeConnection<SqliteConnection, diesel::r2d2::Error>
    for ConnectionOptions
{
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), diesel::r2d2::Error> {
        (|| {
            if let Some(d) = self.busy_timeout {
                conn.batch_execute(&format!("PRAGMA busy_timeout = {};", d.as_millis()))?;
            }
            if self.enable_wal {
                conn.batch_execute("
                    PRAGMA journal_mode = WAL;          -- better write-concurrency
                    PRAGMA synchronous = NORMAL;        -- fsync only in critical moments
                    PRAGMA wal_autocheckpoint = 1000;   -- write WAL changes back every 1000 pages, for an in average 1MB WAL file. May affect readers if number is increased
                    PRAGMA wal_checkpoint(TRUNCATE);    -- free some space by truncating possibly massive WAL files from the last run.
                    PRAGMA foreign_keys = ON;           -- enable foreign key checks
                ")?;
            }
            Ok(())
        })()
        .map_err(diesel::r2d2::Error::QueryError)
    }
}

#[derive(Clone)]
pub struct SqliteQuoteBook {
    pool: Pool<ConnectionManager<SqliteConnection>>,
}

impl SqliteQuoteBook {
    pub fn new(pool: Pool<ConnectionManager<SqliteConnection>>) -> Result<Self, Error> {
        let mut conn = pool
            .get()
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?;
        conn.run_pending_migrations(MIGRATIONS)
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?;

        Ok(Self { pool })
    }

    pub fn new_from_file_path(
        file_path: &impl AsRef<Path>,
        db_connections: u32,
    ) -> Result<Self, Error> {
        let manager = ConnectionManager::<SqliteConnection>::new(
            file_path
                .as_ref()
                .to_str()
                .ok_or_else(|| Error::ImplementationSpecific("Invalid file path".to_string()))?,
        );
        let pool = Pool::builder()
            .max_size(db_connections)
            .connection_customizer(Box::new(ConnectionOptions {
                enable_wal: true,
                busy_timeout: Some(Duration::from_secs(30)),
            }))
            .test_on_check_out(true)
            .build(manager)
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?;

        Self::new(pool)
    }

    pub fn get_conn(&self) -> Result<Conn, Error> {
        Ok(self
            .pool
            .get()
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?)
    }
}

impl QuoteBook for SqliteQuoteBook {
    fn add_sci(
        &self,
        sci: mc_transaction_extra::SignedContingentInput,
        timestamp: Option<u64>,
    ) -> Result<deqs_quote_book_api::Quote, Error> {
        // Convert SCI into an quote. This also validates it.
        let quote = Quote::new(sci, timestamp)?;
        let sql_quote = models::Quote::from(&quote);

        let mut conn = self.get_conn()?;
        insert_into(schema::quotes::dsl::quotes)
            .values(&sql_quote)
            .execute(&mut conn)
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?; // TODO check for duplicate key violation

        Ok(quote)
    }

    fn remove_quote_by_id(
        &self,
        id: &deqs_quote_book_api::QuoteId,
    ) -> Result<deqs_quote_book_api::Quote, Error> {
        use schema::quotes::dsl;

        // TODO do in transaction
        let mut conn = self.get_conn()?;

        let quote = self.get_quote_by_id(id)?.ok_or(Error::QuoteNotFound)?;

        let num_deleted = diesel::delete(dsl::quotes.filter(dsl::id.eq(id.to_vec())))
            .execute(&mut conn)
            .expect("Error deleting posts"); // TODO

        match num_deleted {
            0 => Err(Error::QuoteNotFound),
            1 => Ok(quote),
            _ => Err(Error::ImplementationSpecific(
                "Deleted more than one quote".to_string(),
            )),
        }
    }

    fn remove_quotes_by_key_image(
        &self,
        key_image: &mc_crypto_ring_signature::KeyImage,
    ) -> Result<Vec<deqs_quote_book_api::Quote>, Error> {
        use schema::quotes::dsl;
        // TODO must run in a transaction
        let mut conn = self.get_conn()?;
        let key_image_bytes = key_image.as_bytes().to_vec();

        let sql_quotes = dsl::quotes
            .filter(dsl::key_image.eq(&key_image_bytes))
            .load::<models::Quote>(&mut conn)
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?;

        let quotes = sql_quotes
            .iter()
            .map(|sql_quote| Quote::try_from(sql_quote))
            .collect::<Result<Vec<_>, _>>()?;

        let num_deleted = diesel::delete(dsl::quotes.filter(dsl::key_image.eq(key_image_bytes)))
            .execute(&mut conn)
            .expect("Error deleting posts"); // TODO
                                             // TODO
        assert_eq!(num_deleted, quotes.len());

        Ok(quotes)
    }

    fn remove_quotes_by_tombstone_block(
        &self,
        current_block_index: mc_blockchain_types::BlockIndex,
    ) -> Result<Vec<deqs_quote_book_api::Quote>, Error> {
        use schema::quotes::dsl;
        // TODO must run in a transaction
        let mut conn = self.get_conn()?;

        let sql_quotes = dsl::quotes
            .filter(dsl::tombstone_block.le(current_block_index as i64))
            .load::<models::Quote>(&mut conn)
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?;

        let quotes = sql_quotes
            .iter()
            .map(|sql_quote| Quote::try_from(sql_quote))
            .collect::<Result<Vec<_>, _>>()?;

        // TODO is le current? 0 tombstone block
        let num_deleted =
            diesel::delete(dsl::quotes.filter(dsl::tombstone_block.le(current_block_index as i64)))
                .execute(&mut conn)
                .expect("Error deleting posts"); // TODO
                                                 // TODO
        assert_eq!(num_deleted, quotes.len());

        Ok(quotes)
    }

    fn get_quotes(
        &self,
        pair: &deqs_quote_book_api::Pair,
        base_token_quantity: impl std::ops::RangeBounds<u64>,
        limit: usize,
    ) -> Result<Vec<deqs_quote_book_api::Quote>, Error> {
        use schema::quotes::dsl;
        let mut conn = self.get_conn()?;

        let sql_quotes = dsl::quotes
            .filter(dsl::base_token_id.eq(*pair.base_token_id as i64))
            .filter(dsl::counter_token_id.eq(*pair.counter_token_id as i64))
            .load::<models::Quote>(&mut conn)
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?;

        let quotes = sql_quotes
            .iter()
            .map(|sql_quote| Quote::try_from(sql_quote))
            .collect::<Result<Vec<_>, _>>()?;

        // TODO can this be done in SQL? if not, this whole thing needs to be
        // re-thinked.
        let mut quotes = quotes
            .into_iter()
            .filter(|quote| range_overlaps(&base_token_quantity, quote.base_range()))
            .collect::<Vec<_>>();
        quotes.sort();

        if limit > 0 {
            quotes.truncate(limit);
        }

        Ok(quotes)
    }

    fn get_quote_ids(
        &self,
        pair: Option<&deqs_quote_book_api::Pair>,
    ) -> Result<Vec<deqs_quote_book_api::QuoteId>, Error> {
        todo!()
    }

    fn get_quote_by_id(
        &self,
        id: &deqs_quote_book_api::QuoteId,
    ) -> Result<Option<deqs_quote_book_api::Quote>, Error> {
        let mut conn = self.get_conn()?;
        let sql_quote = schema::quotes::dsl::quotes
            .find(id.to_vec())
            .first::<models::Quote>(&mut conn)
            .optional()
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?;
        let quote = sql_quote
            .map(|sql_quote| Quote::try_from(&sql_quote))
            .transpose()?;
        Ok(quote)
    }
}

// TODO
fn range_overlaps(x: &impl RangeBounds<u64>, y: &impl RangeBounds<u64>) -> bool {
    let x1 = match x.start_bound() {
        Bound::Included(start) => *start,
        Bound::Excluded(start) => start.saturating_add(1),
        Bound::Unbounded => 0,
    };

    let x2 = match x.end_bound() {
        Bound::Included(end) => *end,
        Bound::Excluded(end) => end.saturating_sub(1),
        Bound::Unbounded => u64::MAX,
    };

    let y1 = match y.start_bound() {
        Bound::Included(start) => *start,
        Bound::Excluded(start) => start.saturating_add(1),
        Bound::Unbounded => 0,
    };

    let y2 = match y.end_bound() {
        Bound::Included(end) => *end,
        Bound::Excluded(end) => end.saturating_sub(1),
        Bound::Unbounded => u64::MAX,
    };

    x1 <= y2 && y1 <= x2
}

#[cfg(test)]
mod tests {
    use super::*;
    use deqs_quote_book_test_suite as test_suite;
    use tempdir::TempDir;

    fn create_quote_book(dir: &TempDir) -> SqliteQuoteBook {
        let file_path = dir.path().join("quotes.db");
        SqliteQuoteBook::new_from_file_path(&file_path, 10).unwrap()
    }

    #[test]
    fn basic_happy_flow() {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir);
        test_suite::basic_happy_flow(&quote_book);
    }

    #[test]
    fn cannot_add_invalid_sci() {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir);
        test_suite::cannot_add_invalid_sci(&quote_book);
    }

    #[test]
    fn get_quotes_filtering_works() {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir);
        test_suite::get_quotes_filtering_works(&quote_book);
    }

    #[test]
    fn get_quote_ids_works() {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir);
        test_suite::get_quote_ids_works(&quote_book);
    }

    #[test]
    fn get_quote_by_id_works() {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir);
        test_suite::get_quote_by_id_works(&quote_book);
    }
}
