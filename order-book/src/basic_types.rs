// Copyright (c) 2023 MobileCoin Inc.

use mc_crypto_digestible::{Digestible, MerlinTranscript};
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::TokenId;
use std::hash::Hash;

/// A single trading pair
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Pair {
    /// The token id being offered "for sale".
    pub base_token_id: TokenId,

    /// The token id that needs to be paid to satisfy the offering.
    /// (The SCI is "priced" with this token id)
    pub counter_token_id: TokenId,
}

/// A unique identifier for a single order
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct OrderId([u8; 32]);

impl From<&SignedContingentInput> for OrderId {
    fn from(sci: &SignedContingentInput) -> Self {
        Self(sci.digest32::<MerlinTranscript>(b"deqs-sci-order-id"))
    }
}
