// Copyright (c) 2023 MobileCoin Inc.

use crate::{Msg, MsgSource};
use deqs_quote_book_api::{Error, Pair, Quote, QuoteBook, QuoteId};
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_transaction_extra::SignedContingentInput;
use postage::{broadcast::Sender, sink::Sink};
use std::ops::RangeBounds;

/// A quote book that calls a callback when quotes are added or removed.
#[derive(Clone)]
pub struct NotifyingQuoteBook<QB: QuoteBook> {
    /// Underlying quote book
    quote_book: QB,

    /// Message bus sender channel.
    msg_bus_tx: Sender<Msg>,
}

impl<QB: QuoteBook> NotifyingQuoteBook<QB> {
    pub fn new(quote_book: QB, msg_bus_tx: Sender<Msg>) -> Self {
        Self {
            quote_book,
            msg_bus_tx,
        }
    }

    pub fn add_sci(
        &mut self,
        sci: SignedContingentInput,
        timestamp: Option<u64>,
        source: MsgSource,
    ) -> Result<Quote, Error> {
        // Convert SCI into an quote. This also validates it.
        let quote = Quote::new(sci, timestamp)?;
        self.add_quote(quote.clone(), source)?;
        Ok(quote)
    }

    pub fn add_quote(&mut self, quote: Quote, source: MsgSource) -> Result<(), Error> {
        self.quote_book.add_quote(&quote)?;
        self.msg_bus_tx
            .blocking_send(Msg::SciQuoteAdded(quote, source))
            .expect("msg bus send");
        Ok(())
    }

    pub fn remove_quote_by_id(&mut self, id: &QuoteId, source: MsgSource) -> Result<Quote, Error> {
        let quote = self.quote_book.remove_quote_by_id(id)?;
        self.msg_bus_tx
            .blocking_send(Msg::SciQuoteRemoved(quote.clone(), source))
            .expect("msg bus send");
        Ok(quote)
    }

    pub fn remove_quotes_by_key_image(
        &mut self,
        key_image: &KeyImage,
        source: MsgSource,
    ) -> Result<Vec<Quote>, Error> {
        let quotes = self.quote_book.remove_quotes_by_key_image(key_image)?;
        for quote in &quotes {
            self.msg_bus_tx
                .blocking_send(Msg::SciQuoteRemoved(quote.clone(), source))
                .expect("msg bus send");
        }
        Ok(quotes)
    }

    pub fn remove_quotes_by_tombstone_block(
        &mut self,
        current_block_index: BlockIndex,
        source: MsgSource,
    ) -> Result<Vec<Quote>, Error> {
        let quotes = self
            .quote_book
            .remove_quotes_by_tombstone_block(current_block_index)?;
        for quote in &quotes {
            self.msg_bus_tx
                .blocking_send(Msg::SciQuoteRemoved(quote.clone(), source))
                .expect("msg bus send");
        }
        Ok(quotes)
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
