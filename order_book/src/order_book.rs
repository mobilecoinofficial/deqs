// Copyright (c) 2023 MobileCoin Inc.

use mc_crypto_digestible::{Digestible, MerlinTranscript};
use mc_crypto_ring_signature::KeyImage;
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::{TokenId};
use mc_blockchain_types::BlockIndex;
use std::{
    fmt::{Debug, Display},
    hash::Hash,
    ops::{Deref, RangeBounds},
};

/// A single trading pair
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Pair {
    /// The token id being offered "for sale".
    pub base_token_id: TokenId,

    /// The token id that needs to be paid to satisfy the offering.
    /// (The SCI is "priced" with this token id)
    pub counter_token_id: TokenId,
}

impl From<&SignedContingentInput> for Pair {
    fn from(sci: &SignedContingentInput) -> Self {
        // TODO: This assumes the first output is the one that specifies what we need to
        // pay the offerer
        Self {
            base_token_id: TokenId::from(sci.pseudo_output_amount.token_id),
            counter_token_id: TokenId::from(sci.required_output_amounts[0].token_id),
        }
    }
}

/// A unique identifier for a single order
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderId([u8; 32]);

impl From<&SignedContingentInput> for OrderId {
    fn from(sci: &SignedContingentInput) -> Self {
        Self(sci.digest32::<MerlinTranscript>(b"deqs-sci-order-id"))
    }
}

/// A single "order" in the book. This is a wrapper around an SCI and some
/// auxiliary data
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Order {
    /// SCI
    sci: SignedContingentInput,

    /// Unique identifier
    id: OrderId,
}

impl Order {
    pub fn sci(&self) -> &SignedContingentInput {
        &self.sci
    }

    pub fn id(&self) -> &OrderId {
        &self.id
    }
}

impl From<SignedContingentInput> for Order {
    fn from(sci: SignedContingentInput) -> Self {
        Self {
            id: OrderId::from(&sci),
            sci,
        }
    }
}

impl Deref for Order {
    type Target = SignedContingentInput;

    fn deref(&self) -> &Self::Target {
        &self.sci
    }
}

/// Order book functionality for a single trading pair
pub trait OrderBook {
    /// Error data type
    type Error: Debug + Display + Eq;

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

    /// Search for orders that will pay out `pair.base_token_id` in the range of
    /// `base_token_quantity` tokens, in exchange for being sent
    /// `pair.counter_token_id` at a price range of `counter_token_price_range`
    /// tokens.
    fn get_orders(
        &self,
        pair: &Pair,
        base_token_quantity: impl RangeBounds<u64>,
        counter_token_price_range: impl RangeBounds<u64>,
    ) -> Result<Vec<Order>, Self::Error>;
}
