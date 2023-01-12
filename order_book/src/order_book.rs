// Copyright (c) 2023 MobileCoin Inc.

use crate::{Error, Order, OrderId, Pair};
use mc_blockchain_types::BlockIndex;
use mc_crypto_ring_signature::KeyImage;
use mc_transaction_extra::SignedContingentInput;
use std::{
    fmt::{Debug, Display},
    ops::RangeBounds,
};

/// Order book functionality for a single trading pair
pub trait OrderBook {
    /// Error data type
    type Error: Debug + Display + Eq + Into<Error>;

    /// Add an SCI to the order book
    fn add_sci(&self, sci: SignedContingentInput) -> Result<Order, Self::Error>;

    /// Remove a single order from the book, identified by its id.
    /// Returns the removed order if it was found
    fn remove_order_by_id(&self, id: &OrderId) -> Result<Order, Self::Error>;

    /// Remove all orders matching a given key image, returns the list of orders
    /// removed
    fn remove_orders_by_key_image(&self, key_image: &KeyImage) -> Result<Vec<Order>, Self::Error>;

    /// Remove all orders whose tombstone block is >= current block index,
    /// returns the list of orders removed.
    fn remove_orders_by_tombstone_block(
        &self,
        current_block_index: BlockIndex,
    ) -> Result<Vec<Order>, Self::Error>;

    /// Search for orders that can provide `pair.base_token_id in exchange for
    /// being sent `pair.counter_token_id`. Optionally filtering only for
    /// orders that can provide some amount in the range `base_token_quantity`.
    /// Due to partial fill rules, orders returned may not be able to provide
    /// the entire range. It can also optionally limit the number of orders
    /// returned. A limit of 0 returns all orders. Orders are returned sorted by
    /// the more favorable exchange rate (orders giving more base tokens for
    /// a given amount of counter tokens are returned first). For the exact
    /// sorting details, see documentation in the `Ord` implementation of
    /// `Order`.
    fn get_orders(
        &self,
        pair: &Pair,
        base_token_quantity: impl RangeBounds<u64>,
        limit: usize,
    ) -> Result<Vec<Order>, Self::Error>;
}
