// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error as OrderBookError, Order, OrderBook, OrderId, Pair};
use displaydoc::Display;
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_transaction_core::validation::validate_tombstone;
use mc_transaction_extra::SignedContingentInput;
use std::{
    collections::HashMap,
    ops::{RangeBounds},
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
        // Convert SCI into an order. This also validates it.
        let order = Order::try_from(sci)?;

        // Try adding to order book.
        let mut scis = self.scis.write()?;
        let orders = scis.entry(*order.pair()).or_insert_with(Default::default);

        // Make sure order doesn't already exist. For a single pair we disallow
        // duplicate key images since we don't want the same input with
        // different pricing.
        // This also ensures that we do not encounter a duplicate id, since the id is a
        // hash of the entire SCI including its key image.
        if orders
            .iter()
            .any(|entry| entry.sci().key_image() == order.sci().key_image())
        {
            return Err(OrderBookError::OrderAlreadyExists.into());
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

        Err(OrderBookError::OrderNotFound.into())
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
        base_token_quantity: u64,
        counter_token_quantity: impl RangeBounds<u64>,
    ) -> Result<Vec<Order>, Self::Error> {
        let scis = self.scis.read()?;
        let mut results = Vec::new();
        if let Some(orders) = scis.get(&pair) {
            for order in orders.iter() {
                // Skip orders that are not able to pay base_token_quantity.
                if !order.base_range().contains(&base_token_quantity) {
                    continue;
                }

                // Skip orders that require the fulfiller to pay an amount that is outside of
                // the range `counter_token_quantity`. For that we need to
                // calculate how many tokens the fulfiller would have to pay as change when
                // taking `base_token_quantity` tokens for themselves.
                if let Ok(cost) = order.counter_tokens_cost(base_token_quantity) {
                    if counter_token_quantity.contains(&cost) {
                        results.push(order.clone());
                    }
                }
            }
        }
        Ok(results)
    }
}

// fn range_overlaps(x: &impl RangeBounds<u64>, y: &impl RangeBounds<u64>) -> bool {
//     let x1 = match x.start_bound() {
//         Bound::Included(start) => *start,
//         Bound::Excluded(start) => start.saturating_add(1),
//         Bound::Unbounded => 0,
//     };

//     let x2 = match x.end_bound() {
//         Bound::Included(end) => *end,
//         Bound::Excluded(end) => end.saturating_sub(1),
//         Bound::Unbounded => u64::MAX,
//     };

//     let y1 = match y.start_bound() {
//         Bound::Included(start) => *start,
//         Bound::Excluded(start) => start.saturating_add(1),
//         Bound::Unbounded => 0,
//     };

//     let y2 = match y.end_bound() {
//         Bound::Included(end) => *end,
//         Bound::Excluded(end) => end.saturating_sub(1),
//         Bound::Unbounded => u64::MAX,
//     };

//     x1 <= y2 && y1 <= x2
// }

/// Error data type
#[derive(Debug, Display, Eq, PartialEq)]
pub enum Error {
    /// Order book: {0}
    OrderBook(OrderBookError),

    /// Lock poisoned
    LockPoisoned,
}

impl Into<OrderBookError> for Error {
    fn into(self) -> OrderBookError {
        match self {
            Self::OrderBook(err) => err,
            err => OrderBookError::ImplementationSpecific(err.to_string()),
        }
    }
}

impl From<OrderBookError> for Error {
    fn from(err: OrderBookError) -> Self {
        Self::OrderBook(err)
    }
}

impl<T> From<PoisonError<T>> for Error {
    fn from(_src: PoisonError<T>) -> Self {
        Error::LockPoisoned
    }
}

#[cfg(test)]
mod tests {
    // Tests for this are under the tests/ directory since we want to be able to
    // re-use some test code between implementations and that seems to be the way to make Rust do that.
}
