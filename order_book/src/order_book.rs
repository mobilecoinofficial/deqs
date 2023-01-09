// Copyright (c) 2023 MobileCoin Inc.

use mc_crypto_ring_signature::KeyImage;
use mc_transaction_extra::SignedContingentInput;
use mc_transaction_types::TokenId;
use std::{
    collections::HashMap,
    fmt::{Debug, Display},
};

/// Order book functionality for a single trading pair
pub trait OrderBook {
    /// Error data type
    type Error: Debug + Display;

    /// Unique identifier for a given SCI
    type SciId;

    /// Add an SCI to the order book
    fn add_sci(&self, sci: SignedContingentInput) -> Result<(), Self::Error>;

    /// Remove a single SCI from the order book
    fn remove_single_sci(&self, sci_id: &Self::SciId) -> Result<(), Self::Error>;

    /// Remove all SCIs matching a given key image
    fn remove_scis(&self, key_image: &KeyImage) -> Result<(), Self::Error>;

    /// Search for SCIs, optionally filtering by SCIs that pay out more than a
    /// given threshold or cost up to a given threshold. min_payout is
    /// priced in offered_token_id, max_cost is priced in priced_token_id.
    fn get_scis(
        &self,
        offered_token_id: TokenId,
        priced_token_id: TokenId,
        min_payout: Option<u64>,
        max_cost: Option<u64>,
    ) -> Result<HashMap<Self::SciId, SignedContingentInput>, Self::Error>;
}
