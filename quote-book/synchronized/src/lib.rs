// Copyright (c) 2023 MobileCoin Inc.

use deqs_quote_book_api::{Error, Pair, Quote, QuoteBook, QuoteId};
use mc_blockchain_types::BlockIndex;
use mc_common::logger::{log, Logger};
use mc_crypto_ring_signature::KeyImage;
use mc_ledger_db::{Error as LedgerError, Ledger};
use std::{
    ops::RangeBounds,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{Builder as ThreadBuilder, JoinHandle},
    time::Duration,
};

/// A wrapper for a quote book implementation that syncs quotes with the ledger
#[derive(Clone)]
pub struct SynchronizedQuoteBook<Q: QuoteBook, L: Ledger + Clone + Sync + 'static> {
    /// Quotebook being synchronized to the ledger by the Synchronized Quotebook
    quote_book: Q,

    /// Ledger
    ledger: L,

    /// Index of the highest processed block index by the background thread.
    highest_processed_block_index: Arc<AtomicU64>,

    /// Thread management. This is being held purely for the destructor so that
    /// the SyncThread will be dropped after all the Quotebooks using it.
    _thread_manager: Arc<SyncThread>,
}

impl<Q: QuoteBook, L: Ledger + Clone + Sync + 'static> SynchronizedQuoteBook<Q, L> {
    /// Create a new Synchronized Quotebook
    pub fn new(
        quote_book: Q,
        ledger: L,
        remove_quote_callback: RemoveQuoteCallback,
        logger: Logger,
    ) -> Self {
        let ledger_clone = ledger.clone();
        let quote_book_clone = quote_book.clone();
        let highest_processed_block_index = Arc::new(AtomicU64::new(0));
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
                        logger.clone(),
                    )
                })
                .expect("Could not spawn thread"),
        );
        let _thread_manager = Arc::new(SyncThread {
            join_handle,
            stop_requested,
        });
        Self {
            quote_book,
            ledger,
            highest_processed_block_index,
            _thread_manager,
        }
    }

    pub fn get_current_block_index(&self) -> u64 {
        self.highest_processed_block_index.load(Ordering::SeqCst)
    }

    fn validate_quote_ring_members(&self, quote: &Quote) -> Result<(), Error> {
        let indices = &quote.sci().tx_out_global_indices;
        let ring = &quote.sci().tx_in.ring;
        //We should have an index per txo.
        if ring.len() != indices.len() {
            return Err(Error::InvalidRing("Missing index for txo".to_owned()));
        }

        for (index, tx_out) in indices.iter().zip(ring.iter()) {
            let real_tx_out = self
                .ledger
                .get_tx_out_by_index(*index)
                .map_err(|err| Error::ImplementationSpecific(err.to_string()))?;
            if real_tx_out != *tx_out {
                return Err(Error::InvalidRing("Invalid ring member".to_owned()));
            }
        }
        Ok(())
    }
}

impl<Q, L> QuoteBook for SynchronizedQuoteBook<Q, L>
where
    Q: QuoteBook,
    L: Ledger + Clone + Sync + 'static,
{
    fn add_quote(&self, quote: &Quote) -> Result<(), Error> {
        // Check to see if the ledger's current block index is already at or past the
        // max_tombstone_block for the sci.
        let current_block_index = self
            .ledger
            .num_blocks()
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?
            - 1;
        if let Some(input_rules) = &quote.sci().tx_in.input_rules {
            if input_rules.max_tombstone_block != 0
                && current_block_index >= input_rules.max_tombstone_block
            {
                return Err(Error::QuoteIsStale);
            }
        }
        // Check the ledger to see if the quote is stale before adding it to the
        // quotebook.
        if self
            .ledger
            .contains_key_image(&quote.sci().key_image())
            .map_err(|err| Error::ImplementationSpecific(err.to_string()))?
        {
            return Err(Error::QuoteIsStale);
        }
        self.validate_quote_ring_members(quote)?;

        // Try adding to quote book.
        self.quote_book.add_quote(quote)
    }

    fn remove_quote_by_id(&self, id: &QuoteId) -> Result<Quote, Error> {
        self.quote_book.remove_quote_by_id(id)
    }

    fn remove_quotes_by_key_image(&self, key_image: &KeyImage) -> Result<Vec<Quote>, Error> {
        self.quote_book.remove_quotes_by_key_image(key_image)
    }

    fn remove_quotes_by_tombstone_block(
        &self,
        current_block_index: BlockIndex,
    ) -> Result<Vec<Quote>, Error> {
        self.quote_book
            .remove_quotes_by_tombstone_block(current_block_index)
    }

    fn get_quotes(
        &self,
        pair: &Pair,
        base_token_quantity: impl RangeBounds<u64>,
        limit: usize,
    ) -> Result<Vec<Quote>, Error> {
        self.quote_book.get_quotes(pair, base_token_quantity, limit)
    }

    fn get_quote_ids(&self, pair: Option<&Pair>) -> Result<Vec<QuoteId>, Error> {
        self.quote_book.get_quote_ids(pair)
    }

    fn get_quote_by_id(&self, id: &QuoteId) -> Result<Option<Quote>, Error> {
        self.quote_book.get_quote_by_id(id)
    }

    fn num_scis(&self) -> Result<u64, Error> {
        self.quote_book.num_scis()
    }
}

