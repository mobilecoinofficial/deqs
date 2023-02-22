// Copyright (c) 2023 MobileCoin Inc.
use backoff::{Error as BackoffError, ExponentialBackoff};
use crate::{Error as QuoteBookError, Pair, Quote, QuoteBook, QuoteId};
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_ledger_db::{Error as LedgerError, Ledger};
use mc_transaction_extra::SignedContingentInput;

use std::{
    ops::{RangeBounds},
    sync::{
        atomic::{AtomicBool, Ordering, AtomicU64},
        Arc, Mutex,
    },
    thread::{Builder as ThreadBuilder, JoinHandle},
    time::Duration,
};

use mc_common::logger::{log, Logger};
/// A wrapper for a quote book implementation that syncs quotes with the ledger
pub struct SynchronizedQuoteBook<Q: QuoteBook, L: Ledger + Clone + Sync + 'static> {
    /// Quotebook being synchronized to the ledger by the Synchronized Quotebook
    quote_book: Q,

    /// Ledger
    ledger: L,

    /// Shared state
    highest_processed_block_index: Arc<AtomicU64>,

    /// Join handle used to wait for the thread to terminate.
    join_handle: Option<JoinHandle<()>>,

    /// Stop request trigger, used to signal the thread to stop.
    stop_requested: Arc<AtomicBool>,
}

impl<Q: QuoteBook, L: Ledger + Clone + Sync + 'static> SynchronizedQuoteBook<Q, L> {
    /// Create a new Synchronized Quotebook
    pub fn new(quote_book: Q, ledger: L, remove_quote_callback: RemoveQuoteCallback, logger: Logger) -> Self {
        let ledger_clone = ledger.clone();
        let quote_book_clone = quote_book.clone();
        let highest_processed_block_index= Arc::new(AtomicU64::new(0));
        let thread_highest_processed_block_index = highest_processed_block_index.clone();
        let stop_requested = Arc::new(AtomicBool::new(false));
        let thread_stop_requested = stop_requested.clone();
        let join_handle = Some(
            ThreadBuilder::new()
                .name("LedgerDbFetcher".to_owned())
                .spawn(move || {
                    DbFetcherThread::start(
                        ledger_clone,
                        quote_book_clone,
                        remove_quote_callback,
                        thread_highest_processed_block_index,
                        thread_stop_requested,
                        logger,
                    )
                })
                .expect("Could not spawn thread"),
        );
        Self {
            quote_book,
            ledger,
            highest_processed_block_index,
            join_handle,
            stop_requested,
        }
    }
    pub fn get_current_block_index(&self) -> u64 {
        self.highest_processed_block_index.load(Ordering::SeqCst) 
    }

    /// Stop and join the db poll thread
    pub fn stop(&mut self) {
        if let Some(join_handle) = self.join_handle.take() {
            self.stop_requested.store(true, Ordering::SeqCst);
            join_handle.join().expect("SynchronizedQuoteBookThread join failed");        
        }
    }
}

impl<Q, L> Drop for SynchronizedQuoteBook<Q, L>
where
    Q: QuoteBook,
    L: Ledger + Clone + Sync + 'static,
{
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

impl<Q, L> QuoteBook for Arc<SynchronizedQuoteBook<Q, L>>
where
    Q: QuoteBook,
    L: Ledger + Clone + Sync + 'static,
{
    fn add_sci(
        &self,
        sci: SignedContingentInput,
        timestamp: Option<u64>,
    ) -> Result<Quote, QuoteBookError> {
        // Check to see if the ledger's current block index is already at or past the
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

/// A callback for broadcasting a quote removal. It receives 1 argument:
/// - A vector of quotes that have been removed
pub type RemoveQuoteCallback =
    Arc<Mutex<dyn FnMut(Vec<Quote>) + Sync + Send>>;

struct DbFetcherThread<DB: Ledger, Q: QuoteBook> {
    db: DB,
    quotebook: Q,
    /// Message bus sender.
    remove_quote_callback: RemoveQuoteCallback,
    next_block_index: u64,
    highest_processed_block_index: Arc<AtomicU64>,
    stop_requested: Arc<AtomicBool>,
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
        remove_quote_callback: RemoveQuoteCallback,
        highest_processed_block_index: Arc<AtomicU64>,
        stop_requested: Arc<AtomicBool>,
        logger: Logger,
    ) {
        let next_block_index = highest_processed_block_index.load(Ordering::SeqCst);
        let thread = Self {
            db,
            quotebook,
            remove_quote_callback,
            next_block_index,
            highest_processed_block_index,
            stop_requested,
            logger,
        };
        thread.run();
    }

    fn run(mut self) {
        log::info!(self.logger, "Db fetcher thread started.");
        loop {
            if self.stop_requested.load(Ordering::SeqCst) {
                log::info!(self.logger, "Db fetcher thread stop requested.");
                break;
            }
            let starting_block_index = self.next_block_index;
            while self.load_block_data() && !self.stop_requested.load(Ordering::SeqCst) {}
            if self.next_block_index != starting_block_index {
                let last_processed_block_index = self.next_block_index - 1;
                match self
                    .quotebook
                    .remove_quotes_by_tombstone_block(last_processed_block_index)
                {
                    Ok(quotes) => {
                        (self.remove_quote_callback.lock()
                        .expect("lock poisoned"))(quotes);
                    }
                    Err(err) => {
                        log::error!(
                            self.logger,
                            "Failed to sync to block_index {}: {:?}",
                            last_processed_block_index,
                            err
                        );
                    }
                }
                self.highest_processed_block_index.store(last_processed_block_index, Ordering::SeqCst);
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
                for key_image in block_contents.key_images {
                    let backoff = ExponentialBackoff::default();
                    let working_quotebook = self.quotebook.clone();
                    let f = &move || -> Result<Vec<Quote>, BackoffError<QuoteBookError>> {
                        working_quotebook.remove_quotes_by_key_image(&key_image).map_err(|e| BackoffError::Transient { err: e, retry_after: None })
                    };
                    let quotes = backoff::retry(backoff, f).expect("Could not remove quotes by key_image after retries");
                    (self.remove_quote_callback.lock()
                    .expect("lock poisoned"))(quotes);
                }
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
