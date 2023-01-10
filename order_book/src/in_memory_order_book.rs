// Copyright (c) 2023 MobileCoin Inc.

use crate::{Order, OrderBook, OrderId, Pair};
use displaydoc::Display;
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_transaction_core::validation::validate_tombstone;
use mc_transaction_extra::{SignedContingentInput, SignedContingentInputError};
use std::{
    collections::HashMap,
    ops::RangeBounds,
    sync::{Arc, PoisonError, RwLock},
};

// TODO think about partial fills

/// A naive in-memory order book implementation
#[derive(Clone, Debug)]
pub struct InMemoryOrderBook {
    /// List of all SCIs in the order book, grouped by trading pair.
    /// Naturally sorted by the time they got added to the book.
    scis: Arc<RwLock<HashMap<Pair, Vec<Order>>>>,
}

impl InMemoryOrderBook {
    pub fn new() -> Self {
        Self {
            scis: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl OrderBook for InMemoryOrderBook {
    type Error = Error;

    fn add_sci(&self, sci: SignedContingentInput) -> Result<Order, Self::Error> {
        // The SCI must be valid.
        sci.validate()?;

        // TODO - Sanity - we currently expect the SCI to contain only a single required
        // output. Future version might require another output for paying fees to
        // the DEQS
        if sci.required_output_amounts.len() != 1 {
            return Err(Error::IncorrectNumberOfOutputs);
        }

        // Try adding the order.
        let pair = Pair::from(&sci);
        let order = Order::from(sci);

        let mut scis = self.scis.write()?;
        let orders = scis.entry(pair).or_insert_with(Default::default);

        // Make sure order doesn't already exist. For a single pair we disallow
        // duplicate key images since we don't want the same input with
        // different pricing.
        // This also ensures that we do not encounter a duplicate id, since the id is a
        // hash of the entire SCI including its key image.
        if orders
            .iter()
            .any(|entry| entry.sci().key_image() == order.sci().key_image())
        {
            return Err(Error::AlreadyExists);
        }

        // Add order
        orders.push(order.clone());
        Ok(order)
    }

    fn remove_order_by_id(&self, id: &OrderId) -> Result<Order, Self::Error> {
        let mut scis = self.scis.write()?;

        for entries in scis.values_mut() {
            if let Some(index) = entries.iter().position(|entry| entry.id() == id) {
                // We return since we expect the id to be unique amongst all orders across all
                // pairs. This is to be expected because the id is the hash of
                // the entire SCI, and when adding SCIs we ensure uniqueness.
                return Ok(entries.remove(index));
            }
        }

        Err(Error::SciNotFound)
    }

    fn remove_orders_by_key_image(&self, key_image: &KeyImage) -> Result<Vec<Order>, Self::Error> {
        let mut scis = self.scis.write()?;

        let mut all_removed_orders = Vec::new();

        for entries in scis.values_mut() {
            let mut removed_entries = entries
                .drain_filter(|entry| entry.key_image() == *key_image)
                .collect();

            all_removed_orders.append(&mut removed_entries);
        }

        Ok(all_removed_orders)
    }

    fn remove_orders_by_tombstone_block(
        &self,
        current_block_index: BlockIndex,
    ) -> Result<Vec<Order>, Self::Error> {
        let mut scis = self.scis.write()?;

        let mut all_removed_orders = Vec::new();

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

            all_removed_orders.append(&mut removed_entries);
        }

        Ok(all_removed_orders)
    }

    fn get_orders(
        &self,
        pair: &Pair,
        base_token_quantity: impl RangeBounds<u64>,
        counter_token_price_range: impl RangeBounds<u64>,
    ) -> Result<Vec<Order>, Self::Error> {
        let scis = self.scis.read()?;
        let mut results = Vec::new();
        if let Some(orders) = scis.get(&pair) {
            for order in orders.iter() {
                let payout = order.sci().pseudo_output_amount.value;
                let cost = order.sci().required_output_amounts[0].value; // TODO assumption about number of outputs

                if base_token_quantity.contains(&payout)
                    && counter_token_price_range.contains(&cost)
                {
                    results.push(order.clone());
                }
            }
        }
        Ok(results)
    }
}

/// Error data type
#[derive(Debug, Display, Eq, PartialEq)]
pub enum Error {
    /// SCI: {0}
    Sci(SignedContingentInputError),

    /// Number of outputs is incorrect
    IncorrectNumberOfOutputs,

    /// Lock poisoned
    LockPoisoned,

    /// SCI already exists in book
    AlreadyExists,

    /// SCI not found
    SciNotFound,
}

impl From<SignedContingentInputError> for Error {
    fn from(src: SignedContingentInputError) -> Self {
        Self::Sci(src)
    }
}

impl<T> From<PoisonError<T>> for Error {
    fn from(_src: PoisonError<T>) -> Self {
        Error::LockPoisoned
    }
}

#[cfg(test)]
mod tests {
    // Tests for this are under the tests/ library since we want to be able to
    // re-use some test code between implementations
}