/// A callback for broadcasting a quote removal. It receives 1 argument:
/// - A vector of quotes that have been removed
pub type RemoveQuoteCallback = Box<dyn FnMut(Vec<Quote>) + Sync + Send>;

struct SyncThread {
    /// Join handle used to wait for the thread to terminate.
    join_handle: Option<JoinHandle<()>>,

    /// Stop request trigger, used to signal the thread to stop.
    stop_requested: Arc<AtomicBool>,
}

impl SyncThread {
    /// Stop and join the dbfetcher thread
    pub fn stop(&mut self) {
        if let Some(join_handle) = self.join_handle.take() {
            self.stop_requested.store(true, Ordering::SeqCst);
            join_handle
                .join()
                .expect("SynchronizedQuoteBookThread join failed");
        }
    }
}

impl Drop for SyncThread {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

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
/// polling data out of the database.
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
                let mut quotes = self
                    .quotebook
                    .remove_quotes_by_tombstone_block(last_processed_block_index);
                while quotes.is_err() {
                    log::error!(
                        self.logger,
                        "Unexpected error when removing quotes by tombstone_block {}. Retrying. Error: {}",
                        last_processed_block_index,
                        quotes.expect_err("quotes should be an err if we're printing about the unexpected error")
                    );
                    std::thread::sleep(Self::ERROR_RETRY_FREQUENCY);
                    quotes = self
                        .quotebook
                        .remove_quotes_by_tombstone_block(last_processed_block_index);
                }
                (self.remove_quote_callback)(quotes.unwrap());
                self.highest_processed_block_index
                    .store(last_processed_block_index, Ordering::SeqCst);
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
                    let mut quotes = self.quotebook.remove_quotes_by_key_image(&key_image);
                    while quotes.is_err() {
                        log::error!(
                            self.logger,
                            "Unexpected error when removing quotes by key_image {}. Retrying. Error: {}",
                            key_image,
                            quotes.expect_err("quotes should be an err if we're printing about the unexpected error")
                        );
                        std::thread::sleep(Self::ERROR_RETRY_FREQUENCY);
                        quotes = self.quotebook.remove_quotes_by_key_image(&key_image);
                    }
                    (self.remove_quote_callback)(quotes.unwrap());
                }
                self.next_block_index += 1;
            }
        }
        may_have_more_work
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deqs_quote_book_in_memory::InMemoryQuoteBook;
    use deqs_quote_book_test_suite as test_suite;
    use mc_account_keys::AccountKey;
    use mc_blockchain_types::BlockVersion;
    use mc_common::logger::test_with_logger;
    use mc_crypto_ring_signature_signer::NoKeysRingSigner;
    use mc_fog_report_validation_test_utils::MockFogResolver;
    use mc_ledger_db::{
        test_utils::{
            add_txos_and_key_images_to_ledger, add_txos_to_ledger, create_ledger, initialize_ledger,
        },
        Ledger, LedgerDB,
    };
    use mc_transaction_builder::test_utils::get_transaction;
    use mc_transaction_core::TokenId;
    use rand::{rngs::StdRng, SeedableRng};
    use std::{sync::Mutex, vec};
    struct TestContext {
        ledger: LedgerDB,
        removed_quotes_sent_to_live_updates: Arc<Mutex<Vec<Quote>>>,
        synchronized_quote_book: SynchronizedQuoteBook<InMemoryQuoteBook, LedgerDB>,
    }

    fn setup(logger: Logger) -> TestContext {
        let removed_quotes_sent_to_live_updates = Arc::new(Mutex::new(vec![]));
        let removed_quotes_for_callback = removed_quotes_sent_to_live_updates.clone();
        let remove_quote_callback = Box::new(move |quotes| {
            removed_quotes_for_callback
                .lock()
                .expect("mutex poisoned")
                .extend(quotes);
        });
        let ledger = create_and_initialize_test_ledger();
        let internal_quote_book = InMemoryQuoteBook::default();
        let synchronized_quote_book = SynchronizedQuoteBook::new(
            internal_quote_book,
            ledger.clone(),
            remove_quote_callback,
            logger,
        );
        TestContext {
            removed_quotes_sent_to_live_updates,
            ledger,
            synchronized_quote_book,
        }
    }

    fn create_and_initialize_test_ledger() -> LedgerDB {
        //Create a ledger_db
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let block_version = BlockVersion::MAX;
        let sender = AccountKey::random(&mut rng);
        let mut ledger = create_ledger();

        //Initialize that db
        let n_blocks = 3;
        initialize_ledger(block_version, &mut ledger, n_blocks, &sender, &mut rng);

        ledger
    }

    #[test_with_logger]
    fn basic_happy_flow(logger: Logger) {
        let test_context = setup(logger.clone());
        test_suite::basic_happy_flow(
            &test_context.synchronized_quote_book,
            Some(&test_context.ledger),
        );
        {
            let removed_quotes = test_context
                .removed_quotes_sent_to_live_updates
                .lock()
                .expect("mutex poisoned")
                .clone();
            assert_eq!(removed_quotes, vec![]);
        }
    }

    #[test_with_logger]
    fn cannot_add_invalid_sci(logger: Logger) {
        let test_context = setup(logger.clone());
        let synchronized_quote_book = test_context.synchronized_quote_book;
        test_suite::cannot_add_invalid_sci(&synchronized_quote_book, Some(&test_context.ledger));
    }

    #[test_with_logger]
    fn get_quotes_filtering_works(logger: Logger) {
        let test_context = setup(logger.clone());
        let synchronized_quote_book = test_context.synchronized_quote_book;
        test_suite::get_quotes_filtering_works(
            &synchronized_quote_book,
            Some(&test_context.ledger),
        );
    }

    #[test_with_logger]
    fn get_quote_ids_works(logger: Logger) {
        let test_context = setup(logger.clone());
        let synchronized_quote_book = test_context.synchronized_quote_book;
        test_suite::get_quote_ids_works(&synchronized_quote_book, Some(&test_context.ledger));
    }

    #[test_with_logger]
    fn get_quote_by_id_works(logger: Logger) {
        let test_context = setup(logger.clone());
        let synchronized_quote_book = test_context.synchronized_quote_book;
        test_suite::get_quote_by_id_works(&synchronized_quote_book, Some(&test_context.ledger));
    }

    #[test_with_logger]
    fn cannot_add_sci_with_key_image_in_ledger(logger: Logger) {
        let test_context = setup(logger.clone());
        let mut ledger = test_context.ledger.clone();
        let synchronized_quote_book = test_context.synchronized_quote_book;

        let pair = test_suite::pair();
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

        let sci_builder =
            test_suite::create_sci_builder(&pair, 10, 20, &mut rng, Some(&test_context.ledger));
        let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        let block_version = BlockVersion::MAX;
        let fog_resolver = MockFogResolver::default();

        let offerer_account = AccountKey::random(&mut rng);

        let tx = get_transaction(
            block_version,
            TokenId::from(0),
            2,
            2,
            &offerer_account,
            &offerer_account,
            fog_resolver,
            &mut rng,
        )
        .unwrap();
        add_txos_and_key_images_to_ledger(
            &mut ledger,
            BlockVersion::MAX,
            tx.prefix.outputs,
            vec![sci.key_image()],
            &mut rng,
        )
        .unwrap();

        // Because the key image is already in the ledger, adding this sci should fail
        assert_eq!(
            synchronized_quote_book.add_sci(sci, None).unwrap_err(),
            Error::QuoteIsStale
        );

        // Adding a quote that isn't already in the ledger should work
        let sci = test_suite::create_sci(&pair, 10, 20, &mut rng, Some(&test_context.ledger));
        let quote = synchronized_quote_book.add_sci(sci, None).unwrap();

        let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes, vec![quote]);
    }

    #[test_with_logger]
    fn sci_that_are_added_to_ledger_are_removed_in_the_background(logger: Logger) {
        let test_context = setup(logger.clone());
        let mut ledger = test_context.ledger.clone();
        let synchronized_quote_book = test_context.synchronized_quote_book;

        let pair = test_suite::pair();
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

        let sci_builder =
            test_suite::create_sci_builder(&pair, 10, 20, &mut rng, Some(&test_context.ledger));
        let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();
        let key_image = sci.key_image();
        // Adding a quote that isn't already in the ledger should work
        let quote = synchronized_quote_book.add_sci(sci, None).unwrap();

        let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes, vec![quote.clone()]);

        let block_version = BlockVersion::MAX;
        let fog_resolver = MockFogResolver::default();

        let offerer_account = AccountKey::random(&mut rng);

        let tx = get_transaction(
            block_version,
            TokenId::from(0),
            2,
            2,
            &offerer_account,
            &offerer_account,
            fog_resolver,
            &mut rng,
        )
        .unwrap();
        add_txos_and_key_images_to_ledger(
            &mut ledger,
            BlockVersion::MAX,
            tx.prefix.outputs,
            vec![key_image],
            &mut rng,
        )
        .unwrap();

        // Because the key image is already in the ledger, this sci should be removed in
        // the background after waiting for the quotebook to sync
        while synchronized_quote_book.get_current_block_index() < (ledger.num_blocks().unwrap() - 1)
        {
            std::thread::sleep(Duration::from_millis(1000));
        }
        let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes, vec![]);
        {
            let removed_quotes = test_context
                .removed_quotes_sent_to_live_updates
                .lock()
                .expect("mutex poisoned")
                .clone();
            assert_eq!(removed_quotes, vec![quote])
        }
    }

    #[test_with_logger]
    fn cannot_add_sci_past_tombstone_block(logger: Logger) {
        let pair = test_suite::pair();
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let test_context = setup(logger.clone());
        let ledger = test_context.ledger.clone();
        let synchronized_quote_book = test_context.synchronized_quote_book;

        // Because the tombstone block is lower than the number blocks already in the
        // ledger, adding this sci should fail
        let mut sci_builder =
            test_suite::create_sci_builder(&pair, 10, 20, &mut rng, Some(&test_context.ledger));
        sci_builder.set_tombstone_block(ledger.num_blocks().unwrap() - 2);
        let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        assert_eq!(
            synchronized_quote_book.add_sci(sci, None).unwrap_err(),
            Error::QuoteIsStale
        );

        // Because the tombstone block is 0, adding this sci should pass
        let mut sci_builder2 =
            test_suite::create_sci_builder(&pair, 10, 20, &mut rng, Some(&test_context.ledger));
        sci_builder2.set_tombstone_block(0);
        let sci2 = sci_builder2.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        let quote2 = synchronized_quote_book.add_sci(sci2, None).unwrap();

        let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes, vec![quote2.clone()]);

        // Because the tombstone block is higher than the block index of the highest
        // block already in the ledger, adding this sci should pass
        let mut sci_builder3 =
            test_suite::create_sci_builder(&pair, 10, 20, &mut rng, Some(&test_context.ledger));
        sci_builder3.set_tombstone_block(ledger.num_blocks().unwrap());
        let sci3 = sci_builder3.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        let quote3 = synchronized_quote_book.add_sci(sci3, None).unwrap();

        let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes, vec![quote2, quote3]);

        // Because the tombstone block is equal to the current block index, adding this
        // sci should fail
        let mut sci_builder4 =
            test_suite::create_sci_builder(&pair, 10, 20, &mut rng, Some(&test_context.ledger));
        sci_builder4.set_tombstone_block(ledger.num_blocks().unwrap() - 1);
        let sci4 = sci_builder4.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        assert_eq!(
            synchronized_quote_book.add_sci(sci4, None).unwrap_err(),
            Error::QuoteIsStale
        );
    }

    #[test_with_logger]
    fn sci_past_tombstone_block_get_removed_in_the_background(logger: Logger) {
        let pair = test_suite::pair();
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let test_context = setup(logger.clone());
        let mut ledger = test_context.ledger.clone();
        let starting_blocks = test_context.ledger.num_blocks().unwrap();
        let synchronized_quote_book = test_context.synchronized_quote_book;

        // Because the tombstone block is higher than the number of blocks already in
        // the ledger, adding this sci should succeed
        let mut sci_builder =
            test_suite::create_sci_builder(&pair, 10, 20, &mut rng, Some(&test_context.ledger));
        // Number of blocks has advanced because the TXOs being used by this SCI were
        // added to the ledger.
        assert_eq!(ledger.num_blocks().unwrap(), starting_blocks + 1);
        sci_builder.set_tombstone_block(ledger.num_blocks().unwrap());
        let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        let quote = synchronized_quote_book.add_sci(sci, None).unwrap();

        assert_eq!(
            synchronized_quote_book.get_quotes(&pair, .., 0).unwrap(),
            vec![quote.clone()]
        );

        let block_version = BlockVersion::MAX;
        let fog_resolver = MockFogResolver::default();

        let offerer_account = AccountKey::random(&mut rng);

        let tx = get_transaction(
            block_version,
            TokenId::from(0),
            2,
            2,
            &offerer_account,
            &offerer_account,
            fog_resolver,
            &mut rng,
        )
        .unwrap();
        add_txos_to_ledger(&mut ledger, BlockVersion::MAX, &tx.prefix.outputs, &mut rng).unwrap();
        assert_eq!(ledger.num_blocks().unwrap(), starting_blocks + 2);
        // Because the ledger has advanced, this sci should be removed in the background
        // after waiting for the quotebook to sync
        while synchronized_quote_book.get_current_block_index() < (ledger.num_blocks().unwrap() - 1)
        {
            std::thread::sleep(Duration::from_millis(1000));
        }

        let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes, vec![]);
        {
            let removed_quotes = test_context
                .removed_quotes_sent_to_live_updates
                .lock()
                .expect("mutex poisoned")
                .clone();
            assert_eq!(removed_quotes, vec![quote])
        }
    }

    #[test_with_logger]
    fn sci_with_invalid_ring_is_rejected(logger: Logger) {
        let pair = test_suite::pair();
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let test_context = setup(logger.clone());
        let ledger = test_context.ledger.clone();
        let starting_blocks = test_context.ledger.num_blocks().unwrap();
        let synchronized_quote_book = test_context.synchronized_quote_book;

        // Adding an SCI with TXOs not in the ledger should fail if the sci_builder does
        // not add the txos used by the sci into the ledger.
        let sci_builder = test_suite::create_sci_builder(&pair, 10, 20, &mut rng, None);
        // Number of blocks has not advanced because the TXOs being used by this SCI
        // were not added to the ledger.
        assert_eq!(ledger.num_blocks().unwrap(), starting_blocks);
        let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        assert_eq!(
            synchronized_quote_book.add_sci(sci, None).unwrap_err(),
            Error::InvalidRing("Invalid ring member".to_owned())
        );

        assert_eq!(
            synchronized_quote_book.get_quotes(&pair, .., 0).unwrap(),
            vec![]
        );

        // Adding an SCI with txos in the ledger should succeed. The sci_builder adds
        // the txos used by the SCI into the ledger.
        let sci_builder2 =
            test_suite::create_sci_builder(&pair, 10, 20, &mut rng, Some(&test_context.ledger));
        // Number of blocks has advanced because the TXOs being used by this SCI were
        // added to the ledger.
        assert_eq!(ledger.num_blocks().unwrap(), starting_blocks + 1);
        let sci2 = sci_builder2.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        let quote = synchronized_quote_book.add_sci(sci2, None).unwrap();

        assert_eq!(
            synchronized_quote_book.get_quotes(&pair, .., 0).unwrap(),
            vec![quote]
        );
    }
}
