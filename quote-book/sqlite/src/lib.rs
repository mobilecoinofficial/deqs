// Copyright (c) 2023 MobileCoin Inc.

use std::{path::Path, time::Duration};

use deqs_quote_book_api::{Error, QuoteBook};
use diesel::{
    connection::SimpleConnection,
    prelude::*,
    r2d2::{ConnectionManager, Pool, PooledConnection},
    sql_types, SqliteConnection,
};
use diesel_migrations::{embed_migrations, EmbeddedMigrations, MigrationHarness};

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
    fn add_sci(&self, sci: mc_transaction_extra::SignedContingentInput, timestamp: Option<u64>) -> Result<deqs_quote_book_api::Quote, Error> {
        todo!()
    }

    fn remove_quote_by_id(&self, id: &deqs_quote_book_api::QuoteId) -> Result<deqs_quote_book_api::Quote, Error> {
        todo!()
    }

    fn remove_quotes_by_key_image(&self, key_image: &mc_crypto_ring_signature::KeyImage) -> Result<Vec<deqs_quote_book_api::Quote>, Error> {
        todo!()
    }

    fn remove_quotes_by_tombstone_block(
        &self,
        current_block_index: mc_blockchain_types::BlockIndex,
    ) -> Result<Vec<deqs_quote_book_api::Quote>, Error> {
        todo!()
    }

    fn get_quotes(
        &self,
        pair: &deqs_quote_book_api::Pair,
        base_token_quantity: impl std::ops::RangeBounds<u64>,
        limit: usize,
    ) -> Result<Vec<deqs_quote_book_api::Quote>, Error> {
        todo!()
    }

    fn get_quote_ids(&self, pair: Option<&deqs_quote_book_api::Pair>) -> Result<Vec<deqs_quote_book_api::QuoteId>, Error> {
        todo!()
    }

    fn get_quote_by_id(&self, id: &deqs_quote_book_api::QuoteId) -> Result<Option<deqs_quote_book_api::Quote>, Error> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempdir::TempDir;

    fn create_quote_book(dir: &TempDir) -> SqliteQuoteBook {
        let file_path = dir.path().join("quotes.db");
        SqliteQuoteBook::new_from_file_path(&file_path, 10).unwrap()
    }

    #[test]
    fn test_create_quote_book() {
        let dir = TempDir::new("quote_book_test").unwrap();
        let quote_book = create_quote_book(&dir);
    }
}
