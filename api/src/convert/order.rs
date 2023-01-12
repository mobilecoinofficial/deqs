// Copyright (c) 2023 MobileCoin Inc.

//! Convert to/from Order.

use crate::deqs as api;
use deqs_order_book::Order;
//use mc_api::ConversionError;

const NANOS_PER_SEC: u32 = 1_000_000_000;


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

        let seconds = src.timestamp().checked_div(NANOS_PER_SEC).unwrap();

        order
    }
}

// /// Convert api::Order --> Order.
// impl TryFrom<&api::Order> for Order {
//     type Error = ConversionError;

//     fn try_from(source: &api::Order) -> Result<Self, Self::Error> {
//         let bytes: &[u8] = source.get_data();
//         Ok(Order::try_from(bytes)?)
//     }
// }

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     // Order --> api::Order
//     fn test_order_from() {
//         let source: Order = Order([4; 32]);
//         let converted = api::Order::from(&source);
//         assert_eq!(converted.data, source.to_vec());
//     }

//     #[test]
//     // api::Order --> Order
//     fn test_order_try_from() {
//         let mut source = api::Order::new();
//         source.set_data(Order([5; 32]).to_vec());

//         // try_from should succeed.
//         let order = Order::try_from(&source).unwrap();

//         // order should have the correct value.
//         assert_eq!(order, Order([5; 32]));
//     }

//     #[test]
//     // `Order::try_from` should return ConversionError if the source contains
// the     // wrong number of bytes.
//     fn test_order_try_from_conversion_errors() {
//         // Helper function asserts that a ConversionError::ArrayCastError is
// produced.         fn expects_array_cast_error(bytes: &[u8]) {
//             let mut source = api::Order::new();
//             source.set_data(bytes.to_vec());
//             match Order::try_from(&source).unwrap_err() {
//                 ConversionError::ArrayCastError => {} // Expected outcome.
//                 _ => panic!(),
//             }
//         }

//         // Too many bytes should produce an ArrayCastError.
//         expects_array_cast_error(&[11u8; 119]);

//         // Too few bytes should produce an ArrayCastError.
//         expects_array_cast_error(&[11u8; 3]);
//     }
// }
