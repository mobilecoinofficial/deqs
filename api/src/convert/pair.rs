// Copyright (c) 2023 MobileCoin Inc.

//! Convert to/from Pair.

use crate::deqs as api;
use deqs_quote_book_api::Pair;
use mc_transaction_types::TokenId;

/// Convert Rust Pair to Protobuf Pair.
impl From<&Pair> for api::Pair {
    fn from(src: &Pair) -> Self {
        let mut pair = api::Pair::new();
        pair.set_base_token_id(*src.base_token_id);
        pair.set_counter_token_id(*src.counter_token_id);
        pair
    }
}

/// Convert api::Pair --> Pair.
impl From<&api::Pair> for Pair {
    fn from(source: &api::Pair) -> Self {
        Self {
            base_token_id: TokenId::from(source.get_base_token_id()),
            counter_token_id: TokenId::from(source.get_counter_token_id()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // Pair --> api::Pair --> Pair
    fn test_pair_identity() {
        let source: Pair = Pair {
            base_token_id: TokenId::from(10203040),
            counter_token_id: TokenId::from(40506070),
        };
        let converted = api::Pair::from(&source);
        let recovered = Pair::from(&converted);

        assert_eq!(source, recovered);
    }
}
