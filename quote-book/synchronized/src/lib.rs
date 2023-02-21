// Copyright (c) 2023 MobileCoin Inc.

use deqs_quote_book_api::{Error, Pair, Quote, QuoteBook, QuoteId};
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_ledger_db::Ledger;
use std::ops::RangeBounds;

/// A wrapper for a quote book implementation that syncs quotes with the ledger
#[derive(Clone)]
pub struct SynchronizedQuoteBook<Q: QuoteBook, L: Ledger + Clone + 'static> {
    /// Quotebook being synchronized to the ledger by the Synchronized Quotebook
    quote_book: Q,

    ledger: L,
}

impl<Q: QuoteBook, L: Ledger + Clone + Sync + 'static> SynchronizedQuoteBook<Q, L> {
    /// Create a new Synchronized Quotebook
    pub fn new(quote_book: Q, ledger: L) -> Self {
        Self { quote_book, ledger }
    }
}

impl<Q, L> QuoteBook for SynchronizedQuoteBook<Q, L>
where
    Q: QuoteBook,
    L: Ledger + Clone + Sync + 'static,
{
    fn add_quote(&self, quote: &Quote) -> Result<(), Error> {
        // Check to see if the current_block_index is already at or past the
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

#[cfg(test)]
mod tests {
    use super::*;
    use deqs_quote_book_in_memory::InMemoryQuoteBook;
    use deqs_quote_book_test_suite as test_suite;
    use mc_account_keys::AccountKey;
    use mc_blockchain_types::BlockVersion;
    use mc_crypto_ring_signature_signer::NoKeysRingSigner;
    use mc_fog_report_validation_test_utils::MockFogResolver;
    use mc_ledger_db::{
        test_utils::{add_txos_and_key_images_to_ledger, create_ledger, initialize_ledger},
        Ledger, LedgerDB,
    };
    use mc_transaction_builder::test_utils::get_transaction;
    use mc_transaction_core::TokenId;
    use rand::{rngs::StdRng, SeedableRng};

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

    #[test]
    fn basic_happy_flow() {
        let ledger = create_and_initialize_test_ledger();
        let internal_quote_book = InMemoryQuoteBook::default();
        let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger);
        test_suite::basic_happy_flow(&synchronized_quote_book);
    }

    #[test]
    fn cannot_add_invalid_sci() {
        let ledger = create_and_initialize_test_ledger();
        let internal_quote_book = InMemoryQuoteBook::default();
        let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger);
        test_suite::cannot_add_invalid_sci(&synchronized_quote_book);
    }

    #[test]
    fn get_quotes_filtering_works() {
        let ledger = create_and_initialize_test_ledger();
        let internal_quote_book = InMemoryQuoteBook::default();
        let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger);
        test_suite::get_quotes_filtering_works(&synchronized_quote_book);
    }

    #[test]
    fn get_quote_ids_works() {
        let ledger = create_and_initialize_test_ledger();
        let internal_quote_book = InMemoryQuoteBook::default();
        let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger);
        test_suite::get_quote_ids_works(&synchronized_quote_book);
    }

    #[test]
    fn get_quote_by_id_works() {
        let ledger = create_and_initialize_test_ledger();
        let internal_quote_book = InMemoryQuoteBook::default();
        let synchronized_quote_book = SynchronizedQuoteBook::new(internal_quote_book, ledger);
        test_suite::get_quote_by_id_works(&synchronized_quote_book);
    }

    #[test]
    fn cannot_add_sci_with_key_image_in_ledger() {
        let mut ledger = create_and_initialize_test_ledger();
        let internal_quote_book = InMemoryQuoteBook::default();
        let synchronized_quote_book =
            SynchronizedQuoteBook::new(internal_quote_book, ledger.clone());

        let pair = test_suite::pair();
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

        let sci_builder = test_suite::create_sci_builder(&pair, 10, 20, &mut rng);
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
        let sci = test_suite::create_sci(&pair, 10, 20, &mut rng);
        let quote = synchronized_quote_book.add_sci(sci, None).unwrap();

        let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes, vec![quote.clone()]);
    }

    #[test]
    fn cannot_add_sci_past_tombstone_block() {
        let pair = test_suite::pair();
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);

        let ledger = create_and_initialize_test_ledger();
        let internal_quote_book = InMemoryQuoteBook::default();
        let synchronized_quote_book =
            SynchronizedQuoteBook::new(internal_quote_book, ledger.clone());

        // Because the tombstone block is lower than the number blocks already in the
        // ledger, adding this sci should fail
        let mut sci_builder = test_suite::create_sci_builder(&pair, 10, 20, &mut rng);
        sci_builder.set_tombstone_block(ledger.num_blocks().unwrap() - 2);
        let sci = sci_builder.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        assert_eq!(
            synchronized_quote_book.add_sci(sci, None).unwrap_err(),
            Error::QuoteIsStale
        );

        // Because the tombstone block is higher than the block index of the highest
        // block already in the ledger, adding this sci should pass
        let mut sci_builder2 = test_suite::create_sci_builder(&pair, 10, 20, &mut rng);
        sci_builder2.set_tombstone_block(ledger.num_blocks().unwrap());
        let sci2 = sci_builder2.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        let quote2 = synchronized_quote_book.add_sci(sci2, None).unwrap();

        let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes, vec![quote2.clone()]);

        // Because the tombstone block is 0, adding this sci should pass
        let mut sci_builder3 = test_suite::create_sci_builder(&pair, 10, 20, &mut rng);
        sci_builder3.set_tombstone_block(0);
        let sci3 = sci_builder3.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        let quote3 = synchronized_quote_book.add_sci(sci3, None).unwrap();

        let quotes = synchronized_quote_book.get_quotes(&pair, .., 0).unwrap();
        assert_eq!(quotes, vec![quote2.clone(), quote3.clone()]);

        // Because the tombstone block is equal to the current block index, adding this
        // sci should fail
        let mut sci_builder4 = test_suite::create_sci_builder(&pair, 10, 20, &mut rng);
        sci_builder4.set_tombstone_block(ledger.num_blocks().unwrap() - 1);
        let sci4 = sci_builder4.build(&NoKeysRingSigner {}, &mut rng).unwrap();

        assert_eq!(
            synchronized_quote_book.add_sci(sci4, None).unwrap_err(),
            Error::QuoteIsStale
        );
    }
}
