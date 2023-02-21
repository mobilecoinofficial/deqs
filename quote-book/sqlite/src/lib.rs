// Copyright (c) 2023 MobileCoin Inc.

mod error;
mod models;
mod schema;
mod sql_types;

use deqs_quote_book_api::{Error as QuoteBookError, Quote, QuoteBook};
use deqs_quote_book_in_memory::InMemoryQuoteBook;
use diesel::{
    connection::SimpleConnection,
    insert_into,
    prelude::*,
    r2d2::{ConnectionManager, Pool, PooledConnection},
    result::DatabaseErrorKind,
    SqliteConnection,
};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};
use error::Error;
use mc_common::logger::{log, Logger};
use std::{path::Path, time::Duration};

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
    quote_book: InMemoryQuoteBook,
}

impl SqliteQuoteBook {
    pub fn new(
        pool: Pool<ConnectionManager<SqliteConnection>>,
        logger: Logger,
    ) -> Result<Self, QuoteBookError> {
        let mut conn = pool.get().map_err(Error::from)?;
        conn.run_pending_migrations(MIGRATIONS).map_err(|err| {
            QuoteBookError::ImplementationSpecific(format!("run pending migrations: {}", err))
        })?;

        let quote_book = InMemoryQuoteBook::default();

        let num_quotes = schema::quotes::table
            .count()
            .get_result::<i64>(&mut conn)
            .map_err(Error::from)? as usize;
        log::info!(
            logger,
            "SqliteQuoteBook: loading {} quotes from database",
            num_quotes
        );

        let mut last_percents_loaded = 0;
        for (i, quote) in schema::quotes::table
            .load::<models::Quote>(&mut conn)
            .map_err(Error::from)?
            .into_iter()
            .enumerate()
        {
            let percent_loaded = (i + 1) / num_quotes;

            // Log every ~10% of quotes loaded
            if percent_loaded > last_percents_loaded + 10 {
                log::info!(
                    logger,
                    "SqliteQuoteBook: {}% ({}/{}) quotes loaded from database",
                    percent_loaded,
                    i + 1,
                    num_quotes,
                );
                last_percents_loaded = percent_loaded;
            }

            quote_book.add_quote((&quote).try_into()?)?;
        }

        Ok(Self {
            pool,
            quote_book,
        })
    }

    pub fn new_from_file_path(
        file_path: &impl AsRef<Path>,
        db_connections: u32,
        logger: Logger,
    ) -> Result<Self, QuoteBookError> {
        let manager =
            ConnectionManager::<SqliteConnection>::new(file_path.as_ref().to_str().ok_or_else(
                || QuoteBookError::ImplementationSpecific("Invalid file path".to_string()),
            )?);
        let pool = Pool::builder()
            .max_size(db_connections)
            .connection_customizer(Box::new(ConnectionOptions {
                enable_wal: true,
                busy_timeout: Some(Duration::from_secs(30)),
            }))
            .test_on_check_out(true)
            .build(manager)
            .map_err(Error::from)?;

        Self::new(pool, logger)
    }

    pub fn get_conn(&self) -> Result<Conn, Error> {
        Ok(self.pool.get().map_err(Error::from)?)
    }
}

impl QuoteBook for SqliteQuoteBook {
    fn add_sci(
        &self,
        sci: mc_transaction_extra::SignedContingentInput,
        timestamp: Option<u64>,
    ) -> Result<deqs_quote_book_api::Quote, QuoteBookError> {
        // Convert SCI into an quote. This also validates it.
        let quote = Quote::new(sci, timestamp)?;
        let sql_quote = models::Quote::from(&quote);

        let mut conn = self.get_conn()?;
        conn.immediate_transaction(|conn| -> Result<(), Error> {
            // First, try to store the quote in SQL. However since we are inside a
            // transaction, it will not be committed.
            insert_into(schema::quotes::dsl::quotes)
                .values(&sql_quote)
                .execute(conn)
                .map_err(|err| -> Error {
                    match err {
                        diesel::result::Error::DatabaseError(
                            DatabaseErrorKind::UniqueViolation,
                            _,
                        ) => QuoteBookError::QuoteAlreadyExists.into(),
                        err => err.into(),
                    }
                })?;

            // Try to add to our in-memory quote book. If this fails, we will rollback the
            // transaction.
            self.quote_book.add_quote(quote.clone())?;

            Ok(())
        })?;

        Ok(quote)
    }

