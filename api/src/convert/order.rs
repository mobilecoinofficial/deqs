// Copyright (c) 2023 MobileCoin Inc.

//! Convert to/from Order.

use std::ops::RangeInclusive;

use crate::deqs as api;
use deqs_order_book::{Order, OrderId, Pair};
use mc_api::ConversionError;
use mc_transaction_extra::SignedContingentInput;

/// Convert Rust Order to Protobuf Order.
impl From<&Order> for api::Order {
    fn from(src: &Order) -> Self {
        let mut order = api::Order::new();
        order.set_sci(src.sci().into());
        order.set_id(src.id().into());
        order.set_pair(src.pair().into());
        order.set_base_range_min(*src.base_range().start());
        order.set_base_range_max(*src.base_range().end());
        order.set_max_counter_tokens(src.max_counter_tokens());
        order.set_timestamp(src.timestamp());

        order
    }
}

/// Convert api::Order --> Order.
impl TryFrom<&api::Order> for Order {
    type Error = ConversionError;

    fn try_from(source: &api::Order) -> Result<Self, Self::Error> {
        let sci = SignedContingentInput::try_from(source.get_sci())?;
        let id = OrderId::try_from(source.get_id())?;
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
    // Order --> api::Order --> Order
    fn test_order_identity() {
        let mut rng: StdRng = SeedableRng::from_seed([1u8; 32]);
        let sci = create_sci(
            TokenId::from(43432),
            TokenId::from(6886868),
            10,
            30,
            &mut rng,
        );
        let source = Order::try_from(sci).unwrap();

        let converted = api::Order::from(&source);
        let recovererd = Order::try_from(&converted).unwrap();
        assert_eq!(source, recovererd);
    }
}
