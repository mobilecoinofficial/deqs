// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error as QuoteBookError, Pair, Quote, QuoteBook, QuoteId};
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_ledger_db::Ledger;
use mc_transaction_extra::SignedContingentInput;
use std::ops::RangeBounds;

/// A wrapper for a quote book implementation that syncs quotes with the ledger
#[derive(Clone)]
pub struct SynchronizedQuoteBook<Q: QuoteBook, L: Ledger + Clone + 'static> {
    /// List of all SCIs in the quote book, grouped by trading pair.
    /// Naturally sorted by the time they got added to the book.
    quote_book: Q,

    _ledger: L,
}

impl<Q: QuoteBook, L: Ledger + Clone + Sync + 'static> SynchronizedQuoteBook<Q, L> {
    /// Create a new Synchronized Quotebook
    pub fn new(quote_book: Q, _ledger: L) -> Self {
        Self {
            quote_book,
            _ledger,
        }
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
        // Check the ledger to see if the quote is stale before adding it to the
        // quotebook.
        if self._ledger.contains_key_image(&sci.key_image())? {
            return Err(QuoteBookError::QuoteIsStale.into());
        }
        // Try adding to quote book.
        let result = self.quote_book.add_sci(sci, timestamp);
        match result {
            Ok(quote) => {
                return Ok(quote);
            }
            Err(err) => {
                return Err(err);
            }
        }
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
}

#[cfg(test)]
mod tests {
    // Tests for this are under the tests/ directory since we want to be able to
    // re-use some test code between implementations and that seems to be the
    // way to make Rust do that.
}
