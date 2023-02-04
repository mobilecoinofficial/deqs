// Copyright (c) 2023 MobileCoin Inc.

use mc_crypto_digestible::{Digestible, MerlinTranscript};
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::TokenId;
use serde::{Deserialize, Serialize};
use std::{array::TryFromSliceError, fmt, hash::Hash, ops::Deref};

/// A single trading pair
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Pair {
    /// The token id being offered "for sale".
    pub base_token_id: TokenId,

    /// The token id that needs to be paid to satisfy the offering.
    /// (The SCI is "priced" with this token id)
    pub counter_token_id: TokenId,
}

/// A unique identifier for a single quote
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct QuoteId(pub [u8; 32]);

impl From<&SignedContingentInput> for QuoteId {
    fn from(sci: &SignedContingentInput) -> Self {
        Self(sci.digest32::<MerlinTranscript>(b"deqs-sci-quote-id"))
    }
}

impl TryFrom<&[u8]> for QuoteId {
    type Error = TryFromSliceError;

    fn try_from(bytes: &[u8]) -> Result<Self, Self::Error> {
        Ok(Self(bytes.try_into()?))
    }
}

impl Deref for QuoteId {
    type Target = [u8; 32];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Display for QuoteId {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", hex::encode(&self.0))
    }
}
