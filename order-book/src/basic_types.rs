// Copyright (c) 2023 MobileCoin Inc.

use mc_crypto_digestible::{Digestible, MerlinTranscript};
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::TokenId;
use std::{array::TryFromSliceError, hash::Hash, ops::Deref};

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
pub struct OrderId(pub [u8; 32]);

impl From<&SignedContingentInput> for OrderId {
    fn from(sci: &SignedContingentInput) -> Self {
        Self(sci.digest32::<MerlinTranscript>(b"deqs-sci-order-id"))
    }
}

impl TryFrom<&[u8]> for OrderId {
    type Error = TryFromSliceError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self(bytes.try_into()?))
    }
}

impl Deref for OrderId {
    type Target = [u8; 32];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}