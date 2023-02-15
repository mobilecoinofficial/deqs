// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error as QuoteBookError, Pair, Quote, QuoteBook, QuoteId};
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_ledger_db::Ledger;
use mc_ledger_db::Error as LedgerError;
use mc_transaction_extra::SignedContingentInput;

use std::{
    ops::RangeBounds, 
    thread::{Builder as ThreadBuilder},
    time::{Duration},
};

use mc_common::logger::{log, Logger};

/// A wrapper for a quote book implementation that syncs quotes with the ledger
#[derive(Clone)]
pub struct SynchronizedQuoteBook<Q: QuoteBook, L: Ledger + Clone + 'static> {
    /// Quotebook being synchronized to the ledger by the Synchronized Quotebook
    quote_book: Q,

    /// Ledger
    ledger: L,
}

impl<Q: QuoteBook, L: Ledger + Clone + Sync + 'static> SynchronizedQuoteBook<Q, L> {
    /// Create a new Synchronized Quotebook
    pub fn new(quote_book: Q, ledger: L, logger: Logger) -> Self {
        let ledger_clone = ledger.clone();
        let quote_book_clone = quote_book.clone();
        ThreadBuilder::new()
            .name("LedgerDbFetcher".to_owned())
            .spawn(move || {
                DbFetcherThread::start(
                    ledger_clone,
                    quote_book_clone,
                    0,
                    logger
                )
            })
            .expect("Could not spawn thread");
        Self { quote_book, ledger}
    }
}

impl<Q, L> QuoteBook for SynchronizedQuoteBook<Q, L>
where
    Q: QuoteBook,
    L: Ledger + Clone + Sync + 'static,
{
    fn add_sci(
        &self,
        sci: SignedContingentInput,
        timestamp: Option<u64>,
    ) -> Result<Quote, QuoteBookError> {
        // Check to see if the current_block_index is already at or past the
        // max_tombstone_block for the sci.
        let current_block_index = self.ledger.num_blocks()? - 1;
        if let Some(input_rules) = &sci.tx_in.input_rules {
            if input_rules.max_tombstone_block != 0
                && current_block_index >= input_rules.max_tombstone_block
            {
                return Err(QuoteBookError::QuoteIsStale);
            }
        }
        // Check the ledger to see if the quote is stale before adding it to the
        // quotebook.
        if self.ledger.contains_key_image(&sci.key_image())? {
            return Err(QuoteBookError::QuoteIsStale);
        }

        // Try adding to quote book.
        self.quote_book.add_sci(sci, timestamp)
    }

    fn remove_quote_by_id(&self, id: &QuoteId) -> Result<Quote, QuoteBookError> {
        self.quote_book.remove_quote_by_id(id)
    }

    fn remove_quotes_by_key_image(
        &self,
        key_image: &KeyImage,
    ) -> Result<Vec<Quote>, QuoteBookError> {
        self.quote_book.remove_quotes_by_key_image(key_image)
    }

    fn remove_quotes_by_tombstone_block(
        &self,
        current_block_index: BlockIndex,
    ) -> Result<Vec<Quote>, QuoteBookError> {
        self.quote_book
            .remove_quotes_by_tombstone_block(current_block_index)
    }

    fn get_quotes(
        &self,
        pair: &Pair,
        base_token_quantity: impl RangeBounds<u64>,
        limit: usize,
    ) -> Result<Vec<Quote>, QuoteBookError> {
        self.quote_book.get_quotes(pair, base_token_quantity, limit)
    }

    fn get_quote_ids(&self, pair: Option<&Pair>) -> Result<Vec<QuoteId>, QuoteBookError> {
        self.quote_book.get_quote_ids(pair)
    }

    fn get_quote_by_id(&self, id: &QuoteId) -> Result<Option<Quote>, QuoteBookError> {
        self.quote_book.get_quote_by_id(id)
    }
}

struct DbFetcherThread<DB: Ledger, Q: QuoteBook> {
    db: DB,
    quotebook: Q,
    next_block_index: u64,
    logger: Logger,
}

/// Background worker thread implementation that takes care of periodically
/// polling data out of the database. Add join handle
impl<DB: Ledger, Q: QuoteBook> DbFetcherThread<DB, Q> {
    const POLLING_FREQUENCY: Duration = Duration::from_millis(10);
    const ERROR_RETRY_FREQUENCY: Duration = Duration::from_millis(1000);

    pub fn start(
        db: DB,
        quotebook: Q,
        next_block_index: u64,
        logger: Logger,
    ) {
        let thread = Self {
            db,
            quotebook,
            next_block_index,
            logger
        };
        thread.run();
    }

    fn run(mut self) {
        log::info!(self.logger, "Db fetcher thread started.");
        self.next_block_index = 0;
        loop {
            while self.load_block_data() {
            }
            std::thread::sleep(Self::POLLING_FREQUENCY);
        }
    }

    /// Attempt to load the next block that we
    /// are aware of and remove quotes that match key images from it.
    /// Returns true if we might have more block data to load.
    fn load_block_data(&mut self) -> bool {
        // Default to true: if there is an error, we may have more work, we don't know
        let mut may_have_more_work = true;

        match self.db.get_block_contents(self.next_block_index) {
            Err(LedgerError::NotFound) => may_have_more_work = false,
            Err(e) => {
                log::error!(
                    self.logger,
                    "Unexpected error when checking for block data {}: {:?}",
                    self.next_block_index,
                    e
                );
                std::thread::sleep(Self::ERROR_RETRY_FREQUENCY);
            }
            Ok(block_contents) => {
                // Filter keyimages in quotebook
                for key_image in block_contents
                    .key_images
                {
                    let quotes_removed = self.quotebook.remove_quotes_by_key_image(&key_image);
                    log::info!(self.logger, "Removed Quotes due to key_image {:?}", quotes_removed);
                }
                let quotes_removed = self.quotebook.remove_quotes_by_tombstone_block(self.next_block_index);
                log::info!(self.logger, "Removed Quotes due to tombstone_block {:?}", quotes_removed);
                
                self.next_block_index += 1;
            }
        }
        may_have_more_work
    }
}


#[cfg(test)]
mod tests {
    // Tests for this are under the tests/ directory since we want to be able to
    // re-use some test code between implementations and that seems to be the
    // way to make Rust do that.
}
