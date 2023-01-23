// Copyright (c) 2023 MobileCoin Inc.

//! Convert to/from Quote.

use std::ops::RangeInclusive;

use crate::deqs as api;
use deqs_quote_book::{Pair, Quote, QuoteId};
use mc_api::ConversionError;
use mc_transaction_extra::SignedContingentInput;

/// Convert Rust Quote to Protobuf Quote.
impl From<&Quote> for api::Quote {
    fn from(src: &Quote) -> Self {
        let mut quote = api::Quote::new();
        quote.set_sci(src.sci().into());
        quote.set_id(src.id().into());
        quote.set_pair(src.pair().into());
        quote.set_base_range_min(*src.base_range().start());
        quote.set_base_range_max(*src.base_range().end());
        quote.set_max_counter_tokens(src.max_counter_tokens());
        quote.set_timestamp(src.timestamp());

        quote
    }
}

/// Convert api::Quote --> Quote.
impl TryFrom<&api::Quote> for Quote {
    type Error = ConversionError;

    fn try_from(source: &api::Quote) -> Result<Self, Self::Error> {
        let sci = SignedContingentInput::try_from(source.get_sci())?;
        let id = QuoteId::try_from(source.get_id())?;
        let pair = Pair::try_from(source.get_pair())?;
        let base_range =
            RangeInclusive::new(source.get_base_range_min(), source.get_base_range_max());
        let max_counter_tokens = source.get_max_counter_tokens();
        let timestamp = source.get_timestamp();

        Ok(Self::new_from_fields(
            sci,
            id,
            pair,
            base_range,
            max_counter_tokens,
            timestamp,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deqs_mc_test_utils::create_sci;
    use mc_transaction_types::TokenId;
    use rand::{rngs::StdRng, SeedableRng};

    #[test]
    // Quote --> api::Quote --> Quote
    fn test_quote_identity() {
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let sci = create_sci(
            TokenId::from(43432),
            TokenId::from(6886868),
            10,
            30,
            &mut rng,
        );
        let source = Quote::try_from(sci).unwrap();

        let converted = api::Quote::from(&source);
        let recovererd = Quote::try_from(&converted).unwrap();
        assert_eq!(source, recovererd);
    }
}
