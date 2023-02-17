// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error as QuoteBookError, Pair, Quote, QuoteBook, QuoteId};
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_ledger_db::Ledger;
use mc_ledger_db::Error as LedgerError;
use mc_transaction_extra::SignedContingentInput;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::{
    ops::RangeBounds, 
    thread::{Builder as ThreadBuilder},
    time::{Duration},
};

use mc_common::logger::{log, Logger};
use postage::{
    broadcast::Sender, prelude::Sink
};
use crate::{Msg};
/// A wrapper for a quote book implementation that syncs quotes with the ledger
#[derive(Clone)]
pub struct SynchronizedQuoteBook<Q: QuoteBook, L: Ledger + Clone + Sync + 'static> {
    /// Quotebook being synchronized to the ledger by the Synchronized Quotebook
    quote_book: Q,

    /// Ledger
    ledger: L,

    /// Shared state
    shared_state: Arc<Mutex<DbPollSharedState>>,

    /// Stop request trigger, used to signal the thread to stop.
    stop_requested: Arc<AtomicBool>,
}

impl<Q: QuoteBook, L: Ledger + Clone + Sync + 'static> SynchronizedQuoteBook<Q, L> {
    /// Create a new Synchronized Quotebook
    pub fn new(quote_book: Q, ledger: L, msg_bus_tx: Sender<Msg>, logger: Logger) -> Self {
        let ledger_clone = ledger.clone();
        let quote_book_clone = quote_book.clone();
        let shared_state = Arc::new(Mutex::new(DbPollSharedState::default()));
        let thread_state = shared_state.clone();
        let stop_requested = Arc::new(AtomicBool::new(false));
        let thread_stop_requested = stop_requested.clone();
        ThreadBuilder::new()
            .name("LedgerDbFetcher".to_owned())
            .spawn(move || {
                DbFetcherThread::start(
                    ledger_clone,
                    quote_book_clone,
                    msg_bus_tx,
                    0,
                    thread_state,
                    thread_stop_requested,
                    logger
                )
            })
            .expect("Could not spawn thread");
        Self { quote_book, ledger, shared_state, stop_requested}
    }
    pub fn get_current_block_index(
        &self
    ) -> u64 {
        let shared_state = self.shared_state.lock().expect("mutex poisoned");
        shared_state.highest_processed_block_index
    }

    /// Stop and join the db poll thread
    pub fn stop(&mut self) -> Result<(), ()> {
        self.stop_requested.store(true, Ordering::SeqCst);
        Ok(())
    }

}

impl <Q,L> Drop for SynchronizedQuoteBook<Q, L> 
where
Q: QuoteBook,
L: Ledger + Clone + Sync + 'static,
{
    fn drop(&mut self) {
        let _ = self.stop();
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
        let current_block_index = self.get_current_block_index();
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

/// State that we want to expose from the db poll thread
#[derive(Debug, Default)]
pub struct DbPollSharedState {
    /// The highest block index for which we can guarantee we have loaded all
    /// available data.
    pub highest_processed_block_index: u64,

}
struct DbFetcherThread<DB: Ledger, Q: QuoteBook> {
    db: DB,
    quotebook: Q,
    /// Message bus sender.
    msg_bus_tx: Sender<Msg>,
    next_block_index: u64,
    shared_state: Arc<Mutex<DbPollSharedState>>,
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
        msg_bus_tx: Sender<Msg>,
        next_block_index: u64,
        shared_state: Arc<Mutex<DbPollSharedState>>,
        stop_requested: Arc<AtomicBool>,
        logger: Logger,
    ) {
        let thread = Self {
            db,
            quotebook,
            msg_bus_tx,
            next_block_index,
            shared_state,
            stop_requested,
            logger
        };
        thread.run();
    }

    fn run(mut self) {
        log::info!(self.logger, "Db fetcher thread started.");
        self.next_block_index = 0;
        loop {
            if self.stop_requested.load(Ordering::SeqCst) {
                log::info!(self.logger, "Db fetcher thread stop requested.");
                break;
            }
            let starting_block_index = self.next_block_index;
            while self.load_block_data() && !self.stop_requested.load(Ordering::SeqCst) {
            }
            if self.next_block_index != starting_block_index
            {    
                let last_processed_block_index = self.next_block_index - 1;
                match self.quotebook.remove_quotes_by_tombstone_block(last_processed_block_index) {
                    Ok(quotes) => {
                        for quote in quotes {
                            log::info!(self.logger, "Quote {} removed", quote.id());
                            if let Err(err) = self
                                .msg_bus_tx
                                .blocking_send(Msg::SciQuoteRemoved(*quote.id()))
                            {
                                log::error!(
                                    self.logger,
                                    "Failed to send SCI quote {} removed message to message bus: {:?}",
                                    quote.id(),
                                    err
                                );
                            }
                        }
                    }
                    Err(err) => {
                        log::error!(self.logger, "Failed to sync to block_index {}: {:?}", last_processed_block_index, err);
                    }
                }
                let mut shared_state =
                self.shared_state.lock().expect("mutex poisoned");
                // this is next_block_index because next_block_index is actually the block
                // we just processed
                shared_state.highest_processed_block_index = last_processed_block_index;
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
                    match self.quotebook.remove_quotes_by_key_image(&key_image) {
                        Ok(quotes) => {
                            for quote in quotes {
                                log::info!(self.logger, "Quote {} removed", quote.id());
                                if let Err(err) = self
                                    .msg_bus_tx
                                    .blocking_send(Msg::SciQuoteRemoved(*quote.id()))
                                {
                                    log::error!(
                                        self.logger,
                                        "Failed to send SCI quote {} removed message to message bus: {:?}",
                                        quote.id(),
                                        err
                                    );
                                }
                            }
                        }
                        Err(err) => {
                            log::error!(self.logger, "Failed to remove key_image {}: {:?}", key_image, err);
                        }
                    }
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