    fn remove_quote_by_id(
        &self,
        id: &deqs_quote_book_api::QuoteId,
    ) -> Result<deqs_quote_book_api::Quote, QuoteBookError> {
        use schema::quotes::dsl;

        let mut conn = self.get_conn()?;
        Ok(conn.immediate_transaction(|conn| -> Result<Quote, Error> {
            let num_deleted =
                diesel::delete(dsl::quotes.filter(dsl::id.eq(id.to_vec()))).execute(conn)?;

            match num_deleted {
                0 => Err(QuoteBookError::QuoteNotFound),
                1 => Ok(()),
                _ => Err(QuoteBookError::ImplementationSpecific(
                    "Deleted more than one quote".to_string(),
                )),
            }?;

            Ok(self.quote_book.remove_quote_by_id(id)?)
        })?)
    }

    fn remove_quotes_by_key_image(
        &self,
        key_image: &mc_crypto_ring_signature::KeyImage,
    ) -> Result<Vec<deqs_quote_book_api::Quote>, QuoteBookError> {
        use schema::quotes::dsl;
        let key_image_bytes = key_image.as_bytes().to_vec();

        let mut conn = self.get_conn()?;
        Ok(
            conn.immediate_transaction(|conn| -> Result<Vec<Quote>, Error> {
                let num_deleted =
                    diesel::delete(dsl::quotes.filter(dsl::key_image.eq(key_image_bytes)))
                        .execute(conn)?;

                let quotes = self.quote_book.remove_quotes_by_key_image(key_image)?;
                assert_eq!(quotes.len(), num_deleted);
                Ok(quotes)
            })?,
        )
    }

    fn remove_quotes_by_tombstone_block(
        &self,
        current_block_index: mc_blockchain_types::BlockIndex,
    ) -> Result<Vec<deqs_quote_book_api::Quote>, QuoteBookError> {
        use schema::quotes::dsl;
        let mut conn = self.get_conn()?;

        Ok(
            conn.immediate_transaction(|conn| -> Result<Vec<Quote>, Error> {
                // TODO is le correct ? 0 tombstone block
                let num_deleted = diesel::delete(
                    dsl::quotes.filter(dsl::tombstone_block.le(current_block_index as i64)),
                )
                .execute(conn)?;

                let quotes = self
                    .quote_book
                    .remove_quotes_by_tombstone_block(current_block_index)?;
                assert_eq!(quotes.len(), num_deleted);
                Ok(quotes)
            })?,
        )
    }

    fn get_quotes(
        &self,
        pair: &deqs_quote_book_api::Pair,
        base_token_quantity: impl std::ops::RangeBounds<u64>,
        limit: usize,
    ) -> Result<Vec<deqs_quote_book_api::Quote>, QuoteBookError> {
        self.quote_book.get_quotes(pair, base_token_quantity, limit)
    }

    fn get_quote_ids(
        &self,
        pair: Option<&deqs_quote_book_api::Pair>,
    ) -> Result<Vec<deqs_quote_book_api::QuoteId>, QuoteBookError> {
        self.quote_book.get_quote_ids(pair)
    }

    fn get_quote_by_id(
        &self,
        id: &deqs_quote_book_api::QuoteId,
    ) -> Result<Option<deqs_quote_book_api::Quote>, QuoteBookError> {
        self.quote_book.get_quote_by_id(id)
    }

    fn num_scis(&self) -> Result<u64, QuoteBookError> {
        self.quote_book.num_scis()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deqs_quote_book_test_suite as test_suite;
    use mc_common::logger::{test_with_logger, Logger};
    use tempdir::TempDir;

    fn create_quote_book(dir: &TempDir, logger: Logger) -> SqliteQuoteBook {
        let file_path = dir.path().join("quotes.db");
        SqliteQuoteBook::new_from_file_path(&file_path, 10, logger).unwrap()
    }

    #[test_with_logger]
    fn basic_happy_flow(logger: Logger) {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir, logger);
        test_suite::basic_happy_flow(&quote_book);
    }

    #[test_with_logger]
    fn cannot_add_duplicate_sci(logger: Logger) {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir, logger);
        test_suite::cannot_add_duplicate_sci(&quote_book);
    }

    #[test_with_logger]
    fn cannot_add_invalid_sci(logger: Logger) {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir, logger);
        test_suite::cannot_add_invalid_sci(&quote_book);
    }

    #[test_with_logger]
    fn get_quotes_filtering_works(logger: Logger) {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir, logger);
        test_suite::get_quotes_filtering_works(&quote_book);
    }

    #[test_with_logger]
    fn get_quote_ids_works(logger: Logger) {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir, logger);
        test_suite::get_quote_ids_works(&quote_book);
    }

    #[test_with_logger]
    fn get_quote_by_id_works(logger: Logger) {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir, logger);
        test_suite::get_quote_by_id_works(&quote_book);
    }
}
