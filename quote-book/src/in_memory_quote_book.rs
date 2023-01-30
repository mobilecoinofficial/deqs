// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error as QuoteBookError, Pair, Quote, QuoteBook, QuoteId};
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_ledger_db::LedgerDB;
use mc_transaction_core::validation::validate_tombstone;
use mc_transaction_extra::SignedContingentInput;
use std::{
    collections::{BTreeSet, HashMap},
    ops::{Bound, RangeBounds},
    sync::{Arc, RwLock},
};

/// A naive in-memory quote book implementation
#[derive(Clone)]
pub struct InMemoryQuoteBook {
    /// List of all SCIs in the quote book, grouped by trading pair.
    /// Naturally sorted by the time they got added to the book.
    scis: Arc<RwLock<HashMap<Pair, BTreeSet<Quote>>>>,

    _ledger_db: LedgerDB,
}

impl InMemoryQuoteBook {
    /// Create a new InMemoryQuoteBook
    pub fn new(_ledger_db: LedgerDB) -> Self {
        Self {
            scis: Arc::new(RwLock::new(HashMap::new())),
            _ledger_db,
        }
    }
}

impl QuoteBook for InMemoryQuoteBook {
    fn add_sci(
        &self,
        sci: SignedContingentInput,
        timestamp: Option<u64>,
    ) -> Result<Quote, QuoteBookError> {
        // Convert SCI into an quote. This also validates it.
        let quote = Quote::new(sci, timestamp)?;

        // Try adding to quote book.
        let mut scis = self.scis.write()?;
        let quotes = scis.entry(*quote.pair()).or_insert_with(Default::default);

        // Make sure quote doesn't already exist. For a single pair we disallow
        // duplicate key images since we don't want the same input with
        // different pricing.
        // This also ensures that we do not encounter a duplicate id, since the id is a
        // hash of the entire SCI including its key image.
        if quotes
            .iter()
            .any(|entry| entry.sci().key_image() == quote.sci().key_image())
        {
            return Err(QuoteBookError::QuoteAlreadyExists);
        }

        // Add quote. We assert it doesn't fail since we do not expect duplicate quotes
        // due to the key image check above.
        assert!(quotes.insert(quote.clone()));
        Ok(quote)
    }

    fn remove_quote_by_id(&self, id: &QuoteId) -> Result<Quote, QuoteBookError> {
        let mut scis = self.scis.write()?;

        for entries in scis.values_mut() {
            if let Some(element) = entries.iter().find(|entry| entry.id() == id).cloned() {
                // We return since we expect the id to be unique amongst all quotes across all
                // pairs. This is to be expected because the id is the hash of
                // the entire SCI, and when adding SCIs we ensure uniqueness.
                assert!(entries.remove(&element));
                return Ok(element);
            }
        }

        Err(QuoteBookError::QuoteNotFound)
    }

    fn remove_quotes_by_key_image(
        &self,
        key_image: &KeyImage,
    ) -> Result<Vec<Quote>, QuoteBookError> {
        let mut scis = self.scis.write()?;

        let mut all_removed_quotes = Vec::new();

        for entries in scis.values_mut() {
            let mut removed_entries = entries
                .drain_filter(|entry| entry.key_image() == *key_image)
                .collect();

            all_removed_quotes.append(&mut removed_entries);
        }

        Ok(all_removed_quotes)
    }

    fn remove_quotes_by_tombstone_block(
        &self,
        current_block_index: BlockIndex,
    ) -> Result<Vec<Quote>, QuoteBookError> {
        let mut scis = self.scis.write()?;

        let mut all_removed_quotes = Vec::new();

        for entries in scis.values_mut() {
            let mut removed_entries = entries
                .drain_filter(|entry| {
                    if let Some(input_rules) = &entry.sci().tx_in.input_rules {
                        validate_tombstone(current_block_index, input_rules.max_tombstone_block)
                            .is_err()
                    } else {
                        false
                    }
                })
                .collect();

            all_removed_quotes.append(&mut removed_entries);
        }

        Ok(all_removed_quotes)
    }

    fn get_quotes(
        &self,
        pair: &Pair,
        base_token_quantity: impl RangeBounds<u64>,
        limit: usize,
    ) -> Result<Vec<Quote>, QuoteBookError> {
        let scis = self.scis.read()?;
        let mut results = Vec::new();

        if let Some(quotes) = scis.get(pair) {
            // NOTE: This implementation relies on quotes being sorted due to being held in
            // a BTreeSet
            for quote in quotes.iter() {
                if range_overlaps(&base_token_quantity, quote.base_range()) {
                    results.push(quote.clone());
                    if results.len() == limit {
                        break;
                    }
                }
            }
        }

        Ok(results)
    }
}

fn range_overlaps(x: &impl RangeBounds<u64>, y: &impl RangeBounds<u64>) -> bool {
    let x1 = match x.start_bound() {
        Bound::Included(start) => *start,
        Bound::Excluded(start) => start.saturating_add(1),
        Bound::Unbounded => 0,
    };

    let x2 = match x.end_bound() {
        Bound::Included(end) => *end,
        Bound::Excluded(end) => end.saturating_sub(1),
        Bound::Unbounded => u64::MAX,
    };

    let y1 = match y.start_bound() {
        Bound::Included(start) => *start,
        Bound::Excluded(start) => start.saturating_add(1),
        Bound::Unbounded => 0,
    };

    let y2 = match y.end_bound() {
        Bound::Included(end) => *end,
        Bound::Excluded(end) => end.saturating_sub(1),
        Bound::Unbounded => u64::MAX,
    };

    x1 <= y2 && y1 <= x2
}

#[cfg(test)]
mod tests {
    // Tests for this are under the tests/ directory since we want to be able to
    // re-use some test code between implementations and that seems to be the
    // way to make Rust do that.
}
