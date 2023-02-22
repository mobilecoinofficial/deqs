// Copyright (c) 2023 MobileCoin Inc.

use deqs_quote_book_api::{Error, Pair, Quote, QuoteBook, QuoteId};
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_transaction_extra::SignedContingentInput;
use std::{ops::RangeBounds, sync::Arc};

/// Quote added callback
pub type QuoteAddedCallback = Arc<Box<dyn Fn(&Quote) + 'static + Sync + Send>>;

/// A quote book that calls a callback when quotes are added or removed.
#[derive(Clone)]
pub struct NotifyingQuoteBook<QB: QuoteBook> {
    /// Underlying quote book
    quote_book: QB,

    /// New quote added callback
    quote_added_callback: QuoteAddedCallback,
}

impl<QB: QuoteBook> NotifyingQuoteBook<QB> {
    pub fn new(quote_book: QB, quote_added_callback: QuoteAddedCallback) -> Self {
        Self {
            quote_book,
            quote_added_callback,
        }
    }

    pub fn add_sci(
        &self,
        sci: SignedContingentInput,
        timestamp: Option<u64>,
    ) -> Result<Quote, Error> {
        // Convert SCI into an quote. This also validates it.
        let quote = Quote::new(sci, timestamp)?;
        self.add_quote(&quote)?;
        Ok(quote)
    }

    pub fn add_quote(&self, quote: &Quote) -> Result<(), Error> {
        self.quote_book.add_quote(quote)?;
        (self.quote_added_callback)(quote);
        Ok(())
    }

    pub fn remove_quote_by_id(&self, id: &QuoteId) -> Result<Quote, Error> {
        self.quote_book.remove_quote_by_id(id)
    }

    pub fn remove_quotes_by_key_image(&self, key_image: &KeyImage) -> Result<Vec<Quote>, Error> {
        self.quote_book.remove_quotes_by_key_image(key_image)
    }

    pub fn remove_quotes_by_tombstone_block(
        &self,
        current_block_index: BlockIndex,
    ) -> Result<Vec<Quote>, Error> {
        self.quote_book
            .remove_quotes_by_tombstone_block(current_block_index)
    }

    pub fn get_quotes(
        &self,
        pair: &Pair,
        base_token_quantity: impl RangeBounds<u64>,
        limit: usize,
    ) -> Result<Vec<Quote>, Error> {
        self.quote_book.get_quotes(pair, base_token_quantity, limit)
    }

    pub fn get_quote_ids(&self, pair: Option<&Pair>) -> Result<Vec<QuoteId>, Error> {
        self.quote_book.get_quote_ids(pair)
    }

    pub fn get_quote_by_id(&self, id: &QuoteId) -> Result<Option<Quote>, Error> {
        self.quote_book.get_quote_by_id(id)
    }

    pub fn num_scis(&self) -> Result<u64, Error> {
        self.quote_book.num_scis()
    }
}
