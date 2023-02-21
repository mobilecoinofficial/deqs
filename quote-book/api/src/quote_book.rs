// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error, Pair, Quote, QuoteId};
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_transaction_extra::SignedContingentInput;
use std::ops::RangeBounds;

/// Quote book functionality for a single trading pair
pub trait QuoteBook: Clone + Send + Sync + 'static {
    /// Add an SCI to the quote book.
    ///
    /// # Arguments
    /// * `sci` - The SCI to add.
    /// * `timestamp` - The timestamp of the block containing the SCI. If not
    ///   provided, the current system time is used.
    fn add_sci(&self, sci: SignedContingentInput, timestamp: Option<u64>) -> Result<Quote, Error> {
        // Convert SCI into an quote. This also validates it.
        let quote = Quote::new(sci, timestamp)?;
        self.add_quote(&quote)?;
        Ok(quote)
    }

    /// Add a pre-existing quote to the quote book.
    fn add_quote(&self, quote: &Quote) -> Result<(), Error>;

    /// Remove a single quote from the book, identified by its id.
    /// Returns the removed quote if it was found
    fn remove_quote_by_id(&self, id: &QuoteId) -> Result<Quote, Error>;

    /// Remove all quotes matching a given key image, returns the list of quotes
    /// removed
    fn remove_quotes_by_key_image(&self, key_image: &KeyImage) -> Result<Vec<Quote>, Error>;

    /// Remove all quotes whose tombstone block is >= current block index,
    /// returns the list of quotes removed.
    fn remove_quotes_by_tombstone_block(
        &self,
        current_block_index: BlockIndex,
    ) -> Result<Vec<Quote>, Error>;

    /// Search for quotes that can provide `pair.base_token_id in exchange for
    /// being sent `pair.counter_token_id`. Optionally filtering only for
    /// quotes that can provide some amount in the range `base_token_quantity`.
    /// Due to partial fill rules, quotes returned may not be able to provide
    /// the entire range. It can also optionally limit the number of quotes
    /// returned. A limit of 0 returns all quotes. Quotes are returned sorted by
    /// the more favorable exchange rate (quotes giving more base tokens for
    /// a given amount of counter tokens are returned first). For the exact
    /// sorting details, see documentation in the `Ord` implementation of
    /// `Quote`.
    fn get_quotes(
        &self,
        pair: &Pair,
        base_token_quantity: impl RangeBounds<u64>,
        limit: usize,
    ) -> Result<Vec<Quote>, Error>;

    /// Return all known quote ids, optionally filtering for a specific pair.
    fn get_quote_ids(&self, pair: Option<&Pair>) -> Result<Vec<QuoteId>, Error>;

    /// Get a quote by its id
    fn get_quote_by_id(&self, id: &QuoteId) -> Result<Option<Quote>, Error>;

    /// Get the total number of SCIs in the quote book
    fn num_scis(&self) -> Result<u64, Error>;
}
